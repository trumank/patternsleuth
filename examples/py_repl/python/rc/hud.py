"""A graphical + text HUD overlay, built entirely from stock UMG widgets.

No Blueprint and no custom widget class needed: we construct a bare
`UUserWidget`, hand-build its `WidgetTree` (a `CanvasPanel` root holding a
stack of `Image` and `TextBlock` widgets), add it to the viewport, then
resize/recolor/relabel those widgets every frame from live game state.

Text: FText marshals as a `ue.Text` handle. Raw Python strings are *not*
auto-converted, so we mint handles with `KismetTextLibrary.Conv_StringToText`
(the `_hud_text()` helper) and hand them to `TextBlock.SetText`.

    Title    "ROGUE HUD"  +  live 8-point compass heading (e.g. "W 276")
    Health   green->red meter + "HP cur / max"
    Speed    cyan meter + "SPD ###"  (horizontal velocity, uu/s)
    Compass  a strip with a sliding needle for camera yaw; red tick = North
    Camera   active camera world XYZ -- PlayerCameraManager.GetCameraLocation()
             follows the current view target, so it reads the player's eyes or
             the freecam camera automatically, whichever is active.

Self-healing: level travel tears down the viewport and GCs the widgets (even
root-pinned ones -- their object slots get recycled). A single persistent tick
detects the stale/removed HUD and rebuilds it (throttled), so it survives
travel between the Space Rig and stages.

Run once to arm; run again to tear down and rebuild (idempotent).

Everything below lives inside _install() so the script leaks nothing into the
shared REPL namespace except its persistence handles (`_hud`, `_hud_tick`,
`_hud_cooldown`) — no name here can collide with other scripts (freecam.py,
start_mission.py, …) sharing the same globals.
"""


