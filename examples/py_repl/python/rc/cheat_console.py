"""Make Rogue Core's BP_CheatConsole spawn as a working, usable console.

The console is never destroyed when you spawn it. ~0.2s after BeginPlay a gated
blueprint branch decides the console isn't "allowed" and, on itself, calls:

    SetActorHiddenInGame(True)      # -> invisible ("looks destroyed")
    SetActorEnableCollision(False)  # -> can't be interacted with

That same failed gate means the console's *enable* path never runs, so its
InstantUsable component is left in its default-off state (usable=False,
bIsActive=False) — which is why, even made visible and collidable, you still
can't use it. (No Destroy / EndPlay / SetLifeSpan is ever called, so spawns
just pile up invisible.)

This does two things:
  1. Two class-filtered hooks swallow the console's self-hide and self-collision-
     disable calls (every other actor, and legit show/enable calls, pass through).
  2. On each console's BeginPlay (and for any already in the world) it turns the
     InstantUsable component on: Activate() + SetCanUse(True).

The hooks sit on shared native AActor functions and the begin-play detour is a
process-global registry, so all of this survives level resets. Re-run after a
full game restart.
"""

import ue

CC_PATH = '/Game/UI/HUD_SpaceRig/BP_CheatConsole.BP_CheatConsole_C'
TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject


def _enable_usable(console):
    """Turn the console's InstantUsable component on so it can be interacted with."""
    try:
        iu = console.InstantUsable
        if iu is None:
            return
        iu.Activate(False)        # bIsActive -> True (component starts inactive)
        iu.SetCanUse(True)        # usable    -> True (never set by the gated path)
    except Exception as e:
        print('cheat_console: enable failed:', type(e).__name__, e)


def install(verbose=True):
    CC = ue.find(CC_PATH)
    if CC is None:
        print('cheat_console: class not loaded — are you in Rogue Core on the rig?')
        return

    def is_cc(o):
        try:
            return o is not None and o._class == CC
        except Exception:
            return False

    def keep_visible(this, p):
        if is_cc(this):
            try:
                if bool(p.args[0]):          # SetActorHiddenInGame(True)
                    return None              # swallow the self-hide
            except Exception:
                pass
        return p.original()

    def keep_collision(this, p):
        if is_cc(this):
            try:
                if not bool(p.args[0]):      # SetActorEnableCollision(False)
                    return None              # swallow the self-disable
            except Exception:
                pass
        return p.original()

    for name, cb in [('SetActorHiddenInGame', keep_visible),
                     ('SetActorEnableCollision', keep_collision)]:
        try:
            ue.unhook(CC, name)
        except Exception:
            pass
        ue.hook(CC, name, cb)

    # Auto-enable the InstantUsable component on every future spawn. Replaces any
    # handle from a previous install() so re-running doesn't stack detours.
    old = globals().get('_cc_begin_play')
    if old is not None:
        try: old.remove()
        except Exception: pass
    globals()['_cc_begin_play'] = ue.on_begin_play(CC, lambda a: _enable_usable(a))

    # Fix up any console already in the world.
    fixed = 0
    for o in ue.objects_of_class(CC):
        if o._flags & TEMPLATE:
            continue
        try:
            o.SetActorHiddenInGame(False)
            o.SetActorEnableCollision(True)
            _enable_usable(o)
            fixed += 1
        except Exception:
            pass

    if verbose:
        print('cheat_console: hooks %s + begin-play auto-enable; fixed %d existing console(s)'
              % (ue.hooks(), fixed))


def remove():
    """Undo: drop the hooks and the begin-play detour. Consoles keep their state."""
    CC = ue.find(CC_PATH)
    for name in ('SetActorHiddenInGame', 'SetActorEnableCollision'):
        try:
            ue.unhook(CC, name)
        except Exception:
            pass
    h = globals().pop('_cc_begin_play', None)
    if h is not None:
        try: h.remove()
        except Exception: pass
    print('cheat_console: hooks + auto-enable removed')


install()
