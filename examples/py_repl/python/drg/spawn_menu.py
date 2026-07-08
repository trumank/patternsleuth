"""Spawn-menu GUI — a searchable, scrollable list of spawnable actors.

This is step one of the freecam spawn mode: just the picker UI. It pulls every
actor-derived Blueprint out of the AssetRegistry, lists them in a scrolling
panel with a live search box, and hands the chosen class back to a callback.
Placement (line-trace + ghost mesh) is wired in a later step.

The list is drawn by `VirtualList` (below) — a reusable, recycled-pool virtual
scroller: a SizeBox gives the ScrollBox the full virtual height while only ~a
screenful of row widgets is ever built, recycled (re-captioned + repositioned)
as you scroll. Row height is measured from a real row rather than hard-coded, so
rows aren't squished. `VirtualList` is registered as a global so other scripts
(loaded into the same session) can reuse it.

    ┌─ SPAWN MENU ─────────────────────────────┐
    │  [ search…                             ]  │
    │  1888 actors · showing 1888               │
    │ ┌───────────────────────────────────────┐ │
    │ │ ( AIC_Bomber                        )  │ │
    │ │ ( BP_Barrel                         )  │ │   ← virtual scroll
    │ │ ( BP_Crate                          )  │ │
    │ │ …                                     │ │
    │ └───────────────────────────────────────┘ │
    │  ( Close )                                 │
    └────────────────────────────────────────────┘

Public entry points (persist as globals so freecam.py can call them):
    spawn_menu_open(pc, on_select=None)  -> opens the panel; on_select(entry) is
        called with the picked {'name', 'object_path', 'class_path'} dict, then
        the panel tears down and input returns to the game.
    spawn_menu_close()                   -> dismiss without picking.
    spawn_menu_actors()                  -> the cached actor list (builds once).

Running the file directly opens the menu against the live PlayerController with a
default callback that just prints the pick — handy for probing the UI in the REPL.

Idempotent: opening tears down any previous panel first. Persisted globals:
`_spawn_menu` (live panel state), `_spawn_menu_cache` (the actor list),
`_spawn_ktl` (cached KismetTextLibrary CDO), `_spawn_menu_result` (last pick), and
`_spawn_menu_search` (last search query, restored on re-open).
"""

TEMPLATE = ue.EObjectFlags.RF_ClassDefaultObject | ue.EObjectFlags.RF_ArchetypeObject

U = lambda p: ue.find(p)

# ── colors ────────────────────────────────────────────────────────────────
C_PANEL = {'R': 0.03, 'G': 0.04, 'B': 0.06, 'A': 0.94}
C_LIST  = {'R': 0.02, 'G': 0.03, 'B': 0.05, 'A': 0.85}
C_ITEM  = {'R': 0.12, 'G': 0.16, 'B': 0.24, 'A': 1.0}
C_CLOSE = {'R': 0.45, 'G': 0.18, 'B': 0.20, 'A': 1.0}
C_TITLE = {'R': 0.45, 'G': 0.80, 'B': 1.00, 'A': 1.0}
C_TXT   = {'R': 0.92, 'G': 0.95, 'B': 1.00, 'A': 1.0}
C_DIM   = {'R': 0.60, 'G': 0.66, 'B': 0.78, 'A': 1.0}

# Layout (canvas-local px, top-left origin).
PX, PY, PW, PH = 40.0, 80.0, 460.0, 560.0
PAD = 16.0


# ── asset-registry data layer ───────────────────────────────────────────────

def _assets_by_class(AR, class_name):
    """GetAssetsByClass across the UE4/UE5 signature change.

    UE4.27 takes an FName ClassName ('Blueprint'); UE5.1+ takes an
    FTopLevelAssetPath struct instead. Engine classes like Blueprint live in
    /Script/Engine. We probe the FName form and fall back to the struct."""
    try:
        return AR.GetAssetsByClass(class_name, True)
    except TypeError:
        return AR.GetAssetsByClass(
            {'PackageName': '/Script/Engine', 'AssetName': class_name}, True)


def _object_path(a):
    """Full 'Package.Asset' object path of an FAssetData, on both engines.

    UE4.27 exposes it as FAssetData.ObjectPath; UE5.1+ dropped that field, so
    rebuild it from PackageName + AssetName."""
    try:
        return str(a.ObjectPath)
    except AttributeError:
        return '%s.%s' % (a.PackageName, a.AssetName)


