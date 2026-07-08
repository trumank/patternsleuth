# Deep Rock Galactic (`/Script/FSD`)

Example REPL scripts for base DRG (the FSD engine, an older UE4 build than
`../rc/` Rogue Core). A session is a single **mission**: pick one on the Space
Rig, travel into `LVL_Procedural`, and a procedural cave is generated.

Run any of these with the REPL client:

```
python repl.py --script drg/open_level.py
python repl.py --script drg/start_mission.py
python repl.py --script drg/inject_room_layout.py
```

## Scripts

| script | what it does |
|--------|--------------|
| `open_level.py`        | `OpenLevel(LVL_SpaceRig)` and block until the destination loads |
| `start_mission.py`     | list available missions and launch one, streaming each generation milestone |
| `freecam.py`           | toggleable free-fly camera (press `.` in game) |
| `inject_room_layout.py`| build a custom **dome** RoomGenerator in memory and replace graph construction with a spiral of domes |
| `missions_hud.py`      | right-side overlay listing every available mission on the Space Rig, composed like the game's own mission-board widgets (hexagon type badge, `Icons_Complexity`/`Icons_Length` pips, hexagon-framed warning/anomaly icons, resource icons, Rift Soft font) — asset choices read straight out of `ITM_MisSel_*` widget trees; self-heals across travel |
| `mission_builder.py`   | Space-Rig form widget to assemble a **custom mission from scratch** — cyclers for type / biome / complexity / length / mutator / challenge / secondary, toggle chips for warnings — then LAUNCH builds a fresh `GeneratedMission` and travels in via `SetSelectedMission` + `InstantlyStartMission` |
| `widget_picker.py`     | live **UMG inspector** (browser-devtools-style element picker) — hover to highlight the widget under the cursor and read its class, tree path, bounding box, sizes, visibility and text in a corner panel. Probes the running game with a Slate-style top-down hit-test: it walks *down* from the on-screen roots (HUD tree + in-viewport UserWidgets), descending only into children whose cached geometry contains the cursor, and picks the smallest hit. Visits ~tens–hundreds of widgets per scan (sub-ms) rather than the ~80k that exist. `\` toggle · `L` lock · `[` `]` walk the tree · `P` dump all reflected properties to the REPL |

## Mission-start flow

```
MissionGenerationManager.GetAvailableMissions()   -> [GeneratedMission]
GameInstance.SetSelectedMission(mission, True)
FSDGameModeSpaceRig.InstantlyStartMission()        -- travels into LVL_Procedural
```

`MissionGenerationManager` is a subobject of the `GameInstance`. On the rig the
gamemode is `BP_SpaceRig_Gamemode_C` (a subclass of `FSDGameModeSpaceRig`).

---

## Level generation: call graph & timeline

The `ProceduralSetup` actor (concrete subclass `PLS_<Objective>_C`, e.g.
`PLS_Extraction_C`, base `PLS_Base_C` / `PLS_CaveGenerator_C`) drives generation.
Observed **call nesting** (from bracketing each stage ENTER/EXIT, see below):

```
BeginGenerating                              ProceduralSetup (native)
Generate_Graph {                             PLS_Base_C          (BlueprintNativeEvent)
    CreateCaveGraph {                        PLS_<Objective>_C   (BNE override) ← authors the graph
        AddRoom(start)                       -> room 0
        AddRoom + ConnectRoomIds  x(n-1)     -> rooms 1..n, chained
    }
}
CreateTunnelPaths                            ProceduralSetup (native)
CreateGraphSecondPass                        PLS_Base_C
GenerateRoomsFromGraph_Async                 carves room geometry from the graph
Generate_Tunnels / CarveTunnels / FillTunnels_Async
Generate_Pass_0 .. Generate_Pass_7           decoration / carving passes (server + client)
CreateSpawn / Generate_CriticalLargePass
InitializeObjectives
SpawnObjectiveCriticalItems / SpawnObjectiveEncounter
SpawnCriticalItems / SpawnItems / SpawnDebrisItems / SpawnSpecialEvents / SpawnEncounters
Final Pass -> FinalCommitDone                level fully committed
```

**Completion timeline** (`levelgen_trace.py`, Salt Caves 1st Extraction — each
line is a stage completing, `dur` = time spent in that stage's `original()`).
Note stages complete in *exit* order, so a nested callee (`CreateCaveGraph`)
finishes before its caller (`Generate_Graph`):

```
  1  t=     0.0  dur=  3.00  BeginGenerating
  2  t=     0.0  dur=  0.00  InitializeObjectives
  3  t=     0.0  dur=  0.00  CreateCaveGraph        ← rooms authored here
  5  t=     0.0  dur=  0.00  Generate_Graph
  8  t=    82.0  dur=  0.00  GenerateRoomsFromGraph_Async
 11  t=   278.0  dur= 60.00  CreateTunnelPaths
 16  t=   278.0  dur= 60.00  Generate_Tunnels
 21  t=   406.0  dur=  0.00  Generate_Pass_0
 28  t=   443.0  dur= 16.00  CreateSpawn
 36  t=   552.0  dur= 87.00  SpawnSpecialEvents
 54  t=  2135.0  dur=  0.00  SpawnObjectiveEncounter
 57  t=  2367.0  dur= 29.00  SpawnEncounters
 68  t=  3481.0  dur=  0.00  Generate_Pass_7
 71  t=  5521.0  dur=  4.00  Final Pass
 72  t=  5521.0  dur=  4.00  FinalCommitDone
