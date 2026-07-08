"""A wall-to-wall tour of the stock UMG/Slate widget set, built live from Python.

We construct a bare `UUserWidget`, hand-build its `WidgetTree`, and stuff a
scrollable panel with one live instance of (almost) every reusable stock widget
class the engine ships -- text, buttons, toggles, ranges, spinners, throbbers,
combo boxes, every layout panel, and the content wrappers -- then add it to the
viewport. A single tick animates the ranges/throbbers/switcher so you can see
the widgets are real, running SWidgets and not just static images.

Nothing here needs a Blueprint or a custom C++ widget: every widget is minted
with `ue.new_object` under the widget tree and wired up through its stock
BlueprintCallable setters (discovered by reflecting `cls.functions()`).

Coverage is reported back over the REPL: each widget is added inside a guarded
`demo(...)` so the printed summary lists exactly what rendered and what (if
anything) errored -- handy since the person running this is looking at the game
window, not this console.

Widgets deliberately skipped (they need a `UUserWidget` *entry class* + the
`IUserObjectListEntry` interface, or a design-time-only setup, that can't be
minted from Python): ListView, TileView, TreeView, DynamicEntryBox, ComboBoxKey,
plus the abstract bases (PanelWidget, ContentWidget, TextLayoutWidget) and the
editor/plumbing widgets (NativeWidgetHost, Viewport, WindowTitleBarArea,
PostBufferUpdate).

Run once to arm; run again to tear down and rebuild (idempotent).
"""


