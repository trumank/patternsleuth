# Async: select an available mission, launch it, and block until the level is
# fully generated -- streaming each milestone live -- then return a summary.
#   python repl.py --script start_mission.py
#
# One straight-line coroutine. The runtime drives main() on the engine tick, so
# the game keeps generating between awaits; each print streams to the client as
# its milestone resolves, and the returned value is the final reply. No polling.
#
# Must be run from the Space Rig (open_level.py -> LVL_SpaceRig first if needed).
#
# Launch flow:
#   1. MissionGenerationManager.GetAvailableMissions() -> roster of GeneratedMission
#   2. GameInstance.SetSelectedMission(mission, True)
#   3. SpaceRig gamemode.InstantlyStartMission()  -> travel into LVL_Procedural
#
# Then we await the procedural actor and its generation milestones. (On a listen
# server the per-pass GM.OnLevelGenerationComplete / PLS.CallGenerationComplete
# callbacks never fire, so we key off the ProceduralSetup milestones, which do.)

import ue

SELECT = 0
PLS = '/Script/FSD.ProceduralSetup'
TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject


def live(cls):
    return next((o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE), None)


def label(m):
    biome = m.Biome._name if m.Biome else '?'
    obj = m.PrimaryObjective._name if m.PrimaryObjective else '?'
    return '%s | %s | seed=%s' % (biome, obj, m.Seed)


def on_mission_loaded(pls):
    """Called once the level is fully committed (FinalCommitDone) -- the safe
    point to set up gameplay hooks. Runs on the game thread. Edit for your own
    setup (e.g. ue.hook(...) / ue.on_begin_play(...) on the freshly-built world).
    """
    pass


async def main():
    gm = live('/Script/FSD.FSDGameModeSpaceRig')
    if gm is None:
        return 'not on the Space Rig -- run open_level.py (LVL_SpaceRig) first'
    gi = live('/Script/Engine.GameInstance')
    mgm = live('/Script/FSD.MissionGenerationManager')

    missions = mgm.GetAvailableMissions()
    print('available missions:')
    for i, m in enumerate(missions):
        print('  %2d  %s' % (i, label(m)))
    if not 0 <= SELECT < len(missions):
        return 'SELECT=%d out of range (0..%d)' % (SELECT, len(missions) - 1)

    chosen = missions[SELECT]
    gi.SetSelectedMission(chosen, True)
    gm.InstantlyStartMission()
    print('launching [%d] %s' % (SELECT, label(chosen)))

    # Each await suspends the coroutine and hands the game thread back so
    # generation runs; the following print streams to the client on resume.
    try:
        pls = await ue.spawned(PLS, timeout=30)
        print('  generating (%s)...' % pls._class._name)
        await ue.hook_once(pls._class, 'SpawnEncounters', timeout=90)
        print('  encounters spawned')
        await ue.hook_once(pls._class, 'FinalCommitDone', timeout=90)
        print('  final commit -- level fully loaded')
    except ue.Timeout:
        return 'timed out waiting for level generation'

    on_mission_loaded(pls)

    n = len([o for o in ue.objects_of_class('/Script/FSD.Objective') if not o._flags & TEMPLATE])
    return 'LOADED: %d objectives' % n