```

Key takeaway: **`CreateCaveGraph` (per-objective BNE override) authors the entire
room graph** via the native `AddRoom` / `ConnectRoomIds` primitives, wrapped by
the base `Generate_Graph`. Everything downstream consumes the room graph.

---

## Tracing usage

Everything above was found live with hooks. Reusable recipes:

### 1. Time every pipeline stage (`levelgen_trace.py`)

Hook each stage on the concrete `PLS_*` class; each hook times `params.original()`.

```python
import time
STAGES = ['BeginGenerating', 'CreateCaveGraph', 'Generate_Graph',
          'GenerateRoomsFromGraph_Async', 'CreateTunnelPaths', 'Generate_Tunnels',
          'Generate_Pass_0', ..., 'Final Pass', 'FinalCommitDone']
log = []
def _make(name):
    def cb(this, params):
        t = time.monotonic()
        try: params.original()
        finally: log.append((name, (time.monotonic()-t)*1000))
    return cb
# arm on the actor's concrete class as it spawns (objective-agnostic)
ue.on_begin_play('/Script/FSD.ProceduralSetup',
                 lambda a: [ue.hook(a._class, s, _make(s)) for s in STAGES])
```

Stages absent on a subclass simply raise on `ue.hook` — catch and skip them.

### 2. Reveal call nesting (which stage calls what)

Bracket each candidate with ENTER/EXIT and log the primitive calls in between.
This is how `CreateCaveGraph` was pinned as the room-graph author.

```python
seq = []
def bracket(label):
    def cb(this, p):
        seq.append('ENTER ' + label); r = p.original(); seq.append('EXIT  ' + label); return r
    return cb
def log_add(this, p):        # AddRoom is native + Final on the base class
    l = p.Location
    seq.append('    AddRoom (%.0f,%.0f,%.0f) gen=%s' % (l.X, l.Y, l.Z, p.RoomGenerator._path))
    return p.original()

ue.hook('/Script/FSD.ProceduralSetup', 'AddRoom', log_add)
ue.hook('/Script/FSD.ProceduralSetup', 'ConnectRoomIds',
        lambda t, p: (seq.append('    Connect %s->%s' % (p.fromID, p.toID)), p.original())[1])
for cls, fn in [('PLS_Base_C', 'Generate_Graph'), ('PLS_Extraction_C', 'CreateCaveGraph')]:
    ue.hook(find_class(cls), fn, bracket(fn))   # find_class walks the class list by name
