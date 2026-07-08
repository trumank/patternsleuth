# py_repl TODO

## UFunction invocation — verified
Working: primitive args/returns, FString args/returns, UObject args,
**struct args via Python dict**, **struct returns** (owned-buffer copy-out
so `Drop` calls `UStruct::DestroyStruct` later), **struct out parms** (e.g.
`K2_SetActorLocation` returns `(bool, FHitResult)`), **array/map/set
returns and out parms** (deep-copied into native Python containers, then
`FProperty::destroy_value` runs UE's destructor on the parm buffer), nested
struct field access on owned-copy proxies, `UClass.functions()` introspection.
Tested with KismetMathLibrary, KismetStringLibrary, KismetSystemLibrary,
and direct AActor calls (K2_GetActorLocation / K2_SetActorLocation).

### Remaining gaps
- **Default parameters** — UE BP functions with `CPP_Default_<Name>` metadata
  can be called with fewer args. Currently we strictly require args.len ==
  parms.len (`bindings.rs` `Callable for BoundMethod`). `UStruct::InitializeStruct`
  already gives parms type-default values (0/empty/null); reading editor
  `CPP_Default_X` metadata would let us honor authored defaults. Editor-only
  metadata may not survive in shipping builds.
- **Kwargs** — `obj.Func(target=x, count=3)`. `FuncArgs.kwargs` is currently
  discarded; args bind positionally by index. Match arg name → parm FName.
- **Function picking on overrides** — `UStruct::find_function` returns the first
  hit walking the super chain. For Blueprint-overridden native functions
  that's correct (BP override wins). For ambiguous cases, no API yet to
  pick a specific declaration.

## Per-frame tick — `ue.on_tick`
Native detour on `UWorld::Tick`. Callbacks fire once per frame **per world**
with `(world, delta_seconds)`, where `delta_seconds` is the world's
dilated/clamped frame delta (the value BP `Event Tick` sees), dispatched
*after* the engine's own tick so the world is fully settled (actor ticks,
physics, latent actions, timers all done). API mirrors `on_begin_play`:
`h = ue.on_tick(cb)` / `h.remove()` / `with ue.on_tick(cb): ...`, plus
`ue.tick_hooks()`.

### Remaining
- **Tick-type / world filtering.** `hooked_uworld_tick` has the `ELevelTick`
  value in hand but `dispatch_world_tick` only forwards `(world, delta)` — so
  callbacks fire for every `UWorld::Tick` regardless of tick type (incl.
  `LEVELTICK_TimeOnly` while paused) and for every world context. Callbacks can
  filter on the passed `world`; could add a native `LEVELTICK_All`/game-world
  guard later if it proves noisy.

## More hooks / instrumentation
Existing native hooks: `on_begin_play`, `on_tick`, per-function BP/native hooks,
line-trace hooks. Wanted:
- **Map / level load hooks** — fire on world/level load (and unload) so scripts
  can (re)bind to a fresh world. Detour something like `UEngine::LoadMap` /
  `UWorld::InitWorld` / the post-load-map delegate.
- **More actor init hooks** — beyond `on_begin_play`: construction
  (`AActor::PostActorCreated` / SCS + `OnConstruction`), `PreInitializeComponents`
  / `PostInitializeComponents`, and a pre-`BeginPlay` point. Lets scripts touch
  actors before gameplay logic runs.
- **Object creation / deletion hooks** — a firehose on `StaticConstructObject` /
  GC destruction. **Caveat:** objects are created on arbitrary threads and have
  complex lifetimes, so a naive Python callback per object is both a perf and a
  thread-safety hazard — would need coalescing + marshalling onto the game
  thread (route through the tick queue), and probably a class/name filter so it
  isn't firing millions of times.
- **Engine console hooks** — register custom console commands + handlers
  (`IConsoleManager::RegisterConsoleCommand`, or hook `UConsole`/`Exec`) so mods
  can expose `mymod.dostuff` in the in-game console and intercept typed commands.
