"""Custom Mission Builder — a Space-Rig widget for assembling and launching an
arbitrary generated mission from scratch, built from the game's own icons/widgets.

Pick a mission type, biome, difficulty, complexity, length, mutator, challenge,
secondary objective, Hazard-5+ modifiers and any set of warnings, then SELECT
MISSION: it builds a fresh `GeneratedMission` and selects it on the GameInstance
(`SetSelectedMission`, + sets the desired hazard). It does NOT travel — launch the
selected mission normally from the in-game drop pod / launch terminal.
(`launch_mission()` is also exposed for programmatic select-and-travel.)

UI: categorical fields (type / biome / mutator / challenge / secondary / warnings)
are icon-button grids — every option is a clickable icon, single-select except
warnings which are multi. The three scalar fields (difficulty / complexity /
length) are ◀ ▶ cyclers whose value is the game's own DotMarker widget.

Data + icons (all from the central `/Script/FSD.GameData` singleton and the game
UI assets):
    type      MissionTemplate  -> ITM_MisSel_MissionIconBasic.SetTemplate (hex badge)
    biome     Biome            -> Icon_Map_Biome_* square icon (BIOME_ICON map)
    difficulty DifficultySetting-> DotMarker_WorkEnvironment.SetData(setting)
    complexity MissionComplexity-> DotMarker_CaveDifficulty.SetData(int)
    length    MissionDuration  -> DotMarker_MissionLength.SetData(int)
    mutator   MissionMutator   -> Mutator.Icon, tinted gold
    warning   MissionWarning   -> Warning.Icon, tinted red   (framed icons are
                                  monochrome; the game colours them per category)
    challenge MissionChallenge -> generic Icon_Challenges_Outline (no per-item icon)
    secondary OBJ_2nd_* class  -> objective.Resource.Icon (resource outline icon)

Difficulty is applied by writing `GameInstance.DesiredDifficulty` (a plain
property); we never call `DifficultyManager.GetCurrentDifficulty()` (its native
body resolves the manager through a null GameState back-ref and crashes).

A mission is a `/Script/FSD.GeneratedMission` stored by reference on the
GameInstance and carried through travel; the `ProceduralSetup` reads its Biome /
DNA / warnings / mutator from it at setup. We build one fresh by cloning every
field off a live roster mission (a known-valid baseline) then overriding the
builder-controlled fields.  Caveat: the mission `Seed` field does not drive the
cave-carve seed.

Public: mission_builder_open(pc=None) / mission_builder_close(). Running the file
opens it; idempotent.
"""

import re

TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

U = lambda p: ue.find(p) or ue.load(p)

# ── colors ────────────────────────────────────────────────────────────────
C_PANEL   = {'R': 0.03, 'G': 0.04, 'B': 0.06, 'A': 0.97}
C_ARROW   = {'R': 0.16, 'G': 0.22, 'B': 0.32, 'A': 1.0}
C_CELL    = {'R': 0.10, 'G': 0.12, 'B': 0.17, 'A': 1.0}   # unselected button
C_CELL_ON = {'R': 0.95, 'G': 0.62, 'B': 0.16, 'A': 1.0}   # selected button (gold)
C_WARN_ON = {'R': 0.85, 'G': 0.16, 'B': 0.12, 'A': 1.0}   # armed warning (red)
C_TIER1   = {'R': 0.55, 'G': 0.34, 'B': 0.10, 'A': 1.0}   # Haz+ tier I (cell bg)
C_TIER2   = {'R': 0.90, 'G': 0.34, 'B': 0.06, 'A': 1.0}   # Haz+ tier II (cell bg)
# Haz+ icon tints lifted verbatim from the live selected-mission HUD's
# UI_HazPlus_Icon.Icon_Image (LinearColor): armed = orange, off = dim. The base
# game colours these in the parent widget, not in UI_HazPlus_Icon itself.
C_HAZ_ON  = {'R': 1.0, 'G': 0.3325, 'B': 0.0, 'A': 1.0}
C_HAZ_OFF = {'R': 0.1042, 'G': 0.0525, 'B': 0.0266, 'A': 1.0}
C_LAUNCH  = {'R': 0.16, 'G': 0.52, 'B': 0.22, 'A': 1.0}
C_CLOSE   = {'R': 0.45, 'G': 0.18, 'B': 0.20, 'A': 1.0}
C_TITLE   = {'R': 0.95, 'G': 0.62, 'B': 0.16, 'A': 1.0}
C_HDR     = {'R': 0.62, 'G': 0.68, 'B': 0.80, 'A': 1.0}
C_TXT     = {'R': 0.94, 'G': 0.96, 'B': 1.00, 'A': 1.0}
C_VAL     = {'R': 1.00, 'G': 0.86, 'B': 0.55, 'A': 1.0}
C_WHITE   = {'R': 1.0, 'G': 1.0, 'B': 1.0, 'A': 1.0}
C_ROW     = {'R': 0.07, 'G': 0.09, 'B': 0.13, 'A': 0.92}
# per-category icon tints (framed game icons are monochrome; C&O is LINEAR — the
# game colours warnings pure red, mutators gold, so use fully-saturated values)
TINT_WARN = {'R': 1.0, 'G': 0.0, 'B': 0.0, 'A': 1.0}
TINT_MUT  = {'R': 1.0, 'G': 0.715, 'B': 0.0, 'A': 1.0}

