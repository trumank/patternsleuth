# Dynamic `FName` — deferred implementation plan

**Status: POSTPONED.** Blocked on migrating `FName`'s engine-mirror embedders
first (see [§ Blocker](#blocker-why-this-is-postponed)). This doc captures the
full agreed design so it can be picked up later without re-deriving it.

Context: libue's UObject hierarchy has been migrated to opaque views + a runtime
offset table (`UeLayout`, hard-coded `*Layout::UE_4_27` consts + `set_layout`
injection). `FName` is the remaining version-variant piece — and the hardest,
because it's a **value type**, not a pointer-backed view.

The reference implementation for the version-complete model is
`meatloaf/jmap_dumper` (operates on read-only dumps; we lift its *layout* model
and deliberately **drop** its FNamePool string-walker — see below).

---

## Why `FName` is categorically different from the structs we migrated

Everything migrated so far was **always behind a stable pointer** into engine
memory, so the opaque-ZST trick worked (we never owned one by value). `FName` is
the opposite — a **detached value**: copied out of `name_private()`, produced by
`FName::from_string` with no backing object, compared (`==`), hashed, used as a
map key, and embedded by value in element types. It must have an *owned*
representation, so the ZST-view approach doesn't apply.

### Why the internal fields must be read (can't be fully opaque)

libue **reimplements UE's FName hashing and equality** so it can walk the
engine's object-hash tables directly (`uobject_hash.rs` — `get_object_hash`,
`find_object`), instead of calling engine lookup functions. That fast path
*requires* isolating `comparison_index` + `number`:

- **Hash** (`GetTypeHash for FName`, `lib.rs:2283`) must reproduce UE's hash
  exactly (buckets were built by the engine): hash of `comparison_index` combined
  with `number` — **never** `display_index`.
- **Equality** (the bucket-scan confirm, `obj.name_private() == name`) compares
  `comparison_index` + `number` and **excludes `display_index`** — in
  case-preserving builds two *equal* names with different casing share a
  `comparison_index` but differ in `display_index`; a raw `memcmp` would call them
  unequal and miss.

So a raw/opaque blob can't be hashed or compared correctly. The only way to make
FName fully opaque is to stop reimplementing eq/hash (call `StaticFindObjectFast`
etc. instead) — trading libue's fast table-walk for opacity. Not worth it.

Display and construction, notably, do **not** need field access — they just hand
the engine a correctly-laid-out FName (the marshaling, below).

---

## The variance matrix (lifted from jmap `unreal/src/unreal.gs`)

`ComparisonIndex` is always at offset 0. Only `Number`/`DisplayIndex`/`size` move.

| Build | Fields (in order) | `number_off` | `display_index_off` | `size` | align |
|---|---|---|---|---|---|
| `<4.22` | `int32 ComparisonIndex; uint32 Number` | 4 | — | 8 | **8** |
| `4.22` | `int32 ComparisonIndex; uint32 Number` | 4 | — | 8 | 4 |
| `≥4.23`, non-CPN | `FNameEntryId ComparisonIndex; uint32 Number` | 4 | — | 8 | 4 |
| `≥4.23 <5.01`, **CPN** | `ComparisonIndex; DisplayIndex; Number` | **8** | 4 | 12 | 4 |
| `≥5.01`, **CPN** | `ComparisonIndex; Number; DisplayIndex` | **4** | 8 | 12 | 4 |

Two non-obvious points this matrix nails:
- In CPN **≥5.01** `DisplayIndex` moved *after* `Number`, so `Number` is back at
  +4 — it is **not** "CPN always pushes Number to +8".
- Alignment is **8** on 4.11–4.21 (matters for embedded striding there).

(`CPN` = `WITH_CASE_PRESERVING_NAME`, typically editor builds.)

### What we take vs. drop from jmap

jmap separates **layout** (declared in Gospel, resolved via `struct_member`) from
**string source** (it re-walks the `FNamePool` block/offset table by hand,
`containers.rs:158` `resolve_fname`, because dumps can't call functions).

- **Take:** the entire layout matrix above. It's the same data our table needs.
- **Drop:** the `resolve_fname` pool-walker. libue runs **live**, so it calls the
  engine's `FName::ToString` (already resolved via patternsleuth,
  `lib.rs:61` `fname_to_string`) — faster and authoritative. The pool-walk is
  jmap's "different angle"; layout is shared, string-source is not.

---

## Agreed design

### 1. `FName` is the **superset** value type

A union of every version's physical fields — one fixed Rust type regardless of
target build, lossless (carries `display_index` → exact display casing),
future-proof. **Not `#[repr(C)]`** — it's a logical value, never a memory mirror,
so there is no legal direct read (`read_unaligned::<FName>` is gone; everything
goes through the boundary).

```rust
#[derive(Debug, Clone, Copy)]
pub struct FName {
    pub comparison_index: FNameEntryId,
    pub display_index: FNameEntryId,   // mirrors comparison_index on non-CPN builds
    pub number: u32,
}

// Identity is the version-invariant subset — display_index rides along but is
// NOT part of equality/hash.
impl PartialEq for FName {
    fn eq(&self, o: &Self) -> bool {
        self.comparison_index == o.comparison_index && self.number == o.number
    }
}
impl Eq for FName {}
// GetTypeHash unchanged — already defined on comparison_index + number.

impl FName {
    pub const NONE: FName = FName {
        comparison_index: FNameEntryId { value: 0 },
        display_index: FNameEntryId { value: 0 },
        number: 0,
    };
}
```

### 2. Boundary at the **memory edge**, decode-on-read

`comparison_index + number` is the **version-invariant logical identity**; only
the physical byte layout varies. Cut there: physical (layout-dependent) below,
logical (layout-free) above. The whole rest of libue + py_repl — hashing, the
uobject_hash walks, bindings — sees only the logical value and is version-agnostic.

Decode-on-read (not decode-on-use) because hash/eq run in bucket-scan loops
(decode once at read, not per comparison) and it keeps the layout dependency from
leaking past libue.

The boundary is exactly **two helpers + one table**:

```rust
pub(crate) unsafe fn read_fname(ptr: *const u8) -> FName {
    let l = layout().fname;
    let comparison_index = FNameEntryId { value: (ptr as *const u32).read_unaligned() };
    FName {
        comparison_index,
        number: (ptr.add(l.number_off) as *const u32).read_unaligned(),
        display_index: match l.display_index_off {
            Some(off) => FNameEntryId { value: (ptr.add(off) as *const u32).read_unaligned() },
            None => comparison_index,            // build has no DisplayIndex → mirror
        },
    }
}

pub(crate) unsafe fn marshal_fname(name: &FName, buf: &mut [u8; 16]) -> *const FName {
    let l = layout().fname;
    *buf = [0; 16];
    (buf.as_mut_ptr() as *mut u32).write_unaligned(name.comparison_index.value);
    (buf.as_mut_ptr().add(l.number_off) as *mut u32).write_unaligned(name.number);
    if let Some(off) = l.display_index_off {
        (buf.as_mut_ptr().add(off) as *mut u32).write_unaligned(name.display_index.value);
    }
    buf.as_ptr() as *const FName
}
```

### 3. `FNameLayout` in the runtime table

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FNameLayout {
    pub number_off: usize,
    pub display_index_off: Option<usize>,  // None for non-case-preserving
    pub size: usize,
}
impl FNameLayout {
    pub const UE_4_27: Self = Self { number_off: 4, display_index_off: None, size: 8 };
    // CPN  <5.01:  Self { number_off: 8, display_index_off: Some(4), size: 12 }
    // CPN >=5.01:  Self { number_off: 4, display_index_off: Some(8), size: 12 }
}
```

Add `fname: FNameLayout` to the `ue_layout!` aggregate (it composes via its
`UE_4_27` const like the others).

### 4. The four boundary crossings

| Crossing | Direction | Helper |
|---|---|---|
| `name` macro accessor (`name_private()`) | physical → logical | `read_fname` |
| `UEnum::entries()` striding | physical → logical (per element) | `read_fname` + size-driven stride |
| `from_string` (ctor output) | physical → logical | `read_fname` |
| `Display`/ToString | logical → physical | `marshal_fname` |

Macro `name` accessor kind (added to `ue_struct!`'s `@accessor` rules):

```rust
(@accessor $tbl:ident, name $fname:ident : $fty:ty) => {
    pub fn $fname(&self) -> FName {
        unsafe { crate::read_fname(self.base().add(crate::layout().$tbl.$fname)) }
    }
};
(@accessor $tbl:ident, name $fname:ident as $acc:ident : $fty:ty) => {
    pub fn $acc(&self) -> FName {
        unsafe { crate::read_fname(self.base().add(crate::layout().$tbl.$fname)) }
    }
};
```

Then `UObjectBase`'s field (`lib.rs:1415`): `scalar name_private: FName` →
`name name_private: FName` (same `-> FName` signature, no caller changes).

`Display` / `from_string`:

```rust
impl Display for FName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut buf = [0u8; 16];
        let mut string = FString::new();
        unsafe {
            let p = marshal_fname(self, &mut buf);
            (globals().fname_to_string)(p, &mut string);   // sig change below
        }
        write!(f, "{string}")
    }
}