- **Hook registration ergonomics.** `ue.hook` currently raises on a double-hook,
  so every hooking script wraps it in `try: ue.unhook(...); except: pass` then
  re-hooks. Add an idempotent `ue.hook(cls, fn, cb, replace=True)`, and let
  `on_tick` / `on_begin_play` take a `key=` so re-running a file auto-replaces the
  prior registration instead of the manual `globals()['_h'].remove()` dance. Since
  hooked classes must survive GC across level travel, auto-root them (or a
  `keep_alive=True`) so scripts stop calling `cls.add_to_root()` by hand.

## Property dispatch gaps
- **Multicast delegate `.broadcast(*args)`** — `MulticastDelegate` proxies now
  read/inspect fine (`.bindings`, `len()`, `.add()` / `.remove()` / `.clear()` /
  `.add_callable()`), so they no longer raise. Still missing: firing the
  delegate from Python. Marshal args like a UFunction call and invoke each
  bound `FScriptDelegate`.
- **Refined BP-function hooking API** — hooking a Blueprint function currently
  requires force-loading its asset and pinning it via the root set to keep the
  UFunction alive. Want a first-class API that handles the load/pin/unpin
  lifecycle (and re-binds across map loads / GC) so callers don't manage it by
  hand.

## Class-indexed lookup polish
- `ue.objects_of_class("name only")` could fall back to FName-only lookup
  (`hash_tables().find_object_by_name`) when the arg isn't a full path —
  saves `/Script/Engine.` typing in the REPL. (`resolve_class` currently only
  does `find_object_by_path`.)
- `UClass.subclasses()` — expose `class_to_child_list_map` directly so users
  can browse the class tree. (Not yet exposed; `include_subclasses=` on
  `objects_of_class`/`instances` is the only class-tree walk today.)

## Kismet bytecode
- **Disassembly / analysis** — decode a UFunction's `Script` bytecode
  (`EExprToken` stream) into a readable listing, and analyse it (find calls,
  property touches, jump targets). Basis for understanding BP logic without the
  editor.
- **Mid-hooking** — hook *inside* a Blueprint function at a specific bytecode
  offset (not just entry/exit), to observe or patch mid-execution. Needs the
  disassembler to find safe insertion points.

## Engine surface / accessors
- **Pak / IoStore access** — expose the platform file layer: enumerate mounted
  `.pak` / IoStore containers, mount/unmount at runtime, and read entries. Hook
  or wrap `FPakPlatformFile` / `FIoStore` mount + read paths so scripts can
  inspect and inject content.
- **Plugin / module enumeration** — accessors over `IPluginManager` (loaded
  plugins, their mount points / content dirs) and `FModuleManager` (loaded
  modules), so scripts can discover what's present and query module state.

## Mod system
Right now everything is driven live over the REPL socket with a single shared
global namespace — fine for interactive poking, not for shipping mods.
- **Mod API** — a first-class notion of a "mod": register one, give it a
  lifecycle (load / enable / disable / unload), and a place to keep state.
  Native hook registrations (tick, begin-play, function hooks) should be owned
  by the mod so they're torn down cleanly on unload.
- **Namespacing / isolation** — each mod should get its own module/globals so
  independent scripts can't stomp on each other's globals (today they all share
  one namespace). Give each mod an isolated exec environment; expose a small
  shared/API surface deliberately rather than by accident.
- **Automatic mod loading from filesystem** — discover and load mods from a
  directory on startup (manifest + entry script), instead of requiring every
  script to be pushed in over the REPL socket. The socket stays for interactive
  dev; the filesystem path is for actually deploying mods.

## Ergonomics — recurring idioms in the test scripts
Patterns copy-pasted across `python/` that the `ue` API should absorb:
- **Well-known accessors.** Nearly every script hand-rolls
  `gi = next(o for o in ue.objects_of_class('/Script/Engine.GameInstance') if
  not o._flags & TEMPLATE)` then `PC = gi.LocalPlayers[0].PlayerController`.
  Provide `ue.game_instance()`, `ue.player_controller(index=0)`,
  `ue.player_pawn()`, `ue.world()`, `ue.engine()`.
- **CDO / static-function-library access.** Every script that calls Kismet libs
  does `ue.find('/Script/Engine.Default__KismetMathLibrary')` (and `KSL`, `GS`,
  `WBP`, `AssetRegistryHelpers`, …) just to reach a CDO for static calls. Add
  `ue.cdo(cls)` / `ue.default_object(path)`, and/or let static UFunctions be
  called on the `UClass` directly so the `Default__` dance disappears.
