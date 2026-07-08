# `FText` params & properties — scoping / design options

**Status: SCOPING.** Goal: get/set `FText` values so Python can read and write
text on widgets (`UTextBlock::Text`, `SetText(FText)`, etc.). This doc lays out
the candidate implementations, their trade-offs, and a recommendation, plus the
exact plug-in points. No code written yet.

---

## What `FText` is (and why it's not `FString`/`FName`)

`FText` is **not** a string — it's a handle to refcounted, culture-aware text
data:

```cpp
// UE4 (4.27), Text.h:766
class FText {
    TSharedRef<ITextData, ESPMode::ThreadSafe> TextData;  // 16 bytes
    uint32 Flags;                                          // +4 (+4 pad) → 24 total
};
```

- `TSharedRef<T, ThreadSafe>` = `{ ITextData* Object; FReferenceControllerBase* RefController; }`
  — offset 0 is the `ITextData*`, offset 8 is the **atomic** refcount controller.
- The real string lives behind `ITextData::GetDisplayString() -> const FString&`.

This makes `FText` categorically harder than the property types we already
handle:

| | `FStrProperty` | `FNameProperty` | `FTextProperty` |
|---|---|---|---|
| Value | `FString` (owned `TArray<u16>`) | `FName` (POD index) | `TSharedRef<ITextData>` (refcounted) |
| Write | `*slot = FString::from(s)` — Rust `Drop` frees old | `*slot = FName::from_string(s)` — POD, no drop | **can't** — atomic refcount inc/dec, no by-hand model |
| Read | copy the `TArray` | call `fname_to_string` | indirect through `ITextData` vtable / engine fn |

The crux: **libue cannot safely model `FText`'s atomic-refcounted lifetime by
hand.** `write_property` for `FStrProperty` just does `*slot = new` and lets
Rust `Drop` free the old buffer (`bindings.rs:2580`). For `FText` the old slot
holds a shared ref whose controller must be atomically decremented, and the new
value must be a properly-constructed owning ref. Both operations must go through
**engine semantics**, never a raw byte write.

Good news: those semantics already exist generically. `FProperty`'s vtable ops
— `initialize_value` / `destroy_value` / `copy_complete_value`
(`properties/mod.rs:129-224`) — are dispatched through the engine vtable and
already handle any property type correctly (FString allocates, structs recurse,
**FText inc/decs its refcount**). `FTextProperty` inherits them for free. So the
only thing we must supply is a way to **produce a source `FText` from a string**
and **extract a string from an `FText`**.

---

## One chokepoint covers both properties *and* widget calls

`read_property` / `write_property` in `bindings.rs` are the shared dispatch for
**both** property access **and** UFunction params/returns:

- property get/set → `read_at_path` / `write_property` (`bindings.rs:2139, 2580`)
- UFunction args + return → same `write_property` / `read_property`
  (`bindings.rs:3212`, param frame `3271`)

So adding an `FTextProperty` arm to that `if flags.contains(...)` chain
simultaneously enables:

1. Reading/writing `FTextProperty` fields (e.g. `UTextBlock.Text`).
2. Passing a Python `str` where a UFunction wants an `FText` param — i.e.
   **`text_block.SetText("hi")`** and reading **`text_block.GetText()`** — because
   the param marshaller runs the same dispatch on each `FProperty` param.

That's the whole feature: **one read arm + one write arm**, keyed on
`EClassCastFlags::CASTCLASS_FTextProperty` (flag already defined,
`lib.rs:1542`). Everything else is *how* those two arms obtain/emit the string.

---

## The two sub-problems

### A. READ — `FText` → `String`

Three ways to reach the display string:

**A1. Native `FText::ToString()` resolver.** `const FString& ToString() const`
(`Text.h:490`). Returns a ref to the display string, running `Rebuild()` first if
the text is stale. Authoritative and version-stable at the *signature* level.
- Cost: one new patternsleuth resolver (`FTextToString`).
- Sig: `unsafe extern "system" fn(*const FText) -> *const FString`, then copy.