def _export_text_to_object_path(s):
    """Port of FPackageName::ExportTextPathToObjectPath.

    Object-reference asset-registry tags are stored in export-text form,
    ClassName'/Package/Path.Object' — and UE5 fully-qualifies the ClassName
    (/Script/CoreUObject.Class'...') where UE4.27 used a short prefix. UE unwraps
    it by the same rule in both: if the string ends in a quote, drop everything up
    to and including the first quote plus the trailing quote; otherwise it's
    already a bare path (UE4.27's NativeParentClass) and is returned unchanged."""
    if s.endswith("'"):
        i = s.find("'")
        if i != -1:
            return s[i + 1:-1]
    return s


def _actor_entries():
    """Every actor-derived Blueprint as {name, object_path, class_path}, sorted.

    Cheap: the actor test reads each Blueprint's cached NativeParentClass tag
    (no asset load) and checks that native class against Actor once per distinct
    native parent. ~0.1s for DRG's ~3k blueprints."""
    ARH = U('/Script/AssetRegistry.Default__AssetRegistryHelpers')
    AR = ARH.GetAssetRegistry()
    ACTOR = U('/Script/Engine.Actor')

    native_ok = {}

    def is_actor_native(path):
        hit = native_ok.get(path)
        if hit is None:
            c = U(path)
            hit = bool(c) and (c == ACTOR or any(p == ACTOR for p in c.parents()))
            native_ok[path] = hit
        return hit

    ok, assets = _assets_by_class(AR, 'Blueprint')
    out = []
    for a in assets:
        tag = ARH.GetTagValue(a, 'NativeParentClass')
        if not tag or not tag[0]:
            continue
        native = _export_text_to_object_path(tag[1])
        if not is_actor_native(native):
            continue
        obj_path = _object_path(a)
        # The spawnable UClass of a Blueprint is its generated class: Path.Name_C
        out.append({'name': a.AssetName,
                    'object_path': obj_path,
                    'class_path': obj_path + '_C'})
    out.sort(key=lambda e: e['name'].lower())
    return out


def spawn_menu_actors():
    cache = globals().get('_spawn_menu_cache')
    if cache is None:
        cache = _actor_entries()
        globals()['_spawn_menu_cache'] = cache
    return cache


# ── UMG helpers (shared idiom with interactive_test.py) ──────────────────────

def _widgets():
    return {
        'WBL':         U('/Script/UMG.Default__WidgetBlueprintLibrary'),
        'UserWidget':  U('/Script/UMG.UserWidget'),
        'WidgetTree':  U('/Script/UMG.WidgetTree'),
        'CanvasPanel': U('/Script/UMG.CanvasPanel'),
        'ScrollBox':   U('/Script/UMG.ScrollBox'),
        'SizeBox':     U('/Script/UMG.SizeBox'),
        'Image':       U('/Script/UMG.Image'),
        'TextBlock':   U('/Script/UMG.TextBlock'),
        'Button':      U('/Script/UMG.Button'),
        'EditBox':     U('/Script/UMG.EditableTextBox'),
    }


def _pc():
    gi = next((o for o in ue.objects_of_class('/Script/Engine.GameInstance')
               if not o._flags & TEMPLATE), None)
    return gi.LocalPlayers[0].PlayerController if gi else None


def _ktl():
    # Cache the KismetTextLibrary CDO. ue.find() is ~240us — dwarfing the actual
    # text conversion — so re-resolving it per caption made scrolling hitch.
    ktl = globals().get('_spawn_ktl')
    if ktl is None:
        ktl = U('/Script/Engine.Default__KismetTextLibrary')
        globals()['_spawn_ktl'] = ktl
    return ktl


def _text(s):
    return _ktl().Conv_StringToText(str(s))


def _str(ft):
    try:
        return _ktl().Conv_TextToString(ft)
    except Exception:
        return ''


def _slate(rgba):
    return {'SpecifiedColor': rgba, 'ColorUseRule': 0}


def _font(size):
    f = U('/Script/UMG.Default__TextBlock').Font
    f.Size = int(size)
    return f


