"""Far Far West (project `Canyon`) — joker inspection & spawning.

Everything below was found live over the REPL against a running session on
`L_Map_FarFarNorth`. It documents how jokers are modelled internally and gives
reusable helpers to read the player's loadout, list the master catalog, and
spawn joker cards into the world.

    python repl.py --script farfarwest/jokers.py          # prints a live report
    # then, interactively (helpers persist as globals):
    print(joker_catalog())                                # all 140 row names
    equipped_jokers()                                     # your 5 hero jokers
    cards_on_ground()                                     # live pickables + dist
    spawn_joker('jokerGlassCannon', *in_front(300))       # one card in front
    spawn_grid()                                          # a grid of ALL jokers
    clear_grid()                                           # destroy what spawn_grid made


How jokers work internally
==========================

* **Master catalog** — `/Game/Progress/DT_PlayerJokers` (a `UDataTable`, row
  struct `S_PlayerJokers`). 140 rows, each keyed by the joker's `FName`. That
  same name is the identity used everywhere else. The rows split into:
    - 20 `jokerUpgrade*` passive weapon/stat upgrades
      (Damage, Firerate, ClipSize, ReloadSpeed, HeroSpeed, JumpHeight, ...).
    - 120 collectible jokers (jokerRelentless, jokerRampage, jokerVampire,
      jokerGlassCannon, jokerSecondWind, jokerCoinFlip, ...).

* **Equipped loadout** — `BP_PlayerState_C.itemHeroJokers : Array<Name>`
  (5 hero-joker slots). Picking up a card appends its `jokerName` here.

* **Per-joker runtime state** — the `AC_PlayerStateJoker` component on the
  PlayerState (`/Game/Spells/Jokers/AC_PlayerStateJoker`). Holds each joker's
  live bookkeeping: flags (`isJokerSecondWindTriggered`, `jokerCoinFlipBonus`,
  `isJokerSoulEaterActive`), stacks (`jokerRampageStack` / `jokerRampageTotalStack`),
  and timer handles (`jokerSecondWindTimerHeal`, `timerJokerSpellRoulette`).

* **Effect ledger** — the live `BP_Manager_Jokers` instance (ships merged into a
  `BP_OptimizedDatas_C` actor in `L_Handlers`) exposes
  `cachedItemJokerEffects : Map<Name, double>`, keyed `item<Category>joker<Name>`
  → active flag / magnitude. Equipped hero jokers read `itemHerojoker<Name> = 1.0`;
  a weapon-scoped joker reads e.g. `itemShotgunjokerFocusedFeet` and is 0.0 until
  equipped on that weapon.

* **Physical card** — `/Game/GPE/BP_Pickable_Joker.BP_Pickable_Joker_C`, an actor
  with `jokerName`, `jokerNameReplicated`, `requiredRarity` (`E_Rarity::*`),
  `isPlayerDropped`, `replicates`. Player-dropped variants:
  `BP_Pickable_Joker_Dropped[_Local]`. The card reads its `jokerName` during
  construction to configure rarity glow / widget / mesh from the table row —
  so set the name on a **deferred** spawn, before `FinishSpawningActor`.

* **Supporting assets**
    - Per-joker scaling curves: `/Game/Spells/Jokers/JokerCurve/C_Joker_*`.
    - Unlock gate (player-level -> jokers available): `/Game/Interfaces/Equipment/C_UnlockedJokers`.
    - Special-behavior jokers with their own logic: `BP_Joker_Rage`,
      bullets like `BP_PlayerBullet_Shotgun_JokerRushBlast`, buffs like `UO_Buff_JokerMarkAce`.

Spawning note: Far Far West ships the newer engine build, so the deferred-spawn
calls take a trailing `ESpawnActorScaleMethod` arg (`2` = SelectDefaultAtRuntime).
"""

import math

# ── key asset paths ──────────────────────────────────────────────────────────
DT_JOKERS   = '/Game/Progress/DT_PlayerJokers.DT_PlayerJokers'
PICKABLE    = '/Game/GPE/BP_Pickable_Joker.BP_Pickable_Joker_C'
GAMEPLAY    = '/Script/Engine.Default__GameplayStatics'
DT_LIB      = '/Script/Engine.Default__DataTableFunctionLibrary'

U = lambda p: ue.find(p)


# ── player / world helpers ───────────────────────────────────────────────────

def player_controller():
    return next((p for p in ue.objects_of_class('/Script/Engine.PlayerController')
                 if 'Default__' not in p._path), None)


def player_pawn():
    pc = player_controller()
    return pc.Pawn if pc else None


def player_state():
    pc = player_controller()
    return pc.PlayerState if pc else None


def player_loc():
    """(x, y, z) of the local pawn — RelativeLocation of a root component is its
    world location for an unattached actor, which the player pawn is."""
    p = player_pawn()
    if p is None:
        return None
    l = p.RootComponent.RelativeLocation
    return (l.X, l.Y, l.Z)


def in_front(dist=300.0, up=30.0):
    """A point `dist` uu ahead of the pawn (using its forward), as (x, y, z).
    Handy target for spawn_joker(). Falls back to +X if forward is unavailable."""
    p = player_pawn()
    x, y, z = player_loc()
    try:
        f = p.GetActorForwardVector()
        fx, fy = f.X, f.Y
    except Exception:
        fx, fy = 1.0, 0.0
    return (x + fx * dist, y + fy * dist, z + up)


# ── reads ────────────────────────────────────────────────────────────────────

def joker_catalog():
    """All 140 joker row names from the master DataTable (order preserved)."""
    dt = U(DT_JOKERS) or ue.load(DT_JOKERS)
    return [str(n) for n in U(DT_LIB).GetDataTableRowNames(dt)]


