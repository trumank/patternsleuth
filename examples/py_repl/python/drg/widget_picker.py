"""Widget picker — a live UMG inspector, like a browser's element picker.

Hover the mouse over anything on screen and it highlights the UMG widget under
the cursor and prints, in a corner panel, everything worth knowing about it:

  * class + object name
  * its position in the widget tree (root ▸ … ▸ self, with child indices)
  * bounding box in absolute (Slate) pixels and its local / desired size
  * visibility + enabled state, slot type, and any text it carries

Nothing is precomputed off a blueprint — the game is running, so we probe it:
each frame we walk every live UWidget (`ue.objects_of_class`), ask it for its
cached Slate geometry, convert that to an absolute rect, and pick the smallest
rect the cursor is inside (the topmost / most specific widget). Widgets that were
never painted report empty geometry and fall out for free, so the scan only ever
sees what's actually on screen.

The highlight (translucent fill + outline + a floating class tag) and the info
panel live on our own UserWidget, added to the viewport at a very high ZOrder and
marked HitTestInvisible so it draws over the game's UI without ever stealing a
click or showing up as a hit in its own scan.

Controls (while the picker is armed):
    hover        follow the cursor, highlighting the widget beneath it
    \\  (Backslash)  toggle the whole picker on/off (also frees the cursor + input)
    L            lock / unlock the current pick (freeze it so you can read it)
    [   / ]      walk the selection up toward the root / down toward the leaf
    wheel up/dn  same as [ / ] (locks on first use)
    P            print the full reflected property dump of the pick to the REPL

REPL entry points (persist as globals):
    widget_picker_open(pc=None)   arm it (shows cursor, GameAndUI input)
    widget_picker_close()         tear it down, restore game-only input
    widget_picker_dump()          print the current pick's full property dump

State lives under one module global `_WIDGET_PICKER`; re-run the file to rebuild.
Idempotent — opening tears down any previous instance and purges orphan trees.
"""