- **Non-template instance filter.** `live(cls)` — `next(o for o in
  objects_of_class(cls) if not o._flags & TEMPLATE)` — is copy-pasted in ~9
  files, and `TEMPLATE = RF_ClassDefaultObject | RF_ArchetypeObject` +
  `not o._flags & TEMPLATE` (or the string test `'Default__' not in o._path`)
  shows up in ~23 files for the *same* intent. Exclude CDO/archetype templates by
  default (or a `templates=False` kwarg), a `ue.first_of_class(cls)`, and an
  `o.is_template` predicate so callers stop importing `EObjectFlags` for bit math.
- **Validity predicate.** Freecam scripts detect a proxy killed by level travel
  with `try: _ = pc._name; except: ...` (`_cam_valid` / `_resolve_pc`). Expose
  `o.is_valid()` so this isn't a try/except on a meta-attr.
- **Vector / rotator / quaternion math.** The freecam scripts each re-define
  `_qmul` / `_qrot` / `_qaxis` / `_qnorm` (and vector add/scale) — ~40 lines
  duplicated across all 9. Give `Vector` / `Rotator` / `Quat` struct proxies real
  operators + methods (or a small `ue.math` module / bound KismetMathLibrary
  helpers) so camera math isn't hand-rolled.
- **Struct constructors + literals.** Vectors/rotators/transforms/colors are all
  built as raw dicts, and an identity-transform dict is redefined (`_ident` /
  `_IDENT`) in every freecam. Add `ue.Vector(x,y,z)`, `ue.Rotator(p,y,r)`,
  `ue.Transform(...)` + `ue.Transform.IDENTITY`, `ue.LinearColor(r,g,b,a)`,
  accepted anywhere a struct dict is today (keep dicts working), and let a
  returned `Struct` support `.as_tuple()` to shorten read-modify-write round-trips.
- **Console command.** `_cmd(c) = KSL.ExecuteConsoleCommand(PC, c, None)` recurs;
  expose `ue.exec(cmd)` / `world.exec(cmd)`. (Inbound custom commands are the
  console-hook item above; this is the outbound direction.)
- **FText ↔ str.** Scripts still wrap `KismetTextLibrary.Conv_TextToString` /
  `Conv_StringToText` (`_str` helper). Make `str(text_handle)` work and accept a
  plain `str` wherever an FText arg/property is expected.
- **FKey / input.** `_key(name) = {'KeyName': name}` is redefined per freecam;
  a `ue.key('W')` constructor (or accepting a bare string where an FKey is
  expected) removes it.
- **Actor spawning.** `ue.new_object` only constructs UObjects; spawning an
  actor needs the deferred-spawn + `FinishSpawning` dance by hand. Add a
  `world.spawn_actor(cls, transform, ...)` helper.