def joker_catalog_split():
    """(upgrades, jokers) — the 20 `jokerUpgrade*` passives vs the 120 cards."""
    names = joker_catalog()
    ups = [n for n in names if n.startswith('jokerUpgrade')]
    jok = [n for n in names if not n.startswith('jokerUpgrade')]
    return ups, jok


def equipped_jokers():
    """The player's 5 hero-joker slots (list of names)."""
    ps = player_state()
    return [str(n) for n in ps.itemHeroJokers] if ps else []


def joker_runtime_state():
    """Snapshot of the AC_PlayerStateJoker per-joker bookkeeping fields."""
    ps = player_state()
    if ps is None:
        return {}
    jc = ps.AC_PlayerStateJoker
    out = {}
    for a in dir(jc):
        if a.startswith('_') or 'oker' not in a.lower():
            continue
        try:
            v = getattr(jc, a)
            if not callable(v):
                out[a] = v
        except Exception:
            pass
    return out


def cards_on_ground(near=None):
    """Live `BP_Pickable_Joker` cards as dicts {actor, jokerName, loc, dist},
    sorted by distance from `near` (defaults to the player). Skips CDOs and the
    GEN_VARIABLE child-actor templates."""
    origin = near or player_loc()
    out = []
    for o in ue.objects_of_class(PICKABLE):
        p = o._path
        if 'Default__' in p or p.endswith('_C') or 'GEN_VARIABLE' in p:
            continue
        try:
            l = o.RootComponent.RelativeLocation
            loc = (l.X, l.Y, l.Z)
        except Exception:
            loc = None
        d = math.dist(origin, loc) if (origin and loc) else None
        out.append({'actor': o, 'jokerName': str(o.jokerName), 'loc': loc, 'dist': d})
    out.sort(key=lambda e: (e['dist'] is None, e['dist'] or 0.0))
    return out


# ── spawning ─────────────────────────────────────────────────────────────────

def spawn_joker(name, x, y, z):
    """Deferred-spawn one BP_Pickable_Joker with `jokerName` = name so its
    construction picks up the right rarity/mesh/widget. Returns the actor."""
    gs = U(GAMEPLAY)
    pc = player_controller()
    cls = ue.load(PICKABLE)
    xf = {'Translation': {'X': x, 'Y': y, 'Z': z},
          'Rotation': {'X': 0.0, 'Y': 0.0, 'Z': 0.0, 'W': 1.0},
          'Scale3D': {'X': 1.0, 'Y': 1.0, 'Z': 1.0}}
    a = gs.BeginDeferredActorSpawnFromClass(
        pc, cls, xf, 'ESpawnActorCollisionHandlingMethod::AlwaysSpawn', None, 2)
    if a is None:
        return None
    a.jokerName = name
    a.jokerNameReplicated = name
    return gs.FinishSpawningActor(a, xf, 2)


def spawn_grid(names=None, cols=12, spacing=250.0, ahead=350.0, up=30.0):
    """Spawn a grid of cards (default: every joker in the catalog) laid out in
    front of the player, centered on Y. Handles are stashed in the `_joker_grid`
    global so clear_grid() can tear them down. Returns the list of actors."""
    names = names if names is not None else joker_catalog()
    px, py, pz = player_loc()
    ox = px + ahead
    oy = py - (cols - 1) * spacing / 2.0
    oz = pz + up

    spawned, fails = [], []
    for i, name in enumerate(names):
        r, c = divmod(i, cols)
        try:
            a = spawn_joker(name, ox + r * spacing, oy + c * spacing, oz)
            (spawned if a is not None else fails).append(a if a is not None else name)
        except Exception as e:
            fails.append('%s(%s)' % (name, e))
    globals()['_joker_grid'] = spawned
    print('spawned %d / %d cards (%d cols, %.0f spacing)'
          % (len(spawned), len(names), cols, spacing))
    if fails:
        print('failed:', fails[:20])
    return spawned


def clear_grid():
    """Destroy every card spawn_grid() created this session."""
    n = 0
    for a in globals().get('_joker_grid', []):
        try:
            a.K2_DestroyActor()
            n += 1
        except Exception:
            pass
    globals()['_joker_grid'] = []
    print('destroyed', n)
    return n


# ── report ───────────────────────────────────────────────────────────────────

def report():
    ups, jok = joker_catalog_split()
    print('=== Far Far West — jokers ===')
    print('catalog: %d rows  (%d jokerUpgrade* passives, %d collectible jokers)'
          % (len(ups) + len(jok), len(ups), len(jok)))
    print()
    print('equipped hero jokers (%d slots):' % len(equipped_jokers()))
    for i, n in enumerate(equipped_jokers()):
        print('  [%d] %s' % (i, n))
    print()
    cards = cards_on_ground()
    print('joker cards on the ground: %d' % len(cards))
    for c in cards[:12]:
        d = '%.0f' % c['dist'] if c['dist'] is not None else '?'
        print('  %-24s dist=%s' % (c['jokerName'], d))
    if cards:
        print('nearest: %s (%.0f uu)' % (cards[0]['jokerName'], cards[0]['dist'] or 0))


# Persist helpers as globals so they survive across REPL frames.
for _n in ('U', 'player_controller', 'player_pawn', 'player_state', 'player_loc',
           'in_front', 'joker_catalog', 'joker_catalog_split', 'equipped_jokers',
           'joker_runtime_state', 'cards_on_ground', 'spawn_joker', 'spawn_grid',
           'clear_grid', 'report'):
    globals()[_n] = globals()[_n]


if __name__ != '__mp_main__':
    report()