def _place(canvas, child, x, y, w, h, z=0):
    slot = canvas.AddChildToCanvas(child)
    slot.SetAutoSize(False)
    slot.SetPosition({'X': x, 'Y': y})
    slot.SetSize({'X': w, 'Y': h})
    slot.SetZOrder(z)
    return slot


def _label(W, tree, canvas, x, y, size, rgba, text='', z=5):
    tb = ue.new_object(W['TextBlock'], outer=tree)
    tb.SetFont(_font(size))
    tb.SetColorAndOpacity(_slate(rgba))
    if text:
        tb.SetText(_text(text))
    slot = canvas.AddChildToCanvas(tb)
    slot.SetAutoSize(True)
    slot.SetPosition({'X': x, 'Y': y})
    slot.SetZOrder(z)
    return tb


def _set_input(pc, on):
    W = _widgets()
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


# ── VirtualList — reusable recycled-pool virtual scroller ────────────────────

class VirtualList:
    """A ScrollBox that shows an arbitrarily long `items` list while only ever
    building ~a screenful of row widgets.

    A SizeBox stretches the scroll content to the full virtual height (rows ×
    pitch); a fixed pool of row widgets lives in an inner CanvasPanel, absolutely
    positioned, and is recycled — re-shown and re-filled — as the view scrolls.
    Per-event cost is bounded by the pool size, not the list length or scroll
    distance.

    Row height is *measured*, not assumed: the first `set_items` seeds one row
    with a real item, forces a layout prepass, and reads its desired size. So the
    caller must add the owning UserWidget to the viewport before the first
    `set_items` (Slate has no measure context until then).

    Construction is decoupled from content via two callbacks:
        make_row() -> (widget, update)
            `widget` is a clickable row widget (needs an `OnClicked` delegate,
            e.g. a Button); `update(item)` fills it in for a given item.
        on_select(item)   (optional) fired when a row is clicked.
    """

    GAP = 2.0            # vertical spacing between rows, px
    BUFFER = 3           # extra pooled rows beyond the visible count
    SCROLLBAR_W = 18.0   # width reserved so captions don't sit under the bar

    def __init__(self, W, tree, root, width, height, make_row, on_select=None):
        self._tree = tree
        self._root = root                 # UserWidget, for ForceLayoutPrepass
        self._height = height
        self._make_row = make_row
        self._on_select = on_select
        self._row_w = width - self.SCROLLBAR_W

        self._items = []
        self._pool = []
        self._handles = []
        self._cell_h = None               # measured row height (natural)
        self._pitch = 24.0                # cell_h + GAP; provisional until measured
        self._measured = False

        self._scroll = ue.new_object(W['ScrollBox'], outer=tree)
        self._box = ue.new_object(W['SizeBox'], outer=tree)
        self._inner = ue.new_object(W['CanvasPanel'], outer=tree)
        self._box.SetContent(self._inner)
        self._scroll.AddChild(self._box)
        self.widget = self._scroll        # place this in your canvas

        self._add_cell()                  # seed row (used to measure)
        self._handles.append(
            self._scroll.OnUserScrolled.add_callable(self._on_scroll))

    # -- pool ----------------------------------------------------------------

    def _add_cell(self):
        widget, update = self._make_row()
        slot = self._inner.AddChildToCanvas(widget)
        slot.SetAutoSize(False)
        if self._cell_h:
            slot.SetSize({'X': self._row_w, 'Y': self._cell_h})
        cell = {'w': widget, 'update': update, 'slot': slot,
                'item': None, 'row': -1, 'shown': True}
        self._pool.append(cell)
        self._handles.append(
            widget.OnClicked.add_callable(lambda c=cell: self._click(c)))
        return cell

    def _measure(self, sample):
        # Fill the seed row with a real item, lay it out, read its height.
        self._pool[0]['update'](sample)
        self._root.ForceLayoutPrepass()
        h = self._pool[0]['w'].GetDesiredSize().Y
        self._cell_h = h if h > 4.0 else 24.0
        self._pitch = self._cell_h + self.GAP
        for c in self._pool:
            c['slot'].SetSize({'X': self._row_w, 'Y': self._cell_h})
        need = int(self._height // self._pitch) + self.BUFFER
        while len(self._pool) < need:
            self._add_cell()
        self._measured = True

    # -- public --------------------------------------------------------------

    def set_items(self, items):
        self._items = items
        if not self._measured and items:
            self._measure(items[0])
        try:
            self._box.SetHeightOverride(max(len(items), 1) * self._pitch)
        except Exception:
            pass
        try:
            self._scroll.SetScrollOffset(0.0)
        except Exception:
            pass
        for c in self._pool:              # invalidate so every slot re-fills
            c['row'] = -1
        self._reposition(0.0)

    def teardown(self):
        for h in self._handles:
            try:
                h.unbind()
            except Exception:
                pass
        self._handles = []

    # -- internals -----------------------------------------------------------

    def _reposition(self, offset):
        # Map the pool onto the rows currently in view using a ring buffer: the
        # cell for row r is pool[r % P]. Advancing the view by one row then
        # recycles exactly ONE cell (the one leaving the top wraps to the new
        # bottom row) — every other cell keeps its row, position and content
        # untouched. That's what keeps inertial scrolling flicker-free: without
        # it, a slot-i-shows-first+i mapping re-captions the whole pool at once
        # each time a row boundary is crossed, which flashes mid-animation.
        items = self._items
        n = len(items)
        pitch = self._pitch
        P = len(self._pool)
        first = int(offset // pitch)
        if first < 0:
            first = 0
        for si, c in enumerate(self._pool):
            # The unique row in [first, first+P) that maps to this slot.
            row = first + (si - first) % P
            if row < n:
                if c['row'] != row:
                    it = items[row]
                    c['item'] = it
                    c['row'] = row
                    c['slot'].SetPosition({'X': 2.0, 'Y': row * pitch})
                    c['update'](it)
                if not c['shown']:
                    try:
                        c['w'].SetVisibility(0)      # ESlateVisibility::Visible
                    except Exception:
                        pass
                    c['shown'] = True
            elif c['shown']:
                c['item'] = None
                c['row'] = -1
                try:
                    c['w'].SetVisibility(1)          # Collapsed
                except Exception:
                    # No enum marshalling — park it below the content instead.
                    c['slot'].SetPosition({'X': 2.0, 'Y': (n + si) * pitch})
                c['shown'] = False

    def _on_scroll(self, offset=0.0, *_):
        self._reposition(offset)

    def _click(self, cell):
        if cell['item'] is not None and self._on_select:
            self._on_select(cell['item'])


# ── panel ────────────────────────────────────────────────────────────────

def _build(pc, on_select):
    W = _widgets()
    entries = spawn_menu_actors()

    w = W['WBL'].Create(pc, W['UserWidget'], pc)
    tree = ue.new_object(W['WidgetTree'], outer=w, name='PySpawnMenuTree')
    canvas = ue.new_object(W['CanvasPanel'], outer=tree, name='PySpawnMenuCanvas')
    tree.RootWidget = canvas
    w.WidgetTree = tree

    # backdrop
    bg = ue.new_object(W['Image'], outer=tree)
    bg.SetColorAndOpacity(C_PANEL)
    _place(canvas, bg, PX, PY, PW, PH, z=0)

    # title
    _label(W, tree, canvas, PX + PAD, PY + 12.0, 16, C_TITLE, 'SPAWN MENU')

    # search box
    IN_X = PX + PAD
    IN_W = PW - 2 * PAD
    search = ue.new_object(W['EditBox'], outer=tree)
    search.SetHintText(_text('search…'))
    _place(canvas, search, IN_X, PY + 44.0, IN_W, 32.0, z=3)
    # Restore the query from the previous session (see _apply_filter).
    prev_q = globals().get('_spawn_menu_search', '')
    if prev_q:
        search.SetText(_text(prev_q))

    # count line
    count = _label(W, tree, canvas, IN_X, PY + 84.0, 12, C_DIM, '')

    # list backdrop
    LIST_Y = PY + 108.0
    LIST_H = PH - (108.0 + 60.0)
    lbg = ue.new_object(W['Image'], outer=tree)
    lbg.SetColorAndOpacity(C_LIST)
    _place(canvas, lbg, IN_X, LIST_Y, IN_W, LIST_H, z=1)

    cap_font = _font(12)                  # one font struct, shared by every row

    st = {'w': w, 'tree': tree, 'pc': pc, 'search': search, 'count': count,
          'handles': [], 'entries': entries, 'on_select': on_select, 'vlist': None}

    def _pick(entry):
        print('[spawn_menu] selected:', entry['name'], '->', entry['class_path'])
        globals()['_spawn_menu_result'] = entry
        cb = st['on_select']
        _teardown(st)
        if cb:
            try:
                cb(entry)
            except Exception as e:
                print('[spawn_menu] on_select error:', type(e).__name__, e)

    def make_row():
        b = ue.new_object(W['Button'], outer=tree)
        b.SetBackgroundColor(C_ITEM)
        cap = ue.new_object(W['TextBlock'], outer=tree)
        cap.SetFont(cap_font)
        cap.SetColorAndOpacity(_slate(C_TXT))
        b.SetContent(cap)

        def update(entry):
            # Cache the FText handle on the entry — SetText with a ready handle
            # is free; only the first display of a row builds one.
            ft = entry.get('_ft')
            if ft is None:
                ft = _text(entry['name'])
                entry['_ft'] = ft
            cap.SetText(ft)

        return b, update

    vlist = VirtualList(W, tree, w, IN_W - 8.0, LIST_H - 8.0, make_row, on_select=_pick)
    st['vlist'] = vlist
    _place(canvas, vlist.widget, IN_X + 4.0, LIST_Y + 4.0, IN_W - 8.0, LIST_H - 8.0, z=2)

    def _apply_filter():
        raw = _str(search.GetText())
        globals()['_spawn_menu_search'] = raw   # persist across re-opens
        q = raw.strip().lower()
        matches = [e for e in entries if q in e['name'].lower()] if q else entries
        vlist.set_items(matches)
        count.SetText(_text('%d actors · showing %d' % (len(entries), len(matches))))

    st['handles'].append(search.OnTextChanged.add_callable(_apply_filter))

    # close button
    close = ue.new_object(W['Button'], outer=tree)
    close.SetBackgroundColor(C_CLOSE)
    ccap = ue.new_object(W['TextBlock'], outer=tree)
    ccap.SetFont(_font(13))
    ccap.SetColorAndOpacity(_slate(C_TXT))
    ccap.SetText(_text('Close'))
    close.SetContent(ccap)
    _place(canvas, close, IN_X, PY + PH - 46.0, 108.0, 34.0, z=4)
    st['handles'].append(close.OnClicked.add_callable(lambda: _teardown(st)))

    w.AddToViewport(30000)
    w.add_to_root()
    tree.add_to_root()
    _apply_filter()          # on-screen now: measures the row + fills the list
    _set_input(pc, True)
    return st


def _teardown(st):
    if not st:
        return
    vlist = st.get('vlist')
    if vlist:
        try:
            vlist.teardown()
        except Exception:
            pass
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
    globals().pop('_spawn_menu', None)


def _purge_all():
    """Drop any leftover panel from an earlier run (orphaned trees)."""
    for w in list(ue.objects_of_class('/Script/UMG.UserWidget')):
        try:
            t = w.WidgetTree
            if t is not None and t._name.startswith('PySpawnMenuTree'):
                w.RemoveFromParent()
                try: w.remove_from_root()
                except Exception: pass
                try: t.remove_from_root()
                except Exception: pass
        except Exception:
            pass


# ── public API (persisted as globals) ────────────────────────────────────

def spawn_menu_close():
    _teardown(globals().get('_spawn_menu'))
    _purge_all()


def spawn_menu_open(pc=None, on_select=None):
    spawn_menu_close()
    pc = pc or _pc()
    if pc is None:
        print('spawn_menu: no PlayerController — is the game in a level?')
        return None
    st = _build(pc, on_select)
    globals()['_spawn_menu'] = st
    print('spawn_menu: %d actors listed. Search, click one, or Close.'
          % len(st['entries']))
    return st


globals()['VirtualList'] = VirtualList
globals()['spawn_menu_open'] = spawn_menu_open
globals()['spawn_menu_close'] = spawn_menu_close
globals()['spawn_menu_actors'] = spawn_menu_actors


# Running the file (via repl --script) defines the API above, then opens the
# menu now so it's testable live. freecam.py reuses spawn_menu_open() later.
spawn_menu_open()
