# Force-load a map by name via GameplayStatics.OpenLevel, then block until the
# destination level is actually loaded before returning.
#   python repl.py --script open_level.py
#
# Async: the runtime drives main() on the engine tick, so the game keeps
# ticking through the travel/load while this coroutine awaits. We issue the
# OpenLevel, then suspend until the destination world's GameMode begins play --
# the point at which the new level is live -- and report it.
#
# OpenLevel takes the short map name, not a full /Game/... path. The Space Rig
# (LVL_SpaceRig) and its streaming sublevels come in together.

import ue

LEVEL = 'LVL_SpaceRig'
TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject


def live(cls):
    return next((o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE), None)


async def main():
    # The GameInstance is a stable WorldContextObject -- the current world is
    # torn down by the travel, so don't anchor the call to it.
    gi = live('/Script/Engine.GameInstance')
    gs = ue.find('/Script/Engine.Default__GameplayStatics')
    gs.OpenLevel(gi, LEVEL, True, '')
    print('OpenLevel(%s) issued -- waiting for load...' % LEVEL)

    # The destination world's GameMode begins play once the load completes.
    try:
        gm = await ue.spawned('/Script/Engine.GameModeBase', timeout=60)
    except ue.Timeout:
        return 'OpenLevel(%s) issued but no GameMode began play within 60s' % LEVEL
    return '%s loaded (gamemode %s)' % (LEVEL, gm._class._name)