PX, PY, PW, PH = 30.0, 70.0, 560.0, 720.0
PAD = 18.0

FILL = {'SizeRule': 1, 'Value': 1.0}
AUTO = {'SizeRule': 0, 'Value': 1.0}
HALIGN = {'fill': 0, 'left': 1, 'center': 2, 'right': 3}
VALIGN = {'fill': 0, 'top': 1, 'center': 2, 'bottom': 3}

# game item-widgets we instantiate + drive
_MISSEL = '/Game/UI/Menu_MissionSelectionMK3/'
_ITEM_WIDGETS = {
    'badge':  _MISSEL + 'ITM_MisSel_MissionIconBasic',
    'dot_haz': _MISSEL + 'ITM_MisSel_DotMarker_WorkEnvironment',
    'dot_cx': _MISSEL + 'ITM_MisSel_DotMarker_CaveDifficulty',
    'dot_ln': _MISSEL + 'ITM_MisSel_DotMarker_MissionLength',
    # the game's own haz+ icon widget: SetMutator(DifficultyMutatorItem) draws the
    # correct difficulty icon + I/II tier label (used by the mission-board HUD)
    'haz_icon': '/Game/UI/Global_UI_Elements/UI_HazPlus_Icon',
}

# biome asset name -> distinct biome icon (MisBiomeComp_* milestone set)
_MILE = '/Game/UI/Art/Milestones/%s.%s'
BIOME_ICON = {
    'BIOME_AzureWeald': 'MisBiomeComp_AzureWeald', 'BIOME_CrystalCaves': 'MisBiomeComp_CrystalCaves',
    'BIOME_FungusBogs': 'MisBiomeComp_Fungus', 'BIOME_HollowBough': 'MisBiomeComp_Hollowbough',
    'BIOME_IceCaves': 'MisBiomeComp_GlacialStrata', 'BIOME_LushDownpour': 'MisBiomeComp_Lush',
    'BIOME_MagmaCaves': 'MisBiomeComp_Magma', 'BIOME_OssuaryDepths': 'MisBiomeComp_OssuaryDepths',
    'BIOME_RadioactiveZone': 'MisBiomeComp_Radioactive', 'BIOME_SaltCaves': 'MisBiomeComp_SaltCaves',
    'BIOME_SandblastedCorridors': 'MisBiomeComp_Sandblasted',
}
# secondary objective class -> clean outline resource icon (Icons_Resources_Outline_*)
_RESICON = '/Game/UI/Art/Icons/Icons_Resources/New_Resource_Icons/%s.%s'
SEC_ICON = {
    'OBJ_2nd_DestroyBhaBarnacles_C': 'Icons_Resources_Outline_BhaBarnacle',
    'OBJ_2nd_DestroyEggs_C': 'Icons_Resources_Outline_GlyphidEgg',
    'OBJ_2nd_Find_ApocaBloom_C': 'Icons_Resources_Outline_Apocabloom',
    'OBJ_2nd_Find_BooloCap_C': 'Icons_Resources_Outline_BooloCap',
    'OBJ_2nd_Find_Ebonut_C': 'Icons_Resources_Outline_Ebonut',
    'OBJ_2nd_Find_Fossil_C': 'Icons_Resources_Outline_Fossil',
    'OBJ_2nd_Find_Gunkseed_C': 'Icons_Resources_Outline_Gunk_Seeds',
    'OBJ_2nd_KillFleas_C': 'Icons_Resources_Outline_Festerflea',
    'OBJ_2nd_Mine_Dystrum_C': 'Icons_Resources_Outline_Dystrum',
    'OBJ_2nd_Mine_Hollomite_C': 'Icons_Resources_Outline_hollomite',
}
CHALLENGE_ICON = '/Game/UI/Menu_Seasons/Assets/Icon_Challenges_Outline.Icon_Challenges_Outline'

COMPLEXITY = ['/Game/GameElements/Missions/MissionDescription/MD_Complexity_Simple.MD_Complexity_Simple',
              '/Game/GameElements/Missions/MissionDescription/MD_Complexity_Average.MD_Complexity_Average',
              '/Game/GameElements/Missions/MissionDescription/MD_Complexity_Complex.MD_Complexity_Complex']
DURATION = ['/Game/GameElements/Missions/MissionDescription/MD_Duration_Short.MD_Duration_Short',
            '/Game/GameElements/Missions/MissionDescription/MD_Duration_Normal.MD_Duration_Normal',
            '/Game/GameElements/Missions/MissionDescription/MD_Duration_Long.MD_Duration_Long']


