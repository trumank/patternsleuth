# Spawn a FEW Rogue Core cave constructions into the current level so you can see
# how they carve themselves into the terrain -- the walkways / bridges / platforms
# that cut their own pocket out of the cave.
#
#   with a mission already loaded:
#       python repl.py --script rc/inject_constructions.py   # just defines helpers
#   then, in the REPL:
#       spawn_few()        # drop a handful into a room; they carve into the cave
#       coverage()         # list what's present
#       destroy_all()      # remove them again
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY ONLY A FEW  (learned the hard way):
#
# Each construction carries LevelGenerationCarver components. On spawn they call
# AddCarverComponentToProceduralSetup and register with the terrain; the terrain
# then CSG-carves each one's volume out of the cave voxels -- that IS how a
# construction mesh interfaces with the terrain. But the carve accumulates into a
# fixed pool (DeepArrayPool, MAXSIZE 65536 cells). Spawn too many at once -- e.g.
# all 77 rows of CGA_Default, several carving huge volumes (Holes, SubwayStation,
# FoodCourt) -- and the pool overflows and the game HARD-CRASHES on the next
# terrain tick:
#     Fatal error: DeepArrayPool.h:132 ExpandingArray out of range (MAXSIZE 65536)
# Disabling the carver (CarvingDisabled / actor tick) does NOT help: the actors
# are already registered with the terrain, and the terrain drives the carve.
#
# So the only safe way to see them carving is a handful at a time. spawn_few()
# places a small, curated set (default 4) into one room, carving live.  Keep
# BATCH small; each Hole*/big station eats a lot of pool.  spawn_all() is kept
# below for reference but WILL crash the game -- don't call it.
#
# WHY WE SPAWN THEM OURSELVES AT ALL:
# The game's own cave-construction pass (native SpawnConstructions, gated by
# GetConstructionBudget + GetConstructionQuery) finds no valid anchor slots in
# the BXE runs reachable here, so it naturally places ZERO of these -- cranking
# the budget/weights/query changes nothing.  Direct spawn is how you get them in.

import ue

TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

PLS = '/Script/RogueCore.ProceduralSetup'
CGA = '/Game/Art/Environments/Constructions/GroupAssets/CGA_Default.CGA_Default'
GS = '/Script/Engine.Default__GameplayStatics'
REG = '/Script/AssetRegistry.Default__AssetRegistryHelpers'
CARVER_CLASS = 'LevelGenerationCarverComponent'

# A small, visually distinct, ground-integrating set -- safe to carve at once.
DEFAULT_FEW = [
    'BP_Construction_Cave_Bridge_C',
    'BP_Construction_Cave_Platform01_C',
    'BP_Construction_Cave_SuspendedPlatform_C',
    'BP_Construction_Cave_Scafolding_A_C',
]
SPREAD = 1600.0       # ring radius the few are placed on, around the room centre
ROOM = 1              # which room to place them in (0 = the big start room)


def live(cls):
    return next((o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE), None)


# ── name -> loadable class path, from the asset registry ──────────────────────
# CGA rows expose StructureClass only as a bare name ('BP_Foo_C'); the registry
# gives the package, so ClassName == AssetName + '_C' -> <pkg>.<AssetName>_C.
def _class_index():
    reg = ue.find(REG).GetAssetRegistry()
    ok, assets = reg.GetAllAssets(True)
    idx = {}
    for a in assets:
        an = str(a.AssetName)
        idx[an + '_C'] = '%s.%s_C' % (str(a.PackageName), an)
    return idx


def _row_names():
    cga = ue.load(CGA)
    rows = cga.Constructions
    return [str(rows[i].StructureClass).strip().strip("'") for i in range(len(rows))]


CONSTRUCTIONS = sorted(set(_row_names()))   # every construction type in the table
_IDX = _class_index()
globals()['CONSTRUCTIONS'] = CONSTRUCTIONS