- **Reflective marshal to plain Python.** The 4 `dump_*` scripts each copy an
  identical ~50-line walker that dispatches on `type(x).__name__ in
  ('Struct','Array','Set','Map','UObject')` and recurses. Provide
  `obj.to_dict()` / `ue.to_pytree(obj, expand_owned=False, max_depth=…)`, expose
  the wrapper types (`ue.Struct` / `Array` / `Map` / `Set`) so callers can
  `isinstance` instead of string-typing on `__name__` (already done for
  `UClass`/`UObject`), and give `Struct` an instance-level `.props()` / iteration
  so reflection stops being `for a in dir(x): if not a.startswith('_')` (which
  collides with the bridge's own `_name` / `_class` / `_path` / `_flags` meta).
- **Stable out-param return shape.** Callers can't tell a bare `bool` return from
  a `(ret, out…)` tuple, so they hedge with `res[0] if isinstance(res, tuple)
  else res` (both inject_room_layout, inject_constructions, the AssetRegistry
  dumps). Make the shape predictable — always a tuple when out-params exist (even
  one), or return a namedtuple keyed by out-param name.
- **Uniform enum args.** Enum reads return the `'EEnum::Value'` string, but on
  writes some scripts pass the int and some pass the string — accept either form
  everywhere an enum arg/property is set.
- **UMG scaffolding helpers.** The 4 widget scripts each redefine `_slate(rgba)`
  (FSlateColor), a cloned-`Font` helper, and the same widget-tree bootstrap
  (`WBL.Create` → `WidgetTree` → `CanvasPanel` → wire `RootWidget`). A thin
  `ue.umg` namespace (create-tree / add-to-canvas / `ue.slate_color`) would
  absorb it.
- **State / hot-reload boilerplate.** Stateful scripts all do the
  `_prev = globals().get('_freecam'); _prev['tick'].remove(); …; globals()['_freecam']
  = state` re-arm dance to survive re-running the file — and some fight the shared
  namespace outright (`farfarwest/jokers.py` has a no-op `globals()[n]=globals()[n]`
  loop; others wrap everything in `_install()` + `del _install` to avoid leaking
  names). Offer an explicit cross-frame surface (`ue.state` dict / `@ue.persist`),
  which is also what the **Mod system** section (per-mod owned hooks + state store
  + clean reload) should make unnecessary.
- **Scoped / undoable writes.** `rc/freecam` hand-rolls capture-and-restore of
  `(component, IsVisible())` bundles to revert scene mutations on toggle-off. A
  `with ue.temporarily_set(obj, 'prop', value): …` (or a transaction that records
  + reverts writes) would generalize it.

## Client UX
- **Error surfacing from hooks** — exceptions raised inside hook callbacks
  (tick / begin-play / function hooks, which fire on the game thread, not on a
  REPL request) currently only hit the game's log; the REPL client never sees
  them. Want them routed back to the connected client(s) (and ideally with a
  traceback), so debugging a hook doesn't mean tailing the game log.
- **Pretty printing** — `Struct.__repr__` still shows just `<Struct TypeName>`
  (plus a `stale` marker); could show `{field=value, …}` for small structs.

## Robustness
- Bindings assume main-thread execution. Document this. If we ever want
  background threads to touch UE state safely, route everything through the
  tick queue.

## RustPython port — followups
The interpreter is pure-Rust RustPython (was pyo3/CPython). Notes:
- **GC traversal** of proxies isn't wired. Proxies that capture a
  `PyObjectRef` (`Path`'s `MapKey`, the hook/begin-play/trace callbacks held
  in thread-locals) implement a no-op `MaybeTraverse`. Transient keys are
  fine; long-lived captured callables won't be seen by the cycle collector.
  Add `#[pyclass(traverse)]` + manual `Traverse` if leaks show up.
- **CPython-compat gaps** — RustPython isn't 100% CPython. The REPL
  bootstrap (`code`/`contextlib`/`io`/`enum`) works off the frozen stdlib;
  user scripts that lean on C extensions or exotic stdlib corners won't.
- **Perf** — RustPython is a pure-Rust bytecode interpreter, materially
  slower than CPython. Fine for UE introspection; bad for heavy compute.

## Build / deploy
- **Single self-contained DLL.** RustPython is statically linked with the
  stdlib frozen in (`freeze-stdlib`), so there's no `python3.dll` / stdlib zip
  and no loader stub — the old `py_repl` (stub) + `py_repl_inner` split is gone.
  Drop the one `py_repl.dll` next to the game and inject it.
- **libffi cross-compile.** RustPython's `_ctypes` pulls `libffi-sys`, whose
  *released* vendored C/asm does not cross-compile to windows-msvc from Linux
  (clang errors on `dlmalloc.c`, and cargo-xwin ships no `ml64` for libffi's
  `.asm`). The workspace `[patch.crates-io]` points `libffi-sys` at a local
  checkout (`/home/truman/src/third/libffi-rs/libffi-sys-rs`, 4.2.0 — satisfies
  libffi 5.1.1's `^4.2`) carrying the MSVC cross-build fix; with that, `cargo
  xwin build --target x86_64-pc-windows-msvc` produces `py_repl.dll`. The path
  is machine-specific — switch it to a git ref once the fix is upstream.
- Build: `cargo xwin build -p py_repl --release --target
  x86_64-pc-windows-msvc` → `target/x86_64-pc-windows-msvc/release/py_repl.dll`.
- Add a small `scripts/stage.sh` that builds the DLL and prints the path to
  copy to the game. (Not created yet.)