# ── helpers ──────────────────────────────────────────────────────────────────

def _live(cls):
    return next((o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE), None)


def _ktl():
    k = globals().get('_mb_ktl')
    if k is None:
        k = U('/Script/Engine.Default__KismetTextLibrary')
        globals()['_mb_ktl'] = k
    return k


def _text(s):
    return _ktl().Conv_StringToText(str(s))


def _str(ft):
    try:
        return _ktl().Conv_TextToString(ft)
    except Exception:
        return ''


_CAMEL = re.compile(r'(?<=[a-z0-9])(?=[A-Z])')


def _pretty(name, *prefixes):
    s = name
    for p in prefixes:
        if s.startswith(p):
            s = s[len(p):]
    if s.endswith('_C'):
        s = s[:-2]
    s = _CAMEL.sub(' ', s).replace('_', ' ')
    return re.sub(r'\s+', ' ', s).strip()


def _tex(v):
    if v is None:
        return None
    if isinstance(v, str):
        try:
            return ue.load(v)
        except Exception:
            return None
    return v


def _getattr(o, *names):
    for n in names:
        try:
            v = getattr(o, n)
            if v is not None:
                return v
        except Exception:
            pass
    return None


def _ftext(o, *names):
    for n in names:
        try:
            v = getattr(o, n)
            if v is not None:
                s = _str(v)
                if s:
                    return s
        except Exception:
            pass
    return None


def _cdo(cls):
    try:
        pkg = cls._path.rsplit('.', 1)[0]
        return U(pkg + '.Default__' + cls._name)
    except Exception:
        return None


def _secondary_icon(objclass):
    """objective class -> its resource's outline icon (or None)."""
    cdo = _cdo(objclass)
    if cdo is None:
        return None
    res = _getattr(cdo, 'Resource', 'ItemResource')
    return _getattr(res, 'Icon', 'MinimapIcon', 'Texture') if res is not None else None


# ── data layer: option pools from GameData ───────────────────────────────────

def _build_pools():
    cache = globals().get('_mb_pools')
    if cache is not None:
        return cache

    gd = _live('/Script/FSD.GameData')
    ms = gd.MissionSetup

    def type_entry(it):
        t = it.Template
        return {'name': t._name, 'label': _ftext(t, 'MissionName') or _pretty(t._name, 'MissionType_'),
                'obj': t}

    def biome_entry(it):
        b = it.Biome
        nm = BIOME_ICON.get(b._name)
        icon = _tex(_MILE % (nm, nm)) if nm else None
        return {'name': b._name, 'label': _ftext(b, 'BiomeName') or _pretty(b._name, 'BIOME_'),
                'obj': b, 'icon': icon}

    def mut_entry(m):
        return {'name': m._name, 'label': _ftext(m, 'Name', 'MutatorName', 'Title') or _pretty(m._name, 'MMUT_'),
                'obj': m, 'icon': _tex(_getattr(m, 'Icon', 'MutatorIcon', 'Texture')), 'tint': TINT_MUT}

    def warn_entry(w):
        return {'name': w._name, 'label': _ftext(w, 'Title', 'WarningName', 'Name') or _pretty(w._name, 'WRN_'),
                'obj': w, 'icon': _tex(_getattr(w, 'Icon')), 'tint': TINT_WARN}

    def chal_entry(c):
        return {'name': c._name, 'label': _ftext(c, 'Name', 'Title') or _pretty(c._name, 'MC_', 'CH_'),
                'obj': c, 'icon': _tex(CHALLENGE_ICON)}

    def diff_entry(d, i):
        return {'name': d._name, 'label': _ftext(d, 'ShortDifficultyName') or ('Hazard %d' % (i + 1)),
                'obj': d, 'level': i + 1}

    def hazplus_entry(setup):
        info = setup.Info
        return {'name': setup._name, 'label': _str(info.Title) or _pretty(setup._name, 'DMI_'),
                'icon': _tex(_getattr(info, 'Icon')), 'obj': setup}

    def desc_entry(path, prefix):
        o = ue.load(path)
        lvl = 0
        try:
            lvl = int(o.NumberOfDots)
        except Exception:
            pass
        return {'name': o._name, 'label': _pretty(o._name, prefix), 'obj': o, 'level': lvl}

    types = [type_entry(it) for it in ms.AvailableMissions if 'Tutorial' not in it.Template._name]
    types.sort(key=lambda e: e['label'].lower())

    pools = {
        'type': types,
        'biome': sorted([biome_entry(it) for it in ms.Biomes], key=lambda e: e['label'].lower()),
        'mutator': sorted([mut_entry(m) for m in ms.Mutators], key=lambda e: e['label'].lower()),
        'warning': sorted([warn_entry(w) for w in ms.Warnings], key=lambda e: e['label'].lower()),
        'challenge': sorted([chal_entry(c) for c in ms.Challenges], key=lambda e: e['label'].lower()),
        'difficulty': [diff_entry(d, i) for i, d in enumerate(gd.Difficulty.DifficultySettings)],
        'complexity': [desc_entry(p, 'MD_Complexity_') for p in COMPLEXITY],
        'length': [desc_entry(p, 'MD_Duration_') for p in DURATION],
        'hazplus': [hazplus_entry(x) for x in gd.Difficulty.DifficultyMutators],
    }
    globals()['_mb_pools'] = pools
    return pools


