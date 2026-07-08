# Inject a custom procedural room layout into Deep Rock Galactic (/Script/FSD),
# built from a room generator we construct at runtime -- a simple DOME.
#
#   run from the Space Rig:  python repl.py --script drg/inject_room_layout.py
#   then poll what it built:  python repl.py  ->  inject_log
#
# It arms a hook, then launches a mission; the level generates as whatever
# LAYOUT() returns (a wide, flat SPIRAL of domes by default) instead of the
# usual cave, with every node carved from a RoomGenerator we build in memory.
#
# ─────────────────────────────────────────────────────────────────────────────
# How DRG builds a level's room graph (found by tracing generation with hooks):
#
#   PLS_Base_C::Generate_Graph              (base wrapper, BlueprintNativeEvent)
#     '- <PLS subclass>::CreateCaveGraph    (per-objective override) authors the
#        entire room graph via the native AddRoom / ConnectRoomIds primitives.
#   ...then GenerateRoomsFromGraph_Async carves geometry from the room graph,
#   CreateTunnelPaths / CarveTunnels connect them, the Generate_Pass_* decorate,
#   and the objective / encounter spawners populate the rooms.
#
# We REPLACE CreateCaveGraph wholesale (never call original) and author our own
# rooms with the native, always-hookable primitives:
#   ProceduralSetup.AddRoom(Location, CanHaveEnemies, RoomGenerator, canBeRotated,
#                           ResourceMultiplier)        -> (roomID, center, roomNode)
#   ProceduralSetup.ConnectRoomIds(fromID, toID, hasDirt, tunnelParameterOverride)
# CreateCaveGraph is void, so unlike Rogue Core's CreateLinearPathFromDNA there is
# no out-param contract to honor -- we just build the graph and return.
#
# ── the custom room generator ────────────────────────────────────────────────
# A RoomGenerator (/Script/FSD.RoomGenerator) is a plain UObject asset whose shape
# lives in a `RoomFeatures` array of RoomFeature subobjects. The stock rooms mix
# FloodFillLine (the cavity), EntranceFeature (tunnel attach points) and decor. We
# build a minimal one with ue.new_object:
#   * one FloodFillLine whose Points sweep an ellipsoid cavity: HRange = floor
#     radius, VRange / Cielingheight = dome height, FloorDepth/FloorAngle = a flat
#     floor. A LINE primitive needs >= 2 points to carve, so we place two a hair
#     apart -> a dome.
#   * a ring of EntranceFeatures so ConnectRoomIds tunnels have somewhere to land.
# The generator is add_to_root()'d so GC doesn't reclaim it, and reused for every
# node (exactly as the stock RMA_* assets are shared across rooms).
#
# ── two things that make the hook reliable ───────────────────────────────────
#   1. CreateCaveGraph is a BlueprintNativeEvent overridden on the concrete PLS
#      subclass (PLS_Extraction_C, ...), so we arm on the class the actor reports
#      at BeginPlay -- objective-agnostic. AddRoom / ConnectRoomIds are native and
#      Final on the base ProceduralSetup, so they are always hookable/callable.
#   2. GC pinning. The PLS subclass loads with the procedural level and unloads on
#      travel; once GC'd a hook on it goes stale. cls.add_to_root() keeps it (and
#      our generator) alive across travelling rig <-> mission.

import ue
import math

TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject
RF_TRANSIENT = ue.EObjectFlags.RF_Transient

PLS = '/Script/FSD.ProceduralSetup'
FUNC = 'CreateCaveGraph'

# ── dome shape ───────────────────────────────────────────────────────────────
DOME_RADIUS = 1300.0    # floor radius (FloodFill HRange)
DOME_HEIGHT = 4500.0    # dome ceiling height (FloodFill VRange / Cielingheight)
DOME_ENTRANCES = 4      # tunnel attach points evenly around the rim
DOME_SEGMENT = 100.0    # spacing between the line's two carve points (>0 to carve)

SELECT = 0              # which available mission to launch

inject_log = []
globals()['inject_log'] = inject_log


def _v(x, y, z):
    return {'X': float(x), 'Y': float(y), 'Z': float(z)}


def _rot(pitch, yaw, roll):
    return {'Pitch': float(pitch), 'Yaw': float(yaw), 'Roll': float(roll)}


def live(cls):
    return next((o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE), None)


# ── layouts: return world positions (x, y, z), one per room, from the origin ──
# Room 0 is the start (drop pod); keep it at the origin. The rest chain after it.

def layout_spiral(n=8, radius=11400.0, ang_step=1.15, climb=667.0):
    """A wide, shallow helix -- ~5.5 rooms per turn."""
    return [(0.0, 0.0, 0.0)] + [
        (radius * math.cos(i * ang_step), radius * math.sin(i * ang_step), i * climb)
        for i in range(1, n)
    ]


def layout_zigzag(n=8, step_x=6500.0, amp_y=3500.0, climb=600.0):
    """A lightning-bolt corridor advancing in +X, alternating in Y."""
    return [(0.0, 0.0, 0.0)] + [
        (step_x * i, amp_y if i % 2 else -amp_y, climb * i) for i in range(1, n)
    ]


LAYOUT = layout_spiral   # <- swap for layout_zigzag, or write your own


