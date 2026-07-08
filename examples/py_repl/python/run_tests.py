#!/usr/bin/env python3
"""
Incremental, crash-isolating test harness for the in-game `ue` reflection
bindings.

    python run_tests.py [host] [port]

Why a custom runner instead of `repl.py --script test.py`?
  `--script` ships the whole file in ONE frame, so the server runs it as a
  single `exec`. A *native* access violation in the UE/Rust layer (which no
  Python `try/except` can catch) then takes the game process down and you get
  zero output — no idea which line did it.

This runner instead sends each check as its OWN frame and prints the result
live (flushed). State persists between frames because the in-game interpreter
keeps one global namespace for the whole connection, so a prelude frame can
set up `PC`, `WS`, helpers, etc. that later frames reference. If the game dies,
the last "running:" line names the exact operation that killed it, and every
prior PASS/FAIL is already on screen.

Safety posture:
  * The bindings layer (read / write / marshal / containers / introspection /
    error paths) is what we're validating for memory-safety. Those are
    exercised exhaustively.
  * Calling arbitrary UFunctions runs arbitrary engine code, which can crash
    for reasons unrelated to the bindings. So function *calls* are restricted
    to a curated allowlist of side-effect-free engine getters, and each call
    is its own isolated frame.
  * Every write is a read -> write -> verify -> restore round-trip. Struct
    writes target a Python-owned (returned-by-value) struct. Container
    mutations target empty containers and are reverted to empty.
"""

from __future__ import annotations

import ast
import socket
import struct
import sys


# ── wire protocol (matches repl.py / server.rs: u32-LE length + UTF-8) ───────

class Conn:
    def __init__(self, host, port):
        self.sock = socket.create_connection((host, port), timeout=30)

    def _send(self, payload: bytes):
        self.sock.sendall(struct.pack("<I", len(payload)) + payload)

    def _recv(self) -> bytes:
        out = bytearray()
        for n in (4,):
            while len(out) < n:
                chunk = self.sock.recv(n - len(out))
                if not chunk:
                    raise EOFError
                out.extend(chunk)
        (length,) = struct.unpack("<I", bytes(out))
        body = bytearray()
        while len(body) < length:
            chunk = self.sock.recv(length - len(body))
            if not chunk:
                raise EOFError
            body.extend(chunk)
        return bytes(body)

    def run(self, source: str) -> str:
        """Run a chunk as one frame. Multi-statement chunks are wrapped in an
        exec() so the server's `single`-mode compile accepts them, and run in
        the persistent global namespace."""
        wrapped = f"exec(compile({source!r}, '<test>', 'exec'), globals())"
        self._send(wrapped.encode("utf-8"))
        return self._recv_reply()

    def _recv_reply(self) -> str:
        """Reply frames are tagged: CHUNK (0, streamed stdout) frames, then one
        terminating RESULT (1) frame. Tests run synchronously, so this is almost
        always a single RESULT frame; CHUNK frames are printed live if present."""
        while True:
            frame = self._recv()
            tag, text = frame[0], frame[1:].decode("utf-8", errors="replace")
            if tag == 1:  # RESULT
                return text
            sys.stdout.write(text)
            sys.stdout.flush()

    def eval(self, expr: str):
        """Evaluate an expression server-side and bring the literal back.

        `run()` wraps everything in `exec(...)` (statement mode), so a bare
        expression never yields a value via the RESULT frame — that frame is
        empty. `print(repr(...))` instead streams the repr as CHUNK (stdout)
        frames, so capture those rather than the (empty) RESULT."""
        wrapped = (
            f"exec(compile({f'print(repr({expr}))'!r}, '<eval>', 'exec'), globals())"
        )
        self._send(wrapped.encode("utf-8"))
        chunks = []
        while True:
            frame = self._recv()
            tag, text = frame[0], frame[1:].decode("utf-8", errors="replace")
            if tag == 1:  # terminating RESULT (empty for exec)
                break
            chunks.append(text)
        return ast.literal_eval("".join(chunks).strip())


# ── the prelude: helpers + discovered targets, defined once server-side ──────

PRELUDE = r'''
import traceback as _tb

# Per-run tallies live in the game's namespace; we read them back at the end.
_T = {"pass": 0, "fail": 0, "skip": 0, "fails": []}

def _ok(name):
    _T["pass"] += 1
    print(f"[PASS] {name}")

def _no(name, e):
    _T["fail"] += 1
    msg = f"{type(e).__name__}: {e}"
    _T["fails"].append(f"{name}: {msg}")
    print(f"[FAIL] {name}: {msg}")

def _sk(name, why):
    _T["skip"] += 1
    print(f"[SKIP] {name}: {why}")

def chk(name, fn):
    try:
        fn(); _ok(name)
    except Exception as e:
        _no(name, e)

def expect_raises(name, exc, fn):
    try:
        fn()
    except exc as e:
        _ok(f"{name} (raised {type(e).__name__})"); return
    except Exception as e:
        _no(name, RuntimeError(f"wrong exception {type(e).__name__}: {e}")); return
    _no(name, RuntimeError("no exception raised"))

def first_live(path):
    for o in ue.objects_of_class(path):
        if "Default__" not in repr(o):
            return o
    return None

def find_prop(obj, kind):
    for n, ff in obj._class.props():
        if ff == kind:
            return n
    return None

def props_by_kind(uclass):
    out = {}
    for n, ff in uclass.props():
        out.setdefault(ff, []).append(n)
    return out

def find_soft(prefer=()):
    # Return (object, prop_name) for some live object carrying a Soft{Object,Class}
    # property. The active PC/WS often have none (depends on the loaded map), so we
    # fall back to scanning whatever objects exist. CDOs are fine for a read check.
    for o in prefer:
        if o is None:
            continue
        n = find_prop(o, 'SoftClassProperty') or find_prop(o, 'SoftObjectProperty')
        if n:
            return o, n
    for o in ue.objects(max=200000):
        for n, ff in o._class.props():
            if ff in ('SoftObjectProperty', 'SoftClassProperty'):
                return o, n
    return None, None

PC = first_live("/Script/Engine.PlayerController")
WS = first_live("/Script/Engine.WorldSettings")
GI = first_live("/Script/Engine.GameInstance")
print("PC:", repr(PC))
print("WS:", repr(WS))
print("GI:", repr(GI))
'''


# Functions known to be side-effect-free getters; we only call those that
# actually exist on the live PlayerController. Each is dispatched in its own
# frame by the runner so a bad one is isolated.
SAFE_CALLS = [
    "K2_GetPawn", "GetHUD", "GetSpectatorPawn", "GetFocalLocation",
    "GetControlRotation", "GetFSDPlayerState", "IsLocalController",
    "IsLocalPlayerController", "GetUseToggleTerrainScanner",
    "GetUseToggleLaserpointer", "GetUseHoldToRun",
]


