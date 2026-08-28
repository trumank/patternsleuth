"""Raise per-room enemy caps on every REnemySpawnAreaSegment, now and for all
rooms that stream in later.

The manager-level multipliers (EnemyAmountDifficultyMultiplier etc.) only scale
the node budget; each encounter room is clamped by its segment actor:
  - DebugMaxAmount (behaves like the real per-room cap, ~6 default)
  - bOverrideEnemyLimits + OverridenEnemyAmountLimits (Vector2D min/max)

Both are raised here: DebugMaxAmount -> ROOM_CAP, override limits -> (ROOM_FLOOR, ROOM_CAP).
Segments are per-room level actors, so a on_tick sweeper re-applies the caps to
any segment that appears when a new room streams in (same rescan pattern as
freecam_abyssus).

Run:  python repl.py --script pump_tutorial_enemies.py
Re-running disarms the previous sweeper first.
"""

import ue

ROOM_CAP = 40
ROOM_FLOOR = 10
RESCAN_INTERVAL = 0.5

TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject
SEG_CLASS = '/Script/RGame.REnemySpawnAreaSegment'


def _apply(seg):
    try:
        changed = False
        if seg.DebugMaxAmount < ROOM_CAP:
            seg.DebugMaxAmount = ROOM_CAP
            changed = True
        if not seg.bOverrideEnemyLimits:
            seg.bOverrideEnemyLimits = True
            changed = True
        lim = seg.OverridenEnemyAmountLimits
        if lim.X != ROOM_FLOOR or lim.Y != ROOM_CAP:
            lim.X = ROOM_FLOOR
            lim.Y = ROOM_CAP
            changed = True
        return changed
    except Exception:
        return False    # stale segment streaming out from under us


_prev = globals().get('_enemy_sweep')
if _prev is not None:
    try:
        _prev['tick'].remove()
    except Exception:
        pass


state = {'tick': None, 'scan_t': 0.0, 'capped': 0}


def _on_tick(world, dt):
    state['scan_t'] += dt
    if state['scan_t'] < RESCAN_INTERVAL:
        return
    state['scan_t'] = 0.0
    for seg in ue.objects_of_class(SEG_CLASS):
        if seg._flags & TEMPLATE:
            continue
        if _apply(seg):
            state['capped'] += 1


state['tick'] = ue.on_tick(_on_tick)
globals()['_enemy_sweep'] = state

# Immediate pass so the current room's segments don't wait for the rescan.
for seg in ue.objects_of_class(SEG_CLASS):
    if seg._flags & TEMPLATE:
        continue
    _apply(seg)

print('enemy pump armed: cap=%d floor=%d (re-sweep every %ss)' % (ROOM_CAP, ROOM_FLOOR, RESCAN_INTERVAL))