pub fn from_string(s: &str, find_type: i32) -> Self {
    let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u8; 16];
    unsafe {
        (globals().fname_ctor_wchar)(buf.as_mut_ptr() as *mut FName, wide.as_ptr(), find_type);
        read_fname(buf.as_ptr())
    }
}
```

FFI signature change (avoid handing the engine an under-sized reference):
`pub type FnFNameToString = unsafe extern "system" fn(*const FName, &mut FString);`
(was `&FName`). The `transmute` in `py_repl/src/lib.rs:110` is unaffected.

`UEnum::entries()` — element stride depends on FName size, so replace the
`&[FEnumNameEntry]` slice with a decoding iterator (uses `TArray::as_ptr()` +
`len()`, which are element-type-agnostic):

```rust
pub fn entries(&self) -> impl Iterator<Item = (FName, i64)> + '_ {
    let arr = self.names_array();
    let base = arr.as_ptr() as *const u8;
    let len = arr.len();
    let fname_size = layout().fname.size;
    let value_off = (fname_size + 7) & !7;   // i64 is 8-aligned
    let stride = value_off + 8;
    (0..len).map(move |i| unsafe {
        let p = base.add(i * stride);
        (read_fname(p), (p.add(value_off) as *const i64).read_unaligned())
    })
}
```
Caller ripple (`&[FEnumNameEntry]` → `(FName,i64)` tuples): `name_by_value`
(`lib.rs:1819`), `value_by_name_str` (`lib.rs:1829`), `max_enum_value`
(`lib.rs:1857`), and `bindings.rs:339`. Remove the
`size_of::<FEnumNameEntry>() == 16` assert (`lib.rs:1784`).

---

## Blocker: why this is postponed

The 12-byte superset (and `FName` ceasing to be `#[repr(C)]`) **breaks every
engine-mirror struct that embeds `FName` by value — even on 4.27**, because their
layout assumes an 8-byte FName. These must be migrated **first**.