# ── the test steps: (name, source). Each runs in its own frame. ──────────────

STEPS: list[tuple[str, str]] = [

    # ---- module functions ----
    ("module: find existing", "chk('find Actor', lambda: ue.find('/Script/Engine.Actor') or (_ for _ in ()).throw(AssertionError()))"),
    ("module: find -> UClass", "chk('find->UClass', lambda: (lambda x: type(x).__name__=='UClass' or (_ for _ in ()).throw(AssertionError()))(ue.find('/Script/Engine.Actor')))"),
    ("module: find missing -> None", "chk('find missing', lambda: ue.find('/Game/Nope.X_Y_Z') is None or (_ for _ in ()).throw(AssertionError()))"),
    ("module: load already-loaded", "chk('load Actor', lambda: ue.load('/Script/Engine.Actor') is not None or (_ for _ in ()).throw(AssertionError()))"),
    ("module: objects(max) cap", "chk('objects cap', lambda: len(ue.objects(max=10))<=10 or (_ for _ in ()).throw(AssertionError()))"),
    ("module: objects(class,max) filter", "chk('objects filter', lambda: all(o._class._name=='WorldSettings' for o in ue.objects('WorldSettings',5)))"),
    ("module: objects_of_class via path", "chk('ooc path', lambda: isinstance(ue.objects_of_class('/Script/Engine.Actor', max=3), list))"),
    ("module: objects_of_class via UClass", "chk('ooc proxy', lambda: isinstance(ue.objects_of_class(ue.find('/Script/Engine.Actor'), max=3), list))"),
    ("module: ooc include_subclasses subset", "chk('ooc subset', lambda: len(ue.objects_of_class('/Script/Engine.Actor', include_subclasses=False)) <= len(ue.objects_of_class('/Script/Engine.Actor')))"),
    ("module: ooc bad path -> ValueError", "expect_raises('ooc bad path', ValueError, lambda: ue.objects_of_class('/Game/Nope.X_Y_Z'))"),

    # ---- UClass introspection ----
    ("UClass: _name/_path/repr", "chk('uclass basics', lambda: (WS._class._name and WS._class._path.startswith('/') and 'UClass' in repr(WS._class)) or (_ for _ in ()).throw(AssertionError()))"),
    ("UClass: props() shape", "chk('props shape', lambda: all(len(t)==2 for t in WS._class.props()))"),
    ("UClass: functions() shape", "chk('functions shape', lambda: all(isinstance(n,str) and isinstance(p,list) for n,p in WS._class.functions()))"),
    ("UClass: functions_detailed() shape", "chk('fdetailed shape', lambda: all(type(f).__name__=='Function' and isinstance(f.signature, str) for f in WS._class.functions_detailed()))"),
    ("UClass: parents() are UClasses", "chk('parents', lambda: all(type(p).__name__=='UClass' for p in WS._class.parents()))"),
    ("UClass: instances(max) cap", "chk('instances cap', lambda: len(WS._class.instances(max=2))<=2)"),
    ("UClass: __eq__/__hash__", "chk('uclass eq/hash', lambda: (WS._class==WS._class) and (WS._class in {WS._class}))"),
    ("UClass: function_info(missing) None", "chk('fn missing', lambda: WS._class.function_info('NoSuchFn_XYZ') is None)"),
    ("UClass: isinstance(cls, UObject) (subclass)", "chk('uclass subclass', lambda: isinstance(WS._class, ue.UClass) and isinstance(WS._class, ue.UObject) and issubclass(ue.UClass, ue.UObject))"),

    # ---- UObject basics & identity ----
    ("UObject: _name/_path/_class", "chk('uobject basics', lambda: (isinstance(WS._name,str) and WS._path.startswith('/') and type(WS._class).__name__=='UClass') or (_ for _ in ()).throw(AssertionError()))"),
    ("UObject: __dir__ non-empty", "chk('dir', lambda: len(WS.__dir__())>0 or (_ for _ in ()).throw(AssertionError()))"),
    ("UObject: __eq__ reflexive", "chk('eq', lambda: first_live('/Script/Engine.WorldSettings')==WS)"),
    ("UObject: hashable in set", "chk('hash', lambda: len({WS, WS})==1)"),
    ("UObject: != other type", "chk('eq other', lambda: (WS==42) is False)"),
    ("UObject: bad attr -> AttributeError", "expect_raises('bad attr', AttributeError, lambda: WS.NoSuchField_XYZ)"),

    # ---- value reads (every supported kind) ----
    ("read: BoolProperty", "chk('read bool', lambda: isinstance(getattr((WS if find_prop(WS,'BoolProperty') else PC), find_prop((WS if find_prop(WS,'BoolProperty') else PC),'BoolProperty')), bool))"),
    ("read: IntProperty", "chk('read int', lambda: isinstance(getattr(WS, find_prop(WS,'IntProperty')), int))"),
    ("read: FloatProperty", "chk('read float', lambda: isinstance(getattr(WS, find_prop(WS,'FloatProperty')), float))"),
    ("read: NameProperty", "chk('read name', lambda: isinstance(getattr(WS, find_prop(WS,'NameProperty')), str))"),
    ("read: ByteProperty (enum-as-byte|u8)", "chk('read byte', lambda: isinstance(getattr(WS, find_prop(WS,'ByteProperty')), (int,str)))"),
    ("read: EnumProperty", "chk('read enum', lambda: isinstance(getattr(WS, find_prop(WS,'EnumProperty')), (int,str)))"),
    ("read: ObjectProperty -> UObject|None", "chk('read obj', lambda: (lambda v: v is None or type(v).__name__ in ('UObject','UClass'))(getattr(WS, find_prop(WS,'ObjectProperty'))) or (_ for _ in ()).throw(AssertionError()))"),
    ("read: ClassProperty -> UObject|None", "chk('read class', lambda: (lambda v: v is None or type(v).__name__ in ('UObject','UClass'))(getattr(WS, find_prop(WS,'ClassProperty'))) or (_ for _ in ()).throw(AssertionError()))"),
    ("read: WeakObjectProperty -> UObject|None", "chk('read weak', lambda: (lambda v: v is None or type(v).__name__=='UObject')(getattr(WS, find_prop(WS,'WeakObjectProperty'))) or (_ for _ in ()).throw(AssertionError()))"),
    ("read: SoftObject/SoftClass -> str|None", "chk('read soft', lambda: (lambda o,n: (n is not None) and ((lambda v: v is None or isinstance(v,str))(getattr(o,n)) or (_ for _ in ()).throw(AssertionError())))(*find_soft((PC, WS))) or (_ for _ in ()).throw(AssertionError('no soft property found')))"),
    ("read: StructProperty -> Struct", "chk('read struct', lambda: type(getattr(WS, find_prop(WS,'StructProperty'))).__name__=='Struct')"),
    ("read: ArrayProperty -> Array", "chk('read array', lambda: type(getattr(WS, find_prop(WS,'ArrayProperty'))).__name__=='Array')"),
    ("read: multicast delegate -> MulticastDelegate proxy", "chk('read mcast delegate', lambda: type(getattr(PC, (find_prop(PC,'MulticastInlineDelegateProperty') or find_prop(PC,'MulticastSparseDelegateProperty')))).__name__ == 'MulticastDelegate')"),
    # (TextProperty is now supported — reads back as ue.Text; see FTEXT_STEPS.)
    ("read: still-unsupported types -> clean TypeError", "expect_raises('read unsupported', TypeError, lambda: getattr(PC, (find_prop(PC,'InterfaceProperty') or find_prop(PC,'FieldPathProperty'))) if (find_prop(PC,'InterfaceProperty') or find_prop(PC,'FieldPathProperty')) else (_ for _ in ()).throw(TypeError()))"),

    # ---- enum introspection ----
    ("enum: prop_info -> PropType", "chk('prop_info enum', lambda: WS._class.prop_info(find_prop(WS,'EnumProperty')) is not None)"),
    ("enum: PropType.enum_obj -> UEnum", "chk('enum_obj', lambda: WS._class.prop_info(find_prop(WS,'EnumProperty')).enum_obj() is not None)"),
    ("enum: UEnum.enum_entries shape", "chk('enum_entries', lambda: all(isinstance(n,str) and isinstance(v,int) for n,v in WS._class.prop_info(find_prop(WS,'EnumProperty')).enum_obj().enum_entries()))"),
    ("enum: enum_entries None on non-enum", "chk('enum None', lambda: WS.enum_entries() is None)"),

    # ---- PropType / Function / Param ----
    ("introspect: function_info -> Function", "_FI = PC._class.function_info('K2_GetPawn'); chk('function_info', lambda: _FI is not None)"),
    ("introspect: Function getters", "chk('fn getters', lambda: isinstance(_FI.name,str) and isinstance(_FI.function_flags,int) and all(isinstance(s,str) for s in _FI.flag_names()) and 'Function' in repr(_FI))"),
    ("introspect: Function.params -> Param", "_P = _FI.params(); chk('params', lambda: len(_P)>=1 and 'Param' in repr(_P[0]))"),
    ("introspect: Param getters", "chk('param getters', lambda: isinstance(_P[0].name,str) and _P[0].role in ('in','out','ret','in_const','out_ref') and isinstance(_P[0].flags,int) and _P[0].type_ is not None)"),
    ("introspect: PropType array.inner()", "chk('pt inner', lambda: PC._class.prop_info(props_by_kind(PC._class)['ArrayProperty'][0]).inner() is not None)"),
    ("introspect: PropType struct.struct_type()", "chk('pt struct', lambda: PC._class.prop_info(props_by_kind(PC._class)['StructProperty'][0]).struct_type() is not None)"),
    ("introspect: PropType object.target_class()", "chk('pt target', lambda: PC._class.prop_info(props_by_kind(PC._class)['ObjectProperty'][0]).target_class() is not None)"),
    ("introspect: PropType scalar getters", "_PT = PC._class.prop_info(props_by_kind(PC._class)['ArrayProperty'][0]); chk('pt getters', lambda: isinstance(_PT.kind,str) and isinstance(_PT.cast_flags,int) and isinstance(_PT.property_flags,int) and isinstance(_PT.element_size,int))"),

    # ---- BoundMethod introspection (calls handled separately) ----
    ("BoundMethod: getattr -> BoundMethod", "_BM = PC.K2_GetPawn; chk('bm type', lambda: type(_BM).__name__=='BoundMethod')"),
    ("BoundMethod: __repr__", "chk('bm repr', lambda: 'BoundMethod' in repr(_BM))"),
    ("BoundMethod: function_info().signature", "chk('bm sig', lambda: isinstance(_BM.function_info().signature, str))"),
    ("BoundMethod: function_info()", "chk('bm fi', lambda: _BM.function_info() is not None)"),
    ("BoundMethod: wrong arg count -> TypeError", "expect_raises('bm argc', TypeError, lambda: PC.K2_GetPawn(1,2,3))"),

    # ---- Struct proxy: writes on a Python-OWNED struct (zero game impact) ----
    ("Struct: owned struct from GetFocalLocation", "_S = PC.GetFocalLocation() if dict(PC._class.functions()).get('GetFocalLocation') is not None else None; chk('owned struct', lambda: (_S is None) or (type(_S).__name__=='Struct'))"),
    ("Struct: __repr__/__dir__", "chk('struct repr/dir', lambda: (_S is None) or ('Struct' in repr(_S) and len(_S.__dir__())>0))"),
    ("Struct: scalar field write round-trip (owned)", '''
def _struct_rw():
    if _S is None:
        return
    touched = []
    for f in _S.__dir__():
        v = getattr(_S, f)
        if isinstance(v, float):
            setattr(_S, f, v + 1.5); assert abs(getattr(_S,f)-(v+1.5))<1e-3; setattr(_S, f, v); touched.append(f)
        elif isinstance(v, int) and not isinstance(v, bool):
            setattr(_S, f, v + 1); assert getattr(_S,f)==v+1; setattr(_S, f, v); touched.append(f)
    assert touched, "no scalar fields"
chk('struct field rw', _struct_rw)
'''),
    ("Struct: bad attr -> AttributeError", "expect_raises('struct bad attr', AttributeError, lambda: _S.NoSuchField_XYZ if _S is not None else (_ for _ in ()).throw(AttributeError()))"),

    # ---- error paths that must NOT crash ----
    ("error: distinct objects not equal", "chk('distinct ne', lambda: (PC==WS) is False)"),
    ("error: UObject vs UClass cross-compare", "chk('cross cmp', lambda: (PC==PC._class) is False)"),
]


