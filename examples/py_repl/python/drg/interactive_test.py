"""A small *interactive* UMG panel — the first widget wired to live click events.

Unlike hud.py / widget_showcase.py (which only display), this one responds:
buttons are bound to Python callables through the new
`Button.OnClicked.add_callable(fn)` API, and an EditableTextBox feeds those
callbacks live text.

    ┌─ PY INTERACTIVE TEST ─────────────┐
    │  [ type here…                    ] │
    │  ( Echo ) ( Count ) ( Clear )      │
    │  ( Close )                         │
    │  status: …                        │
    └───────────────────────────────────┘

    Echo   -> copy the textbox into the status line (and print it over the REPL)
    Count  -> increment a click counter shown in the status line
    Clear  -> empty the textbox
    Close  -> tear the panel down and hand input back to the game

Clicking is real: running this shows the mouse cursor and switches the player to
Game+UI input so the pointer can reach the widget; Close (or re-running the
script) restores game-only input.

Everything lives in _install(); the only globals leaked are the persistence
handles (`_it`), so re-running is idempotent — it tears the previous panel down
(unbinding every callable, so we never leak manufactured sink functions) and
rebuilds.
"""


def _install():
    TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

    U           = lambda p: ue.find(p)
    WBL         = U('/Script/UMG.Default__WidgetBlueprintLibrary')
    KTL         = U('/Script/Engine.Default__KismetTextLibrary')
    UserWidget  = U('/Script/UMG.UserWidget')
    WidgetTree  = U('/Script/UMG.WidgetTree')
    CanvasPanel = U('/Script/UMG.CanvasPanel')
    Image       = U('/Script/UMG.Image')
    TextBlock   = U('/Script/UMG.TextBlock')
    Button      = U('/Script/UMG.Button')
    EditBox     = U('/Script/UMG.EditableTextBox')

    # Layout (canvas-local px, top-left origin).
    PX, PY, PW, PH = 60.0, 120.0, 380.0, 250.0
    PAD = 18.0
    IN_X = PX + PAD
    IN_W = PW - 2 * PAD

    C_PANEL  = {'R': 0.03, 'G': 0.04, 'B': 0.06, 'A': 0.90}
    C_BTN    = {'R': 0.15, 'G': 0.35, 'B': 0.55, 'A': 1.0}
    C_CLOSE  = {'R': 0.45, 'G': 0.18, 'B': 0.20, 'A': 1.0}
    C_TITLE  = {'R': 0.45, 'G': 0.80, 'B': 1.00, 'A': 1.0}
    C_TXT    = {'R': 0.92, 'G': 0.95, 'B': 1.00, 'A': 1.0}
    C_DIM    = {'R': 0.60, 'G': 0.66, 'B': 0.78, 'A': 1.0}

    def live(cls):
        return next((o for o in ue.objects_of_class(cls) if not o._flags & TEMPLATE), None)

    def _pc():
        gi = live('/Script/Engine.GameInstance')
        return gi.LocalPlayers[0].PlayerController if gi else None

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
        f.Size = int(size)   # FSlateFontInfo.Size is int32 on this build
        return f

    def _place(canvas, child, x, y, w, h, z=0):
        slot = canvas.AddChildToCanvas(child)
        slot.SetAutoSize(False)
        slot.SetPosition({'X': x, 'Y': y})
        slot.SetSize({'X': w, 'Y': h})
        slot.SetZOrder(z)
        return slot

    def _label(tree, canvas, x, y, size, rgba, text='', z=5):
        tb = ue.new_object(TextBlock, outer=tree)
        tb.SetFont(_font(size))
        tb.SetColorAndOpacity(_slate(rgba))
        if text:
            tb.SetText(_text(text))
        slot = canvas.AddChildToCanvas(tb)
        slot.SetAutoSize(True)
        slot.SetPosition({'X': x, 'Y': y})
        slot.SetZOrder(z)
        return tb

    def _button(tree, canvas, x, y, w, h, caption, color, on_click, handles):
        b = ue.new_object(Button, outer=tree)
        b.SetBackgroundColor(color)
        cap = ue.new_object(TextBlock, outer=tree)
        cap.SetFont(_font(13))
        cap.SetColorAndOpacity(_slate(C_TXT))
        cap.SetText(_text(caption))
        b.SetContent(cap)
        _place(canvas, b, x, y, w, h, z=4)
        handles.append(b.OnClicked.add_callable(on_click))
        return b

    def _set_input(pc, on):
        """Toggle mouse cursor + input routing so the panel is clickable (on) or
        the game has input back (off). All guarded — signatures vary by build."""
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

    def _build(pc):
        w = WBL.Create(pc, UserWidget, pc)
        tree = ue.new_object(WidgetTree, outer=w, name='PyInteractiveTree')
        canvas = ue.new_object(CanvasPanel, outer=tree, name='PyInteractiveCanvas')
        tree.RootWidget = canvas
        w.WidgetTree = tree

        # backdrop
        bg = ue.new_object(Image, outer=tree)
        bg.SetColorAndOpacity(C_PANEL)
        _place(canvas, bg, PX, PY, PW, PH, z=0)

        # title
        _label(tree, canvas, PX + PAD, PY + 12.0, 15, C_TITLE, 'PY INTERACTIVE TEST')

        # text input
        editbox = ue.new_object(EditBox, outer=tree)
        editbox.SetHintText(_text('type here…'))
        _place(canvas, editbox, IN_X, PY + 44.0, IN_W, 32.0, z=3)

        # status line
        status = _label(tree, canvas, IN_X, PY + 190.0, 13, C_DIM, 'status: ready')

        st = {'w': w, 'tree': tree, 'pc': pc, 'editbox': editbox,
              'status': status, 'handles': [], 'count': 0}

        def _status(s):
            status.SetText(_text(s))

        def on_echo():
            s = _str(editbox.GetText())
            _status('echo: ' + (s if s else '(empty)'))
            print('[interactive_test] echo:', repr(s))

        def on_count():
            st['count'] += 1
            _status('count: %d' % st['count'])
            print('[interactive_test] count =', st['count'])

        def on_clear():
            editbox.SetText(_text(''))
            _status('cleared')

        def on_close():
            print('[interactive_test] close clicked')
            _teardown(st)

        by = PY + 92.0
        _button(tree, canvas, IN_X,          by, 108.0, 36.0, 'Echo',  C_BTN,   on_echo,  st['handles'])
        _button(tree, canvas, IN_X + 118.0,  by, 108.0, 36.0, 'Count', C_BTN,   on_count, st['handles'])
        _button(tree, canvas, IN_X + 236.0,  by, 108.0, 36.0, 'Clear', C_BTN,   on_clear, st['handles'])
        _button(tree, canvas, IN_X,   PY + 140.0, 108.0, 34.0, 'Close', C_CLOSE, on_close, st['handles'])

        w.AddToViewport(30000)
        w.add_to_root()
        tree.add_to_root()
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
        globals().pop('_it', None)

    def _purge_all():
        """Drop any leftover panel from an earlier run (orphaned trees)."""
        for w in list(ue.objects_of_class('/Script/UMG.UserWidget')):
            try:
                t = w.WidgetTree
                if t is not None and t._name.startswith('PyInteractiveTree'):
                    w.RemoveFromParent()
                    try: w.remove_from_root()
                    except Exception: pass
                    try: t.remove_from_root()
                    except Exception: pass
            except Exception:
                pass

    # ── idempotent teardown of a previous run ─────────────────────────────────
    _teardown(globals().get('_it'))
    _purge_all()

    # ── arm ───────────────────────────────────────────────────────────────────
    pc = _pc()
    if pc is None:
        print('interactive_test: no PlayerController — is the game in a level?')
        return
    globals()['_it'] = _build(pc)
    print('interactive_test: panel armed. Click the buttons; press Close to dismiss.')


_install()
del _install
