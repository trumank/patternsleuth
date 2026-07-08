import random

ENEMY_BASE = '/Script/FSD.EnemyDeepPathfinderCharacter'
TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

gi = next(o for o in ue.objects_of_class('/Script/Engine.GameInstance') if not o._flags & TEMPLATE)

def on_enemy(actor):
    try:
        name = actor._class._name
        loc = actor.K2_GetActorLocation()
        pc = gi.LocalPlayers[0].PlayerController
        pc.Server_NewMessage('SPAWN', '%s @ (%d, %d, %d)' % (name, loc.X, loc.Y, loc.Z), 5)
        rc = actor.K2_GetRootComponent()
        def rng():
            return random.uniform(0.3, 2.0)
        rc.SetWorldScale3D({'X': rng(), 'Y': rng(), 'Z': rng()})
    except Exception:
        pass

for _h in ('_enemy_spawn_hook', '_bp_hook'):
    if _h in globals():
        try: globals()[_h].remove()
        except Exception: pass

globals()['_enemy_spawn_hook'] = ue.on_begin_play(ENEMY_BASE, on_enemy)