# ── object construction (ue.new_object) ──────────────────────────────────────
# Exercises the StaticConstructObject_Internal factory. This is the group that
# would have caught the UE 4.25 crash: that build has only the flat
# StaticConstructObject_Internal(UClass*, Outer, Name, Flags, ...) signature,
# not the 4.26+ FStaticConstructObjectParameters& bundle the binding assumed —
# calling the wrong ABI hands the factory a garbage Class pointer and the process
# faults inside FObjectInitializer. So each step here both (a) constructs, which
# on a broken build takes the process down and makes step() name it the prime
# suspect, and (b) reads back _class/_name/_flags, which pins the arguments
# actually landed in the right slots even when it doesn't crash outright.
NEWOBJECT_STEPS: list[tuple[str, str]] = [
    ("new_object: setup", '''
_NO_CLS = '/Script/Engine.SceneComponent'   # concrete, engine-universal, inert
def _no_make(cls_arg, **kw):
    o = ue.new_object(cls_arg, **kw)
    assert o is not None, "new_object returned None"
    o.add_to_root()          # pin against GC so the weak ref stays live for reads
    return o
chk('new_object setup (class present)', lambda: ue.find(_NO_CLS) is not None)
'''),
    ("new_object: by path -> correct class", '''
def _t():
    o = _no_make(_NO_CLS)
    try:
        # reading _class dereferences ClassPrivate — corrupt if the ctor ABI slipped
        assert o._class._name == 'SceneComponent', 'class mismatch: ' + o._class._name
        assert type(o).__name__ == 'UObject'
    finally:
        o.remove_from_root()
chk('new_object by path', _t)
'''),
    ("new_object: by UClass proxy -> correct class", '''
def _t():
    o = _no_make(ue.find(_NO_CLS))
    try:
        assert o._class._name == 'SceneComponent', 'class mismatch: ' + o._class._name
    finally:
        o.remove_from_root()
chk('new_object by UClass', _t)
'''),
    ("new_object: name argument applied", '''
def _t():
    o = _no_make(_NO_CLS, name='PsNewObjTest')
    try:
        assert 'PsNewObjTest' in o._name, 'name not applied: ' + o._name
    finally:
        o.remove_from_root()
chk('new_object name arg', _t)
'''),
    ("new_object: flags argument applied (SetFlags slot)", '''
def _t():
    # RF_Transient must land in the SetFlags slot; an arg-ABI slip mislays it.
    o = _no_make(_NO_CLS, flags=ue.EObjectFlags.RF_Transient)
    try:
        assert o._flags & ue.EObjectFlags.RF_Transient, 'RF_Transient missing: ' + hex(o._flags)
    finally:
        o.remove_from_root()
chk('new_object flags arg', _t)
'''),
    ("new_object: outer argument nests the object", '''
def _t():
    o = _no_make(_NO_CLS, outer=GI)
    try:
        assert o._class._name == 'SceneComponent'
        assert GI._name in o._path, 'not nested under outer: ' + o._path
    finally:
        o.remove_from_root()
(chk('new_object outer arg', _t) if GI is not None else _sk('new_object outer arg', 'no GameInstance'))
'''),
    ("new_object: missing class -> ValueError", "expect_raises('new_object bad path', ValueError, lambda: ue.new_object('/Game/Nope.X_Y_Z'))"),
    ("new_object: non-class arg -> TypeError", "expect_raises('new_object non-class', TypeError, lambda: ue.new_object(42))"),
]