```

Yields, per generation:

```
ENTER Generate_Graph
  ENTER CreateCaveGraph
        AddRoom (0,0,0)          gen=.../RMA_NewLinearY_Start02
        AddRoom (7090,144,-3651) gen=.../RMA_NewLinearX_End01
        Connect 0->1
        AddRoom (13853,96,-7017) gen=.../RMA_Medium02
        Connect 1->2
  EXIT  CreateCaveGraph
EXIT  Generate_Graph
```

### Hooking notes

- **`AddRoom` / `ConnectRoomIds`** are `Final Native` on the base
  `/Script/FSD.ProceduralSetup` — hook once there; every subclass shares them, no
  per-class rooting needed. They are also directly *callable* (`this.AddRoom(...)`).
- **`CreateCaveGraph` / `Generate_Graph`** are `BlueprintNativeEvent`s;
  `CreateCaveGraph` is overridden per objective, so arm it on the class the actor
  reports at `BeginPlay`.
- **Pin per-level classes against GC.** `PLS_*` (and any RoomGenerator you build)
  load with the procedural level and unload on travel; `cls.add_to_root()` /
  `obj.add_to_root()` keeps a hook or generator alive across rig <-> mission.
- Hooking BP-script (non-native) stages and calling `params.original()` on them
  requires the host's script-frame reconstruction (see `libue` /
  `bindings.rs` `run_bytecode_hook`); older builds crashed here.

---

## Room-graph primitives

```
ProceduralSetup.AddRoom(Location, CanHaveEnemies, RoomGenerator, canBeRotated,
                        ResourceMultiplier)              -> (roomID, center, roomNode)
    # NOTE (older engine): out-params outCenter/outRoom sit MID-signature, so the
    # call takes 5 in-args and returns (roomID, center, roomNode) as a tuple.
ProceduralSetup.ConnectRoomIds(fromID, toID, hasDirt, tunnelParameterOverride) -> tunnelID
```

`PLS.Rooms` is `[RoomNode]` (`.ID .Position .Radius .EntranceIDs .ExitIDs
.RoomGenerators .CanHaveEnemies .ResourceMultiplier`). Stock room-generator
assets live under `/Game/Maps/Rooms/RoomGenerators/RMA_*` (e.g.
`RMA_NewLinearY_Start02` start, `RMA_Medium02` mid, `RMA_NewLinearX_End01` end).

---

## Custom RoomGenerator recipe (the dome)

A `RoomGenerator` (`/Script/FSD.RoomGenerator`) is a plain UObject asset whose
shape is a `RoomFeatures` array of `RoomFeature` subobjects. Build one at runtime
with `ue.new_object`:

- **`FloodFillLine`** — the cavity. Its `Points` sweep an ellipsoid: `HRange`
  (floor radius), `VRange` / `Cielingheight` (dome height), `FloorDepth` /
  `FloorAngle` (a flat floor). A **line primitive needs ≥ 2 points to carve**, so
  place two a hair apart for a dome. Point fields: `Location, HRange, VRange,
  Cielingheight, FloorDepth, FloorAngle, HeightScale, CielingNoiseRange,
  FloorNoiseRange, WallNoiseRange` (note the engine's `Ciel*` spelling).
- **`EntranceFeature`** — tunnel attach points, one ring around the rim. Fields:
  `Location, Direction (Rotator), EntranceType='ECaveEntranceType::EntranceAndExit',
  Priority='ECaveEntrancePriority::Primary'`.

```python
gen = ue.new_object('/Script/FSD.RoomGenerator', flags=ue.EObjectFlags.RF_Transient)
gen.Bounds = radius + 200.0
ffl = ue.new_object('/Script/FSD.FloodFillLine', outer=gen, flags=RF_TRANSIENT)
ffl.Points = [dome_point(-seg/2), dome_point(seg/2)]     # two points -> a dome
gen.RoomFeatures = [ffl] + entrances
gen.add_to_root()                                        # survive GC + travel
```

`inject_room_layout.py` puts it all together: build the dome generator, replace
`CreateCaveGraph` (author a spiral of domes via `AddRoom`/`ConnectRoomIds`, never
calling `original()`), travel to the rig if needed, and launch.
