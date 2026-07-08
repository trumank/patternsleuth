"""Missions HUD — a right-side overlay listing every mission available on the
Space Rig, composed from stock UMG widgets to match the game's own mission-board
icon widget (ITM_MisSel_MissionIconBasic):

  * Hexagon badge = three stacked BasicShape_Hexagon01 layers (mission-color
    frame, dark center, mission-type icon).
  * Complexity / length pips = the game's OWN DotMarker widgets
    (ITM_MisSel_DotMarker_CaveDifficulty / _MissionLength), instantiated and
    filled via SetData(int Level) so they're pixel-identical to the board.
  * Warnings / anomalies = MisMap_Warning_Icon_* icons flanking the badge
    (warnings red, lower-right; mutator gold, lower-left).
  * Resource / mineral icons = Icons_Resources_* on the objective line.

Data (all off the persistent GameInstance, so it survives travel):
    MissionGenerationManager.GetAvailableMissions() -> [GeneratedMission]
      .Biome.BiomeName    .Template.MissionIcon / .MissionColor
      .PrimaryObjective/.SecondaryObjectives -> CDO <resource>.Icon / .Color
      .ComplexityLimit.NumberOfDots / .DurationLimit.NumberOfDots  (1..3, may be None)
      .MissionWarnings[].Icon      .MissionMutator.Icon

Lifecycle (one persistent tick): shown only on the Space Rig, parented under the
game HUD's RootCanvasPanel so it inherits HUD visibility, self-heals across
travel, roster polled on a throttle and rows rebuilt only when the set changes.
State lives under one module global dict `_MISSIONS_HUD`; re-run to rebuild.
"""