def _secondaries_for(template):
    """[{name,label,obj(class),icon}] for a template's secondary objectives, +none."""
    out = [{'name': None, 'label': '(none)', 'obj': None, 'icon': None}]
    try:
        for c in (template.SecondaryObjectives or []):
            sn = SEC_ICON.get(c._name)
            icon = _tex(_RESICON % (sn, sn)) if sn else _secondary_icon(c)
            out.append({'name': c._name, 'label': _pretty(c._name, 'OBJ_2nd_'),
                        'obj': c, 'icon': icon})
    except Exception as e:
        print('[mission_builder] secondary list err:', e)
    return out


# ── UMG plumbing ─────────────────────────────────────────────────────────────

def _W():
    d = {k: U('/Script/UMG.' + k) for k in (
        'UserWidget', 'WidgetTree', 'CanvasPanel', 'VerticalBox', 'HorizontalBox',
        'WrapBox', 'ScrollBox', 'SizeBox', 'Border', 'Image', 'TextBlock', 'Button',
        'EditableTextBox', 'Overlay')}
    d['WBL'] = U('/Script/UMG.Default__WidgetBlueprintLibrary')
    return d


def _icls(kind):
    p = _ITEM_WIDGETS[kind]
    short = p.rsplit('/', 1)[-1]
    return U(p + '.' + short + '_C')


def _pc():
    gi = _live('/Script/Engine.GameInstance')
    return gi.LocalPlayers[0].PlayerController if gi else None


def _slate(rgba):
    return {'SpecifiedColor': rgba, 'ColorUseRule': 0}


def _font(size, weight='Bold'):
    f = U('/Script/UMG.Default__TextBlock').Font
    f.Size = int(size)
    return f


def _set_input(pc, on):
    W = _W()
    try:
        pc.bShowMouseCursor = on
    except Exception:
        pass
    try:
        if on:
            try:
                W['WBL'].SetInputMode_GameAndUIEx(pc, None, 0, False)
            except Exception:
                W['WBL'].SetInputMode_GameAndUI(pc)
        else:
            W['WBL'].SetInputMode_GameOnly(pc)
    except Exception:
        pass


# ── mission assembly (proven fresh-build recipe) ─────────────────────────────

_COPY_FIELDS = ['Biome', 'Seed', 'GlobalSeed', 'Template', 'MissionName',
                'PrimaryObjective', 'SecondaryObjectives', 'MissionWarnings',
                'MissionMutator', 'MissionChallenge', 'ComplexityLimit',
                'DurationLimit', 'MissionDNA', 'MissionStructure',
                'IsInSeasonEventZone', 'WouldBeInSeasonEventZone',
                'CanHaveMutators', 'LoaderLevelSequence']


def build_mission(sel):
    gi = _live('/Script/Engine.GameInstance')
    mgm = _live('/Script/FSD.MissionGenerationManager')
    base = mgm.GetAvailableMissions()[0]

    GM = U('/Script/FSD.GeneratedMission')
    m = ue.new_object(GM, outer=gi, flags=ue.EObjectFlags.RF_Transient)
    for f in _COPY_FIELDS:
        try:
            setattr(m, f, getattr(base, f))
        except Exception:
            pass

    tmpl = sel['template']
    m.Template = tmpl
    try:
        m.PrimaryObjective = tmpl.PrimaryObjective
    except Exception:
        pass
    try:
        dna = tmpl.DNA
        if dna and len(dna):
            m.MissionDNA = dna[0]
    except Exception:
        pass

    m.Biome = sel['biome']
    m.ComplexityLimit = sel['complexity']
    m.DurationLimit = sel['length']
    m.MissionWarnings = list(sel.get('warnings', []))
    m.MissionMutator = sel.get('mutator')
    m.MissionChallenge = sel.get('challenge')
    m.CanHaveMutators = bool(sel.get('mutator'))
    m.SecondaryObjectives = [sel['secondary']] if sel.get('secondary') else []
    try:
        m.Seed = int(sel.get('seed') or 0)
    except Exception:
        pass
    m.add_to_root()
    return m


