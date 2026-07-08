# Inject a custom procedural room layout into DRG: Rogue Core.
#
#   run from anywhere:       python repl.py --script rc/inject_room_layout.py
#   then poll what it built:  python repl.py  ->  inject_log
#
# It arms a hook, travels to the Ramrod (RC's Space Rig) if needed, then launches
# a run; the level generates as whatever LAYOUT() returns (a SPIRAL TOWER by
# default) instead of the usual linear cave.
#
# ─────────────────────────────────────────────────────────────────────────────
# How Rogue Core builds a level's room graph (found by tracing generation with
# ue.trace -- see the game-thread call timeline):
#
#   PLS_Base_C::Generate_Graph                     (base wrapper, BlueprintNativeEvent)
#     '- PLS_RC_Random_C::CreateCaveGraph          (BNE override) authors the graph;
#        its body (the event graph) runs:
#            SelectCaveStructure
#            AddGenericBXEStartingRoom()                 -> room 0 (the start / drop pod)
#            BPL_ProceduralLevelSetup.CreateLinearPathFromDNA(...) -> rooms 1..n + tunnels
#   ...then GenerateRoomsFromGraph_Async carves geometry, tunnels connect, etc.
#
# We REPLACE CreateCaveGraph wholesale (never call original) -- one hook on the
# ProceduralSetup itself. CreateCaveGraph is a BlueprintNativeEvent, called
# virtually from Generate_Graph, so it routes through `Func` and is directly
# hookable. (The old approach hooked the BPL library's CreateLinearPathFromDNA
# because same-class BP-script calls used to bypass Func-swap hooks; the host now
# reconstructs a proper script frame for such calls, so hooking the PLS event
# directly works and is simpler -- no BPL lookup, no Last* out-param contract.)
#
# We keep the real start room by calling AddGenericBXEStartingRoom() ourselves,
# then author the path with the native, always-hookable primitives:
#   ProceduralSetup.AddRoom(Location, CanHaveEnemies, RoomGenerator, canBeRotated,
#                           ResourceMultiplier, CanHaveExitElevator)
#                                                     -> (roomID, center, roomNode)
#   ProceduralSetup.ConnectRoomIds(fromID, toID, hasDirt, tunnelParamOverride)
#
# GC pinning: the PLS subclass loads with the procedural level and unloads on
# travel; cls.add_to_root() keeps the hook alive across rig <-> mission.

import ue
import math

TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

PLS = '/Script/RogueCore.ProceduralSetup'
FUNC = 'CreateCaveGraph'

# Room-generator assets to place. Any RMA_* under this folder works; swap freely.
GEN_DIR = '/Game/Maps/Rooms/RoomGenerators/'
PATH_GEN = GEN_DIR + 'RMA_BXE_Tiny_Steps01.RMA_BXE_Tiny_Steps01'
END_GEN = GEN_DIR + 'RMA_BXE_NewLinearX_End01.RMA_BXE_NewLinearX_End01'

SELECT = 0  # which unlocked run to launch

inject_log = []
globals()['inject_log'] = inject_log


# ── layouts: world positions (x, y, z) for the PATH rooms (1..N) ──────────────
# Seeded from the start room's primary exit: `loc`/`direction` are that exit's
# world position and unit facing, so the path emanates OUT of the exit instead of
# orbiting the (large) start room's centre -- which would embed the first rooms in
# the start room and make the connecting tunnel double back. `gap` pushes the
# first room clear of the start room before the layout proper begins.

def _fwd(direction):
    """The exit's horizontal facing as a unit (fx, fy)."""
    dn = math.hypot(direction.X, direction.Y) or 1.0
    return direction.X / dn, direction.Y / dn


def layout_spiral(loc, direction, n=12, radius=3800.0, ang_step=1.15,
                  climb=2200.0, gap=4500.0):
    """A helix rising out past the exit -- ~5.5 rooms per turn. Its axis sits
    `radius + gap` beyond the exit along the facing, and room 1 is on the near
    side (facing back at the exit) so the first tunnel runs straight out."""
    fx, fy = _fwd(direction)
    cx, cy = loc.X + fx * (radius + gap), loc.Y + fy * (radius + gap)
    a0 = math.atan2(-fy, -fx)
    return [(cx + radius * math.cos(a0 + i * ang_step),
             cy + radius * math.sin(a0 + i * ang_step),
             loc.Z + i * climb) for i in range(n)]


def layout_zigzag(loc, direction, n=7, step=6000.0, amp=3500.0, climb=500.0, gap=4500.0):
    """A lightning-bolt corridor advancing along the exit facing, alternating
    side to side."""
    fx, fy = _fwd(direction)
    rx, ry = -fy, fx                       # right = facing rotated +90
    return [(loc.X + fx * (gap + step * i) + rx * (amp if i % 2 == 0 else -amp),
             loc.Y + fy * (gap + step * i) + ry * (amp if i % 2 == 0 else -amp),
             loc.Z + climb * i) for i in range(n)]


LAYOUT = layout_spiral   # <- swap for layout_zigzag, or write your own


def live(cls):
    return next((o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE), None)