def _spawn_one(cls_path, x, y, z):
    """Spawn one construction actor at (x, y, z). Carving is LIVE -- it carves
    itself into the cave terrain as soon as it registers."""
    gs = ue.find(GS)
    ctx = live(PLS) or live('/Script/Engine.GameInstance')
    cls = ue.load(cls_path)
    if cls is None:
        return None
    xf = {'Translation': {'X': x, 'Y': y, 'Z': z},
          'Rotation': {'X': 0.0, 'Y': 0.0, 'Z': 0.0, 'W': 1.0},
          'Scale3D': {'X': 1.0, 'Y': 1.0, 'Z': 1.0}}
    a = gs.BeginDeferredActorSpawnFromClass(
        ctx, cls, xf, 'ESpawnActorCollisionHandlingMethod::AlwaysSpawn', None,
        'ESpawnActorScaleMethod::MultiplyWithRoot')
    if a is None:
        return None
    return gs.FinishSpawningActor(a, xf, 'ESpawnActorScaleMethod::MultiplyWithRoot')


def _room_center(room):
    pls = live(PLS)
    if pls is None or not len(pls.Rooms):
        print('spawn: no live ProceduralSetup -- are you in a mission?')
        return None
    room = max(0, min(room, len(pls.Rooms) - 1))
    p = pls.Rooms[room].position
    return float(p.X), float(p.Y), float(p.Z)


def spawn_few(names=None, room=ROOM, spread=SPREAD):
    """Spawn a small set of constructions in a ring inside `room`, carving live.
    Keep the list short (~4) so the terrain carve pool doesn't overflow."""
    import math
    names = names or DEFAULT_FEW
    center = _room_center(room)
    if center is None:
        return []
    cx, cy, cz = center
    spawned = []
    n = len(names)
    for i, nm in enumerate(names):
        path = _IDX.get(nm)
        if path is None:
            print('  unknown construction: %s' % nm)
            continue
        ang = (2 * math.pi * i) / max(1, n)
        x = cx + spread * math.cos(ang)
        y = cy + spread * math.sin(ang)
        try:
            a = _spawn_one(path, x, y, cz)
            if a is not None:
                spawned.append(nm)
                print('  spawned %-42s @ (%.0f, %.0f, %.0f)' % (nm, x, y, cz))
        except Exception as e:
            print('  spawn failed %s: %r' % (nm, e))
    print('spawn_few: %d/%d spawned into room %d' % (len(spawned), n, room))
    return spawned


def spawn_at_camera(names=None, dist=2200.0, step=1700.0):
    """Spawn a small set in a row in front of the active camera -- handy when you
    are freecamming in void space and want them right where you're looking."""
    import math
    names = names or DEFAULT_FEW
    pc = live('/Script/Engine.PlayerController')
    cm = pc.PlayerCameraManager if pc else None
    if cm is None:
        print('spawn_at_camera: no active camera')
        return []
    loc, rot = cm.GetCameraLocation(), cm.GetCameraRotation()
    py, yw = math.radians(rot.Pitch), math.radians(rot.Yaw)
    fx, fy, fz = math.cos(py) * math.cos(yw), math.cos(py) * math.sin(yw), math.sin(py)
    rx, ry = -math.sin(yw), math.cos(yw)              # horizontal right vector
    spawned = []
    n = len(names)
    for i, nm in enumerate(names):
        path = _IDX.get(nm)
        if path is None:
            print('  unknown construction: %s' % nm)
            continue
        off = (i - (n - 1) / 2.0) * step
        x = loc.X + fx * dist + rx * off
        y = loc.Y + fy * dist + ry * off
        z = loc.Z + fz * dist
        try:
            a = _spawn_one(path, x, y, z)
            if a is not None:
                spawned.append(nm)
                print('  spawned %-42s @ (%.0f, %.0f, %.0f)' % (nm, x, y, z))
        except Exception as e:
            print('  spawn failed %s: %r' % (nm, e))
    print('spawn_at_camera: %d/%d spawned ahead of the camera' % (len(spawned), n))
    return spawned


def spawn_all(*_a, **_k):
    """DANGER: spawns all 77 -> overflows the terrain carve pool -> CRASH.
    Kept only to document what NOT to do; use spawn_few() instead."""
    print('refusing: spawning all 77 crashes the terrain carve pool. '
          'Use spawn_few() (a handful at a time).')