**A2. `ITextData` vtable-walk.** Read `Object = *(text as *const *const u8)`
(offset 0), read its vtable, call the `GetDisplayString() -> const FString&`
slot. No resolver.
- ⚠️ **The slot index is genuinely version-variant.** In 4.27 `ITextData` has no
  base and `GetDisplayString` is ~slot 2. In 5.7 `ITextData : public
  IRefCountedObject` (prepends AddRef/Release/GetRefCount), **adds
  `GetSourceString` before `GetDisplayString`**, and drops
  `OwnsLocalizedString` — so the slot moves. This would need its own
  `layout().ftext.get_display_string_slot` entry, and skips `Rebuild()` (a
  runtime-generated text that hasn't been built could read empty). Fragile;
  trades a resolver for a per-version vtable table + a correctness caveat.

**A3. Kismet `Conv_TextToString` UFunction.** Call
`/Script/Engine.KismetTextLibrary:Conv_TextToString` through the UFunction
machinery we already have. Zero new native resolvers.
- Cost: a CDO lookup + a VM call per read (slow), and it's a `static` BP function
  so needs a valid target object. Heavier at runtime, lighter to build.

### B. WRITE — `String` → `FText`

**B1. Native `FText::AsCultureInvariant(FString&&)` (or `FromString`)
resolver.** Constructs a fresh `FText` (refcount 1). `AsCultureInvariant` is the
*right* constructor for user-set literal text — it won't try to localize the
string (matches BP "Make Literal Text"). `FromString` is equivalent for our
purposes.
- Cost: one new resolver (`FTextAsCultureInvariant` / `FTextFromString`).
- Sig: `unsafe extern "system" fn(out: *mut FText, s: *const FString)` (RVO —
  return-by-value struct is a hidden first pointer under Win64 ABI).
- Assignment: `destroy_value(slot)` then construct **directly into `slot`**, or
  construct into a temp and `copy_complete_value(slot, &temp)` then destroy the
  temp. Either way refcounts stay correct because the engine ops own them.

**B2. Kismet `Conv_StringToText` / `MakeLiteralText` UFunction.** Same VM-call
approach as A3; produces a culture-invariant `FText`. Zero resolvers.
- Cost: VM call per write; produces the value into a param slot we then move/copy
  into the property with the vtable ops.

---

## Version-variance risks to confirm

1. **`FText` internal storage may differ UE4 vs UE5.** UE4 is
   `TSharedRef<ITextData, ThreadSafe>` (16B: object + external controller). UE5's
   `ITextData : IRefCountedObject` strongly suggests the switch to an **intrusive
   `TRefCountPtr<ITextData>`** (8B, refcount on the object itself) — which would
   change `FText`'s size/layout. **Must confirm against a real 5.x dump.** This is
   the single biggest reason to prefer engine functions (A1/B1) over
   reaching into the layout by hand (A2): the functions abstract the storage
   change; a hand-walk does not.
2. `Flags` field offset/presence (post-`TextData`) — irrelevant if we only ever
   touch offset 0 or go through engine fns.
3. We never hardcode `FText` size — `element_size` comes from the `FProperty`, so
   striding in arrays/maps is already correct regardless of the above.

---

## Recommendation

**Engine-function path: A1 (`ToString`) + B1 (`AsCultureInvariant`), leaning on
the existing generic `FProperty` vtable ops for lifetime.**

Rationale:
- Correct on refcounting and `Rebuild()` for free — the two things a hand model
  gets wrong.
- Robust to the UE4→UE5 `TSharedRef`→`TRefCountPtr` storage change (the risk that
  most endangers the vtable-walk A2).
- Matches the codebase's established shape: `FName` already works exactly this way
  (`fname_to_string` + `fname_ctor_wchar` resolvers wired through `UeGlobals`).
  `FText` is the direct analogue — two engine fns, two boundary helpers.
- Cost is two patternsleuth resolvers, which is the "expensive" part but is
  one-time and pattern-matched like the existing FName/StaticLoadObject ones.

Use **A3/B2 (Kismet UFunctions) as the fallback** only if the `ToString` /
`AsCultureInvariant` signatures prove hard to pattern-match — they need zero
native resolvers and reuse the UFunction caller, at a runtime-cost we don't care
about for REPL-driven widget edits.

Avoid **A2 (vtable-walk)** as the primary: it needs a per-version slot table, a
`Rebuild()` caveat, and is the approach most exposed to the UE5 storage-model
change.

---

## Plug-in points (for whichever read/write pair we pick)

| Where | Change |
|---|---|
| `patternsleuth src/resolvers/unreal/` | New `FTextToString` + `FTextAsCultureInvariant` resolvers (A1/B1 only) |
| `py_repl/src/lib.rs:85` `PyReplResolution` | Add the two resolver fields; transmute into globals (`:120`) |
| `libue/src/lib.rs:91` `UeGlobals` | Add `ftext_to_string` / `ftext_from_string` fn-ptr fields + typedefs (near `FnFNameToString` `:43,547`) |
| `libue/src/lib.rs:2655` (near `FString`) | Define `FText` view type (opaque, offset-0 = `ITextData*`) + `Display`/`from_str` helpers mirroring `FName` (`:547`) |
| `libue/src/properties/mod.rs:296` (after `FNameProperty`) | `ue_struct! { FTextProperty: FProperty; cast CASTCLASS_FTextProperty; }` + a custom `PropTrait` (not the plain `value` clause — read/write go through engine fns, not `*slot`) |
| `py_repl/src/bindings.rs:2146` (read, after `FNameProperty`) | `if flags.contains(CASTCLASS_FTextProperty) { → String → pyobj }` |
| `py_repl/src/bindings.rs:2592` (write, after `FNameProperty`) | `if flags.contains(CASTCLASS_FTextProperty) { destroy_value(slot); construct into slot }` |

Note `CASTCLASS_FTextProperty` is already defined (`lib.rs:1542`); no flag work.

---

## Open questions (resolve before implementing)

1. Confirm UE5 `FText` storage (`TSharedRef` vs intrusive `TRefCountPtr`) against
   a 5.x dump — decides whether offset 0 is still the only field we touch (it is
   for A1/B1 regardless, but confirms A2 is correctly ruled out).
2. `AsCultureInvariant` vs `FromString` for writes — do we want set-text to be
   non-localizing (yes, for literal widget text) → `AsCultureInvariant`.
3. Should `read_property` return the display string only, or expose namespace/key
   (localization identity)? Widgets only need display; start there.
4. Reading an *empty/default* `FTextProperty` slot: the engine value-inits it to
   `FText::GetEmpty()` (a valid shared ref), so `ToString` is safe on a
   zero-constructed slot — verify `CPF_ZeroConstructor` is **not** set on
   `FTextProperty` (it shouldn't be; it has a non-trivial ctor).
