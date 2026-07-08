# Trace DRG procedural level generation: hook every generation-pipeline stage
# on the ProceduralSetup (PLS_*) actor and log each completion with its
# duration. Doubles as living documentation of how a mission level is built.
#   run from the Space Rig: python repl.py --script levelgen_trace.py
#   then poll the log:       python repl.py  ->  levelgen_trace
#
# Why hook these and not the "official" callbacks: on a listen server the
# per-pass GM.OnLevelGenerationComplete / PLS.CallGenerationComplete events
# never fire, so we hook the concrete pipeline functions directly. Each hook
# times params.original(), so the log shows real per-stage durations.
#
# This is self-contained (it also selects + launches the mission) so it never
# fights start_mission.py for the shared native hooks. It supersedes any
# start_mission tracking that's currently armed.

import time

SELECT = 0
PLS = '/Script/FSD.ProceduralSetup'
TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

# Pipeline stages in rough execution order. Hooked on the concrete PLS_* class,
# so each name resolves up to wherever it's defined (native base or BP). Stages
# that don't fire for this mission type simply never appear in the log.
STAGES = [
    'BeginGenerating', 'BeginLiveGeneration',
    'Generate_Graph', 'CreateCaveGraph', 'CreateGraphFirstPass_1', 'CreateGraphSecondPass',
    'GenerateRoomsFromGraph', 'GenerateRoomsFromGraph_Async',
    'Generate_Tunnels', 'CreateTunnelPaths', 'AddInfluenceToTunnelEntrances',
    'CarveTunnels', 'FillTunnels', 'FillTunnels_Async',
    'Generate_CriticalLargePass',
    'Generate_Pass_0', 'Generate_Pass_1', 'Generate_Pass_2', 'Generate_Pass_3',
    'Generate_Pass_4', 'Generate_Pass_5_Server', 'Generate_Pass_6', 'Generate_Pass_7',
    'GenerateLandscape', 'GenerateLandscapeFromData',
    'CreateGeneratedInfluenceSet', 'CreateSpawn',
    'CarveWithGeneratedInstantCarvers', 'CommitCarving', 'DoneCarving', 'BaseLayerCommitDone',
    'InitializeObjectives',
    'SpawnCriticalItems', 'SpawnObjectiveCriticalItems', 'SpawnObjectiveEncounter',
    'SpawnItems', 'SpawnDebrisItems', 'SpawnSpecialEvents', 'SpawnEncounters',
    'Final Pass',
    'CallGenerationComplete',   # included to confirm whether it ever fires
    'FinalCommitDone',
]

MAX = 5000
levelgen_trace = []
globals()['levelgen_trace'] = levelgen_trace
_t0 = [None]
_seq = [0]


def live(cls):
    return next((o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE), None)


def label(m):
    biome = m.Biome._name if m.Biome else '?'
    obj = m.PrimaryObjective._name if m.PrimaryObjective else '?'
    return '%s | %s | seed=%s' % (biome, obj, m.Seed)


def _log(name, dur_ms):
    if len(levelgen_trace) >= MAX:
        return
    now = time.monotonic()
    if _t0[0] is None:
        _t0[0] = now
    _seq[0] += 1
    levelgen_trace.append('%3d  t=%8.1f  dur=%8.2f  %s'
                          % (_seq[0], (now - _t0[0]) * 1000.0, dur_ms, name))


def _make(name):
    def cb(this, params):
        t = time.monotonic()
        try:
            params.original()        # run the real stage; time it
        finally:
            _log(name, (time.monotonic() - t) * 1000.0)
    return cb


# ── tear down prior arming, incl. start_mission's tracker (shares hooks) ──────
def _remove(reg):
    try:
        reg.remove()
    except Exception:
        pass


_prev = globals().get('_levelgen_tracker')
if _prev:
    _remove(_prev['begin_play'])
    for cls in _prev['classes']:
        for s in STAGES:
            try:
                ue.unhook(cls, s)
            except Exception:
                pass

_mt = globals().get('_mission_tracker')   # start_mission.py's lighter tracker
if _mt:
    _remove(_mt['begin_play'])
    for s in ('BeginGenerating', 'InitializeObjectives', 'SpawnEncounters'):
        try:
            ue.unhook(PLS, s)
        except Exception:
            pass

_tracker = {'begin_play': None, 'classes': []}


def _on_pls_begin(actor):
    cls = actor._class
    armed, skipped = [], []
    for s in STAGES:
        try:
            ue.unhook(cls, s)        # take over any prior hook on the same UFunction
        except Exception:
            pass
        try:
            ue.hook(cls, s, _make(s))
            armed.append(s)
        except Exception:
            skipped.append(s)        # not a UFunction on this PLS subclass
    _tracker['classes'].append(cls)
    levelgen_trace.append('# armed %d stages on %s; skipped (absent): %s'
                          % (len(armed), cls._name, ', '.join(skipped) or 'none'))


_tracker['begin_play'] = ue.on_begin_play(PLS, _on_pls_begin)
globals()['_levelgen_tracker'] = _tracker
del levelgen_trace[:]
_t0[0] = None
_seq[0] = 0


# ── select + launch ──────────────────────────────────────────────────────────
gi = live('/Script/Engine.GameInstance')
gm = live('/Script/FSD.FSDGameModeSpaceRig')
mgm = live('/Script/FSD.MissionGenerationManager')

if gm is None:
    raise RuntimeError('SpaceRig gamemode not found -- are you on LVL_SpaceRig? '
                       'Run open_level.py first.')

missions = mgm.GetAvailableMissions()
if not 0 <= SELECT < len(missions):
    raise IndexError('SELECT=%d out of range (0..%d)' % (SELECT, len(missions) - 1))
chosen = missions[SELECT]
print('tracing generation of [%d] %s' % (SELECT, label(chosen)))

gi.SetSelectedMission(chosen, True)
gm.InstantlyStartMission()
print('launched -- poll `levelgen_trace` after the level loads')