# ── build the custom dome RoomGenerator ──────────────────────────────────────
def build_dome_generator(radius=DOME_RADIUS, height=DOME_HEIGHT,
                         entrances=DOME_ENTRANCES, seg=DOME_SEGMENT):
    gen = ue.new_object('/Script/FSD.RoomGenerator', flags=RF_TRANSIENT)
    gen.Bounds = radius + 200.0

    # The cavity: two flood-fill points a hair apart sweep a flat-floored,
    # domed-ceiling ellipsoid. (A single point is a degenerate line and won't
    # carve -- a line primitive needs >= 2 points.)
    ffl = ue.new_object('/Script/FSD.FloodFillLine', outer=gen, flags=RF_TRANSIENT)

    def point(x):
        return {
            'Location': _v(x, 0.0, 0.0),
            'HRange': radius, 'VRange': height, 'Cielingheight': height,
            'FloorDepth': 0.0, 'FloorAngle': 0.0, 'HeightScale': 1.0,
            'CielingNoiseRange': 0.0, 'FloorNoiseRange': 0.0, 'WallNoiseRange': 0.0,
        }

    ffl.Points = [point(-seg / 2.0), point(seg / 2.0)]
    ffl.UseDetailNoise = False
    feats = [ffl]

    # A ring of entrances so tunnels (ConnectRoomIds) have somewhere to attach.
    for i in range(entrances):
        ang = 2.0 * math.pi * i / entrances
        e = ue.new_object('/Script/FSD.EntranceFeature', outer=gen, flags=RF_TRANSIENT)
        e.Location = _v(radius * math.cos(ang), radius * math.sin(ang), 0.0)
        e.Direction = _rot(0.0, math.degrees(ang), 0.0)   # face outward
        e.EntranceType = 'ECaveEntranceType::EntranceAndExit'
        e.Priority = 'ECaveEntrancePriority::Primary'
        feats.append(e)

    gen.RoomFeatures = feats
    gen.add_to_root()                 # survive GC + travel; reused for every node
    return gen


# ── the substitute: runs in place of CreateCaveGraph ─────────────────────────
def _build_graph(this, p):
    try:
        dome = _state['dome']
        ids = []
        for i, (x, y, z) in enumerate(LAYOUT()):
            is_start = (i == 0)
            # DRG AddRoom: out-params (outCenter, outRoom) sit mid-signature and
            # come back after the roomID, so the result is (roomID, center, node).
            # The start room (0) holds the drop pod: no enemies, not rotated.
            res = this.AddRoom(_v(x, y, z), not is_start, dome, not is_start, 1.0)
            ids.append(res[0] if isinstance(res, tuple) else res)

        for a, b in zip(ids, ids[1:]):            # chain start(0) -> our rooms
            this.ConnectRoomIds(a, b, True, None)

        inject_log.append('injected %s: %d dome rooms=%s' % (LAYOUT.__name__, len(ids), ids))
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
    # The concrete PLS subclass is known once the actor spawns; arm CreateCaveGraph
    # on it before generation runs.
    _state['cls'] = actor._class
    _arm_on(actor._class)


# tear down any previous arming (unhook, unroot the old class + old generator)
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
    if _prev.get('dome') is not None:
        try:
            _prev['dome'].remove_from_root()      # let the old generator be GC'd
        except Exception:
            pass

del inject_log[:]
_state = {
    'bp': ue.on_begin_play(PLS, _on_pls_begin),
    'cls': None,
    'dome': build_dome_generator(),
}
inject_log.append('dome generator %s built (rooted=%s)'
                  % (_state['dome']._name, _state['dome'].is_rooted()))
_pls = live(PLS)                      # arm now too, if we're already in a mission
if _pls is not None:
    _state['cls'] = _pls._class
    _arm_on(_pls._class)
globals()['_inject_state'] = _state


# ── travel to the rig if needed, then launch so the layout generates ─────────
# Async: the runtime drives main() across engine ticks, so we can OpenLevel back
# to the Space Rig and await it before launching -- no manual travel first. The
# GameInstance is the stable anchor across the travel; the rig gamemode beginning
# play marks the load complete.

async def main():
    gm = live('/Script/FSD.FSDGameModeSpaceRig')
    if gm is None:
        gi = live('/Script/Engine.GameInstance')
        gs = ue.find('/Script/Engine.Default__GameplayStatics')
        gs.OpenLevel(gi, 'LVL_SpaceRig', True, '')
        inject_log.append('not on the rig -- travelling to LVL_SpaceRig...')
        try:
            await ue.spawned('/Script/FSD.FSDGameModeSpaceRig', timeout=90)
        except ue.Timeout:
            return 'armed, but could not reach the Space Rig within 90s'
        gm = live('/Script/FSD.FSDGameModeSpaceRig')
        if gm is None:
            return 'armed, but the Space Rig gamemode was not found after travel'

    gi = live('/Script/Engine.GameInstance')
    mgm = live('/Script/FSD.MissionGenerationManager')
    missions = mgm.GetAvailableMissions()
    if not 0 <= SELECT < len(missions):
        return 'armed, but SELECT=%d out of range (0..%d)' % (SELECT, len(missions) - 1)
    m = missions[SELECT]
    gi.SetSelectedMission(m, True)
    gm.InstantlyStartMission()
    biome = m.Biome._name if m.Biome else '?'
    obj = m.PrimaryObjective._name if m.PrimaryObjective else '?'
    return ('armed + launched [%d] %s | %s -- level will generate as %s of domes; poll inject_log'
            % (SELECT, biome, obj, LAYOUT.__name__))