def _install():
    import re

    G = '_MISSIONS_HUD'
    TREE_TAG = 'PyMissionsHUDTree'

    TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

    UserWidget  = ue.find('/Script/UMG.UserWidget')
    WidgetTree  = ue.find('/Script/UMG.WidgetTree')
    CanvasPanel = ue.find('/Script/UMG.CanvasPanel')
    Image       = ue.find('/Script/UMG.Image')
    TextBlock   = ue.find('/Script/UMG.TextBlock')
    HorizontalBox = ue.find('/Script/UMG.HorizontalBox')
    VerticalBox = ue.find('/Script/UMG.VerticalBox')
    Border      = ue.find('/Script/UMG.Border')
    SizeBox     = ue.find('/Script/UMG.SizeBox')
    WBL         = ue.find('/Script/UMG.Default__WidgetBlueprintLibrary')
    KTL         = ue.find('/Script/Engine.Default__KismetTextLibrary')

    RIG_GM = '/Script/FSD.FSDGameModeSpaceRig'
    MGM    = '/Script/FSD.MissionGenerationManager'

    # ── game assets (the exact ones the game's own mission board uses) ──────────
    FONT   = '/Game/Art/Fonts/Rift_Soft.Rift_Soft'
    HEX    = '/Game/UI/_GlobalAssets/BasicShapes/BasicShape_Hexagon01.BasicShape_Hexagon01'
    # The game's own complexity / length pip widgets (each has SetData(int Level)).
    DOT_CX = '/Game/UI/Menu_MissionSelectionMK3/ITM_MisSel_DotMarker_CaveDifficulty.ITM_MisSel_DotMarker_CaveDifficulty_C'
    DOT_LN = '/Game/UI/Menu_MissionSelectionMK3/ITM_MisSel_DotMarker_MissionLength.ITM_MisSel_DotMarker_MissionLength_C'

    # ── layout (panel-local px; container-driven, so most sizing is automatic) ──
    # Tree: VerticalBox(rows) → Border(strip, padded) → HorizontalBox(badge |
    # Fill text column | pips). Only the badge and pip clusters (overlapping /
    # scaled compositions) use a small absolute CanvasPanel.
    PANEL_W   = 384.0
    MARGIN    = 24.0
    ROW_PAD   = 10.0          # Border inner padding: content inset from strip edges
    ROW_GAP   = 2.0           # vertical gap between row strips
    MAX_ROWS  = 18            # rows rendered; the rig usually has this many or fewer
    ICON      = 42.0          # left hex badge (fixed SizeBox)
    DOT_SCALE = 0.5           # render scale for the reused game DotMarker widgets
    PIP_ZONE  = 118.0         # fixed width reserved for both pip markers (title line)
    PIP_H     = 22.0          # fixed height reserved for the (scaled) pip markers
    SM        = 18.0          # resource icons on the objective line
    WSZ       = 21.0          # warning / anomaly icons flanking the badge
    WOUT      = 8.0           # how far those icons sit outside the badge's edges

    POLL_EVERY = 3.0
    REBUILD_COOLDOWN = 0.5

    # ── box-slot constants ──────────────────────────────────────────────────────
    FILL = {'SizeRule': 1, 'Value': 1.0}   # box slot fills remaining space
    AUTO = {'SizeRule': 0, 'Value': 1.0}   # box slot sizes to child's desired size
    HALIGN = {'fill': 0, 'left': 1, 'center': 2, 'right': 3}
    VALIGN = {'fill': 0, 'top': 1, 'center': 2, 'bottom': 3}

    # ── colors (RGBA 0..1, linear) ──────────────────────────────────────────────
    # UMG ColorAndOpacity is linear. Screenshot-sampled sRGB values are converted
    # to linear here; game-sourced colors (MissionColor, pip/warning C&O) are
    # already linear and used as-is.
    C_ROW   = {'R': 0.0, 'G': 0.0, 'B': 0.0, 'A': 0.60}
    C_GOLD  = {'R': 0.913, 'G': 0.402, 'B': 0.021, 'A': 1.0}  # sRGB(245,170,40) -> linear
    C_COUNT = {'R': 0.72, 'G': 0.76, 'B': 0.82, 'A': 1.0}
    C_OBJ   = {'R': 1.0, 'G': 1.0, 'B': 1.0, 'A': 1.0}       # objective text, white
    C_WARN  = {'R': 1.0, 'G': 0.0, 'B': 0.0, 'A': 1.0}       # hazard red (MisMap_Warning_Icon C&O)
    C_ANOM  = {'R': 1.0, 'G': 0.715, 'B': 0.0, 'A': 1.0}     # anomaly yellow (MisMap_Mutator_Icons C&O)
    C_BLACK = {'R': 0.0, 'G': 0.0, 'B': 0.0, 'A': 1.0}       # badge Icon_BG (game value)
    C_WHITE = {'R': 1.0, 'G': 1.0, 'B': 1.0, 'A': 1.0}

    # ── tiny helpers ───────────────────────────────────────────────────────────
    def live(cls):
        return next((o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE), None)

    def _pc():
        gi = live('/Script/Engine.GameInstance')
        return gi.LocalPlayers[0].PlayerController if gi else None

    def _hud_canvas():
        """The game HUD's RootCanvasPanel. We parent the overlay here instead of the
        raw viewport so it inherits the HUD's visibility (hidden by the ESC / pause
        menu, torn down on travel). Path: MyHUD.HUD -> PlatformSafeZone ->
        RootCanvasPanel; walk down single-child wrappers to the first canvas."""
        pc = _pc()
        if pc is None:
            return None
        try:
            node = pc.MyHUD.HUD.WidgetTree.RootWidget
        except Exception:
            return None
        for _ in range(5):
            if node is None:
                return None
            if 'Canvas' in getattr(node, '_name', ''):
                return node
            try:
                node = node.GetChildAt(0)
            except Exception:
                return None
        return None

    def _text(s):
        return KTL.Conv_StringToText(str(s))

    def _str(ft):
        try:
            return KTL.Conv_TextToString(ft)
        except Exception:
            return ''

    def _slate(rgba):
        return {'SpecifiedColor': rgba, 'ColorUseRule': 0}

    _texcache = {}

    def _tex(path):
        if path not in _texcache:
            _texcache[path] = ue.load(path)
        return _texcache[path]

    _clscache = {}

    def _class(path):
        if path not in _clscache:
            _clscache[path] = ue.find(path) or ue.load(path)
        return _clscache[path]

    _rift = [None]

    def _font(size, weight='Bold'):
        if _rift[0] is None:
            _rift[0] = ue.load(FONT)
        f = ue.find('/Script/UMG.Default__TextBlock').Font
        f.FontObject = _rift[0]
        f.TypefaceFontName = 'Rift Soft ' + weight
        f.Size = int(size)
        return f

    def _getf(o, *names):
        for n in names:
            try:
                v = getattr(o, n)
                if v is not None:
                    return v
            except Exception:
                pass
        return None

    def _cdo(cls):
        if cls is None:
            return None
        try:
            pkg = cls._path.rsplit('.', 1)[0]
            return ue.find(pkg + '.Default__' + cls._name)
        except Exception:
            return None

    def _lin(color):
        if color is None:
            return C_WHITE
        try:
            r, g, b, a = color.R, color.G, color.B, color.A
        except Exception:
            return C_WHITE
        if isinstance(r, int) or r > 1.0 or g > 1.0 or b > 1.0:
            aa = (a / 255.0) if (isinstance(a, int) or a > 1.0) else a
            return {'R': r / 255.0, 'G': g / 255.0, 'B': b / 255.0, 'A': aa}
        return {'R': r, 'G': g, 'B': b, 'A': a}

    _PLURAL = re.compile(r'\{[^}]*\}\|plural\(\s*one=[^,]*,\s*other=([^)]*)\)')
    _BRACE = re.compile(r'\{[^}]*\}')

    def _clean(s):
        s = _PLURAL.sub(lambda m: m.group(1).strip(), s)
        s = _BRACE.sub('', s)
        return re.sub(r'\s+', ' ', s).strip()

    def _biome_minerals(m):
        """The biome's own secondary mineral icons (Biome.Resources[].Resource
        .Icon) — e.g. Crystalline Caverns -> Jadiz + Bismor. This is what the game
        shows per mission, not the objective's own resource."""
        out = []
        for sp in (_getf(m.Biome, 'Resources') or []):
            ic = _getf(_getf(sp, 'Resource'), 'Icon')
            if ic is not None:
                out.append(ic)
        return out[:2]

    def _dots(m, limit_attr, dna_attr):
        """Pip count: the mission's ComplexityLimit/DurationLimit, or — when those
        are null (deep-dive-style missions) — the MissionDNA CDO's Complexity/
        Duration, which the game itself falls back to."""
        o = _getf(m, limit_attr)
        if o is None:
            o = _getf(_cdo(_getf(m, 'MissionDNA')), dna_attr)
        return o.NumberOfDots if o is not None else 0

    # ── mission-roster read ────────────────────────────────────────────────────
    def _mission_rows():
        mgm = live(MGM)
        if mgm is None:
            return [], None
        try:
            missions = mgm.GetAvailableMissions()
        except Exception:
            return [], None
        rows, sig = [], []
        for m in missions:
            try:
                biome = m.Biome
                name = _str(biome.BiomeName) if biome else '?'
                tmpl = m.Template
                oc = _cdo(m.PrimaryObjective)
                obj = _clean(_str(_getf(oc, 'ObjectiveName', 'ObjectiveDescription',
                                        'MissionDescription')) or '')
                if not obj and tmpl:
                    obj = _str(tmpl.MissionName)
                mcol = _lin(_getf(tmpl, 'MissionColor'))   # raw, like the game
                mut = m.MissionMutator
                rows.append({
                    'biome': name.upper(),
                    'obj': obj.upper(),
                    'type_icon': _getf(tmpl, 'MissionIcon', 'MissionIconSmall'),
                    'mcol': mcol,
                    'cx': _dots(m, 'ComplexityLimit', 'Complexity'),
                    'ln': _dots(m, 'DurationLimit', 'Duration'),
                    'rsrc': _biome_minerals(m),
                    'warns': [_getf(w, 'Icon') for w in (m.MissionWarnings or []) if _getf(w, 'Icon')],
                    'mut': _getf(mut, 'Icon', 'MutatorIcon', 'Texture') if mut else None,
                })
                sig.append(m.Seed)
            except Exception:
                continue
        return rows[:MAX_ROWS], (tuple(sig), len(missions))

    # ── widget construction (container-driven) ──────────────────────────────────
    def _build(pc, rows, total):
        canvas = _hud_canvas()
        if canvas is None:
            return None            # HUD not ready yet; the tick will retry
        w = WBL.Create(pc, UserWidget, pc)
        tree = ue.new_object(WidgetTree, outer=w, name=TREE_TAG)
        root = ue.new_object(CanvasPanel, outer=tree, name='PyMissionsHUDRoot')
        tree.RootWidget = root
        w.WidgetTree = tree

        # — construction helpers (capture the fresh `tree`/`pc`) —
        def mk(cls, name=None):
            return (ue.new_object(cls, outer=tree, name=name) if name
                    else ue.new_object(cls, outer=tree))

        def text(size, rgba, s='', weight='Bold', clip=False):
            tb = mk(TextBlock)
            tb.SetFont(_font(size, weight))
            tb.SetColorAndOpacity(_slate(rgba))
            if s:
                tb.SetText(_text(s))
            if clip:
                # Clip to the allotted rect so a long title can't bleed into the pips.
                tb.SetClipping(1)
            return tb

        def image(tex=None, color=None):
            im = mk(Image)
            if tex is not None:
                im.SetBrushFromTexture(tex, False)
            if color is not None:
                im.SetColorAndOpacity(color)   # tints the texture (multiply) when set
            return im

        def _slot(slot, size, pad, h, v):
            if size is not None:
                slot.SetSize(size)
            if pad is not None:
                slot.SetPadding(pad)
            if h is not None:
                slot.SetHorizontalAlignment(HALIGN[h])
            if v is not None:
                slot.SetVerticalAlignment(VALIGN[v])
            return slot

        def hadd(box, child, size=AUTO, pad=None, h=None, v='center'):
            return _slot(box.AddChildToHorizontalBox(child), size, pad, h, v)

        def vadd(box, child, size=AUTO, pad=None, h=None, v=None):
            return _slot(box.AddChildToVerticalBox(child), size, pad, h, v)

        def sized(child, sw, sh=None, name=None):
            sb = mk(SizeBox, name)
            sb.SetWidthOverride(sw)
            if sh is not None:
                sb.SetHeightOverride(sh)
            sb.AddChild(child)
            return sb

        def canvas_img(canvas, x, y, cw, ch, tex=None, color=None, z=2):
            im = mk(Image)
            if tex is not None:
                im.SetBrushFromTexture(tex, False)
            if color is not None:
                im.SetColorAndOpacity(color)
            s = canvas.AddChildToCanvas(im)
            s.SetAutoSize(False)
            s.SetPosition({'X': x, 'Y': y})
            s.SetSize({'X': cw, 'Y': ch})
            s.SetZOrder(z)
            return im

        def badge(r):
            """Hexagon badge like ITM_MisSel_MissionIconBasic: bright-color frame +
            dark center + type icon, warnings/mutator flanking. Its layers overlap,
            so it stays a small absolute CanvasPanel — wrapped in a fixed ICON×ICON
            SizeBox so the surrounding boxes can lay it out."""
            canvas = mk(CanvasPanel)
            hexo = _tex(HEX)
            col = r['mcol']
            # Exact game layering (ITM_MisSel_MissionIconBasic), sampled live:
            #   ICON_Frame black, full  ICON_Outline mission-color, full
            #   Icon_BG    black        ICON_MissionType mission-color
            canvas_img(canvas, 0.0, 0.0, ICON, ICON, tex=hexo, color=C_BLACK, z=1)
            canvas_img(canvas, 0.0, 0.0, ICON, ICON, tex=hexo, color=col, z=2)
            d = ICON * 0.05
            canvas_img(canvas, d, d, ICON - 2 * d, ICON - 2 * d, tex=hexo, color=C_BLACK, z=3)
            if r['type_icon'] is not None:
                o = ICON * 0.025
                canvas_img(canvas, -o, -o, ICON + 2 * o, ICON + 2 * o,
                           tex=r['type_icon'], color=col, z=4)
            cy = ICON - WSZ + 3.0
            if r['mut'] is not None:
                canvas_img(canvas, -WOUT, cy, WSZ, WSZ, tex=r['mut'], color=C_ANOM, z=7)
            wx = ICON - WSZ + WOUT
            for tex in r['warns'][:2]:
                canvas_img(canvas, wx, cy, WSZ, WSZ, tex=tex, color=C_WARN, z=7)
                cy -= WSZ - 3.0    # stack extra warnings upward
            return sized(canvas, ICON, ICON)

        def dot_marker(canvas, path, level, right_x):
            dm = WBL.Create(pc, _class(path), pc)
            dm.SetData(int(level or 0))
            dm.SetRenderScale({'X': DOT_SCALE, 'Y': DOT_SCALE})
            # Scale about the top-right corner so the shrunk marker stays pinned
            # to the top of the pip box.
            dm.SetRenderTransformPivot({'X': 1.0, 'Y': 0.0})
            s = canvas.AddChildToCanvas(dm)
            s.SetAutoSize(True)
            s.SetAlignment({'X': 1.0, 'Y': 0.0})
            s.SetPosition({'X': right_x, 'Y': 0.0})
            return dm

        def pips(r):
            """The game's own length + complexity DotMarkers, render-scaled and
            right-aligned on a small absolute canvas (they're externally sized game
            widgets we shrink visually), wrapped in a fixed PIP_ZONE×PIP_H SizeBox."""
            canvas = mk(CanvasPanel)
            dot_marker(canvas, DOT_LN, r['ln'], PIP_ZONE)
            dot_marker(canvas, DOT_CX, r['cx'], PIP_ZONE - 58.0)
            return sized(canvas, PIP_ZONE, PIP_H)

        def row_widget(r):
            row = mk(HorizontalBox)
            hadd(row, badge(r), size=AUTO, v='center')

            col = mk(VerticalBox)
            # title line: biome name (fills) + pip markers (fixed width, right)
            title = mk(HorizontalBox)
            hadd(title, text(14, C_GOLD, r['biome'], 'Bold', clip=True), size=FILL, v='center')
            hadd(title, pips(r), size=AUTO, v='top')
            vadd(col, title, size=AUTO)
            # objective line: resource / mineral icons, then objective text (fills)
            obj = mk(HorizontalBox)
            for icon in r['rsrc']:
                # Wrap in a SizeBox to constrain the icon to SM×SM — otherwise it
                # renders at its 32px native size and inflates the objective line.
                hadd(obj, sized(image(tex=icon, color=C_WHITE), SM, SM),
                     size=AUTO, v='center',
                     pad={'Left': 0.0, 'Top': 0.0, 'Right': 2.0, 'Bottom': 0.0})
            hadd(obj, text(11, C_OBJ, r['obj'], 'Regular', clip=True), size=FILL, v='center',
                 pad={'Left': 3.0 if r['rsrc'] else 0.0, 'Top': 0.0, 'Right': 0.0, 'Bottom': 0.0})
            vadd(col, obj, size=AUTO, pad={'Left': 0.0, 'Top': 1.0, 'Right': 0.0, 'Bottom': 0.0})

            hadd(row, col, size=FILL, v='center',
                 pad={'Left': 12.0, 'Top': 0.0, 'Right': 0.0, 'Bottom': 0.0})

            # Border = the black row strip: brush color + inner padding.
            strip = mk(Border)
            strip.SetBrushColor(C_ROW)
            strip.SetPadding({'Left': ROW_PAD, 'Top': 3.0, 'Right': ROW_PAD, 'Bottom': 3.0})
            strip.AddChild(row)
            return strip

        # — assemble the column (VerticalBox → header + one Border per row) —
        col = mk(VerticalBox, 'PyMissionsHUDColumn')
        header = mk(HorizontalBox)
        hadd(header, text(15, C_GOLD, 'AVAILABLE MISSIONS', 'Bold'), size=FILL, v='center')
        hadd(header, text(15, C_COUNT, str(total), 'Bold'), size=AUTO, v='center')
        vadd(col, header, size=AUTO,
             pad={'Left': ROW_PAD + 2.0, 'Top': 0.0, 'Right': ROW_PAD + 2.0, 'Bottom': 6.0})
        for r in rows:
            vadd(col, row_widget(r), size=AUTO,
                 pad={'Left': 0.0, 'Top': 0.0, 'Right': 0.0, 'Bottom': ROW_GAP})

        # SizeBox fixes the overall width; the column auto-sizes its height, and the
        # canvas slot (autosize) anchors the whole thing top-right, MARGIN in.
        panel = sized(col, PANEL_W, name='PyMissionsHUDPanel')
        pslot = root.AddChildToCanvas(panel)
        pslot.SetAnchors({'Minimum': {'X': 1.0, 'Y': 0.0}, 'Maximum': {'X': 1.0, 'Y': 0.0}})
        pslot.SetAlignment({'X': 1.0, 'Y': 0.0})
        pslot.SetAutoSize(True)
        pslot.SetPosition({'X': -MARGIN, 'Y': MARGIN})

        # Attach under the game HUD's canvas (fill it; our inner SizeBox keeps the
        # top-right anchor) so the overlay lives inside the HUD, not the viewport.
        cslot = canvas.AddChildToCanvas(w)
        cslot.SetAnchors({'Minimum': {'X': 0.0, 'Y': 0.0}, 'Maximum': {'X': 1.0, 'Y': 1.0}})
        cslot.SetOffsets({'Left': 0.0, 'Top': 0.0, 'Right': 0.0, 'Bottom': 0.0})
        w.add_to_root()
        tree.add_to_root()
        return {'w': w, 'tree': tree}

    # ── teardown / purge ────────────────────────────────────────────────────────
    def _destroy(hud):
        if not hud:
            return
        for step in (lambda: hud['w'].RemoveFromParent(),
                     lambda: hud['w'].remove_from_root(),
                     lambda: hud['tree'].remove_from_root()):
            try:
                step()
            except Exception:
                pass

    def _purge_all():
        for w in list(ue.objects_of_class('/Script/UMG.UserWidget')):
            try:
                t = w.WidgetTree
                if t is not None and t._name.startswith(TREE_TAG):
                    w.RemoveFromParent()
                    try: w.remove_from_root()
                    except Exception: pass
                    try: t.remove_from_root()
                    except Exception: pass
            except Exception:
                pass

    def _alive(hud):
        if not hud:
            return False
        try:
            # Parented under the HUD canvas → alive while it still has a parent (the
            # HUD tears our child down with it on travel, which the tick then rebuilds).
            return hud['w'].GetParent() is not None
        except Exception:
            return False

    # ── per-frame driver ────────────────────────────────────────────────────────
    def _on_tick(world, dt):
        st = globals().get(G)
        if st is None:
            return
        on_rig = live(RIG_GM) is not None
        if not on_rig:
            if st['hud'] is not None or globals().get('_mh_present'):
                _destroy(st['hud'])
                _purge_all()
                st['hud'] = None
                st['sig'] = None
                globals()['_mh_present'] = False
            return
        globals()['_mh_present'] = True

        st['poll'] -= dt
        changed = False
        if st['poll'] <= 0.0:
            st['poll'] = POLL_EVERY
            rows, sig = _mission_rows()
            if sig is not None and sig != st['sig']:
                st['rows'] = rows
                st['sig'] = sig
                changed = True

        if not _alive(st['hud']) or changed:
            st['cool'] -= dt
            if not changed and st['cool'] > 0.0:
                return
            st['cool'] = REBUILD_COOLDOWN
            pc = _pc()
            if pc is None or not st['rows']:
                if st['sig'] is None:
                    st['poll'] = 0.0
                return
            _destroy(st['hud'])
            _purge_all()
            try:
                total = st['sig'][1] if st['sig'] else len(st['rows'])
                st['hud'] = _build(pc, st['rows'], total)
            except Exception as e:
                print('[missions_hud] build failed:', type(e).__name__, e)
                st['hud'] = None

    # ── idempotent re-arm ───────────────────────────────────────────────────────
    _prev = globals().get(G)
    if _prev is not None:
        try:
            _prev['tick'].remove()
        except Exception:
            pass
        _destroy(_prev.get('hud'))
    _purge_all()

    globals()[G] = {
        'tick': ue.on_tick(_on_tick),
        'hud': None,
        'rows': [],
        'sig': None,
        'poll': 0.0,
        'cool': 0.0,
    }
    print('missions_hud armed — game-composed, parented under the HUD overlay on '
          'the Space Rig, self-heals across travel. Re-run to rebuild.')


_install()
del _install