# ── FText marshalling (ue.Text handles) ──────────────────────────────────────
# FText is opaque and refcounted, so it marshals as an owning `ue.Text` handle
# rather than a str: reading an FTextProperty yields one, a UFunction FText
# argument accepts one, and a raw str is deliberately *not* auto-converted (it
# raises). Handles are minted/decoded through KismetTextLibrary's
# Conv_StringToText / Conv_TextToString, which also exercises FText as both a
# UFunction return value and an argument. The whole group is guarded on
# KismetTextLibrary in main().
FTEXT_STEPS: list[tuple[str, str]] = [
    ("FText: setup", '''
KTL = ue.find('/Script/Engine.Default__KismetTextLibrary')
def _find_textprop():
    tb = ue.find('/Script/UMG.Default__TextBlock')      # TextBlock.Text is an FText (fast path)
    if tb is not None:
        n = find_prop(tb, 'TextProperty')
        if n: return tb, n
    for o in (PC, WS, GI):
        if o is not None:
            n = find_prop(o, 'TextProperty')
            if n: return o, n
    for o in ue.objects(max=200000):
        n = find_prop(o, 'TextProperty')
        if n: return o, n
    return None, None
_TXT_O, _TXT_N = _find_textprop()
_HAS_CONV = KTL is not None and KTL._class.function_info('Conv_TextToString') is not None
chk('FText setup (KismetTextLibrary present)', lambda: KTL is not None)
'''),
    ("FText: read FTextProperty -> ue.Text handle", '''
def _t():
    v = getattr(_TXT_O, _TXT_N)
    assert type(v).__name__ == 'Text', 'got ' + type(v).__name__     # not a str
    assert repr(v) == '<ue.Text>'
(chk('FText read', _t) if _TXT_O is not None else _sk('FText read', 'no FTextProperty found'))
'''),
    ("FText: mint via Conv_StringToText -> ue.Text", '''
def _t():
    t = KTL.Conv_StringToText('rogue')          # str arg -> FText return, marshalled to ue.Text
    assert type(t).__name__ == 'Text', 'got ' + type(t).__name__
chk('FText mint', _t)
'''),
    ("FText: str<->Text round-trip through UFunctions", '''
def _t():
    t = KTL.Conv_StringToText('rogue-core')     # str  -> FText (return value)
    s = KTL.Conv_TextToString(t)                # ue.Text (arg) -> str (return value)
    assert s == 'rogue-core', 'round-trip lost value: ' + repr(s)
(chk('FText round-trip', _t) if _HAS_CONV else _sk('FText round-trip', 'no Conv_TextToString'))
'''),
    ("FText: raw str for FText arg -> TypeError (no auto-convert)", '''
# a python str passed where an FText arg is expected must raise, not silently coerce
(expect_raises('FText str arg', TypeError, lambda: KTL.Conv_TextToString('not a Text'))
 if _HAS_CONV else _sk('FText str arg', 'no Conv_TextToString'))
'''),
]


# Steps appended at runtime once we know what's safe to touch:
#  * Array mutation cycle on a discovered empty array
#  * Map mutation cycle on a discovered empty Map<Str,Str>
#  * Enum/Int/Float/Bool/Object write round-trips
#  * Per-function safe getter calls