def select_mission(sel):
    """Build a fresh mission and select it on the GameInstance (+ set desired
    hazard). Does NOT travel — launch it normally from the in-game drop pod /
    launch terminal, which reads the selected mission."""
    gi = _live('/Script/Engine.GameInstance')
    if gi is None:
        print('[mission_builder] no GameInstance')
        return False
    m = build_mission(sel)
    gi.SetSelectedMission(m, True)

    # Apply difficulty via the GAMESTATE's DifficultyManager (the gamemode's own
    # manager has a null GameState back-ref and crashes). Do it AFTER selecting a
    # mission so the OnDifficultyChanged broadcast has a valid GeneratedMission.
    #  - base hazard: SelectDifficulty(index)
    #  - Hazard 5+ mods: SelectGameDifficulty(index, [DifficultyMutatorItem]) where
    #    each item = {SetupItem: DMI_* object, AppliedCount: tier, HazardBonus}.
    #    (An item with a null SetupItem crashes GetTotalHazardBonus — always set it.)
    idx = sel.get('difficulty_index')
    hp = sel.get('hazplus') or []
    gs = _live('/Script/FSD.FSDGameState')
    dm = getattr(gs, 'DifficultyManagerComponent', None) if gs else None
    if dm is not None and idx is not None:
        try:
            if hp:
                items = [{'SetupItem': o, 'AppliedCount': int(t),
                          'HazardBonus': float(_getattr(o, 'HazardBonus') or 0.0) * int(t)}
                         for o, t in hp]
                dm.SelectGameDifficulty(int(idx), items)
            else:
                dm.SelectDifficulty(int(idx))
        except Exception as e:
            print('[mission_builder] difficulty apply failed:', e)
    if sel.get('difficulty') is not None:
        try:
            gi.DesiredDifficulty = sel['difficulty']
        except Exception:
            pass
    if hp:
        print('[mission_builder] Haz+ applied: %s'
              % ', '.join('%s=T%d' % (o._name, t) for o, t in hp))
    print('[mission_builder] selected: %s / %s / haz=%s — now launch it from the '
          'drop pod / launch terminal.' % (
              m.Template._name, m.Biome._name,
              sel['difficulty']._name if sel.get('difficulty') else '?'))
    return True


def launch_mission(sel):
    """Select the mission and immediately travel into it (Space Rig only)."""
    if not select_mission(sel):
        return False
    gm = _live('/Script/FSD.FSDGameModeSpaceRig')
    if gm is None:
        print('[mission_builder] not on the Space Rig — cannot travel')
        return False
    gm.InstantlyStartMission()
    return True


# ── panel ────────────────────────────────────────────────────────────────────