Embedders found (`grep ': FName'` in struct defs), with their hazard:

| Struct | File | Hazard |
|---|---|---|
| `FScriptDelegate.function_name` | `delegate.rs:14` | `size_of == 16` assert (`delegate.rs:17`); read from `TArray<FScriptDelegate>` and `*mut FScriptDelegate` (`properties/mod.rs:540,565`, `bindings.rs:2787`) → mis-stride |
| `FFieldClass.name` | `lib.rs:1889` | FField tree; cast from engine; ~10 use sites via `prop.ffield.class().name` (`bindings.rs:484,621,766,1602,1817,1818,1975,2131,2648,2687`) |
| `FSoftObjectPath.asset_path_name` | `lib.rs:989` | embedded in soft-object property value types; sites `lib.rs:1007,1045` |
| `FFieldBase.name_private` | `lib.rs:1955` | FField tree (already deferred); `name()` reads it |
| `FProperty.rep_notify_func` | `properties/mod.rs:63` | FField tree; not currently read, but shifts later fields |
| `FEnumNameEntry.name` | `lib.rs:1781` | handled by `entries()` manual striding; `size==16` assert removed |

**Required prerequisite work:** migrate these embedders so they no longer embed a
`FName` value of build-dependent size. Two viable shapes:
1. **Physical embed type** (`FNameSlot` / `FNameId`, `#[repr(C)]`, the build's
   physical FName) embedded in mirror structs, with `.get() -> FName` (decode) and
   `From<FName>` (write). Give it the same field names + `Display`/`PartialEq` as
   today's `FName` so embedder *use sites* barely change; only struct fields and
   construction sites flip `FName` → `FNameSlot`. Lowest churn.
