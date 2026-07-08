# DRG: Rogue Core (`/Script/RogueCore`)

Example REPL scripts for Rogue Core, the rogue-lite built on the Deep Rock
(FSD) engine. Same lineage as the `../drg/` scripts, but the module is
`/Script/RogueCore` and a session is a **run of stages**, not a single mission.

Run any of these with the REPL client:

```
python repl.py --script rc/start_mission.py
python repl.py --script rc/inject_room_layout.py
```

## Scripts

| script | what it does |
|--------|--------------|
| `start_mission.py`      | list unlocked runs and launch one (`StartRun` + `InstantlyStartMission`) |
| `inject_room_layout.py` | hook `CreateCaveGraph` on the ProceduralSetup and generate a custom room layout (spiral tower / zigzag); travels to the rig first if needed |

The freecam lives in [`../drg/freecam.py`](../drg/freecam.py) — one script that
auto-detects base DRG vs Rogue Core (game module, actor-spawn arity, sky cubemap)
and runs on both. Load it with `python repl.py --script drg/freecam.py`.

## Mission-start flow

```
RunGenerationManager.GenerateRuns(GetGlobalMissionSeed())  -> [GeneratedRunsForBiome]
    bundle.Biome / .PlanetZone / .Runs
    run.Biome .PlanetZone .RunDepth .RunSeed .DNA .Mutator .Run
PlanetGenerationLibrary.CreateRunCreationParameters(run)   -> RunCreationParameters
RunManager.StartRun(params)                                -- primes active run/stage
BP_SpaceRig_Gamemode_C.InstantlyStartMission()             -- travels into LVL_Procedural
RunFunctionLibrary.HavePlayerUnlockedRun(worldCtx, run)    -- is it selectable?
```

`RunManager` and `RunGenerationManager` are subobjects of the `GameInstance`
(`BP_GameInstance_C`). In a mission the gamemode is `GM_BXE_C` — it has
`GotoNextLevel()`, `LoadToStage(stage)`, `LoadSpaceRig()`, `EndRunNormally()`.

RC's **Space Rig is the Ramrod ship**, map `/Game/Maps/Spaceship/LVL_Ramrod_MAIN`
(gamemode `BP_SpaceRig_Gamemode_C`). `inject_room_layout.py`'s async `main()`
`OpenLevel`s there if you aren't already on it, so it runs from anywhere.

## Level generation: call graph & timeline

The `ProceduralSetup` actor (`PLS_RC_Random_C`) drives generation. Observed call
nesting on the game thread (from a `ue.trace` over level init):

```
PLS_Base_C::Generate_Graph                        (base wrapper, BlueprintNativeEvent)
  PLS_RC_Random_C::CreateCaveGraph                (BNE override) -- its body authors the graph:
      SelectCaveStructure
      AddGenericBXEStartingRoom()                 -> room 0 (the start / drop pod)
      BPL_ProceduralLevelSetup_C::CreateLinearPathFromDNA(...)  -> rooms 1..n + tunnels
  SaveInitialState -> CallGenerationComplete
StartGenerationOnClient -> GenerateLandscapeFromData -> BaseLayerCommitDone
Generate_Tunnels -> (per-room construction actors begin play: doors, entrances, ...)
...then GenerateRoomsFromGraph_Async carves geometry from the room graph.
```

`inject_room_layout.py` **replaces `CreateCaveGraph` wholesale** (skips
`original()`): it re-adds the real start room with `AddGenericBXEStartingRoom()`,
then authors the path itself. `CreateCaveGraph` is a `BlueprintNativeEvent`
called virtually from `Generate_Graph`, so it routes through `Func` and is
directly hookable — no need to touch the BPL library.

Native graph-building primitives on `ProceduralSetup` (native `Final`, always
hookable and directly callable):

```
AddRoom(Location, CanHaveEnemies, RoomGenerator, canBeRotated,
        <out center>, <out roomNode>, ResourceMultiplier, CanHaveExitElevator) -> roomID
    # the out-params sit mid-signature, so the call takes 6 in-args and returns
    # (roomID, center, roomNode).
ConnectRoomIds(fromID, toID, hasDirt, tunnelParamOverride) -> tunnelID
```

`PLS.Rooms` is `[RoomNode]` (`.ID .position .Radius .EntranceIDs .ExitIDs
.RoomEntrances .RoomGenerators .CanHaveEnemies .CanHaveExitElevator`). Each
`RoomEntrance` has `.Location .Direction .EntranceType .Priority`. Room-generator
assets live under `/Game/Maps/Rooms/RoomGenerators/RMA_*`.

### Seed the path from the start room's exit

The start room (`AddGenericBXEStartingRoom`) is large (radius ~6500–7000). A
layout that orbits the graph origin (the start's centre) embeds the first rooms
inside it, and the connecting tunnel doubles back. The injector reads the start
room's **primary exit** (`RoomEntrances`, populated by the time `CreateCaveGraph`
runs) and grows the path OUT of that exit, `gap` units clear of the start room —
so the first tunnel runs straight out. This is layout geometry, not a hook issue.

## Hooking notes

1. **BP-script functions are now fully hookable.** `ue.hook` forces `FUNC_Native`
   on the target so the VM dispatches through `Func` (the trampoline), and the
   host reconstructs a proper script frame so `params.original()` can re-enter the
   real bytecode — including for same-class / local-final calls that used to
   bypass a Func-swap hook. That is why we can hook `CreateCaveGraph` on the
   ProceduralSetup directly now, instead of the cross-class BPL library function
   `CreateLinearPathFromDNA` the way earlier versions had to.

2. **Pin per-level classes against GC.** `PLS_RC_Random_C` loads with the
   procedural level and unloads on travel; once GC'd, a hook on it goes stale and
   is reclaimed. `cls.add_to_root()` keeps it alive. Root-set API:
   `obj.add_to_root()` / `obj.remove_from_root()` / `obj.is_rooted()`. Arm on
   `ue.on_begin_play(PLS, cb)` and hook `cb`'s `actor._class`, so it re-arms on
   whatever concrete subclass spawns and survives travelling rig <-> mission.

## Debugging aids used to map all this

- `obj._class.functions()` / `.function_info(name).signature` — reflect UFunctions.
- `ue.hook(cls, name, cb)` with `cb(this, params)`; `params.<arg>` reads/writes an
  arg, `params.original()` runs the real function (skip it to fully replace).
- `ue.on_begin_play(class_path, cb)` — arm hooks when an actor spawns.
- `ue.trace(func, class_, opcodes, max_events)` — bytecode tracer; drain with
  `.drain()`. Each event is `{op, raw, offset, func}` (`func` resolved at record
  time). Records game-thread frames only (worker-thread ops are forwarded, not
  recorded). Bracket the generation window (start on `BeginGenerating`, stop on
  `FinalCommitDone`) and collapse consecutive events by `func` to get the call
  timeline above — that's how `CreateCaveGraph` was pinned as the layout builder.
- `ue.new_object(cls, outer, name, flags)` — construct UObjects at runtime (see
  the `../drg/` dome-generator recipe for building a custom `RoomGenerator`).