def _build(pc):
    W = _W()
    pools = _build_pools()

    w = W['WBL'].Create(pc, W['UserWidget'], pc)
    tree = ue.new_object(W['WidgetTree'], outer=w, name='PyMissionBuilderTree')
    canvas = ue.new_object(W['CanvasPanel'], outer=tree, name='PyMissionBuilderCanvas')
    tree.RootWidget = canvas
    w.WidgetTree = tree

    def mk(cls, name=None):
        return ue.new_object(cls, outer=tree, name=name) if name else ue.new_object(cls, outer=tree)

    def text(size, rgba, s='', weight='Bold'):
        tb = mk(W['TextBlock'])
        tb.SetFont(_font(size, weight))
        tb.SetColorAndOpacity(_slate(rgba))
        if s:
            tb.SetText(_text(s))
        return tb

    def image(tex=None, color=None):
        im = mk(W['Image'])
        if tex is not None:
            im.SetBrushFromTexture(tex, False)
        if color is not None:
            im.SetColorAndOpacity(color)
        return im

    def sized(child, sw, sh=None):
        sb = mk(W['SizeBox'])
        sb.SetWidthOverride(sw)
        if sh is not None:
            sb.SetHeightOverride(sh)
        sb.AddChild(child)
        return sb

    def slot(s, size=AUTO, pad=None, h=None, v='center'):
        if size is not None:
            s.SetSize(size)
        if pad is not None:
            s.SetPadding(pad)
        if h is not None:
            s.SetHorizontalAlignment(HALIGN[h])
        if v is not None:
            s.SetVerticalAlignment(VALIGN[v])
        return s

    def hadd(box, child, **kw):
        return slot(box.AddChildToHorizontalBox(child), **kw)

    def vadd(box, child, **kw):
        return slot(box.AddChildToVerticalBox(child), **kw)

    def game_widget(kind):
        host = W['WBL'].Create(pc, _icls(kind), pc)
        return host

    st = {'w': w, 'tree': tree, 'pc': pc, 'handles': [],
          'sel': {}, 'warnings': set(), 'scalar': {}, 'grids': {},
          'seed': mk(W['EditableTextBox']), 'sec_wrap': None, 'sec_buttons': {}}

    # ── single/multi-select icon grid ──
    def cell_content(entry, kind):
        """Build a fixed-size button-content widget for one option."""
        if kind == 'type':
            badge = game_widget('badge')
            try:
                badge.SetTemplate(entry['obj'], False)
            except Exception as e:
                print('[mission_builder] badge err:', e)
            return sized(badge, 54.0, 46.0)   # flat-top hexagon: wider than tall
        ic = entry.get('icon')
        if ic is not None:
            im = image(ic, entry.get('tint') or C_WHITE)
            return sized(im, 38.0, 38.0)
        # no icon -> short label
        return sized(text(9, C_TXT, entry['label'][:8], 'Regular'), 38.0, 38.0)

    def make_grid(entries, key, kind, multi=False, on_change=None):
        wrap = mk(W['WrapBox'])
        try:
            wrap.SetInnerSlotPadding({'X': 5.0, 'Y': 5.0})
        except Exception:
            pass
        buttons = {}
        cellsz = 60.0 if kind == 'type' else 46.0

        def paint():
            for nm, b in buttons.items():
                if multi:
                    on = nm in st['warnings']
                    b.SetBackgroundColor(C_WARN_ON if on else C_CELL)
                else:
                    sel = st['sel'].get(key)
                    on = sel is not None and sel['name'] == nm
                    b.SetBackgroundColor(C_CELL_ON if on else C_CELL)

        def click(entry):
            if multi:
                nm = entry['name']
                st['warnings'].discard(nm) if nm in st['warnings'] else st['warnings'].add(nm)
            else:
                st['sel'][key] = entry
            paint()
            if on_change:
                on_change(entry)

        for e in entries:
            b = mk(W['Button'])
            b.SetToolTipText(_text(e['label']))
            b.SetContent(cell_content(e, kind))
            buttons[e['name']] = b
            st['handles'].append(b.OnClicked.add_callable(lambda e=e: click(e)))
            wrap.AddChild(sized(b, cellsz, cellsz))
        st['grids'][key] = {'wrap': wrap, 'buttons': buttons, 'paint': paint}
        paint()          # colour the initial selection immediately
        return wrap

    # ── scalar cycler (game DotMarker value; clamps at ends, no wrap) ──
    def cycler(form, label, key, entries, dot_kind, default=0):
        st['scalar'][key] = max(0, min(len(entries) - 1, default))
        host = game_widget(dot_kind)
        lbl = text(15, C_VAL, '', 'Bold')

        def refresh():
            e = entries[st['scalar'][key]]
            try:
                if dot_kind == 'dot_haz':
                    host.SetData(e['obj'])
                else:
                    host.SetData(int(e.get('level') or 0))
            except Exception as ex:
                print('[mission_builder] %s set err: %s' % (key, ex))
            lbl.SetText(_text(e['label']))

        def cyc(d):
            st['scalar'][key] = max(0, min(len(entries) - 1, st['scalar'][key] + d))
            refresh()

        def arrow(txt, d):
            b = mk(W['Button'])
            b.SetBackgroundColor(C_ARROW)
            b.SetContent(text(16, C_TXT, txt))
            st['handles'].append(b.OnClicked.add_callable(lambda: cyc(d)))
            return b

        # header (matches the grid sections) then a plain arrow row
        vadd(form, text(13, C_HDR, label.upper(), 'Bold'),
             pad={'Left': 2.0, 'Top': 6.0, 'Right': 0.0, 'Bottom': 3.0})
        row = mk(W['HorizontalBox'])
        hadd(row, sized(arrow('<', -1), 30.0, 28.0), v='center')
        hadd(row, sized(host, 132.0, 40.0), v='center', h='center',
             pad={'Left': 6.0, 'Top': 0.0, 'Right': 6.0, 'Bottom': 0.0})
        hadd(row, sized(arrow('>', 1), 30.0, 28.0), v='center')
        hadd(row, lbl, v='center', pad={'Left': 14.0, 'Top': 0.0, 'Right': 0.0, 'Bottom': 0.0})
        vadd(form, row, pad={'Left': 0.0, 'Top': 0.0, 'Right': 0.0, 'Bottom': 4.0})
        st['grids'][key] = {'refresh': refresh}
        return refresh

    # ── Hazard 5+ modifier grid (each cell cycles Off -> I -> II) ──
    # Each cell hosts the game's own UI_HazPlus_Icon widget; SetMutator(struct)
    # draws the right difficulty icon + I/II label. We only apply the icon tint the
    # game's parent widget would (armed orange / off dim), lifted from the live HUD.
    def make_hazplus(entries):
        st['hazplus'] = {}          # name -> tier (1 or 2)
        wrap = mk(W['WrapBox'])
        try:
            wrap.SetInnerSlotPadding({'X': 5.0, 'Y': 5.0})
        except Exception:
            pass
        cells = {}

        def apply(e):
            c = cells[e['name']]
            t = st['hazplus'].get(e['name'], 0)
            obj = e['obj']
            bonus = float(_getattr(obj, 'HazardBonus') or 0.0) * t
            try:
                c['host'].SetMutator({'SetupItem': obj, 'AppliedCount': t,
                                      'HazardBonus': bonus})
                c['host'].Icon_Image.SetColorAndOpacity(C_HAZ_ON if t else C_HAZ_OFF)
            except Exception as ex:
                print('[mission_builder] hazplus icon err:', ex)
            c['btn'].SetBackgroundColor(
                C_TIER2 if t == 2 else C_TIER1 if t == 1 else C_CELL)

        def paint():
            for e in entries:
                apply(e)

        def click(e):
            t = (st['hazplus'].get(e['name'], 0) + 1) % 3
            if t == 0:
                st['hazplus'].pop(e['name'], None)
            else:
                st['hazplus'][e['name']] = t
            apply(e)

        for e in entries:
            b = mk(W['Button'])
            b.SetToolTipText(_text(e['label']))
            host = game_widget('haz_icon')
            b.SetContent(sized(host, 38.0, 38.0))
            cells[e['name']] = {'btn': b, 'host': host}
            st['handles'].append(b.OnClicked.add_callable(lambda e=e: click(e)))
            wrap.AddChild(sized(b, 46.0, 46.0))
        st['grids']['hazplus'] = {'paint': paint}
        paint()
        return wrap

    # ── assemble the scrollable form ──
    IN_W = PW - 2 * PAD
    scroll = mk(W['ScrollBox'])
    form = mk(W['VerticalBox'])
    scroll.AddChild(form)

    def header(t):
        vadd(form, text(13, C_HDR, t, 'Bold'),
             pad={'Left': 2.0, 'Top': 6.0, 'Right': 0.0, 'Bottom': 3.0})

    vadd(form, text(20, C_TITLE, 'CUSTOM MISSION', 'Bold'),
         pad={'Left': 2.0, 'Top': 0.0, 'Right': 0.0, 'Bottom': 6.0})

    header('MISSION TYPE')
    st['sel']['type'] = pools['type'][0]
    vadd(form, make_grid(pools['type'], 'type', 'type',
                         on_change=lambda e: rebuild_secondary()))

    header('BIOME')
    st['sel']['biome'] = pools['biome'][0]
    vadd(form, make_grid(pools['biome'], 'biome', 'icon'))

    # scalar cyclers
    diff_default = 2
    try:
        dd = _live('/Script/Engine.GameInstance').DesiredDifficulty
        if dd is not None:
            for i, e in enumerate(pools['difficulty']):
                if e['name'] == dd._name:
                    diff_default = i
                    break
    except Exception:
        pass
    cyc_diff = cycler(form, 'Difficulty', 'difficulty', pools['difficulty'], 'dot_haz', diff_default)
    cyc_cx = cycler(form, 'Complexity', 'complexity', pools['complexity'], 'dot_cx', 1)
    cyc_ln = cycler(form, 'Length', 'length', pools['length'], 'dot_ln', 0)

    header('HAZARD 5+ MODIFIERS  (click: off / I / II — applies at Hazard 5)')
    vadd(form, make_hazplus(pools['hazplus']))

    header('MUTATOR')
    NONE_MUT = {'name': None, 'label': '(none)', 'obj': None, 'icon': None}
    st['sel']['mutator'] = NONE_MUT
    vadd(form, make_grid([NONE_MUT] + pools['mutator'], 'mutator', 'icon'))

    header('CHALLENGE')
    NONE_CH = {'name': None, 'label': '(none)', 'obj': None, 'icon': None}
    st['sel']['challenge'] = NONE_CH
    vadd(form, make_grid([NONE_CH] + pools['challenge'], 'challenge', 'icon'))

    header('SECONDARY OBJECTIVE')
    st['sel']['secondary'] = {'name': None, 'label': '(none)', 'obj': None, 'icon': None}
    sec_holder = mk(W['VerticalBox'])
    st['sec_holder'] = sec_holder
    vadd(form, sec_holder)

    def rebuild_secondary():
        holder = st['sec_holder']
        try:
            holder.ClearChildren()
        except Exception:
            pass
        secs = _secondaries_for(st['sel']['type']['obj'])
        st['sel']['secondary'] = secs[0]
        holder.AddChildToVerticalBox(make_grid(secs, 'secondary', 'icon'))

    header('WARNINGS  (multi-select)')
    vadd(form, make_grid(pools['warning'], 'warning', 'icon', multi=True))

    # seed row
    seedrow = mk(W['HorizontalBox'])
    hadd(seedrow, sized(text(13, C_HDR, 'Seed', 'Regular'), 96.0), v='center')
    st['seed'].SetText(_text('1'))
    hadd(seedrow, sized(st['seed'], 150.0, 30.0), v='center')

    def randomize():
        import random
        st['seed'].SetText(_text(str(random.randint(1, 2000000000))))
    rb = mk(W['Button'])
    rb.SetBackgroundColor(C_ARROW)
    rb.SetContent(text(12, C_TXT, 'Random'))
    st['handles'].append(rb.OnClicked.add_callable(randomize))
    hadd(seedrow, sized(rb, 88.0, 26.0), v='center',
         pad={'Left': 8.0, 'Top': 0.0, 'Right': 0.0, 'Bottom': 0.0})
    vadd(form, seedrow, pad={'Left': 0.0, 'Top': 8.0, 'Right': 0.0, 'Bottom': 8.0})

    # actions
    def _do_select():
        s = st['sel']
        sel = {
            'template': s['type']['obj'], 'biome': s['biome']['obj'],
            'complexity': pools['complexity'][st['scalar']['complexity']]['obj'],
            'length': pools['length'][st['scalar']['length']]['obj'],
            'difficulty': pools['difficulty'][st['scalar']['difficulty']]['obj'],
            'difficulty_index': st['scalar']['difficulty'],
            'mutator': s['mutator']['obj'], 'challenge': s['challenge']['obj'],
            'secondary': s['secondary']['obj'],
            'warnings': [e['obj'] for e in pools['warning'] if e['name'] in st['warnings']],
            'seed': _str(st['seed'].GetText()).strip() or '1',
            # Haz+ modifiers: list of (setup-item obj, tier 1|2)
            'hazplus': [(e['obj'], st['hazplus'][e['name']])
                        for e in pools['hazplus'] if e['name'] in st['hazplus']],
        }
        globals()['_mb_last'] = sel
        _teardown(st)
        select_mission(sel)

    actions = mk(W['HorizontalBox'])
    lb = mk(W['Button'])
    lb.SetBackgroundColor(C_LAUNCH)
    lb.SetContent(text(15, C_TXT, 'SELECT MISSION'))
    st['handles'].append(lb.OnClicked.add_callable(_do_select))
    hadd(actions, sized(lb, 250.0, 40.0), v='center')
    cb = mk(W['Button'])
    cb.SetBackgroundColor(C_CLOSE)
    cb.SetContent(text(13, C_TXT, 'Close'))
    st['handles'].append(cb.OnClicked.add_callable(lambda: _teardown(st)))
    hadd(actions, sized(cb, 100.0, 40.0), v='center',
         pad={'Left': 12.0, 'Top': 0.0, 'Right': 0.0, 'Bottom': 0.0})
    vadd(form, actions, pad={'Left': 0.0, 'Top': 4.0, 'Right': 0.0, 'Bottom': 0.0})

    # backdrop (Border draws a solid fill; a no-texture Image draws nothing)
    bg = mk(W['Border'])
    bg.SetBrushColor(C_PANEL)
    bs = canvas.AddChildToCanvas(bg)
    bs.SetAutoSize(False)
    bs.SetPosition({'X': PX, 'Y': PY})
    bs.SetSize({'X': PW, 'Y': PH})
    bs.SetZOrder(0)

    # scroll: AutoSize True so the inner SizeBox's fixed dims drive the slot
    ss = canvas.AddChildToCanvas(sized(scroll, IN_W, PH - 2 * PAD))
    ss.SetAutoSize(True)
    ss.SetPosition({'X': PX + PAD, 'Y': PY + PAD})
    ss.SetZOrder(2)

    w.AddToViewport(30000)
    w.add_to_root()
    tree.add_to_root()

    # initial paint / value set (widgets are on-viewport now)
    for key in ('type', 'biome', 'mutator', 'challenge', 'hazplus'):
        st['grids'][key]['paint']()
    cyc_diff(); cyc_cx(); cyc_ln()
    rebuild_secondary()

    _set_input(pc, True)
    return st