def destroy_all():
    """Remove every construction actor of a table type from the level."""
    want = set(CONSTRUCTIONS)
    killed = 0
    for a in list(ue.objects_of_class('/Script/Engine.Actor')):
        if a._flags & TEMPLATE:
            continue
        if a._class._name in want:
            try:
                a.K2_DestroyActor()
                killed += 1
            except Exception:
                pass
    print('destroy_all: removed %d construction actors' % killed)
    return killed


def coverage():
    """List which table construction types are currently present in the level."""
    want = set(CONSTRUCTIONS)
    seen = {}
    for a in ue.objects_of_class('/Script/Engine.Actor'):
        if a._flags & TEMPLATE:
            continue
        n = a._class._name
        if n in want:
            seen[n] = seen.get(n, 0) + 1
    print('coverage: %d/%d table types present, %d actors total'
          % (len(seen), len(want), sum(seen.values())))
    for n in sorted(seen):
        print('  %2d  %s' % (seen[n], n))
    return seen


globals()['spawn_few'] = spawn_few
globals()['spawn_at_camera'] = spawn_at_camera
globals()['spawn_all'] = spawn_all
globals()['destroy_all'] = destroy_all
globals()['coverage'] = coverage


# ── carve them IN: boost the construction budget and regenerate ────────────────
# The only way a construction actually CARVES into the cave is to be placed during
# generation, when the terrain carve pass reads it -- exactly how the game's own
# constructions integrate at room floors. Post-gen spawns (spawn_few / spawn_at_
# camera) float; they can't re-cut committed terrain.
#
# So main() arms a hook that raises GetConstructionBudget (default 10 -> BUDGET),
# then relaunches a run. The game's SpawnConstructions fills more room slots and
# the carve pass integrates every one. Keep BUDGET MODERATE: each construction's
# carve eats into the fixed 65536-cell pool, and too many overflow it -> crash.

BUDGET = 40
GAMEINSTANCE = '/Script/Engine.GameInstance'
GAMEMODE = '/Script/Engine.GameModeBase'
RIG_LEVEL = 'LVL_Ramrod_MAIN'
SELECT = 0


def _budget_hook(this, p):
    p.Budget = BUDGET                 # out-param; skip original -> our value stands


# ── gen-time carve test: inject a carver into rock DURING generation ───────────
# Hooked onto CreateCaveGraph (runs before the terrain carve pass, the validated
# pre-carve injection point). We author the graph normally (original) then spawn
# a big Hole straddling a room wall -- half in open room, half in solid rock. If
# gen-time carvers cut the surrounding rock, the wall by the Hole will have carved
# outward past the room radius. verify_carve() line-traces to check.
INJECT_TEST = True
INJECT_CLASS = 'BP_Construction_Cave_HoleBig03_C'
# Absolute spot deep in solid rock, far (~4200) from every room/tunnel -- so any
# open pocket that appears here was carved by our construction in virgin rock,
# not connected to an existing cave. (Set from the terrain profiling.)
INJECT_ISOLATED_POS = (15561.0, -6000.0, -800.0)
_HOOKS = ('GetConstructionBudget', 'CreateCaveGraph')

# Grid of constructions carved into the solid rock beside the level. A vertical
# wall (cols along X, rows along Z) at a fixed Y that's confirmed virgin rock
# (~4000 clear of every room). Each cell carves its own chamber. Keep the count
# MODERATE -- every carver eats the 65536-cell terrain pool; too many overflow it.
GRID_COLS = 4                      # along +X
GRID_ROWS = 4                      # along +Z
GRID_DX = 1600.0
GRID_DZ = 1300.0
GRID_ORIGIN = (15561.0, -6000.0, -800.0)   # centre, in confirmed solid rock
GRID_CLASSES = [                   # cycled across cells for varied carve shapes
    'BP_Construction_Cave_HoleBig03_C',
    'BP_Construction_Cave_Hole_A_C',
    'BP_Construction_Cave_Bunker01_C',
    'BP_Construction_Cave_Tower_C',
]