def _install():
    G = '_WIDGET_PICKER'
    TREE_TAG = 'PyWidgetPickerTree'

    TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

    U = lambda p: ue.find(p)
    WBL         = U('/Script/UMG.Default__WidgetBlueprintLibrary')
    KTL         = U('/Script/Engine.Default__KismetTextLibrary')
    SLATE       = U('/Script/UMG.Default__SlateBlueprintLibrary')
    WLL         = U('/Script/UMG.Default__WidgetLayoutLibrary')
    UserWidget  = U('/Script/UMG.UserWidget')
    WidgetTree  = U('/Script/UMG.WidgetTree')
    CanvasPanel = U('/Script/UMG.CanvasPanel')
    Image       = U('/Script/UMG.Image')
    TextBlock   = U('/Script/UMG.TextBlock')

    WIDGET_CLS = '/Script/UMG.Widget'

    # ── scan cadence ──────────────────────────────────────────────────────────
    # We never enumerate all ~80k UWidgets to hit-test. Root discovery (an
    # IsInViewport sweep of every UserWidget, ~20 ms) is cached and refreshed
    # slowly; the per-cursor hit-test walks DOWN from those roots and visits only
    # the widgets under the pointer (~80, sub-millisecond), so it can run at rate.
    ROOT_PERIOD = 0.5      # s — rediscover the on-screen root widgets
    SCAN_PERIOD = 0.03     # s — cursor hit-test cadence (~30 Hz)

    # ── layout ───────────────────────────────────────────────────────────────
    T        = 2.0            # highlight outline thickness (px)
    NLINES   = 13             # info-panel text lines
    LINE_H   = 16.0
    PANEL_X  = 22.0
    PANEL_Y  = 22.0
    PANEL_W  = 520.0
    PANEL_PAD = 10.0
    ZORDER   = 2_000_000      # draw over essentially everything

    # ── colors (linear RGBA) ─────────────────────────────────────────────────
    C_FILL  = {'R': 0.15, 'G': 0.55, 'B': 1.0, 'A': 0.18}
    C_EDGE  = {'R': 0.35, 'G': 0.75, 'B': 1.0, 'A': 0.95}
    C_TAGBG = {'R': 0.05, 'G': 0.10, 'B': 0.18, 'A': 0.92}
    C_PANEL = {'R': 0.02, 'G': 0.03, 'B': 0.05, 'A': 0.88}
    C_TITLE = {'R': 0.45, 'G': 0.82, 'B': 1.00, 'A': 1.0}
    C_KEY   = {'R': 0.62, 'G': 0.70, 'B': 0.82, 'A': 1.0}
    C_TXT   = {'R': 0.92, 'G': 0.95, 'B': 1.00, 'A': 1.0}
    C_LOCK  = {'R': 1.00, 'G': 0.72, 'B': 0.20, 'A': 1.0}

    # ── tiny helpers ─────────────────────────────────────────────────────────
    def live(cls):
        return next((o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE), None)

    def _pc():
        gi = live('/Script/Engine.GameInstance')
        return gi.LocalPlayers[0].PlayerController if gi else None

    def _key(name):
        return {'KeyName': name}

    def _text(s):
        return KTL.Conv_StringToText(str(s))

    def _str(ft):
        try:
            return KTL.Conv_TextToString(ft)
        except Exception:
            return ''

    def _slate(rgba):
        return {'SpecifiedColor': rgba, 'ColorUseRule': 0}

    def _font(size):
        f = U('/Script/UMG.Default__TextBlock').Font
        f.Size = int(size)
        return f

    def _set_input(pc, on):
        try:
            pc.bShowMouseCursor = on
        except Exception:
            pass
        try:
            if on:
                try:
                    WBL.SetInputMode_GameAndUIEx(pc, None, 0, False)
                except Exception:
                    WBL.SetInputMode_GameAndUI(pc)
            else:
                WBL.SetInputMode_GameOnly(pc)
        except Exception:
            pass

    # ── geometry probing (pure Slate-absolute space, no world context) ────────
    def _geom(wdg):
        """The widget's last painted Slate geometry, or None if it wasn't."""
        for getter in ('GetCachedGeometry', 'GetTickSpaceGeometry', 'GetPaintSpaceGeometry'):
            try:
                return getattr(wdg, getter)()
            except Exception:
                continue
        return None

    def _abs_rect(geo):
        """(x, y, w, h) of `geo` in absolute Slate px, or None if degenerate.

        GetLocalSize is the pre-transform size; LocalToAbsolute maps the two
        local corners through the accumulated render transform (DPI + scales),
        giving a true on-screen rect."""
        try:
            sz = SLATE.GetLocalSize(geo)
            lw, lh = sz.X, sz.Y
            if lw <= 0.0 or lh <= 0.0:
                return None
            tl = SLATE.LocalToAbsolute(geo, {'X': 0.0, 'Y': 0.0})
            br = SLATE.LocalToAbsolute(geo, {'X': lw, 'Y': lh})
            x0, y0, x1, y1 = tl.X, tl.Y, br.X, br.Y
        except Exception:
            return None
        return (min(x0, x1), min(y0, y1), abs(x1 - x0), abs(y1 - y0))

    def _mouse_abs():
        try:
            m = WLL.GetMousePositionOnPlatform()
            return (m.X, m.Y)
        except Exception:
            return None

    def _abs_to_local(og, x, y):
        p = SLATE.AbsoluteToLocal(og, {'X': x, 'Y': y})
        return (p.X, p.Y)

    def _local_area(geo):
        try:
            sz = SLATE.GetLocalSize(geo)
            if sz.X <= 0.0 or sz.Y <= 0.0:
                return None
            return sz.X * sz.Y
        except Exception:
            return None

    def _under(geo, mx, my):
        """Is (mx, my) [absolute Slate px] inside this geometry? Prefers the
        engine's own hit test (handles render transforms); falls back to a plain
        rect containment if that UFunction isn't present on this build."""
        try:
            return bool(SLATE.IsUnderLocation(geo, {'X': mx, 'Y': my}))
        except Exception:
            r = _abs_rect(geo)
            return r is not None and r[0] <= mx <= r[0] + r[2] and r[1] <= my <= r[1] + r[3]

    def _children(node):
        """A widget's child widgets: a UserWidget yields its WidgetTree root; any
        panel / content widget yields its slotted children. (UContentWidget —
        Border, Button, SizeBox … — extends UPanelWidget, so GetChildAt is
        uniform.)"""
        try:
            wt = node.WidgetTree
        except Exception:
            wt = None
        if wt is not None:
            try:
                r = wt.RootWidget
                return [r] if r is not None else []
            except Exception:
                return []
        try:
            return [node.GetChildAt(i) for i in range(node.GetChildrenCount())]
        except Exception:
            return []

    # ── on-screen roots (cached; ~20 ms to discover, so refreshed slowly) ─────
    def _hud_root():
        pc = _pc()
        if pc is None:
            return None
        try:
            return pc.MyHUD.HUD.WidgetTree.RootWidget
        except Exception:
            return None

    def _refresh_roots(st):
        """The widgets Slate actually paints from: the HUD tree root plus every
        UserWidget currently added to the viewport (minus our own overlay). There
        are only a couple dozen, but finding them means an `IsInViewport` sweep of
        every UserWidget, so we do it on a slow cadence and cache the result."""
        own = st['ui']['own']
        roots = []
        hr = _hud_root()
        if hr is not None:
            roots.append(hr)
        for u in ue.objects_of_class('/Script/UMG.UserWidget'):
            if u in own:
                continue
            try:
                if u.IsInViewport():
                    roots.append(u)
            except Exception:
                pass
        st['roots'] = roots

    # ── the hit-test walk: smallest widget under the cursor (sub-millisecond) ──
    def _pick_at(st, mx, my):
        """Walk DOWN from the on-screen roots, descending only into children whose
        geometry contains the cursor (Slate's own hit-test discipline). This visits
        the handful of widgets under the pointer — not the ~80k that exist — so it
        costs well under a frame. Returns the smallest-area widget hit."""
        own = st['ui']['own']
        best = [None, None]     # [area, widget]
        seen = set()
        stack = list(st.get('roots') or [])
        while stack:
            node = stack.pop()
            if node is None or node in seen or node in own:
                continue
            seen.add(node)
            geo = _geom(node)
            if geo is None or not _under(geo, mx, my):
                continue           # prune: cursor not inside → subtree can't contain it
            area = _local_area(geo)
            if area is not None and (best[0] is None or area < best[0]):
                best[0], best[1] = area, node
            stack.extend(_children(node))
        return best[1]

    # ── widget-tree introspection ─────────────────────────────────────────────
    def _ancestors(wdg):
        """[root, …, wdg] within its WidgetTree, via GetParent()."""
        out = []
        node = wdg
        for _ in range(64):
            if node is None:
                break
            out.append(node)
            try:
                node = node.GetParent()
            except Exception:
                node = None
        out.reverse()
        return out

    def _child_index(parent, child):
        try:
            return parent.GetChildIndex(child)
        except Exception:
            return -1

    def _short_cls(wdg):
        try:
            return wdg._class._name
        except Exception:
            return '?'

    def _short_name(wdg):
        try:
            return wdg._name
        except Exception:
            return '?'

    def _tree_path_str(chain, sel_idx):
        parts = []
        for i, node in enumerate(chain):
            nm = _short_name(node)
            mark = '»%s«' % nm if i == sel_idx else nm
            if i > 0:
                idx = _child_index(chain[i - 1], node)
                if idx >= 0:
                    mark = '%s[%d]' % (mark, idx)
            parts.append(mark)
        s = ' ▸ '.join(parts)
        return s if len(s) <= 78 else '… ▸ ' + ' ▸ '.join(parts[-4:])

    def _widget_text(wdg):
        for getter in ('GetText', 'GetToolTipText'):
            try:
                t = _str(getattr(wdg, getter)())
                if t:
                    return t
            except Exception:
                continue
        return ''

    def _slot_cls(wdg):
        try:
            slot = wdg.Slot
            return slot._class._name if slot is not None else '(none)'
        except Exception:
            return '(none)'

    def _flag(wdg, getter):
        try:
            return bool(getattr(wdg, getter)())
        except Exception:
            return None

    # ── full property dump (P key / widget_picker_dump) ───────────────────────
    def _dump(wdg):
        if wdg is None:
            print('[widget_picker] nothing selected')
            return
        print('── %s  (%s)' % (_short_name(wdg), _short_cls(wdg)))
        print('   path: %s' % getattr(wdg, '_path', '?'))
        geo = _geom(wdg)
        if geo is not None:
            r = _abs_rect(geo)
            if r:
                print('   bbox: x=%.0f y=%.0f  %.0f×%.0f px' % r)
        try:
            props = wdg._class.props()
        except Exception:
            props = []
        for name, _pt in props:
            try:
                v = repr(getattr(wdg, name))
            except Exception as e:
                v = '<%s>' % type(e).__name__
            if len(v) > 100:
                v = v[:100] + '…'
            print('   %-32s %-22s = %s' % (name, kind, v))

    # ── overlay construction ──────────────────────────────────────────────────
    def _build(pc):
        w = WBL.Create(pc, UserWidget, pc)
        tree = ue.new_object(WidgetTree, outer=w, name=TREE_TAG)
        root = ue.new_object(CanvasPanel, outer=tree, name='PyWidgetPickerRoot')
        tree.RootWidget = root
        w.WidgetTree = tree

        def mk_img(color, z):
            im = ue.new_object(Image, outer=tree)
            im.SetColorAndOpacity(color)
            slot = root.AddChildToCanvas(im)
            slot.SetAutoSize(False)
            slot.SetZOrder(z)
            return im, slot

        def mk_label(x, y, size, color, z, autosize=True):
            tb = ue.new_object(TextBlock, outer=tree)
            tb.SetFont(_font(size))
            tb.SetColorAndOpacity(_slate(color))
            slot = root.AddChildToCanvas(tb)
            slot.SetAutoSize(autosize)
            slot.SetPosition({'X': x, 'Y': y})
            slot.SetZOrder(z)
            return tb, slot

        # highlight: translucent fill + four edge bars
        fill, fill_slot = mk_img(C_FILL, 1)
        edges = [mk_img(C_EDGE, 2) for _ in range(4)]     # top, bottom, left, right

        # floating class tag near the widget's top-left
        tag_bg, tag_bg_slot = mk_img(C_TAGBG, 3)
        tag_tb, tag_tb_slot = mk_label(0.0, 0.0, 12, C_TXT, 4)

        # info panel: backdrop + fixed stack of text lines
        panel_bg, panel_bg_slot = mk_img(C_PANEL, 5)
        panel_bg_slot.SetPosition({'X': PANEL_X - PANEL_PAD, 'Y': PANEL_Y - PANEL_PAD})
        panel_bg_slot.SetSize({'X': PANEL_W, 'Y': NLINES * LINE_H + 2 * PANEL_PAD})
        lines = []
        for i in range(NLINES):
            tb, _ = mk_label(PANEL_X, PANEL_Y + i * LINE_H, 11, C_TXT, 6)
            lines.append(tb)

        # never eat input / never appear in our own scan
        for setvis in (lambda: w.SetVisibility(3),        # HitTestInvisible
                       lambda: root.SetVisibility(3)):
            try:
                setvis()
            except Exception:
                pass

        w.AddToViewport(ZORDER)
        w.add_to_root()
        tree.add_to_root()

        # Every widget we own, by identity — the scan skips these so our own
        # highlight/panel can never be picked (a path-substring test proved
        # unreliable; UObject identity is by address and exact).
        own = set()
        for o in [w, root, fill, tag_bg, tag_tb, panel_bg] + [e[0] for e in edges] + lines:
            own.add(o)

        return {
            'w': w, 'tree': tree, 'root': root, 'own': own,
            'fill': fill_slot, 'edges': [e[1] for e in edges],
            'tag_bg': tag_bg_slot, 'tag_tb': tag_tb, 'tag_tb_slot': tag_tb_slot,
            'lines': lines,
        }

    # ── highlight + panel rendering ───────────────────────────────────────────
    def _hide_highlight(st):
        ui = st['ui']
        for slot in [ui['fill']] + ui['edges'] + [ui['tag_bg']]:
            try:
                slot.SetSize({'X': 0.0, 'Y': 0.0})
            except Exception:
                pass
        try:
            ui['tag_tb'].SetText(_text(''))
        except Exception:
            pass

    def _draw_highlight(st, rect):
        ui = st['ui']
        og = _geom(st['ui']['root'])
        if og is None:
            return
        x, y, w, h = rect
        try:
            ox, oy = _abs_to_local(og, x, y)
            bx, by = _abs_to_local(og, x + w, y + h)
        except Exception:
            return
        ow, oh = bx - ox, by - oy

        def place(slot, px, py, pw, ph):
            slot.SetPosition({'X': px, 'Y': py})
            slot.SetSize({'X': max(pw, 0.0), 'Y': max(ph, 0.0)})

        place(ui['fill'], ox, oy, ow, oh)
        place(ui['edges'][0], ox, oy, ow, T)            # top
        place(ui['edges'][1], ox, oy + oh - T, ow, T)   # bottom
        place(ui['edges'][2], ox, oy, T, oh)            # left
        place(ui['edges'][3], ox + ow - T, oy, T, oh)   # right

    def _draw_tag(st, wdg, rect):
        ui = st['ui']
        og = _geom(ui['root'])
        if og is None:
            return
        x, y, _w, _h = rect
        try:
            ox, oy = _abs_to_local(og, x, y)
        except Exception:
            return
        label = ' %s ' % _short_cls(wdg)
        ui['tag_tb'].SetText(_text(label))
        tx, ty = ox + 1.0, max(oy - 18.0, 0.0)
        ui['tag_tb_slot'].SetPosition({'X': tx + 4.0, 'Y': ty + 2.0})
        ui['tag_bg'].SetPosition({'X': tx, 'Y': ty})
        ui['tag_bg'].SetSize({'X': 9.0 * len(label) + 8.0, 'Y': 17.0})

    def _render(st):
        chain = st['chain']
        if not chain:
            _hide_highlight(st)
            for i in range(NLINES):
                st['ui']['lines'][i].SetText(_text(''))
            st['ui']['lines'][0].SetText(_text('WIDGET PICKER — hover a widget'))
            st['ui']['lines'][0].SetColorAndOpacity(_slate(C_TITLE))
            return
        idx = st['idx']
        wdg = chain[idx]
        geo = _geom(wdg)
        rect = _abs_rect(geo) if geo is not None else None
        if rect is not None:
            _draw_highlight(st, rect)
            _draw_tag(st, wdg, rect)
        else:
            _hide_highlight(st)

        L = st['ui']['lines']
        status = '  [LOCKED]' if st['locked'] else ''
        L[0].SetText(_text('WIDGET PICKER%s' % status))
        L[0].SetColorAndOpacity(_slate(C_LOCK if st['locked'] else C_TITLE))
        L[1].SetText(_text('class   %s' % _short_cls(wdg)))
        L[1].SetColorAndOpacity(_slate(C_TXT))
        L[2].SetText(_text('name    %s' % _short_name(wdg)))
        L[2].SetColorAndOpacity(_slate(C_TXT))
        L[3].SetText(_text('tree    %s' % _tree_path_str(chain, idx)))
        L[3].SetColorAndOpacity(_slate(C_KEY))
        if rect:
            L[4].SetText(_text('bbox    x=%.0f y=%.0f   %.0f × %.0f px'
                               % (rect[0], rect[1], rect[2], rect[3])))
        else:
            L[4].SetText(_text('bbox    (not currently painted)'))
        L[4].SetColorAndOpacity(_slate(C_TXT))
        try:
            ds = wdg.GetDesiredSize()
            L[5].SetText(_text('desired %.0f × %.0f px' % (ds.X, ds.Y)))
        except Exception:
            L[5].SetText(_text('desired —'))
        L[5].SetColorAndOpacity(_slate(C_TXT))
        vis = _flag(wdg, 'IsVisible')
        ena = _flag(wdg, 'GetIsEnabled')
        if ena is None:
            ena = _flag(wdg, 'IsEnabled')
        L[6].SetText(_text('state   visible=%s  enabled=%s  slot=%s'
                           % (vis, ena, _slot_cls(wdg))))
        L[6].SetColorAndOpacity(_slate(C_TXT))
        depth = '%d of %d  (child idx %d)' % (
            idx, len(chain) - 1,
            _child_index(chain[idx - 1], wdg) if idx > 0 else -1)
        L[7].SetText(_text('depth   %s' % depth))
        L[7].SetColorAndOpacity(_slate(C_TXT))
        txt = _widget_text(wdg)
        L[8].SetText(_text(('text    "%s"' % txt) if txt else 'text    —'))
        L[8].SetColorAndOpacity(_slate(C_TXT))
        L[9].SetText(_text(''))
        L[10].SetText(_text('[ \\ ] toggle   [ L ] lock   [ [ ] up   [ ] ] down'))
        L[10].SetColorAndOpacity(_slate(C_KEY))
        L[11].SetText(_text('[ P ] dump props to REPL   ·   wheel = up/down'))
        L[11].SetColorAndOpacity(_slate(C_KEY))
        L[12].SetText(_text(''))

    # ── liveness / teardown ───────────────────────────────────────────────────
    def _alive(st):
        try:
            return st['ui']['w'].GetParent() is not None or st['ui']['w'].IsInViewport()
        except Exception:
            try:
                return st['ui']['w']._name is not None
            except Exception:
                return False

    def _destroy_ui(ui):
        if not ui:
            return
        for step in (lambda: ui['w'].RemoveFromParent(),
                     lambda: ui['w'].remove_from_root(),
                     lambda: ui['tree'].remove_from_root()):
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

    # ── selection update ──────────────────────────────────────────────────────
    def _reselect(st, mx, my):
        wdg = _pick_at(st, mx, my)
        if wdg is None:
            st['chain'] = []
            st['idx'] = 0
            return
        # Same leaf as last frame? keep the user's up/down position in the chain.
        if st['chain'] and st['idx'] == len(st['chain']) - 1 and st['chain'][-1] == wdg:
            return
        st['chain'] = _ancestors(wdg)
        st['idx'] = len(st['chain']) - 1

    def _move(st, delta):
        if not st['chain']:
            return
        st['locked'] = True
        st['idx'] = max(0, min(len(st['chain']) - 1, st['idx'] + delta))

    # ── per-frame driver ──────────────────────────────────────────────────────
    def _on_tick(world, dt):
        st = globals().get(G)
        if st is None:
            return

        pc = st.get('pc')
        try:
            _ = pc._name if pc is not None else None
        except Exception:
            pc = None
        if pc is None:
            pc = _pc()
            st['pc'] = pc
        if pc is None:
            return

        # self-heal across travel
        if not _alive(st):
            _destroy_ui(st.get('ui'))
            _purge_all()
            try:
                st['ui'] = _build(pc)
                st['chain'] = []
                st['idx'] = 0
                st['roots'] = None
                st['last_mouse'] = None
                _set_input(pc, st['on'])
            except Exception as e:
                print('[widget_picker] rebuild failed:', type(e).__name__, e)
                st['ui'] = None
            return

        # ── hotkeys ──
        def pressed(name):
            try:
                return pc.WasInputKeyJustPressed(_key(name))
            except Exception:
                return False

        if pressed('Backslash'):
            st['on'] = not st['on']
            _set_input(pc, st['on'])
            try:
                st['ui']['root'].SetVisibility(3 if st['on'] else 1)  # 1 = Collapsed
            except Exception:
                pass
            if not st['on']:
                return
        if not st['on']:
            return

        if pressed('L'):
            st['locked'] = not st['locked']
        if pressed('LeftBracket') or pressed('MouseScrollUp'):
            _move(st, -1)
        if pressed('RightBracket') or pressed('MouseScrollDown'):
            _move(st, +1)
        if pressed('P'):
            _dump(st['chain'][st['idx']] if st['chain'] else None)

        # ── root discovery (slow; cached) ──
        st['root_t'] -= dt
        if st['root_t'] <= 0.0 or st['roots'] is None:
            st['root_t'] = ROOT_PERIOD
            _refresh_roots(st)

        # ── selection (cheap top-down hit-test walk from the roots) ──
        if not st['locked']:
            st['scan_t'] -= dt
            if st['scan_t'] <= 0.0:
                st['scan_t'] = SCAN_PERIOD
                m = _mouse_abs()
                if m is not None:
                    lm = st.get('last_mouse')
                    if lm is None or abs(m[0] - lm[0]) > 0.5 or abs(m[1] - lm[1]) > 0.5:
                        st['last_mouse'] = m
                        _reselect(st, m[0], m[1])

        try:
            _render(st)
        except Exception as e:
            print('[widget_picker] render error:', type(e).__name__, e)

    # ── public API ────────────────────────────────────────────────────────────
    def widget_picker_close():
        st = globals().pop(G, None)
        if st:
            try:
                st['tick'].remove()
            except Exception:
                pass
            pc = st.get('pc')
            if pc is not None:
                _set_input(pc, False)
            _destroy_ui(st.get('ui'))
        _purge_all()
        print('[widget_picker] closed.')

    def widget_picker_open(pc=None):
        widget_picker_close()
        pc = pc or _pc()
        if pc is None:
            print('widget_picker: no PlayerController — is the game in a level?')
            return None
        ui = _build(pc)
        _set_input(pc, True)
        st = {
            'ui': ui, 'pc': pc, 'on': True, 'locked': False,
            'chain': [], 'idx': 0, 'last_mouse': None,
            'roots': None, 'root_t': 0.0, 'scan_t': 0.0,
            'tick': ue.on_tick(_on_tick),
        }
        globals()[G] = st
        print('widget_picker: armed. Hover to inspect · \\ toggle · L lock · '
              '[ ] navigate · P dump.')
        return st

    def widget_picker_dump():
        st = globals().get(G)
        _dump(st['chain'][st['idx']] if (st and st['chain']) else None)

    globals()['widget_picker_open'] = widget_picker_open
    globals()['widget_picker_close'] = widget_picker_close
    globals()['widget_picker_dump'] = widget_picker_dump

    widget_picker_open()


_install()
del _install