def _primary_exit(room):
    """Location + unit facing of the start room's primary exit (fallback: its
    first entrance). RoomEntrances are populated by AddGenericBXEStartingRoom, so
    this reads correctly while CreateCaveGraph runs, before any carving."""
    ents = room.RoomEntrances
    pick = None
    for j in range(len(ents)):
        e = ents[j]
        if str(e.Priority).endswith('Primary'):
            pick = e
            break
    if pick is None and len(ents):
        pick = ents[0]
    return (pick.Location, pick.Direction) if pick else (None, None)


# ── the substitute: runs in place of CreateCaveGraph ─────────────────────────
def _build_graph(this, p):
    try:
        # Keep the real start room (drop pod) -- it becomes room id 0.
        this.AddGenericBXEStartingRoom()
        loc, direction = _primary_exit(this.Rooms[0])
        if loc is None:
            inject_log.append('inject: start room has no exit -- falling back')
            return p.original()
        pathgen, endgen = ue.load(PATH_GEN), ue.load(END_GEN)

        ids = [0]                                  # start room authored above = id 0
        pts = LAYOUT(loc, direction)               # path emanates from the start exit
        for i, (x, y, z) in enumerate(pts):
            is_end = (i == len(pts) - 1)
            gen = endgen if is_end else pathgen
            # AddRoom's out-params (center, roomNode) come back after the id.
            res = this.AddRoom({'X': x, 'Y': y, 'Z': z}, True, gen, True, 1.0, is_end)
            ids.append(res[0] if isinstance(res, tuple) else res)

        for a, b in zip(ids, ids[1:]):             # chain start(0) -> our rooms
            this.ConnectRoomIds(a, b, False, None)

        inject_log.append('injected %s: rooms=%s' % (LAYOUT.__name__, ids))
        # NOTE: p.original() deliberately NOT called -> we replace the whole graph.
    except Exception as e:
        inject_log.append('inject FAILED (%r) -- falling back to original' % e)
        return p.original()


def _arm_on(cls):
    cls.add_to_root()                 # survive rig <-> mission travel (GC)
    try:
        ue.unhook(cls, FUNC)
    except Exception:
        pass
    ue.hook(cls, FUNC, _build_graph)
    inject_log.append('armed+rooted %s (rooted=%s)' % (cls._name, cls.is_rooted()))


def _on_pls_begin(actor):
    # The concrete PLS subclass (PLS_RC_Random_C, ...) is known once the actor
    # spawns; arm CreateCaveGraph on it before generation runs.
    _state['cls'] = actor._class
    _arm_on(actor._class)


# tear down any previous arming (and unroot the old class) before re-arming
_prev = globals().get('_inject_state')
if _prev is not None:
    try:
        _prev['bp'].remove()
    except Exception:
        pass
    if _prev.get('cls') is not None:
        try:
            ue.unhook(_prev['cls'], FUNC)
            _prev['cls'].remove_from_root()
        except Exception:
            pass

_state = {'bp': ue.on_begin_play(PLS, _on_pls_begin), 'cls': None}
del inject_log[:]
_pls = live(PLS)                      # arm now too, if we're already in a mission
if _pls is not None:
    _state['cls'] = _pls._class
    _arm_on(_pls._class)
globals()['_inject_state'] = _state


# ── travel to the rig if needed, then launch so the injected layout generates ─
# Async: the runtime drives main() across engine ticks, so we can OpenLevel back
# to the Ramrod (RC's Space Rig hub) and await it before launching -- no manual
# travel first. The GameInstance is the stable anchor across the travel; the rig
# gamemode (BP_SpaceRig_Gamemode_C) beginning play marks the load complete.

RIG_LEVEL = 'LVL_Ramrod_MAIN'    # RC's Space Rig hub (contains BP_SpaceRig_Gamemode_C)


def _rig_gamemode():
    return next((g for g in ue.objects_of_class('/Script/Engine.GameModeBase')
                 if g._class._name == 'BP_SpaceRig_Gamemode_C' and not g._flags & TEMPLATE), None)


async def main():
    gm = _rig_gamemode()
    if gm is None:
        gi = live('/Script/Engine.GameInstance')
        gs = ue.find('/Script/Engine.Default__GameplayStatics')
        gs.OpenLevel(gi, RIG_LEVEL, True, '')
        inject_log.append('not on the rig -- travelling to %s...' % RIG_LEVEL)
        try:
            gm = await ue.spawned('/Script/Engine.GameModeBase',
                                  where=lambda a: a._class._name == 'BP_SpaceRig_Gamemode_C',
                                  timeout=90)
        except ue.Timeout:
            return 'armed, but could not reach the Ramrod within 90s'

    gi = live('/Script/Engine.GameInstance')
    rgm = live('/Script/RogueCore.RunGenerationManager')
    pgl = ue.find('/Script/RogueCore.Default__PlanetGenerationLibrary')
    rfl = ue.find('/Script/RogueCore.Default__RunFunctionLibrary')
    runs = [r for b in rgm.GenerateRuns(gi.GetGlobalMissionSeed())
            for r in b.Runs if rfl.HavePlayerUnlockedRun(gi, r)]
    if not 0 <= SELECT < len(runs):
        return 'armed, but SELECT=%d out of range (0..%d)' % (SELECT, len(runs) - 1)
    gi.RunManager.StartRun(pgl.CreateRunCreationParameters(runs[SELECT]))
    gm.InstantlyStartMission()
    return 'armed + launched -- level will generate as %s; poll inject_log' % LAYOUT.__name__