2. **Opaque-migrate** each embedder (like the UObject hierarchy) and read its
   `name` field via `read_fname`. Heavier; right if those structs need full
   version-dynamic layout anyway (the FField tree probably does).

The FField tree (`FFieldBase`/`FFieldClass`/`FProperty`) was already the deferred
"second tree" — it overlaps heavily with this list, so **migrating the FField
tree is the natural unblocking step**, with `FScriptDelegate`/`FSoftObjectPath`
handled alongside (option 1).

---

## Sequenced plan (when unblocked)

1. **Prereq — migrate embedders.** FField tree (`FFieldBase`, `FFieldClass`,
   `FField`, `FProperty` + subclasses) to opaque + offset table (note: this tree
   has **by-value test fixtures**, e.g. `properties/set.rs:427,494,643` — they
   construct properties by value and will need a different strategy). Handle
   `FScriptDelegate` / `FSoftObjectPath` via a physical `FNameSlot` embed type.
   Each step compiled + tested through the three gates below.
2. **Stage 1 — boundary infra, FName still 8-byte `repr(C)`.** Add `FNameLayout`
   + `read_fname`/`marshal_fname` + macro `name` kind + route `name_private`,
   `Display`, `from_string` through them. Behavior-neutral on 4.27 (read_fname of
   {cmp@0, number@4} == today's struct read); no embedder impact. Green checkpoint.
3. **Stage 2 — grow FName to the superset.** 12-byte `{comparison_index,
   display_index, number}`, drop `repr(C)`, manual `Eq`, `FName::NONE`; embedders
   now use the physical type from step 1; `UEnum::entries()` dynamic striding.
4. **Add a real non-4.27 row** (a CPN build) to `FNameLayout` + the relevant
   `*Layout` rows and validate against a dump.

---

## Tests (host-runnable — the first logic, not just offsets, that's unit-testable)

```rust
#[test]
fn ue_4_27_fname_layout() {
    assert_eq!(UeLayout::UE_4_27.fname,
        FNameLayout { number_off: 4, display_index_off: None, size: 8 });
}

#[test]
fn fname_roundtrip_all_layouts() {
    for lay in [
        FNameLayout::UE_4_27,
        FNameLayout { number_off: 8, display_index_off: Some(4), size: 12 }, // CPN <5.01
        FNameLayout { number_off: 4, display_index_off: Some(8), size: 12 }, // CPN >=5.01
    ] {
        let mut t = UeLayout::UE_4_27; t.fname = lay;
        unsafe { set_layout(Box::leak(Box::new(t))) };
        let n = FName { comparison_index: FNameEntryId { value: 0xABCD },
                        display_index:   FNameEntryId { value: 0xABCD }, number: 9 };
        let mut buf = [0u8; 16];
        unsafe { assert_eq!(read_fname(marshal_fname(&n, &mut buf) as *const u8), n); }
    }
    unsafe { set_layout(&DEFAULT_LAYOUT) };
}
```

## Verification gates (per established loop)

1. `cargo test -p libue --target x86_64-unknown-linux-gnu` (host) — fast logic.
2. `cargo xwin test -p libue --target x86_64-pc-windows-msvc --release` — real ABI.
3. `cargo xwin build -p py_repl --target x86_64-pc-windows-msvc --release` —
   catches the `bindings.rs` ripple (`entries()`, `class().name`); **only**
   type-checks under MSVC.

---

## Out of scope (note for later)

- `FNAME_OUTLINE_NUMBER` (4-byte FName, number stored separately): make
  `number_off: Option<usize>`. Rare console builds; skip unless needed.
- Pre-4.23 `comparison_index` is a raw int32 index (not a block/offset handle) and
  hashes differently — `GetTypeHash` would need a version branch. Targets are 4.23+.
- UE5.7+ `FNameData` SoA enum-name representation (jmap `objects.rs:351`) — only if
  targeting 5.7+; `entries()` branches on a UEnum-names-layout flag.