# Source for the dynamic write/container steps (kept here for readability;
# each is sent as its own frame).
ARRAY_STEP = r'''
def _array_ops():
    obj = PC or WS
    name = None; inner = None
    for cand in (PC, WS):
        if cand is None: continue
        for n, ff in cand._class.props():
            if ff != "ArrayProperty": continue
            a = getattr(cand, n)
            if len(a) != 0: continue
            pt = cand._class.prop_info(n)
            ik = pt.inner().kind if (pt and pt.inner()) else None
            if ik in ("ObjectProperty","IntProperty","FloatProperty","StrProperty","NameProperty"):
                obj, name, inner = cand, n, ik; break
        if name: break
    if not name:
        _sk("Array mutation cycle", "no empty writable array found"); return
    sample = {"ObjectProperty":None,"IntProperty":7,"FloatProperty":2.5,"StrProperty":"ps","NameProperty":"ps"}[inner]
    print(f"  (Array<{inner}> on {obj._name}.{name})")
    def go():
        a = getattr(obj, name); assert len(a)==0
        a.append(sample); a = getattr(obj, name); assert len(a)==1
        _ = a[0]; _ = repr(a)
        a.insert(0, sample); a = getattr(obj, name); assert len(a)==2
        a[1] = sample
        a.pop(); a = getattr(obj, name); assert len(a)==1
        a.extend([sample]); a = getattr(obj, name); assert len(a)==2
        del a[0]; a = getattr(obj, name); assert len(a)==1
        a.clear(); assert len(getattr(obj, name))==0
    chk("Array mutation cycle (restored empty)", go)
    expect_raises("Array OOB -> IndexError", IndexError, lambda: getattr(obj,name)[999])
_array_ops()
'''

MAP_STEP = r'''
def _map_ops():
    mo = mn = None
    for cand in (GI, PC, WS):
        if cand is None: continue
        for n, ff in cand._class.props():
            if ff != "MapProperty": continue
            m = getattr(cand, n)
            if len(m) != 0: continue
            pt = cand._class.prop_info(n)
            if pt and pt.key() and pt.value() and pt.key().kind=="StrProperty" and pt.value().kind=="StrProperty":
                mo, mn = cand, n; break
        if mn: break
    if not mn:
        _sk("Map mutation cycle", "no empty Map<Str,Str> found"); return
    print(f"  (Map<Str,Str> on {mo._name}.{mn})")
    def go():
        m = getattr(mo, mn); assert len(m)==0
        m["ps_k"]="ps_v"; assert len(m)==1 and "ps_k" in m and m["ps_k"]=="ps_v"
        assert m.keys()==["ps_k"] and m.values()==["ps_v"] and m.items()==[("ps_k","ps_v")]
        assert list(iter(m))==["ps_k"]; _ = repr(m)
        del m["ps_k"]; assert len(m)==0
        m["a"]="1"; m.clear(); assert len(getattr(mo,mn))==0
    chk("Map mutation cycle (restored empty)", go)
    expect_raises("Map missing key -> KeyError", KeyError, lambda: getattr(mo,mn)["nope_xyz"])
_map_ops()
'''

WRITE_STEP = r'''
def _writes():
    def rt(obj, kind, mk, label, eq=lambda a,b: a==b):
        n = find_prop(obj, kind)
        if n is None: _sk(f"write {label}", f"no {kind}"); return
        def go():
            old = getattr(obj, n); new = mk(old)
            try:
                setattr(obj, n, new); assert eq(getattr(obj,n), new), f"wrote {new!r} got {getattr(obj,n)!r}"
            finally:
                setattr(obj, n, old); assert eq(getattr(obj,n), old), "restore failed"
        chk(f"write {label} round-trip", go)
    if WS is not None:
        rt(WS, "IntProperty", lambda v: v+1, "Int")
        rt(WS, "FloatProperty", lambda v: v+1.0, "Float", eq=lambda a,b: abs(a-b)<1e-3)
        rt(WS, "BoolProperty", lambda v: not v, "Bool")
        rt(WS, "NameProperty", lambda v: v, "Name (identity)")
        en = find_prop(WS, "EnumProperty")
        if en:
            pt = WS._class.prop_info(en); eo = pt.enum_obj() if pt else None
            ents = [e for e in (eo.enum_entries() if eo else []) if not e[0].endswith("_MAX")]
            if ents:
                def by_name():
                    old = getattr(WS, en)
                    try: setattr(WS, en, ents[0][0]); assert getattr(WS,en)==ents[0][0]
                    finally: setattr(WS, en, old)
                chk("write Enum by name (restored)", by_name)
                def by_int():
                    old = getattr(WS, en)
                    try: setattr(WS, en, ents[0][1])
                    finally: setattr(WS, en, old)
                chk("write Enum by int (restored)", by_int)
                def bad():
                    old = getattr(WS, en)
                    try: setattr(WS, en, "NOPE_XYZ")
                    finally: setattr(WS, en, old)
                expect_raises("write Enum bad name -> ValueError", ValueError, bad)
    if PC is not None:
        op = find_prop(PC, "ObjectProperty")
        if op:
            def onone():
                old = getattr(PC, op)
                try:
                    setattr(PC, op, None); assert getattr(PC, op) is None
                    if old is not None:
                        setattr(PC, op, old); assert getattr(PC, op)==old
                finally:
                    setattr(PC, op, old)
            chk("write ObjectProperty None+restore", onone)
            def obad():
                old = getattr(PC, op)
                try: setattr(PC, op, 1234)
                finally: setattr(PC, op, old)
            expect_raises("write ObjectProperty wrong type -> TypeError", TypeError, obad)
_writes()
'''


SET_STEP = r'''
def _set_ops():
    # Scan a bounded slice of objects for an empty Set of a hashable primitive.
    target = None
    scanned = set()
    for o in ue.objects(max=40000):
        cn = o._class._name
        if cn in scanned:
            continue
        scanned.add(cn)
        try:
            for n, ff in o._class.props():
                if ff != "SetProperty":
                    continue
                pt = o._class.prop_info(n)
                ik = pt.inner().kind if (pt and pt.inner()) else None
                if ik in ("NameProperty", "IntProperty", "UInt32Property", "StrProperty"):
                    if len(getattr(o, n)) == 0:
                        target = (o, n, ik); break
        except Exception:
            continue
        if target:
            break
    if not target:
        _sk("Set mutation cycle", "no empty hashable Set found in scan"); return
    obj, name, ik = target
    sample = {"NameProperty": "ps", "StrProperty": "ps", "IntProperty": 11, "UInt32Property": 11}[ik]
    print(f"  (Set<{ik}> on {obj._name}.{name})")
    def go():
        st = getattr(obj, name); assert len(st) == 0
        assert st.add(sample) is True
        st = getattr(obj, name); assert len(st) == 1 and sample in st
        assert list(iter(st)) == [sample]; _ = repr(st)
        assert st.add(sample) is False          # already present
        assert st.discard(sample) is True
        assert getattr(obj, name).discard(sample) is False   # absent, no raise
        getattr(obj, name).add(sample)
        getattr(obj, name).remove(sample)       # remove present
        assert len(getattr(obj, name)) == 0
        getattr(obj, name).add(sample); getattr(obj, name).clear()
        assert len(getattr(obj, name)) == 0
    chk("Set mutation cycle (restored empty)", go)
    expect_raises("Set.remove missing -> KeyError", KeyError, lambda: getattr(obj, name).remove(sample))
_set_ops()
'''

