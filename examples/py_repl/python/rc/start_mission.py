# Select and start a mission in DRG: Rogue Core.
#
#   run from the Space Rig:  python repl.py --script rc/start_mission.py
#
# Rogue Core is built on the Deep Rock (FSD) engine but ships as /Script/RogueCore
# and structures a session as a rogue-lite "run" of stages rather than a single
# DRG mission. The launch flow:
#
#   1. RunGenerationManager.GenerateRuns(seed) -> [GeneratedRunsForBiome]
#        each bundle has .Biome, .PlanetZone, and .Runs ([GeneratedRunForBiome]);
#        each run has .Biome .PlanetZone .RunDepth .RunSeed .DNA .Mutator .Run
#   2. PlanetGenerationLibrary.CreateRunCreationParameters(run) -> RunCreationParameters
#   3. RunManager.StartRun(params)                 -- primes the active run/stage
#   4. SpaceRig gamemode.InstantlyStartMission()   -- travels into LVL_Procedural
#
# RunFunctionLibrary.HavePlayerUnlockedRun(worldCtx, run) says which runs are
# actually selectable (depth/unlock gated). We list unlocked runs and launch
# SELECT. The RunGenerationManager is a subobject of the GameInstance.

import ue

SELECT = 0
TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject


def live(cls):
    return next((o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE), None)


def name(o):
    return o._name if o else '?'


def label(run):
    return '%-22s %-16s seed=%-10s dna=%s' % (
        name(run.Biome), run.RunDepth, run.RunSeed, name(run.DNA))


def main():
    gm = next((g for g in ue.objects_of_class('/Script/Engine.GameModeBase')
               if g._class._name == 'BP_SpaceRig_Gamemode_C' and not g._flags & TEMPLATE), None)
    if gm is None:
        return 'not on the Space Rig -- open the rig first (rc: gm.LoadSpaceRig())'

    gi = live('/Script/Engine.GameInstance')
    rgm = live('/Script/RogueCore.RunGenerationManager')   # GameInstance subobject
    pgl = ue.find('/Script/RogueCore.Default__PlanetGenerationLibrary')
    rfl = ue.find('/Script/RogueCore.Default__RunFunctionLibrary')

    roster = []
    for bundle in rgm.GenerateRuns(gi.GetGlobalMissionSeed()):
        for run in bundle.Runs:
            roster.append((run, bool(rfl.HavePlayerUnlockedRun(gi, run))))

    unlocked = [r for r, ok in roster if ok]
    print('available runs (* = unlocked):')
    for run, ok in roster:
        print('  %s %s' % ('*' if ok else ' ', label(run)))
    if not 0 <= SELECT < len(unlocked):
        return 'SELECT=%d out of range (0..%d unlocked)' % (SELECT, len(unlocked) - 1)

    chosen = unlocked[SELECT]
    gi.RunManager.StartRun(pgl.CreateRunCreationParameters(chosen))
    gm.InstantlyStartMission()
    return 'launching [%d] %s' % (SELECT, label(chosen))


print(main())