def _install():
    import math

    TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

    UserWidget  = ue.find('/Script/UMG.UserWidget')
    WidgetTree  = ue.find('/Script/UMG.WidgetTree')
    CanvasPanel = ue.find('/Script/UMG.CanvasPanel')
    Image       = ue.find('/Script/UMG.Image')
    TextBlock   = ue.find('/Script/UMG.TextBlock')
    WBL         = ue.find('/Script/UMG.Default__WidgetBlueprintLibrary')
    KTL         = ue.find('/Script/Engine.Default__KismetTextLibrary')

    # Layout constants (canvas-local pixels; top-left origin).
    PANEL_X, PANEL_Y = 40.0, 40.0
    PANEL_W, PANEL_H = 340.0, 178.0
    BAR_X            = PANEL_X + 20.0
    BAR_W            = 300.0
    SPEED_REF        = 1200.0          # velocity (uu/s) that fills the speed bar
    REBUILD_COOLDOWN = 0.5             # seconds between rebuild attempts while stale

    # Colors (RGBA, 0..1).
    C_PANEL   = {'R': 0.02, 'G': 0.02, 'B': 0.03, 'A': 0.55}
    C_TRACK   = {'R': 0.10, 'G': 0.10, 'B': 0.12, 'A': 0.85}
    C_SPEED   = {'R': 0.20, 'G': 0.85, 'B': 0.95, 'A': 0.95}
    C_NEEDLE  = {'R': 1.00, 'G': 1.00, 'B': 1.00, 'A': 0.95}
    C_NORTH   = {'R': 0.90, 'G': 0.20, 'B': 0.20, 'A': 0.95}
    C_TICK    = {'R': 0.45, 'G': 0.45, 'B': 0.50, 'A': 0.80}
    C_TXT     = {'R': 0.95, 'G': 0.97, 'B': 1.00, 'A': 1.00}
    C_TXT_DIM = {'R': 0.55, 'G': 0.60, 'B': 0.70, 'A': 1.00}

    COMPASS = ('N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW')

    def live(cls):
        return next((o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE), None)

    def _hud_text(s):
        """Python str -> ue.Text (FText) handle."""
        return KTL.Conv_StringToText(s)

    def _slate(rgba):
        """FLinearColor rgba dict -> FSlateColor (ColorUseRule 0 = UseColor_Specified)."""
        return {'SpecifiedColor': rgba, 'ColorUseRule': 0}

    def _pawn_and_health():
        """Re-resolve every frame -- the pawn is replaced on death/respawn/travel."""
        gi = live('/Script/Engine.GameInstance')
        pc = gi.LocalPlayers[0].PlayerController if gi else None
        pawn = pc.K2_GetPawn() if pc else None
        hc = None
        if pawn is not None:
            try:
                hc = pawn.HealthComponent
            except Exception:
                hc = None
        return pc, pawn, hc

    def _add_box(tree, canvas, x, y, w, h, color, z=0):
        """Add a solid-color Image to the canvas at an absolute rect; return (img, slot)."""
        img = ue.new_object(Image, outer=tree)
        slot = canvas.AddChildToCanvas(img)
        slot.SetAutoSize(False)
        slot.SetPosition({'X': x, 'Y': y})
        slot.SetSize({'X': w, 'Y': h})
        slot.SetZOrder(z)
        img.SetColorAndOpacity(color)
        return img, slot

    def _add_label(tree, canvas, x, y, size, rgba, z=5, text=''):
        """Add an auto-sized TextBlock; return the TextBlock.

        Font must start from a struct with a real FontObject or nothing renders,
        so we clone the TextBlock CDO's font and only override its Size.
        """
        tb = ue.new_object(TextBlock, outer=tree)
        slot = canvas.AddChildToCanvas(tb)
        slot.SetAutoSize(True)
        slot.SetPosition({'X': x, 'Y': y})
        slot.SetZOrder(z)
        font = ue.find('/Script/UMG.Default__TextBlock').Font
        font.Size = float(size)
        tb.SetFont(font)
        tb.SetColorAndOpacity(_slate(rgba))
        if text:
            tb.SetText(_hud_text(text))
        return tb

    def _lerp(a, b, t):
        return a + (b - a) * t

    def _health_color(pct):
        # green (full) -> amber -> red (empty)
        return {'R': _lerp(0.15, 0.95, 1.0 - pct),
                'G': _lerp(0.90, 0.15, 1.0 - pct) if pct > 0.5 else _lerp(0.60, 0.90, pct * 2),
                'B': 0.20, 'A': 0.95}

    def _build_hud(pc):
        """Construct the whole widget tree and add it to the viewport. Returns the
        state dict of live widget handles (no tick)."""
        w = WBL.Create(pc, UserWidget, pc)
        tree = ue.new_object(WidgetTree, outer=w, name='PyHUDTree')
        canvas = ue.new_object(CanvasPanel, outer=tree, name='PyHUDCanvas')
        tree.RootWidget = canvas
        w.WidgetTree = tree

        # background panel
        _add_box(tree, canvas, PANEL_X, PANEL_Y, PANEL_W, PANEL_H, C_PANEL, z=0)

        # title row: label + live heading
        _add_label(tree, canvas, PANEL_X + 16.0, PANEL_Y + 4.0, 14, C_TXT_DIM, z=5, text='ROGUE HUD')
        lbl_head = _add_label(tree, canvas, PANEL_X + PANEL_W - 90.0, PANEL_Y + 4.0, 14, C_TXT, z=5)

        # health meter: track + fill (width + color animate) + overlaid value
        _add_box(tree, canvas, BAR_X, PANEL_Y + 30.0, BAR_W, 24.0, C_TRACK, z=1)
        hp_img, hp_slot = _add_box(tree, canvas, BAR_X, PANEL_Y + 30.0, BAR_W, 24.0, C_SPEED, z=2)
        lbl_hp = _add_label(tree, canvas, BAR_X + 8.0, PANEL_Y + 31.0, 14, C_TXT, z=5)

        # speed meter: track + fill + overlaid value
        _add_box(tree, canvas, BAR_X, PANEL_Y + 66.0, BAR_W, 18.0, C_TRACK, z=1)
        sp_img, sp_slot = _add_box(tree, canvas, BAR_X, PANEL_Y + 66.0, BAR_W, 18.0, C_SPEED, z=2)
        lbl_sp = _add_label(tree, canvas, BAR_X + 8.0, PANEL_Y + 65.0, 13, C_TXT, z=5)

        # compass strip: track, quarter ticks, north tick, needle
        _add_box(tree, canvas, BAR_X, PANEL_Y + 100.0, BAR_W, 12.0, C_TRACK, z=1)
        for frac in (0.25, 0.5, 0.75):                    # E / S / W marks
            _add_box(tree, canvas, BAR_X + BAR_W * frac - 1.0, PANEL_Y + 100.0, 2.0, 12.0, C_TICK, z=2)
        _add_box(tree, canvas, BAR_X - 1.0, PANEL_Y + 98.0, 3.0, 16.0, C_NORTH, z=3)   # North = left edge
        cmp_img, cmp_slot = _add_box(tree, canvas, BAR_X, PANEL_Y + 96.0, 4.0, 20.0, C_NEEDLE, z=4)

        # active-camera world coordinates
        lbl_pos = _add_label(tree, canvas, BAR_X, PANEL_Y + 120.0, 13, C_TXT, z=5)

        w.AddToViewport(10000)
        w.add_to_root()
        tree.add_to_root()

        return {'w': w, 'tree': tree, 'canvas': canvas,
                'hp_img': hp_img, 'hp_slot': hp_slot, 'sp_slot': sp_slot, 'cmp_slot': cmp_slot,
                'lbl_head': lbl_head, 'lbl_hp': lbl_hp, 'lbl_sp': lbl_sp, 'lbl_pos': lbl_pos,
                'cache': {}, 'cooldown': 0.0}

    def _set_text(st, tb, key, s):
        """Push a new FText only when the string changes (SetText is a ProcessEvent)."""
        cache = st['cache']
        if cache.get(key) != s:
            cache[key] = s
            tb.SetText(_hud_text(s))

    def _alive(st):
        """True if the HUD widget still exists and is on screen. Re-adds it if it
        merely fell out of the viewport; reports dead (needs full rebuild) on stale."""
        if st is None:
            return False
        try:
            if not st['w'].IsInViewport():
                st['w'].AddToViewport(10000)
            return True
        except Exception:
            return False          # object slot recycled -> rebuild from scratch

    def _update(st):
        pc, pawn, hc = _pawn_and_health()
        if pawn is None:
            return

        # health: meter width + color + "HP cur / max"
        if hc is not None:
            try:
                cur, mx = hc.GetHealth(), hc.GetMaxHealth()
            except Exception:
                cur, mx = 0.0, 1.0
            pct = max(0.0, min(1.0, cur / mx)) if mx else 0.0
            st['hp_slot'].SetSize({'X': BAR_W * pct, 'Y': 24.0})
            st['hp_img'].SetColorAndOpacity(_health_color(pct))
            _set_text(st, st['lbl_hp'], 'hp', 'HP  %d / %d' % (round(cur), round(mx)))
        else:
            st['hp_slot'].SetSize({'X': 0.0, 'Y': 24.0})
            _set_text(st, st['lbl_hp'], 'hp', 'HP  --')

        # speed: horizontal velocity magnitude vs reference top speed
        try:
            v = pawn.GetVelocity()
            spd = math.sqrt(v.X * v.X + v.Y * v.Y)
        except Exception:
            spd = 0.0
        frac = max(0.0, min(1.0, spd / SPEED_REF))
        st['sp_slot'].SetSize({'X': BAR_W * frac, 'Y': 18.0})
        _set_text(st, st['lbl_sp'], 'sp', 'SPD  %d' % round(spd))

        # compass + camera coords, both off the active camera (PlayerCameraManager
        # follows the current view target -> player eyes or freecam automatically).
        if pc is not None:
            pcm = pc.PlayerCameraManager
            try:
                yaw = pcm.GetCameraRotation().Yaw % 360.0
            except Exception:
                yaw = 0.0
            st['cmp_slot'].SetPosition({'X': BAR_X + (yaw / 360.0) * BAR_W - 2.0, 'Y': PANEL_Y + 96.0})
            card = COMPASS[int((yaw + 22.5) % 360.0 // 45.0)]
            _set_text(st, st['lbl_head'], 'head', '%-2s %03d' % (card, round(yaw)))
            try:
                loc = pcm.GetCameraLocation()
                _set_text(st, st['lbl_pos'], 'pos',
                          'X %d   Y %d   Z %d' % (round(loc.X), round(loc.Y), round(loc.Z)))
            except Exception:
                pass

    def _destroy(st):
        """Remove one HUD instance from the viewport and drop its root pins (guarded)."""
        if not st:
            return
        for step in (lambda: st['w'].RemoveFromParent(),
                     lambda: st['w'].remove_from_root(),
                     lambda: st['tree'].remove_from_root()):
            try:
                step()
            except Exception:
                pass

    def _purge_all():
        """Remove every HUD widget we've ever created (any PyHUDTree), so a re-run
        or a rebuild can never leave an orphaned/duplicate panel on screen. Returns
        the count removed."""
        n = 0
        for w in list(ue.objects_of_class('/Script/UMG.UserWidget')):
            try:
                tree = w.WidgetTree
                if tree is not None and tree._name.startswith('PyHUDTree'):
                    w.RemoveFromParent()
                    try:
                        w.remove_from_root()
                    except Exception:
                        pass
                    try:
                        tree.remove_from_root()
                    except Exception:
                        pass
                    n += 1
            except Exception:
                pass
        return n

    def _on_tick(world, dt):
        st = globals().get('_hud')
        if not _alive(st):
            # throttle rebuild attempts so we don't spawn 15 widgets/frame during travel
            cd = globals().get('_hud_cooldown', 0.0) - dt
            if cd > 0.0:
                globals()['_hud_cooldown'] = cd
                return
            globals()['_hud_cooldown'] = REBUILD_COOLDOWN
            pc, _pawn, _hc = _pawn_and_health()
            if pc is None:
                return
            # tear the old one down first -- if _alive() was a transient false
            # negative the widget is still live+rooted, and overwriting _hud
            # without this would orphan a rooted duplicate on screen.
            _destroy(globals().get('_hud'))
            try:
                globals()['_hud'] = _build_hud(pc)
            except Exception:
                pass
            return
        try:
            _update(st)
        except Exception:
            pass          # transient staleness mid-frame; _alive() will rebuild next tick

    # ── idempotent teardown of a previous run ────────────────────────────────
    _prev = globals().get('_hud')
    # remove the current HUD *and* any orphaned duplicates from earlier runs
    _purge_all()
    # remove any tick hook from a prior run: this file's ('_hud_tick') and the
    # earlier version that stored its hook under _hud['tick'].
    for _h in (globals().pop('_hud_tick', None), (_prev or {}).get('tick')):
        if _h is not None:
            try:
                _h.remove()
            except Exception:
                pass
    # clear any throwaway probe widgets left around from experimentation
    for _k in ('_hudtest', '_probe_tb'):
        _p = globals().pop(_k, None)
        if _p is not None:
            try:
                (_p['w'] if isinstance(_p, dict) else _p[0]).RemoveFromParent()
            except Exception:
                pass

    # ── arm ──────────────────────────────────────────────────────────────────
    _pc0, _pawn0, _hc0 = _pawn_and_health()
    globals()['_hud'] = _build_hud(_pc0) if _pc0 is not None else None
    globals()['_hud_cooldown'] = 0.0
    globals()['_hud_tick'] = ue.on_tick(_on_tick)
    print('HUD armed -- self-healing across level travel. Re-run to rebuild.')


_install()
del _install