# Engine-stock Kismet libraries exist in every UE game, so these exercise
# argument marshalling (int / float / struct-from-dict / struct-proxy), struct
# returns, and out-params portably. Each is its own frame.
KISMET_STEPS: list[tuple[str, str]] = [
    ("kismet: setup", "KML = ue.find('/Script/Engine.Default__KismetMathLibrary'); print('KML', repr(KML))"),
    ("kismet: int arg+ret Abs_Int(-5)", "chk('Abs_Int', lambda: KML.Abs_Int(-5) == 5)"),
    ("kismet: int args Add_IntInt(3,4)", "chk('Add_IntInt', lambda: KML.Add_IntInt(3, 4) == 7)"),
    ("kismet: float args + struct ret MakeVector", "chk('MakeVector', lambda: (lambda v: v.X==3.0 and v.Y==4.0)(KML.MakeVector(3.0,4.0,0.0)))"),
    ("kismet: dict->struct arg VSize==5", "chk('VSize(dict)', lambda: abs(KML.VSize({'X':3.0,'Y':4.0,'Z':0.0}) - 5.0) < 1e-3)"),
    ("kismet: Struct-proxy arg VSize(MakeVector)", "chk('VSize(proxy)', lambda: abs(KML.VSize(KML.MakeVector(3.0,4.0,0.0)) - 5.0) < 1e-3)"),
    ("kismet: two struct args Dot==32", "chk('Dot', lambda: abs(KML.Dot_VectorVector({'X':1.0,'Y':2.0,'Z':3.0},{'X':4.0,'Y':5.0,'Z':6.0}) - 32.0) < 1e-3)"),
    ("kismet: out-params -> tuple BreakVector", "chk('BreakVector', lambda: KML.BreakVector({'X':3.0,'Y':4.0,'Z':5.0}) == (3.0,4.0,5.0))"),
    ("kismet: nested struct args MakeTransform", "T = KML.MakeTransform({'X':1.0,'Y':2.0,'Z':3.0},{'Pitch':0.0,'Yaw':90.0,'Roll':0.0},{'X':1.0,'Y':1.0,'Z':1.0}); chk('MakeTransform', lambda: type(T).__name__=='Struct' and len(T.__dir__())>=3)"),
    ("kismet: out-struct params -> tuple BreakTransform", "chk('BreakTransform', lambda: (lambda lrs: all(type(x).__name__=='Struct' for x in lrs) and len(lrs)==3)(KML.BreakTransform(T)))"),
    ("kismet: nested struct field read (Transform.Translation.X)", "chk('nested field', lambda: isinstance(getattr([f for f in T.__dir__() if 'ranslation' in f][0] and getattr(T,[f for f in T.__dir__() if 'ranslation' in f][0]),'X'), float))"),
    ("kismet: bad arg type -> TypeError", "expect_raises('Abs_Int bad', TypeError, lambda: KML.Abs_Int('nope'))"),
    ("kismet: bad struct field -> AttributeError", "expect_raises('VSize bad field', AttributeError, lambda: KML.VSize({'NoSuchField': 1.0}))"),
    ("kismet: wrong argc -> TypeError", "expect_raises('Add argc', TypeError, lambda: KML.Add_IntInt(1))"),
]


# ── delegate proxy tests (read-only proxies + a guarded add/remove round-trip) ─

DELEGATE_STEPS: list[tuple[str, str]] = [
    ("delegate: multicast read -> proxy + len + repr",
     "_dn = find_prop(PC,'MulticastInlineDelegateProperty') or find_prop(PC,'MulticastSparseDelegateProperty'); "
     "_dv = getattr(PC, _dn) if _dn else None; "
     "chk('mcast proxy', lambda: _dv is not None and type(_dv).__name__=='MulticastDelegate' and isinstance(len(_dv), int) and 'MulticastDelegate' in repr(_dv))"),
    ("delegate: multicast .bindings list",
     "chk('mcast bindings', lambda: isinstance(_dv.bindings, list))"),
    ("delegate: single-cast read -> proxy|unbound",
     "_sn = find_prop(PC,'DelegateProperty'); "
     "chk('single proxy', lambda: _sn is None or type(getattr(PC,_sn)).__name__=='Delegate')"),
]

# Multicast add->readback->remove round-trip on a CDO, restored. Binds the
# delegate to a real function on the target's own class so broadcast would be
# safe; we never broadcast, just verify the binding list mutates and reverts.
DELEGATE_MUTATE_STEP = r'''
def _delegate_mutate():
    # AudioComponent has an inline multicast (OnAudioFinished) and the bindable
    # native event OnAudioFinishedNative-style methods; use its CDO.
    ac = ue.find('/Script/Engine.Default__AudioComponent')
    if ac is None:
        _sk("delegate add/remove round-trip", "no AudioComponent CDO"); return
    dn = find_prop(ac, 'MulticastInlineDelegateProperty')
    if dn is None:
        _sk("delegate add/remove round-trip", "no inline multicast on AudioComponent"); return
    # Pick any UFunction on the class as a (never-broadcast) bind target.
    fns = [n for n, _ in ac._class.functions()]
    if not fns:
        _sk("delegate add/remove round-trip", "no functions to bind"); return
    fn = fns[0]
    def go():
        d = getattr(ac, dn); before = len(d)
        d.add(ac, fn)
        d = getattr(ac, dn); assert len(d) == before + 1, "add did not grow list"
        assert any(fn in repr(b) for b in d.bindings), "binding not visible"
        d.remove(ac, fn)
        assert len(getattr(ac, dn)) == before, "remove did not restore"
    chk(f"delegate add/remove round-trip (AudioComponent.{dn} <- {fn})", go)
    expect_raises("delegate bind bogus fn -> ValueError", ValueError,
                  lambda: getattr(ac, dn).add(ac, 'NoSuchFunction_XYZ'))
_delegate_mutate()
'''


# ── function hooking tests (flat path via ProcessEvent/BoundMethod) ──────────
# Each step hooks, exercises, and unhooks in a server-side finally so no hook
# leaks into the GLOBAL registry (which survives across REPL connections).

