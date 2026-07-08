"""Announce who presses the Hoops minigame score-reset button in chat.

The reset button is the InstantUsable component on BP_HoopsGame. Its native
UsableComponent.BeginUse is invoked directly from C++, so it never passes
through ProcessEvent and can't be hooked. Instead we hook the Blueprint event
the graph binds to the usable's OnUsedBy delegate -- that one runs in the
Blueprint VM and carries the pressing PlayerCharacter as `User`.
"""

TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

HOOPS_BASE = '/Script/FSD.HoopsGame'
# BP graph handler bound to the InstantUsable's OnUsedBy(User, Key) delegate.
USED_BY_EVENT = ('BndEvt__InstantUsable_K2Node_ComponentBoundEvent_3'
                 '_UsedBySignature__DelegateSignature')

hoops = next(o for o in ue.objects_of_class(HOOPS_BASE, include_subclasses=True)
             if not o._flags & TEMPLATE)
hoops_class = hoops._class

gi = next(o for o in ue.objects_of_class('/Script/Engine.GameInstance') if not o._flags & TEMPLATE)

def on_reset_pressed(target, params):
    try:
        name = params.User.GetPlayerName()
        pc = gi.LocalPlayers[0].PlayerController
        # SenderType 2 == EChatSenderType::Developer (distinct styling).
        pc.Server_NewMessage('Hoops', '%s reset the hoop score!' % name, 2)
    except Exception:
        pass
    return params.original()  # let the real score reset run

# Idempotent: drop any hook a previous run left behind before re-installing.
try:
    ue.unhook(hoops_class, USED_BY_EVENT)
except Exception:
    pass

ue.hook(hoops_class, USED_BY_EVENT, on_reset_pressed)
print('hooked %s.%s' % (hoops_class._name, USED_BY_EVENT))