def _install():
    import math

    TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

    U   = lambda p: ue.find(p)
    WBL = U('/Script/UMG.Default__WidgetBlueprintLibrary')
    KTL = U('/Script/Engine.Default__KismetTextLibrary')

    UserWidget  = U('/Script/UMG.UserWidget')
    WidgetTree  = U('/Script/UMG.WidgetTree')
    CanvasPanel = U('/Script/UMG.CanvasPanel')

    # Alignment / size-rule enum values (looked up once, hard-coded here).
    HFILL, HLEFT, HCENTER = 0, 1, 2
    VFILL, VTOP, VCENTER = 0, 1, 2
    SIZE_AUTO, SIZE_FILL = 0, 1

    # palette
    BG      = {'R': 0.03, 'G': 0.04, 'B': 0.06, 'A': 0.86}
    HEADER  = {'R': 0.45, 'G': 0.80, 'B': 1.00, 'A': 1.00}
    TXT     = {'R': 0.92, 'G': 0.95, 'B': 1.00, 'A': 1.00}
    DIM     = {'R': 0.60, 'G': 0.66, 'B': 0.78, 'A': 1.00}
    ACCENT  = {'R': 0.20, 'G': 0.85, 'B': 0.95, 'A': 1.00}
    WARM    = {'R': 1.00, 'G': 0.55, 'B': 0.20, 'A': 1.00}
    GOOD    = {'R': 0.30, 'G': 0.90, 'B': 0.45, 'A': 1.00}
    CHIPS   = [{'R': 0.85, 'G': 0.25, 'B': 0.35, 'A': 0.9},
               {'R': 0.25, 'G': 0.65, 'B': 0.85, 'A': 0.9},
               {'R': 0.85, 'G': 0.70, 'B': 0.20, 'A': 0.9},
               {'R': 0.45, 'G': 0.80, 'B': 0.35, 'A': 0.9},
               {'R': 0.65, 'G': 0.40, 'B': 0.85, 'A': 0.9}]

    report = []           # (label, 'ok' | 'ERR: ...')
    anim = {}             # live widget handles the tick pokes each frame

    def txt(s):
        return KTL.Conv_StringToText(str(s))

    def slate(rgba):
        return {'SpecifiedColor': rgba, 'ColorUseRule': 0}

    def margin(l, t, r, b):
        return {'Left': float(l), 'Top': float(t), 'Right': float(r), 'Bottom': float(b)}

    def mk(path, name=None):
        return ue.new_object(U(path), outer=tree, name=name) if name else ue.new_object(U(path), outer=tree)

    def font(size):
        f = U('/Script/UMG.Default__TextBlock').Font
        f.Size = float(size)
        return f

    def label(s, size=13, color=None, wrap=False):
        tb = mk('/Script/UMG.TextBlock')
        tb.SetFont(font(size))
        tb.SetColorAndOpacity(slate(color or TXT))
        tb.SetText(txt(s))
        if wrap:
            tb.SetAutoWrapText(True)
        return tb

    def demo(name, fn):
        """Run a widget-building thunk; record ok/err; return the widget or None."""
        try:
            w = fn()
            report.append((name, 'ok'))
            return w
        except Exception as e:
            report.append((name, 'ERR: %s' % e))
            return None

    # ── root scaffold ─────────────────────────────────────────────────────────
    w = WBL.Create(_pc, UserWidget, _pc)
    tree = ue.new_object(WidgetTree, outer=w, name='ShowcaseTree')
    canvas = ue.new_object(CanvasPanel, outer=tree, name='ShowcaseCanvas')
    tree.RootWidget = canvas
    w.WidgetTree = tree

    PANEL_X, PANEL_Y, PANEL_W, PANEL_H = 24.0, 24.0, 620.0, 1000.0

    def canvas_add(child, x, y, cw, ch, z=0):
        slot = canvas.AddChildToCanvas(child)
        slot.SetAutoSize(False)
        slot.SetPosition({'X': x, 'Y': y})
        slot.SetSize({'X': cw, 'Y': ch})
        slot.SetZOrder(z)
        return slot

    # translucent backdrop
    bg = mk('/Script/UMG.Image')
    bg.SetColorAndOpacity(BG)
    canvas_add(bg, PANEL_X, PANEL_Y, PANEL_W, PANEL_H, z=0)

    # the scroll container everything lives in
    scroll = mk('/Script/UMG.ScrollBox')
    canvas_add(scroll, PANEL_X, PANEL_Y, PANEL_W, PANEL_H, z=1)

    root = mk('/Script/UMG.VerticalBox')          # main stack inside the scrollbox
    rs = scroll.AddChild(root)
    try:
        rs.SetHorizontalAlignment(HFILL)
        rs.SetPadding(margin(14, 12, 14, 24))
    except Exception:
        pass

    def stack(widget, pad=(0, 4, 0, 4), halign=HFILL, valign=VTOP, fill=False):
        """Append a widget to the main vertical stack."""
        slot = root.AddChildToVerticalBox(widget)
        slot.SetHorizontalAlignment(halign)
        slot.SetVerticalAlignment(valign)
        slot.SetPadding(margin(*pad))
        slot.SetSize({'Value': 1.0, 'SizeRule': SIZE_FILL if fill else SIZE_AUTO})
        return slot

    def section(title):
        stack(label(title, 15, HEADER), pad=(0, 16, 0, 2))
        rule = mk('/Script/UMG.Image')
        rule.SetColorAndOpacity({'R': 0.30, 'G': 0.45, 'B': 0.60, 'A': 0.7})
        sb = mk('/Script/UMG.SizeBox'); sb.SetHeightOverride(2.0); sb.SetContent(rule)
        stack(sb, pad=(0, 0, 0, 6))

    def row(*widgets, pad=(0, 3, 0, 3), gap=8, valign=VCENTER):
        """Lay widgets left-to-right in a HorizontalBox and stack the box."""
        hb = mk('/Script/UMG.HorizontalBox')
        for i, wd in enumerate(widgets):
            s = hb.AddChildToHorizontalBox(wd)
            s.SetVerticalAlignment(valign)
            s.SetPadding(margin(0 if i == 0 else gap, 0, 0, 0))
        stack(hb, pad=pad)
        return hb

    def sized(widget, cw=None, ch=None):
        sb = mk('/Script/UMG.SizeBox')
        if cw is not None:
            sb.SetWidthOverride(float(cw))
        if ch is not None:
            sb.SetHeightOverride(float(ch))
        sb.SetContent(widget)
        return sb

    # ── title ─────────────────────────────────────────────────────────────────
    stack(label('UMG / SLATE WIDGET SHOWCASE', 20, HEADER))
    head = label('constructed live from Python', 11, DIM)
    stack(head, pad=(0, 0, 0, 2))
    anim['frame'] = label('frame 0', 11, WARM)
    stack(anim['frame'], pad=(0, 0, 0, 4))

    # ── TEXT ──────────────────────────────────────────────────────────────────
    section('Text')

    demo('TextBlock', lambda: stack(label('TextBlock — the workhorse label.', 13)))

    def rich():
        r = mk('/Script/UMG.RichTextBlock')
        r.SetText(txt('RichTextBlock — supports decorators & markup.'))
        r.SetDefaultColorAndOpacity(slate(TXT))
        r.SetAutoWrapText(True)
        return stack(r)
    demo('RichTextBlock', rich)

    def etb():
        e = mk('/Script/UMG.EditableTextBox')
        e.SetHintText(txt('EditableTextBox — click and type…'))
        e.SetText(txt('editable single line'))
        return stack(sized(e, ch=34))
    demo('EditableTextBox', etb)

    def et():
        e = mk('/Script/UMG.EditableText')
        e.SetHintText(txt('EditableText (borderless)'))
        e.SetText(txt('EditableText'))
        return stack(e)
    demo('EditableText', et)

    def pw():
        e = mk('/Script/UMG.EditableTextBox')
        e.SetIsPassword(True)
        e.SetText(txt('secret123'))
        return stack(sized(e, ch=34))
    demo('EditableTextBox(password)', pw)

    def mletb():
        e = mk('/Script/UMG.MultiLineEditableTextBox')
        e.SetText(txt('MultiLineEditableTextBox\nsecond line\nthird line'))
        return stack(sized(e, ch=74))
    demo('MultiLineEditableTextBox', mletb)

    def mlet():
        e = mk('/Script/UMG.MultiLineEditableText')
        e.SetText(txt('MultiLineEditableText — no border box.'))
        return stack(e)
    demo('MultiLineEditableText', mlet)

    # ── BUTTONS & TOGGLES ───────────────────────────────────────────────────────
    section('Buttons & Toggles')

    def button():
        b = mk('/Script/UMG.Button')
        b.SetBackgroundColor({'R': 0.15, 'G': 0.35, 'B': 0.55, 'A': 1.0})
        b.SetContent(label('  Button — SetContent(TextBlock)  ', 13, TXT))
        return stack(sized(b, ch=40), halign=HLEFT)
    demo('Button', button)

    def checks():
        c1 = mk('/Script/UMG.CheckBox'); c1.SetIsChecked(True)
        c2 = mk('/Script/UMG.CheckBox'); c2.SetIsChecked(False)
        c3 = mk('/Script/UMG.CheckBox'); c3.SetCheckedState(2)   # Undetermined
        row(c1, label('checked', 12, DIM), c2, label('unchecked', 12, DIM),
            c3, label('undetermined', 12, DIM))
        return c1
    demo('CheckBox', checks)

    def keysel():
        k = mk('/Script/UMG.InputKeySelector')
        try:
            k.SetSelectedKey({'KeyName': 'SpaceBar'})
        except Exception:
            pass
        k.SetKeySelectionText(txt('press a key'))
        row(label('InputKeySelector:', 12, DIM), sized(k, cw=160, ch=32))
        return k
    demo('InputKeySelector', keysel)

    # ── RANGES ──────────────────────────────────────────────────────────────────
    section('Ranges & Spinners')

    def slider():
        s = mk('/Script/UMG.Slider')
        s.SetMinValue(0.0); s.SetMaxValue(1.0); s.SetValue(0.35)
        s.SetSliderHandleColor(ACCENT); s.SetSliderBarColor(DIM)
        anim['slider'] = s
        row(label('Slider', 12, DIM), sized(s, cw=380, ch=24))
        return s
    demo('Slider', slider)

    def spin():
        s = mk('/Script/UMG.SpinBox')
        s.SetMinValue(0.0); s.SetMaxValue(100.0); s.SetValue(42.0)
        row(label('SpinBox', 12, DIM), sized(s, cw=180, ch=32))
        return s
    demo('SpinBox', spin)

    def progress():
        p = mk('/Script/UMG.ProgressBar')
        p.SetPercent(0.66); p.SetFillColorAndOpacity(GOOD)
        anim['progress'] = p
        row(label('ProgressBar', 12, DIM), sized(p, cw=380, ch=20))
        return p
    demo('ProgressBar', progress)

    def marquee():
        p = mk('/Script/UMG.ProgressBar')
        p.SetIsMarquee(True)
        row(label('ProgressBar(marquee)', 12, DIM), sized(p, cw=380, ch=14))
        return p
    demo('ProgressBar(marquee)', marquee)

    def radial():
        r = mk('/Script/AdvancedWidgets.RadialSlider')
        try:
            r.SetSliderProgressColor(ACCENT); r.SetSliderHandleColor(TXT)
        except Exception:
            pass
        r.SetValue(0.7)
        anim['radial'] = r
        row(label('RadialSlider', 12, DIM), sized(r, cw=90, ch=90))
        return r
    demo('RadialSlider', radial)

    # ── BUSY INDICATORS ─────────────────────────────────────────────────────────
    section('Busy Indicators')

    def throbber():
        t = mk('/Script/UMG.Throbber')
        t.SetNumberOfPieces(5)
        row(label('Throbber', 12, DIM), t)
        return t
    demo('Throbber', throbber)

    def circ():
        c = mk('/Script/UMG.CircularThrobber')
        c.SetNumberOfPieces(9)
        c.SetRadius(22.0)
        row(label('CircularThrobber', 12, DIM), sized(c, cw=52, ch=52))
        return c
    demo('CircularThrobber', circ)

    # ── CHOICE ──────────────────────────────────────────────────────────────────
    section('Choice')

    def combo():
        cb = mk('/Script/UMG.ComboBoxString')
        for opt in ('Space Rig', 'Salt Pits', 'Crystalline Caverns', 'Magma Core'):
            cb.AddOption(opt)
        cb.SetSelectedIndex(0)
        row(label('ComboBoxString', 12, DIM), sized(cb, cw=240, ch=34))
        return cb
    demo('ComboBoxString', combo)

    # ── LAYOUT PANELS ─────────────────────────────────────────────────────────────
    section('Layout Panels')

    def color_cell(rgba, text, cw=None, ch=None):
        b = mk('/Script/UMG.Border')
        b.SetBrushColor(rgba)
        b.SetContent(label(text, 11, {'R': 0.0, 'G': 0.0, 'B': 0.0, 'A': 1.0}))
        return sized(b, cw, ch) if (cw or ch) else b

    def grid():
        g = mk('/Script/UMG.GridPanel')
        for r in range(2):
            for c in range(3):
                cell = color_cell(CHIPS[(r * 3 + c) % len(CHIPS)], 'r%dc%d' % (r, c))
                sl = g.AddChildToGrid(sized(cell, cw=90, ch=42), r, c)
                sl.SetPadding(margin(3, 3, 3, 3))
        stack(label('GridPanel (2×3)', 11, DIM), pad=(0, 2, 0, 0))
        return stack(g)
    demo('GridPanel', grid)

    def ugrid():
        g = mk('/Script/UMG.UniformGridPanel')
        g.SetMinDesiredSlotWidth(70.0); g.SetMinDesiredSlotHeight(38.0)
        for r in range(2):
            for c in range(4):
                cell = color_cell(CHIPS[(r + c) % len(CHIPS)], '%d,%d' % (r, c))
                sl = g.AddChildToUniformGrid(cell, r, c)
                sl.SetHorizontalAlignment(HFILL); sl.SetVerticalAlignment(VFILL)
        stack(label('UniformGridPanel (2×4)', 11, DIM), pad=(0, 6, 0, 0))
        return stack(g)
    demo('UniformGridPanel', ugrid)

    def wrap():
        wb = mk('/Script/UMG.WrapBox')
        wb.SetInnerSlotPadding({'X': 6.0, 'Y': 6.0})
        for i in range(9):
            chip = color_cell(CHIPS[i % len(CHIPS)], 'chip %d' % i, cw=84, ch=30)
            wb.AddChildToWrapBox(chip)
        stack(label('WrapBox (flows & wraps)', 11, DIM), pad=(0, 6, 0, 0))
        return stack(wb)
    demo('WrapBox', wrap)

    def overlay():
        ov = mk('/Script/UMG.Overlay')
        base = color_cell(ACCENT, '')
        s0 = ov.AddChildToOverlay(base)
        s0.SetHorizontalAlignment(HFILL); s0.SetVerticalAlignment(VFILL)
        top = label('Overlay: stacked & centered', 12, {'R': 0.0, 'G': 0.0, 'B': 0.0, 'A': 1.0})
        s1 = ov.AddChildToOverlay(top)
        s1.SetHorizontalAlignment(HCENTER); s1.SetVerticalAlignment(VCENTER)
        stack(label('Overlay', 11, DIM), pad=(0, 6, 0, 0))
        return stack(sized(ov, ch=48))
    demo('Overlay', overlay)

    def stackbox():
        sbx = mk('/Script/UMG.StackBox')
        try:
            sbx.SetOrientation(0)   # horizontal
        except Exception:
            pass
        for i in range(3):
            sbx.AddChildToStackBox(color_cell(CHIPS[i], 'StackBox %d' % i, cw=120, ch=30))
        stack(label('StackBox', 11, DIM), pad=(0, 6, 0, 0))
        return stack(sbx)
    demo('StackBox', stackbox)

    def switcher():
        sw = mk('/Script/UMG.WidgetSwitcher')
        for i, col in enumerate((ACCENT, WARM, GOOD)):
            sw.AddChild(color_cell(col, 'WidgetSwitcher page %d' % i))
        sw.SetActiveWidgetIndex(0)
        anim['switcher'] = sw
        stack(label('WidgetSwitcher (auto-cycles)', 11, DIM), pad=(0, 6, 0, 0))
        return stack(sized(sw, ch=40))
    demo('WidgetSwitcher', switcher)

    # ── CONTENT WRAPPERS ─────────────────────────────────────────────────────────
    section('Content Wrappers')

    def border():
        b = mk('/Script/UMG.Border')
        b.SetBrushColor({'R': 0.10, 'G': 0.30, 'B': 0.20, 'A': 0.9})
        b.SetPadding(margin(10, 8, 10, 8)) if hasattr(b, 'SetPadding') else None
        b.SetContent(label('Border — brush + padded content', 12, TXT))
        return stack(b)
    demo('Border', border)

    def sizebox():
        sb = mk('/Script/UMG.SizeBox')
        sb.SetWidthOverride(300.0); sb.SetHeightOverride(30.0)
        inner = mk('/Script/UMG.Border'); inner.SetBrushColor({'R': 0.2, 'G': 0.2, 'B': 0.3, 'A': 0.9})
        inner.SetContent(label('SizeBox 300×30', 11, TXT))
        sb.SetContent(inner)
        return stack(sb, halign=HLEFT)
    demo('SizeBox', sizebox)

    def scalebox():
        sc = mk('/Script/UMG.ScaleBox')
        try:
            sc.SetStretch(2)   # ScaleToFit
        except Exception:
            pass
        sc.SetContent(label('ScaleBox scales this text to fit its box', 13, TXT))
        stack(label('ScaleBox', 11, DIM), pad=(0, 6, 0, 0))
        return stack(sized(sc, ch=40))
    demo('ScaleBox', scalebox)

    def blur():
        bb = mk('/Script/UMG.BackgroundBlur')
        bb.SetBlurStrength(8.0)
        bb.SetContent(label('BackgroundBlur — frosts what is behind it', 12, TXT))
        bb.SetPadding(margin(10, 8, 10, 8))
        return stack(sized(bb, ch=40))
    demo('BackgroundBlur', blur)

    def safezone():
        sz = mk('/Script/UMG.SafeZone')
        sz.SetContent(label('SafeZone — insets from TV-unsafe edges', 12, DIM))
        return stack(sz)
    demo('SafeZone', safezone)

    def retainer():
        rb = mk('/Script/UMG.RetainerBox')
        rb.SetContent(label('RetainerBox — renders content to a texture', 12, DIM))
        return stack(rb)
    demo('RetainerBox', retainer)

    def invalidation():
        ib = mk('/Script/UMG.InvalidationBox')
        ib.SetContent(label('InvalidationBox — caches child geometry', 12, DIM))
        return stack(ib)
    demo('InvalidationBox', invalidation)

    def namedslot():
        ns = mk('/Script/UMG.NamedSlot')
        ns.SetContent(label('NamedSlot — an injectable content hole', 12, DIM))
        return stack(ns)
    demo('NamedSlot', namedslot)

    def menuanchor():
        ma = mk('/Script/UMG.MenuAnchor')
        ma.SetContent(label('MenuAnchor — anchors a pop-up menu', 12, DIM))
        return stack(ma)
    demo('MenuAnchor', menuanchor)

    def expandable():
        ea = mk('/Script/UMG.ExpandableArea')
        ea.HeaderContent = label('ExpandableArea header (expanded)', 12, TXT)
        body = color_cell({'R': 0.12, 'G': 0.14, 'B': 0.20, 'A': 0.9},
                          'ExpandableArea body content', ch=40)
        ea.BodyContent = body
        ea.SetIsExpanded(True)
        return stack(ea)
    demo('ExpandableArea', expandable)

    # ── IMAGE / SPACER ───────────────────────────────────────────────────────────
    section('Image & Spacer')

    def image():
        img = mk('/Script/UMG.Image')
        img.SetColorAndOpacity(WARM)
        img.SetDesiredSizeOverride({'X': 64.0, 'Y': 64.0})
        sp = mk('/Script/UMG.Spacer'); sp.SetSize({'X': 40.0, 'Y': 40.0})
        img2 = mk('/Script/UMG.Image'); img2.SetColorAndOpacity(ACCENT)
        img2.SetDesiredSizeOverride({'X': 64.0, 'Y': 64.0})
        row(label('Image', 12, DIM), img, label('← Spacer(40) →', 11, DIM), sp, img2, valign=VCENTER)
        return img
    demo('Image + Spacer', image)

    # ── go live ───────────────────────────────────────────────────────────────
    w.AddToViewport(20000)
    w.add_to_root()
    tree.add_to_root()

    def on_tick(world, dt):
        st = globals().get('_showcase')
        if not st:
            return
        try:
            if not st['w'].IsInViewport():
                st['w'].AddToViewport(20000)
        except Exception:
            globals()['_showcase'] = None
            return
        a = st['anim']
        t = st['t'] = st.get('t', 0.0) + dt
        try:
            a['frame'].SetText(txt('frame %d   t=%.1fs' % (st.setdefault('n', 0), t)))
            st['n'] += 1
            pulse = 0.5 + 0.5 * math.sin(t * 1.5)
            if 'progress' in a:
                a['progress'].SetPercent(pulse)
            if 'slider' in a:
                a['slider'].SetValue(pulse)
            if 'radial' in a:
                a['radial'].SetValue(pulse)
            if 'switcher' in a:
                a['switcher'].SetActiveWidgetIndex(int(t // 1.2) % 3)
        except Exception:
            pass

    globals()['_showcase'] = {'w': w, 'tree': tree, 'anim': anim}
    globals()['_showcase']['tick'] = ue.on_tick(on_tick)

    ok = sum(1 for _, s in report if s == 'ok')
    print('Widget showcase armed: %d/%d widgets rendered.' % (ok, len(report)))
    for name, status in report:
        if status != 'ok':
            print('  FAIL %-26s %s' % (name, status))
    print('Skipped (need an entry UserWidget class / editor-only): '
          'ListView, TileView, TreeView, DynamicEntryBox, ComboBoxKey, '
          'NativeWidgetHost, Viewport, WindowTitleBarArea, PostBufferUpdate.')


def _teardown():
    """Remove any previously-installed showcase (and orphans) + its tick."""
    prev = globals().pop('_showcase', None)
    if prev:
        h = prev.get('tick')
        if h is not None:
            try:
                h.remove()
            except Exception:
                pass
    for uw in list(ue.objects_of_class('/Script/UMG.UserWidget')):
        try:
            t = uw.WidgetTree
            if t is not None and t._name.startswith('ShowcaseTree'):
                uw.RemoveFromParent()
                try:
                    uw.remove_from_root()
                except Exception:
                    pass
                try:
                    t.remove_from_root()
                except Exception:
                    pass
        except Exception:
            pass


def _pawn_pc():
    TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject
    gi = next((o for o in ue.objects_of_class('/Script/Engine.GameInstance')
               if not o._flags & TEMPLATE), None)
    return gi.LocalPlayers[0].PlayerController if gi else None


_teardown()
_pc = _pawn_pc()
if _pc is None:
    print('No local PlayerController — is the game in a level? Showcase not armed.')
else:
    _install()
del _install