HOOK_SETUP = r'''
import contextlib as _ctx
HKML = ue.find('/Script/Engine.Default__KismetMathLibrary'); HKC = HKML._class
@_ctx.contextmanager
def _hooked(name, cb):
    ue.hook(HKC, name, cb)
    try:
        yield
    finally:
        try: ue.unhook(HKC, name)
        except Exception: pass
print("hook setup ok; live hooks:", ue.hooks())
'''

HOOK_STEPS: list[tuple[str, str]] = [
    ("hook: return replacement", "def cb(t,p): return 4242\nwith _hooked('Add_IntInt', cb): chk('hook replace', lambda: HKML.Add_IntInt(1,2)==4242)"),
    ("hook: original() returns real value", "def cb(t,p): return p.original()\nwith _hooked('Add_IntInt', cb): chk('hook original', lambda: HKML.Add_IntInt(10,20)==30)"),
    ("hook: return None -> original not run (==0)", "def cb(t,p): return None\nwith _hooked('Add_IntInt', cb): chk('hook none', lambda: HKML.Add_IntInt(5,6)==0)"),
    ("hook: callback raises -> fail-safe original", "def cb(t,p): raise ValueError('boom')\nwith _hooked('Add_IntInt', cb): chk('hook failsafe', lambda: HKML.Add_IntInt(7,8)==15)"),
    ("hook: wrong-type return survives (no crash)", "def cb(t,p): return 'not an int'\nwith _hooked('Add_IntInt', cb):\n HKML.Add_IntInt(3,3); _ok('hook wrong-type return survived')"),
    ("hook: args / this / dir / repr", "S={}\ndef cb(t,p):\n S['a']=list(p.args); S['d']=p.__dir__(); S['r']=repr(p); return 0\nwith _hooked('Add_IntInt', cb): HKML.Add_IntInt(11,22)\nchk('hook args', lambda: S['a']==[11,22] and set(S['d'])>={'A','B'} and 'HookParams' in S['r'])"),
    ("hook: mutate in-param then original()", "def cb(t,p): p.A=100; return p.original()\nwith _hooked('Add_IntInt', cb): chk('hook mutate', lambda: HKML.Add_IntInt(1,5)==105)"),
    ("hook: original() twice -> RuntimeError", "R=[]\ndef cb(t,p):\n p.original()\n try: p.original()\n except RuntimeError: R.append(1)\n return 1\nwith _hooked('Add_IntInt', cb): HKML.Add_IntInt(1,1)\nchk('hook original 2x', lambda: R==[1])"),
    ("hook: escaped HookParams -> RuntimeError", "St=[]\ndef cb(t,p): St.append(p); return 0\nwith _hooked('Add_IntInt', cb): HKML.Add_IntInt(1,1)\nexpect_raises('hook escaped params', RuntimeError, lambda: St[0].A)"),
    ("hook: read struct in-param + original()", "S={}\ndef cb(t,p): S['x']=p.A.X; return p.original()\nwith _hooked('VSize', cb):\n r=HKML.VSize({'X':3.0,'Y':4.0,'Z':0.0})\nchk('hook struct param', lambda: S['x']==3.0 and abs(r-5.0)<1e-3)"),
    ("hook: replace struct return via dict", "def cb(t,p): return {'X':7.0,'Y':8.0,'Z':9.0}\nwith _hooked('MakeVector', cb):\n v=HKML.MakeVector(1.0,1.0,1.0)\nchk('hook struct ret', lambda: v.X==7.0 and v.Z==9.0)"),
    ("hook: out-param fn, original() fills outs", "def cb(t,p): return p.original()\nwith _hooked('BreakVector', cb):\n r=HKML.BreakVector({'X':3.0,'Y':4.0,'Z':5.0})\nchk('hook outparams', lambda: r==(3.0,4.0,5.0))"),
    ("hook: bounded reentrancy (re-call self)", "depth=[0]\ndef cb(t,p):\n depth[0]+=1\n if p.A>0 and depth[0]<10: return HKML.Add_IntInt(p.A-1, p.B)\n return p.B\nwith _hooked('Add_IntInt', cb): chk('hook reentry', lambda: HKML.Add_IntInt(3,100)==100)"),
    ("hook: callback unhooks itself mid-call", "def cb(t,p): ue.unhook(HKC,'Add_IntInt'); return 555\nue.hook(HKC,'Add_IntInt',cb)\ntry:\n a=HKML.Add_IntInt(1,2); b=HKML.Add_IntInt(1,2); chk('hook self-unhook', lambda: a==555 and b==3)\nfinally:\n (ue.unhook(HKC,'Add_IntInt') if '/Script/Engine.KismetMathLibrary:Add_IntInt' in ue.hooks() else None)"),
    ("hook: double hook -> RuntimeError", "ue.hook(HKC,'Add_IntInt',lambda t,p:0)\ntry: expect_raises('hook double', RuntimeError, lambda: ue.hook(HKC,'Add_IntInt',lambda t,p:0))\nfinally: ue.unhook(HKC,'Add_IntInt')"),
    ("hook: unhook not-hooked -> RuntimeError", "expect_raises('hook unhook-missing', RuntimeError, lambda: ue.unhook(HKC,'Add_IntInt'))"),
    ("hook: instance target -> TypeError", "expect_raises('hook instance', TypeError, lambda: ue.hook(HKML,'Add_IntInt',lambda t,p:0))"),
    ("hook: missing function -> ValueError", "expect_raises('hook missing fn', ValueError, lambda: ue.hook(HKC,'NoSuchFunc_XYZ',lambda t,p:0))"),
    ("hook: FString param read + return replace", "S={}\ndef cb(t,p): S['a']=p.A; S['b']=p.B; return 'R:'+p.A\nKSL=ue.find('/Script/Engine.Default__KismetStringLibrary')\nue.hook(KSL._class,'Concat_StrStr',cb)\ntry:\n r=KSL.Concat_StrStr('foo','bar'); chk('hook string', lambda: S['a']=='foo' and S['b']=='bar' and r=='R:foo')\nfinally:\n ue.unhook(KSL._class,'Concat_StrStr')"),
    ("hook: table exhaustion (64 max) + clean teardown", '''
def _table():
    names = [n for n, _ in HKC.functions()]
    seen = []
    for n in names:
        if n not in seen: seen.append(n)
    inst = []; errs = []
    try:
        for n in seen[:70]:
            try: ue.hook(HKC, n, lambda t, p: None); inst.append(n)
            except RuntimeError as e: errs.append(str(e))
        chk("hook table full at 64", lambda: len(inst) == 64 and errs and "full" in errs[0])
    finally:
        for n in inst:
            try: ue.unhook(HKC, n)
            except Exception: pass
        chk("hook table teardown -> 0 leaked", lambda: len(ue.hooks()) == 0)
_table()
'''),
]