def _teardown(st):
    if not st:
        return
    for h in st.get('handles', []):
        try:
            h.unbind()
        except Exception:
            pass
    for step in (lambda: _set_input(st['pc'], False),
                 lambda: st['w'].RemoveFromParent(),
                 lambda: st['w'].remove_from_root(),
                 lambda: st['tree'].remove_from_root()):
        try:
            step()
        except Exception:
            pass
    globals().pop('_mission_builder', None)


def _purge_all():
    for w in list(ue.objects_of_class('/Script/UMG.UserWidget')):
        try:
            t = w.WidgetTree
            if t is not None and t._name.startswith('PyMissionBuilderTree'):
                w.RemoveFromParent()
                try: w.remove_from_root()
                except Exception: pass
                try: t.remove_from_root()
                except Exception: pass
        except Exception:
            pass


def mission_builder_close():
    _teardown(globals().get('_mission_builder'))
    _purge_all()


def mission_builder_open(pc=None):
    mission_builder_close()
    pc = pc or _pc()
    if pc is None:
        print('mission_builder: no PlayerController — is the game in a level?')
        return None
    if _live('/Script/FSD.FSDGameModeSpaceRig') is None:
        print('mission_builder: not on the Space Rig — LAUNCH only works from the rig.')
    st = _build(pc)
    globals()['_mission_builder'] = st
    print('mission_builder: open. Click icons to pick, then SELECT MISSION '
          '(launch it from the drop pod / terminal).')
    return st


globals()['mission_builder_open'] = mission_builder_open
globals()['mission_builder_close'] = mission_builder_close
globals()['build_mission'] = build_mission
globals()['select_mission'] = select_mission
globals()['launch_mission'] = launch_mission


mission_builder_open()