def _spawn_and_register(gs, this, cls_path, x, y, z):
    """Spawn a construction at (x,y,z) during gen and register its carvers with
    the PLS so the carve pass cuts it into the terrain. Returns (actor, n_carvers)."""
    cls = ue.load(cls_path)
    xf = {'Translation': {'X': x, 'Y': y, 'Z': z},
          'Rotation': {'X': 0.0, 'Y': 0.0, 'Z': 0.0, 'W': 1.0},
          'Scale3D': {'X': 1.0, 'Y': 1.0, 'Z': 1.0}}
    a = gs.BeginDeferredActorSpawnFromClass(
        this, cls, xf, 'ESpawnActorCollisionHandlingMethod::AlwaysSpawn', None,
        'ESpawnActorScaleMethod::MultiplyWithRoot')
    fin = gs.FinishSpawningActor(a, xf, 'ESpawnActorScaleMethod::MultiplyWithRoot')
    regs = 0
    for f in dir(fin):
        if 'Carver' not in f:
            continue
        try:
            cc = getattr(fin, f)
            if getattr(cc, '_class', None) is not None and cc._class._name == CARVER_CLASS:
                cc.AddCarverComponentToProceduralSetup()   # <- the step that makes it carve
                regs += 1
        except Exception:
            pass
    return fin, regs


def _inject_hook(this, p):
    p.original()                                   # build the real room graph first
    if not INJECT_TEST:
        return
    try:
        gs = ue.find('/Script/Engine.Default__GameplayStatics')
        ox, oy, oz = GRID_ORIGIN
        hx = (GRID_COLS - 1) / 2.0
        hz = (GRID_ROWS - 1) / 2.0
        cells, total_regs, i = [], 0, 0
        for row in range(GRID_ROWS):
            for col in range(GRID_COLS):
                x = ox + (col - hx) * GRID_DX
                y = oy
                z = oz + (row - hz) * GRID_DZ
                path = _IDX[GRID_CLASSES[i % len(GRID_CLASSES)]]
                _, regs = _spawn_and_register(gs, this, path, x, y, z)
                cells.append((x, y, z))
                total_regs += regs
                i += 1
        globals()['_inject_at'] = {'center': GRID_ORIGIN, 'radius': 0.0,
                                   'dir': (1.0, 0.0, 0.0), 'pos': GRID_ORIGIN,
                                   'cells': cells}
        print('gen-time grid inject: %d constructions in rock, %d carvers registered'
              % (len(cells), total_regs))
    except Exception as e:
        print('gen-time grid inject FAILED: %r' % e)


def _arm_budget(cls):
    cls.add_to_root()                 # survive rig <-> mission travel (GC)
    for fn in _HOOKS:
        try:
            ue.unhook(cls, fn)
        except Exception:
            pass
    ue.hook(cls, 'GetConstructionBudget', _budget_hook)
    if INJECT_TEST:
        ue.hook(cls, 'CreateCaveGraph', _inject_hook)
    print('armed budget=%d + gen-inject=%s on %s' % (BUDGET, INJECT_TEST, cls._name))


def _on_pls_begin(actor):
    _budget_state['cls'] = actor._class
    _arm_budget(actor._class)


# tear down any previous arming before re-arming
_prev = globals().get('_budget_state')
if _prev is not None:
    try:
        _prev['bp'].remove()
    except Exception:
        pass
    if _prev.get('cls') is not None:
        for fn in _HOOKS:
            try:
                ue.unhook(_prev['cls'], fn)
            except Exception:
                pass
        try:
            _prev['cls'].remove_from_root()
        except Exception:
            pass

_budget_state = {'bp': ue.on_begin_play(PLS, _on_pls_begin), 'cls': None}
_p = live(PLS)
if _p is not None:
    _budget_state['cls'] = _p._class
    _arm_budget(_p._class)
globals()['_budget_state'] = _budget_state


def _rig_gamemode():
    return next((g for g in ue.objects_of_class(GAMEMODE)
                 if g._class._name == 'BP_SpaceRig_Gamemode_C' and not g._flags & TEMPLATE), None)