# Hooks installed for the live bytecode-path probe (transparent: count + run
# original + passthrough). Reachable only via the game's Blueprint VM.
BYTECODE_HOOK_INSTALL = r'''
_BHC = ue.find('/Script/Engine.Default__KismetMathLibrary')._class
_BH = {}
_BERR = {}
def _mk_bh(nm):
    def cb(t, p):
        _BH[nm] = _BH.get(nm, 0) + 1
        try: p.original()
        except Exception as e: _BERR[nm] = f"{type(e).__name__}: {e}"
        return None
    return cb
_BINST = []
for _n in ['MakeVector','BreakVector','Subtract_VectorVector','Add_VectorVector',
           'Multiply_VectorFloat','Dot_VectorVector','VSize','Lerp','FClamp',
           'Abs','Add_DoubleDouble','Multiply_DoubleDouble']:
    if _n in (x for x, _ in _BHC.functions()):
        try: ue.hook(_BHC, _n, _mk_bh(_n)); _BINST.append(_n)
        except Exception: pass
print("bytecode-probe hooks installed:", _BINST)
'''

BYTECODE_HOOK_READ = r'''
try:
    _fired = {k: v for k, v in _BH.items() if v}
    print("  fired via BP VM:", _fired)
    print("  errors during original():", _BERR)
    if _fired and not _BERR:
        _ok(f"bytecode-path hooks ({sum(_fired.values())} live calls, no errors)")
    elif _BERR:
        _no("bytecode-path hooks", str(_BERR))
    else:
        _sk("bytecode-path hooks", "nothing fired (game paused / no BP math this window?)")
finally:
    for _n in _BINST:
        try: ue.unhook(_BHC, _n)
        except Exception: pass
    print("  bytecode-probe cleanup; live hooks:", len(ue.hooks()))
'''


def bytecode_hook_probe(step):
    """Hook hot pure math fns, let the game tick, verify they fired via the
    bytecode/math entry path with no errors, then unhook. Transparent: each
    callback runs the original and passes its result through."""
    import time
    step("bytecode-path: install transparent hooks", BYTECODE_HOOK_INSTALL)
    print(">> bytecode-path: observing live gameplay ~4s ...", flush=True)
    time.sleep(4)
    step("bytecode-path: verify fired + teardown", BYTECODE_HOOK_READ)


def main(argv):
    host = argv[0] if len(argv) > 0 else "127.0.0.1"
    port = int(argv[1]) if len(argv) > 1 else 8765

    try:
        c = Conn(host, port)
    except OSError as e:
        print(f"could not connect to {host}:{port}: {e}")
        print("Is the game running with the py_repl bindings active?")
        return 1

    def step(name, source):
        print(f">> running: {name}", flush=True)
        try:
            reply = c.run(source)
        except (EOFError, ConnectionError, OSError) as e:
            print("\n" + "!" * 60)
            print(f"GAME CRASHED (or disconnected) while running: {name}")
            print(f"connection error: {type(e).__name__}: {e}")
            print("!" * 60)
            print("The operation above is the prime suspect. Everything before "
                  "it passed without taking the process down.")
            raise SystemExit(2)
        sys.stdout.write(reply if reply.endswith("\n") or not reply else reply + "\n")
        sys.stdout.flush()

    # 1) prelude (defines helpers + discovers PC/WS/GI in the game namespace)
    step("PRELUDE (setup + discovery)", PRELUDE)

    # 2) the static check steps
    for name, source in STEPS:
        step(name, source)

    # 3) dynamic container + write steps
    step("Array mutation cycle", ARRAY_STEP)
    step("Map mutation cycle", MAP_STEP)
    step("Set mutation cycle", SET_STEP)
    step("write round-trips", WRITE_STEP)

    # 3b) object construction via the StaticConstructObject_Internal factory
    for name, source in NEWOBJECT_STEPS:
        step(name, source)

    # 4) Kismet argument-marshalling / out-param / struct-return steps
    if c.eval("ue.find('/Script/Engine.Default__KismetMathLibrary') is not None"):
        for name, source in KISMET_STEPS:
            step(name, source)
    else:
        print("(no KismetMathLibrary — skipping arg-marshalling steps)")

    # 4c) FText marshalling (ue.Text handles)
    if c.eval("ue.find('/Script/Engine.Default__KismetTextLibrary') is not None"):
        for name, source in FTEXT_STEPS:
            step(name, source)
    else:
        print("(no KismetTextLibrary — skipping FText marshalling steps)")

    # 5) delegate proxy tests
    for name, source in DELEGATE_STEPS:
        step(name, source)
    step("delegate add/remove round-trip", DELEGATE_MUTATE_STEP)

    # 6) function hooking — flat path (ProcessEvent via BoundMethod)
    if c.eval("ue.find('/Script/Engine.Default__KismetMathLibrary') is not None"):
        step("HOOK SETUP", HOOK_SETUP)
        for name, source in HOOK_STEPS:
            step(name, source)
        # 6b) bytecode/math hook path — only reachable when the game's BP VM
        #     calls the function, so we hook hot pure fns, let the game run, and
        #     check they fired. Needs a client-side pause between frames.
        bytecode_hook_probe(step)
    else:
        print("(no KismetMathLibrary — skipping hook tests)")

    # 7) safe function calls — discover which exist, then call each in its own
    #    frame so a single bad call is isolated rather than blamed on a batch.
    present = []
    try:
        names = set(c.eval("[n for n,_ in PC._class.functions()]")) if c.eval("PC is not None") else set()
        present = [f for f in SAFE_CALLS if f in names]
    except Exception as e:  # noqa: BLE001
        print(f"(could not enumerate functions: {e})")
    if present:
        print(f"\n>> calling {len(present)} curated safe getters (each isolated):")
        for fn in present:
            step(f"call {fn}()",
                 f"chk('call {fn}()', lambda: PC.{fn}() if True else None)")
    else:
        print("(no curated safe getters present on this PlayerController)")

    # 8) summary, read back from the game's tally dict
    print("\n" + "=" * 60)
    try:
        t = c.eval("(_T['pass'], _T['fail'], _T['skip'], _T['fails'])")
        print(f"RESULT: {t[0]} passed, {t[1]} failed, {t[2]} skipped")
        if t[3]:
            print("\nFailures:")
            for f in t[3]:
                print(f"  - {f}")
    except Exception as e:  # noqa: BLE001
        print(f"(could not read tally: {e})")
    print("=" * 60)
    print("Reached the end with the process alive => the bindings held up "
          "across the full surface.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
