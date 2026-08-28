"""Multiply RoboQuest's per-encounter enemy counts at level-generation time.

A level's whole enemy population comes from one row of DT_EnemyBoard, copied
onto the ALevelGenerator as `EnemyBoardRow`:

    EnemyBoardRow.RegularEncounters[] / SpecificEncounters[] / IndieEncounters[]
        -> PredefinedEncounter.Enemies[] -> PredefinedEnemy{Class, Position, Count}

One encounter is handed to each tile, and its enemies are placed on the
ASquadPoint actors baked into that tile (Normal/Turret/Flying x Front/Mid/Back).
Base Canyon encounters total 6-7 enemies against ~19 squad points per tile, so
there is a lot of unused placement headroom.

Generation is one-shot during level load, in this order (ELoadingStep):
    GeneratingLevel -> GeneratingNavigation -> GeneratingEnemies -> GeneratingLoot
so `SendLoadingStep` is a precise, race-free place to rewrite the board. A tick
sweeper backs it up, in case the row is filled in between two loading steps.

The generator is *reused* across levels -- it just reloads EnemyBoardRow for the
new level -- so "already pumped" is tracked as (board row name, pumped total)
per generator rather than per generator: a new row, or the same row reloaded at
its baseline total, is pumped again; an already-pumped row is left alone.

Takes effect on the NEXT level load; the current level's enemies were placed at
its own load.

Run:  python repl.py --script roboquest_enemy_pump.py
Then travel to the next level. Re-running re-arms with the current constants.
Read back what it did with:
    python repl.py -c "print(enemy_pump['log'])"
"""

import ue

MULTIPLIER = 3          # every PredefinedEnemy.Count is scaled by this
FOLD_MULTIPLAYER = True # also spawn the co-op-only extras in solo
EXTRA_ELITE = 0         # added to ALevelGenerator.AddEliteValue
EXTRA_BIG = 0           # added to ALevelGenerator.AddBigValue

LG_CLASS = '/Script/RoboQuest.ALevelGenerator'
SQ_CLASS = '/Script/RoboQuest.ASquadPoint'
TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject


def _encounters(row):
    """Every PredefinedEncounter on the board, arena ones included."""
    for lst in (row.RegularEncounters, row.SpecificEncounters, row.IndieEncounters):
        for enc in lst:
            yield enc
    for ae in row.ArenaEncounters:
        yield ae.Base
        yield ae.Reinforcements


def _board_total(lg):
    return sum(pe.Count for enc in _encounters(lg.EnemyBoardRow) for pe in enc.Enemies)


def _pump(enc):
    if FOLD_MULTIPLAYER:
        for pe in list(enc.AdditionalsInMultiplayer):
            enc.Enemies.append(pe)
    for pe in enc.Enemies:
        pe.Count = max(1, pe.Count * MULTIPLIER)


def _squad_capacity():
    """Squad points still alive -- only meaningful during generation, they are
    destroyed once the enemies are placed."""
    counts = {}
    for s in ue.objects_of_class(SQ_CLASS, True, None):
        try:
            if s._flags & TEMPLATE:
                continue
            key = '%s/%s' % (str(s.PointType).split('::')[-1],
                             str(s.PointTag).split('::')[-1])
        except Exception:
            continue    # streaming out from under us
        counts[key] = counts.get(key, 0) + 1
    return counts


def _try_patch(lg):
    """Pump this generator's board row unless it is already pumped."""
    try:
        board = str(lg.EnemyBoardRowName)
        encs = list(_encounters(lg.EnemyBoardRow))
        if not encs:
            return          # row not populated yet; a later step will catch it
        before = _board_total(lg)
        if not before:
            return
        mark = state['marks'].get(lg._name)
        if mark is not None and mark[0] == board and mark[1] == before:
            return          # this exact row was already pumped
        for enc in encs:
            _pump(enc)
        after = _board_total(lg)
        state['marks'][lg._name] = (board, after)
        state['log'].append({
            'generator': lg._name,
            'board': board,
            'encounters': len(encs),
            'enemies_before': before,
            'enemies_after': after,
            'squad_points': _squad_capacity(),
        })
    except Exception as e:
        state['log'].append({'error': repr(e)})


def _on_loading_step(this, params):
    _try_patch(this)
    try:
        step = str(params.LoadingStep).split('::')[-1]
        key = '%s:%s' % (step, 'start' if params.bIsStarting else 'end')
        state['steps'][key] = state['steps'].get(key, 0) + 1
        if (step == 'GeneratingEnemies'
                and not params.bIsStarting):
            state['log'].append({
                'generator': this._name,
                'board': str(this.EnemyBoardRowName),
                'spawned_by_encounter': [c for c in this.EnemyCountByEncounterID if c],
                'spawned_total': sum(this.EnemyCountByEncounterID),
                'by_class': dict(sorted(
                    ((k._name, v) for k, v in this.EnemyCountByClass.items() if k),
                    key=lambda kv: -kv[1])),
            })
    except Exception as e:
        state['log'].append({'error': repr(e)})
    return None     # falls through to the real SendLoadingStep


def _on_tick(world, dt):
    for lg in ue.objects_of_class(LG_CLASS, True, None):
        try:
            if lg._flags & TEMPLATE:
                continue
        except Exception:
            continue
        _try_patch(lg)


_prev = globals().get('enemy_pump')
if _prev is not None:
    try:
        ue.unhook(LG_CLASS, 'SendLoadingStep')
    except Exception:
        pass
    try:
        _prev['tick'].remove()
    except Exception:
        pass

state = {
    'marks': dict(_prev['marks']) if _prev else {},
    'log': list(_prev['log']) if _prev else [],
    'steps': dict(_prev['steps']) if _prev and 'steps' in _prev else {},
    'tick': None,
}
ue.hook(LG_CLASS, 'SendLoadingStep', _on_loading_step)
state['tick'] = ue.on_tick(_on_tick)
globals()['enemy_pump'] = state

print('enemy pump armed: x%d counts%s, +%d elite, +%d big' % (
    MULTIPLIER, ', co-op extras folded in' if FOLD_MULTIPLAYER else '',
    EXTRA_ELITE, EXTRA_BIG))
print('takes effect on the next level load -- walk through the exit door.')