async def main():
    """Travel to the rig if needed, then launch a fresh run whose level generates
    with the boosted construction budget -- so lots of constructions carve in."""
    gm = _rig_gamemode()
    if gm is None:
        gi = live(GAMEINSTANCE)
        ue.find(GS).OpenLevel(gi, RIG_LEVEL, True, '')
        print('not on the rig -- travelling to %s...' % RIG_LEVEL)
        try:
            gm = await ue.spawned(GAMEMODE,
                                  where=lambda a: a._class._name == 'BP_SpaceRig_Gamemode_C',
                                  timeout=90)
        except ue.Timeout:
            return 'could not reach the Ramrod within 90s'

    gi = live(GAMEINSTANCE)
    rgm = live('/Script/RogueCore.RunGenerationManager')
    pgl = ue.find('/Script/RogueCore.Default__PlanetGenerationLibrary')
    rfl = ue.find('/Script/RogueCore.Default__RunFunctionLibrary')
    runs = [r for b in rgm.GenerateRuns(gi.GetGlobalMissionSeed())
            for r in b.Runs if rfl.HavePlayerUnlockedRun(gi, r)]
    if not 0 <= SELECT < len(runs):
        return 'SELECT=%d out of range (0..%d)' % (SELECT, len(runs) - 1)
    gi.RunManager.StartRun(pgl.CreateRunCreationParameters(runs[SELECT]))
    gm.InstantlyStartMission()
    print('launched with construction budget %d -- waiting for generation...' % BUDGET)

    try:
        pls = await ue.spawned(PLS, timeout=90)
    except ue.Timeout:
        return 'launched, but no ProceduralSetup spawned within 90s'
    for _ in range(60):
        await ue.sleep(0.5)
        if len(pls.Rooms):
            break
    await ue.sleep(6.0)                    # let the carve pass finish integrating
    seen = coverage()
    return 'level generated with %d construction types carved in; fly around to see them' % len(seen)


def verify_carve():
    """Line-trace from the injected room's centre toward the gen-time Hole and in
    a control direction. If the wall by the Hole carved outward, its hit distance
    exceeds the room radius (and the control ~= radius)."""
    import math
    at = globals().get('_inject_at')
    if at is None:
        print('verify_carve: nothing injected (relaunch with INJECT_TEST=True)')
        return
    ksl = ue.find('/Script/Engine.Default__KismetSystemLibrary')
    pc = live('/Script/Engine.PlayerController')
    cx, cy, cz = at['center']
    rad = at['radius']
    S = {'X': cx, 'Y': cy, 'Z': cz}
    RED = {'R': 1.0, 'G': 0.0, 'B': 0.0, 'A': 1.0}
    GRN = {'R': 0.0, 'G': 1.0, 'B': 0.0, 'A': 1.0}

    def wall(dx, dy):
        E = {'X': cx + dx * rad * 3, 'Y': cy + dy * rad * 3, 'Z': cz}
        r = ksl.LineTraceSingle(pc, S, E, 0, False, [], 0, True, RED, GRN, 0.0)
        hit, h = (r if isinstance(r, tuple) else (r, None))
        if hit and h is not None:
            l = h.Location
            return math.dist((cx, cy, cz), (l.X, l.Y, l.Z))
        return None

    inj = wall(*at['dir'][:2])
    ctrl = wall(0.0, 1.0)               # +Y control (no injection there)
    print('room radius        = %.0f' % rad)
    print('wall toward Hole   = %s' % (round(inj) if inj else 'NO HIT (carved through)'))
    print('wall control (+Y)  = %s' % (round(ctrl) if ctrl else 'NO HIT'))
    if inj and ctrl:
        print('=> %s' % ('CARVED: wall by Hole is %.0f further than control'
                         % (inj - ctrl) if inj - ctrl > 200 else
                         'no carve: Hole did not cut the rock (delta %.0f)' % (inj - ctrl)))


globals()['verify_carve'] = verify_carve

print('loaded: %d construction types indexed. budget armed = %d.' % (len(CONSTRUCTIONS), BUDGET))
print('run with --script to relaunch a run whose constructions CARVE into the cave;')
print('or use spawn_few()/spawn_at_camera() for (non-carving) visual-only drops.')
