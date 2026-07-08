mod allocator;
mod array;
mod bitarray;
mod delegate;
mod layout;
mod malloc;
mod map;
mod properties;
mod script_array;
mod script_map;
mod script_set;
mod script_sparse_array;
mod set;
mod sparsearray;
mod uobject_hash;

pub use allocator::*;
pub use array::*;
pub use bitarray::*;
pub use delegate::*;
pub use layout::*;
pub use malloc::*;
pub use map::*;
pub use properties::*;
pub use script_array::*;
pub use script_map::*;
pub use script_set::*;
pub use script_sparse_array::*;
pub use set::*;
pub use sparsearray::*;
pub use uobject_hash::*;

use std::{
    cell::UnsafeCell,
    ffi::c_void,
    fmt::Display,
    ops::{Deref, DerefMut},
};
use windows::Win32::System::Threading::{
    CRITICAL_SECTION, EnterCriticalSection, LeaveCriticalSection,
};

pub type FnFNameCtorWchar =
    unsafe extern "system" fn(this: *mut FName, name: *const u16, find_type: i32);
pub type FnFUObjectHashTablesGet = extern "system" fn() -> &'static FUObjectHashTables;

/// `StaticLoadObject(UClass*, UObject*, TCHAR*, TCHAR*, u32, UPackageMap*, bool, FLinkerInstancingContext*)`.
/// Resolves a path (optionally loading the package from disk) and returns a
/// `UObject*`, or null on failure. This is the UE ≤5.4 signature, with
/// null-terminated `const TCHAR*` name/filename.
pub type FnStaticLoadObject = unsafe extern "system" fn(
    class: *const UClass,
    in_outer: *const UObject,
    name: *const u16,
    filename: *const u16,
    load_flags: u32,
    sandbox: *const c_void,
    allow_object_reconciliation: bool,
    instancing_context: *const c_void,
) -> *const UObjectBase;

/// UE 5.7 `StaticLoadObject` — identical but for `FStringView` name/filename.
/// Each 16-byte view is passed by hidden pointer under the Win64 ABI, so at the
/// register level the only difference from [`FnStaticLoadObject`] is that those
/// two arguments point at an [`FStringView`] instead of at raw wide chars.
type FnStaticLoadObjectView = unsafe extern "system" fn(
    class: *const UClass,
    in_outer: *const UObject,
    name: *const FStringView,
    filename: *const FStringView,
    load_flags: u32,
    sandbox: *const c_void,
    allow_object_reconciliation: bool,
    instancing_context: *const c_void,
) -> *const UObjectBase;

/// `FStringView` / `TStringView<TCHAR>` — a non-owning `{ Data, Size }` view.
/// `Size` is a character count; the buffer is not null-terminated.
#[repr(C)]
struct FStringView {
    data: *const u16,
    size: i32,
}

/// `StaticConstructObject_Internal(const FStaticConstructObjectParameters&)` —
/// the engine's UObject factory. Takes the params bundle by const reference
/// (i.e. a pointer under the Win64 ABI) and returns the new `UObject*`.
pub type FnStaticConstructObjectInternal =
    unsafe extern "system" fn(params: *const c_void) -> *const UObjectBase;

pub struct UeGlobals {
    pub gmalloc: *const *const FMalloc,
    pub fname_to_string: FnFNameToString,
    pub fname_ctor_wchar: FnFNameCtorWchar,
    pub guobject_array: *const FUObjectArray,
    pub fuobject_hash_tables_get: FnFUObjectHashTablesGet,
    pub static_load_object: FnStaticLoadObject,
    /// The engine's `StaticConstructObject_Internal` UObject factory.
    pub static_construct_object: FnStaticConstructObjectInternal,
    /// The Kismet VM opcode dispatch table (`GNatives`). Writable: instruction
    /// tracing swaps entries for a trace handler and restores them.
    pub gnatives: *mut GNatives,
    /// Engine memory layout (member offsets), fixed for the process lifetime.
    pub layout: UeLayout,
}
unsafe impl Sync for UeGlobals {}
unsafe impl Send for UeGlobals {}

static mut GLOBALS: Option<UeGlobals> = None;

/// Initialize libue globals. Must be called before using FName::Display or FWeakObjectPtr.
///
/// # Safety
/// All pointers in `globals` must be valid for the lifetime of the process.
pub unsafe fn init(globals: UeGlobals) {
    unsafe {
        GLOBALS = Some(globals);
    }
}

pub fn globals() -> &'static UeGlobals {
    #[allow(static_mut_refs)]
    unsafe {
        GLOBALS.as_ref().unwrap()
    }
}

pub fn hash_tables() -> &'static FUObjectHashTables {
    (globals().fuobject_hash_tables_get)()
}

/// Cached `UObject::StaticClass()` — the `UClass*` for the root UObject type,
/// which every call to `StaticLoadObject` needs as the search class.
static UOBJECT_STATIC_CLASS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn uobject_static_class() -> Option<*const UClass> {
    let cached = UOBJECT_STATIC_CLASS.load(std::sync::atomic::Ordering::Relaxed);
    if cached != 0 {
        return Some(cached as *const UClass);
    }
    let obj = hash_tables().find_object_by_path("/Script/CoreUObject.Object")?;
    let cls: *const UClass = obj.cast::<UClass>()? as *const UClass;
    UOBJECT_STATIC_CLASS.store(cls as usize, std::sync::atomic::Ordering::Relaxed);
    Some(cls)
}

/// Resolve a UE object path to a `UObjectBase`, loading the owning package
/// from disk on demand if needed. Equivalent to the engine's
/// `StaticLoadObject(UObject::StaticClass(), nullptr, path, …)` — accepts
/// fully-qualified paths like `/Game/Foo/Bar.Bar_C`. Returns `None` if the
/// object cannot be found or loaded.
pub fn static_load_object(path: &str) -> Option<&'static UObjectBase> {
    let class = uobject_static_class()?;
    let load = globals().static_load_object;

    let mut utf16: Vec<u16> = path.encode_utf16().collect();
    let ptr = unsafe {
        match globals().layout.string_arg_abi {
            StringArgAbi::NullTerminated => {
                // `const TCHAR*` — needs an explicit terminator.
                utf16.push(0);
                load(
                    class,
                    std::ptr::null(),
                    utf16.as_ptr(),
                    std::ptr::null(),
                    0, // LOAD_None
                    std::ptr::null(),
                    true,
                    std::ptr::null(),
                )
            }
            StringArgAbi::StringView => {
                // `FStringView` — counted, not null-terminated; passed by
                // pointer (Win64 ABI). `filename` is an empty/default view.
                let name = FStringView {
                    data: utf16.as_ptr(),
                    size: utf16.len() as i32,
                };
                let filename = FStringView {
                    data: std::ptr::null(),
                    size: 0,
                };
                let load: FnStaticLoadObjectView = std::mem::transmute(load);
                load(
                    class,
                    std::ptr::null(),
                    &raw const name,
                    &raw const filename,
                    0, // LOAD_None
                    std::ptr::null(),
                    true,
                    std::ptr::null(),
                )
            }
        }
    };
    unsafe { ptr.as_ref() }
}

/// Construct a new UObject instance via the engine's
/// `StaticConstructObject_Internal`. `class` is required; `outer` (null →
/// transient package), `name` (NAME_None → engine-assigned unique name),
/// `set_flags` (`EObjectFlags`) and `internal_set_flags` (`EInternalObjectFlags`)
/// are optional. Returns `None` if the engine returned null. The new object
/// is registered in `GUObjectArray` (and the object hash) but nothing
/// resolves or mutates it yet, so a game-thread caller doing follow-up setup
/// writes can assert exclusivity via the per-type `handle_mut` until it first
/// makes the object reachable (FuncMap insert, re-parent target, …).
///
/// `internal_set_flags` is applied by the engine during allocation. Passing
/// `EInternalObjectFlags::RootSet` here is the correct way to make a
/// freshly-constructed object a GC root on every engine version: on UE5.4+
/// `StaticAllocateObject` routes root bits through `FUObjectItem::SetRootFlags`,
/// which populates the `GRoots` `TSet` under its lock — something a plain
/// after-the-fact flag OR ([`UObjectBase::add_to_root`]) can't do without
/// resolving `SetRootFlags`. On pre-5.4 builds it just sets the flag, which is
/// all those builds need.
///
/// # Safety
/// `class` must be a valid `UClass*` and `outer`, if non-null, a valid
/// `UObject*` that is a legal outer for `class`.
pub unsafe fn static_construct_object(
    class: *const UClass,
    outer: *const UObjectBase,
    name: FName,
    set_flags: u32,
    internal_set_flags: u32,
) -> Option<&'static UObjectBase> {
    let construct = globals().static_construct_object;

    match globals().layout.construct_object_abi {
        // UE ≥4.26: single `const FStaticConstructObjectParameters&` argument.
        // The leading fields (Class@0x0, Outer@0x8, Name@0x10, SetFlags@0x18)
        // are stable across every supported engine version (verified 4.26–5.7);
        // every field past them defaults to zero/null/None/false, which a zeroed
        // buffer provides. Over-size it so the callee never reads past the buffer
        // regardless of the build's struct size.
        ConstructObjectAbi::ParamsStruct => unsafe {
            let mut params = [0u8; 0x100];
            let base = params.as_mut_ptr();
            (base.add(0x00) as *mut *const UClass).write(class);
            (base.add(0x08) as *mut *const UObjectBase).write(outer);
            (base.add(0x10) as *mut FName).write(name);
            (base.add(0x18) as *mut u32).write(set_flags);
            // `EInternalObjectFlags InternalSetFlags` sits at 0x1C, immediately
            // after the 4-byte `EObjectFlags SetFlags` — stable across the same
            // 4.26–5.7 range the leading fields are. Left 0 (None) by the zeroed
            // buffer unless the caller asks for e.g. RootSet.
            (base.add(0x1c) as *mut u32).write(internal_set_flags);
            construct(base as *const c_void).as_ref()
        },
        // UE ≤4.25: the params-struct overload doesn't exist; the factory takes
        // flat positional arguments. Calling it with the ParamsStruct ABI passes
        // the buffer pointer where `Class` is expected, so the callee builds an
        // object with a garbage class pointer and later crashes dereferencing it
        // as a vtable. The trailing args mirror the engine's own defaults.
        ConstructObjectAbi::LegacyArgs => unsafe {
            type FnLegacy = unsafe extern "system" fn(
                class: *const UClass,
                outer: *const UObjectBase,
                name: FName,
                set_flags: u32,
                internal_set_flags: u32,
                template: *const UObjectBase,
                copy_transients_from_class_defaults: bool,
                instance_graph: *const c_void,
                assume_template_is_archetype: bool,
            ) -> *const UObjectBase;
            let construct: FnLegacy = std::mem::transmute(construct);
            construct(
                class,
                outer,
                name,
                set_flags,
                0,                // EInternalObjectFlags::None
                std::ptr::null(), // Template
                false,            // bCopyTransientsFromClassDefaults
                std::ptr::null(), // InstanceGraph
                false,            // bAssumeTemplateIsArchetype
            )
            .as_ref()
        },
    }
}

// Size assertions to verify UE memory layout compatibility
const _: () = {
    // Basic types
    assert!(std::mem::size_of::<FSetElementId>() == 4);

    // Allocator sizes
    assert!(std::mem::size_of::<TSizedHeapAllocatorForAnyElementType<FSetElementId>>() == 8);
    assert!(std::mem::size_of::<TSizedHeapAllocatorForAnyElementType<u32>>() == 8);
    assert!(std::mem::size_of::<TInlineAllocatorForElementType<FSetElementId, 1>>() == 16);
    assert!(std::mem::size_of::<TInlineAllocatorForElementType<u32, 4>>() == 24);

    // TArray sizes
    assert!(std::mem::size_of::<TArray<u32, TSizedHeapAllocator32>>() == 16);
    assert!(
        std::mem::size_of::<
            TArray<
                TSparseArrayElementOrFreeListLink<TSetElement<TTuple<u32, u32>>>,
                TSizedHeapAllocator32,
            >,
        >() == 16
    );
    assert!(
        std::mem::size_of::<
            TArray<
                TSparseArrayElementOrFreeListLink<TSetElement<TTuple<*mut UObject, *mut UObject>>>,
                TSizedHeapAllocator32,
            >,
        >() == 16
    );

    // TBitArray size
    assert!(std::mem::size_of::<TBitArray<TInlineAllocator<4>>>() == 32);

    // TSparseArray sizes
    assert!(
        std::mem::size_of::<TSparseArray<TSetElement<TTuple<u32, u32>>, TSparseArrayAllocator32>>()
            == 56
    );
    assert!(
        std::mem::size_of::<
            TSparseArray<TSetElement<TTuple<*mut UObject, *mut UObject>>, TSparseArrayAllocator32>,
        >() == 56
    );

    // TSet sizes
    assert!(std::mem::size_of::<TSet<TTuple<u32, u32>>>() == 80);
    assert!(std::mem::size_of::<TSet<TTuple<*mut UObject, *mut UObject>>>() == 80);
    assert!(std::mem::size_of::<TSet<u32, DefaultKeyFuncs<u32>, FDefaultSetAllocator>>() == 80);
};

macro_rules! impl_deref {
    ($class:ty, $member:ident: $parent:ty) => {
        impl std::ops::Deref for $class {
            type Target = $parent;
            fn deref(&self) -> &Self::Target {
                &self.$member
            }
        }
        impl std::ops::DerefMut for $class {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.$member
            }
        }
    };
    ($class:ty => $parent:ty) => {
        impl std::ops::Deref for $class {
            type Target = $parent;
            fn deref(&self) -> &Self::Target {
                unsafe { &*(self as *const Self as *const Self::Target) }
            }
        }
        impl std::ops::DerefMut for $class {
            fn deref_mut(&mut self) -> &mut Self::Target {
                unsafe { &mut *(self as *mut Self as *mut Self::Target) }
            }
        }
    };
}
pub(crate) use impl_deref;

/// A zero-sized view over live engine memory whose members are read at
/// version-dependent byte offsets (see [`layout`](crate::layout)). Supplies the
/// raw offset read/write primitives shared by every layout-dynamic struct; the
/// [`ue_struct!`] macro implements it and builds typed accessors on top, and the
/// few structs whose access can't be macro-expressed (atomics, variant
/// decoding) implement it to reuse the same primitives instead of re-deriving
/// `base`/`read`.
pub(crate) trait LayoutView: Sized {
    /// Address of this view = the object's base address (these are ZSTs at the
    /// start of the engine object).
    #[inline]
    fn base(&self) -> *const u8 {
        self as *const Self as *const u8
    }
    /// Read a member by value at byte offset `off`.
    ///
    /// # Safety
    /// `off` must be in bounds and hold a valid `T`.
    #[inline]
    unsafe fn read<T>(&self, off: usize) -> T {
        unsafe { (self.base().add(off) as *const T).read() }
    }
    /// Borrow a member in place at byte offset `off`.
    ///
    /// # Safety
    /// See [`read`](LayoutView::read).
    #[inline]
    unsafe fn read_ref<T>(&self, off: usize) -> &T {
        unsafe { &*(self.base().add(off) as *const T) }
    }
    /// Mutably borrow a member in place at byte offset `off`. The `&mut`
    /// receiver is the exclusivity proof (asserted via the per-type
    /// `handle_mut`), matching [`place_mut`](LayoutView::place_mut).
    ///
    /// # Safety
    /// See [`read`](LayoutView::read).
    #[inline]
    unsafe fn read_ref_mut<T>(&mut self, off: usize) -> &mut T {
        unsafe { &mut *((self as *mut Self as *mut u8).add(off) as *mut T) }
    }
    /// Project a member as a [`Racy`] in-place-atomic [`Place`].
    ///
    /// # Safety
    /// `off` must be in bounds and hold a valid `T`, aligned for `T`-sized
    /// atomic access.
    #[inline]
    unsafe fn place<T>(&self, off: usize) -> Place<'_, T, Racy> {
        Place {
            ptr: self.base().wrapping_add(off).cast_mut().cast(),
            _ph: std::marker::PhantomData,
        }
    }
    /// Project a member as an [`Excl`] plain read/write [`Place`]. The `&mut`
    /// receiver is the exclusivity proof (asserted via the per-type
    /// `handle_mut`), and the place's lifetime keeps the handle mutably
    /// borrowed while it lives.
    ///
    /// # Safety
    /// `off` must be in bounds and hold a valid `T`.
    #[inline]
    unsafe fn place_mut<T>(&mut self, off: usize) -> Place<'_, T, Excl> {
        Place {
            ptr: self.base().wrapping_add(off).cast_mut().cast(),
            _ph: std::marker::PhantomData,
        }
    }
}

/// Exclusive-access marker for [`Place`]: plain reads and writes, sound
/// because the projecting handle's `&mut` pedigree (an explicit `handle_mut`
/// assertion) guarantees nothing else touches the field.
pub enum Excl {}

/// Concurrent-access marker for [`Place`]: the engine reads or writes the
/// field from arbitrary threads, so on the Rust side the only defined access
/// is in-place atomic — plain (or volatile) accesses racing anything are UB in
/// the abstract machine. The engine's own accesses stay plain loads (that half
/// is out of our hands), but atomics make our half sound and provide the
/// release/acquire edges publication protocols rely on.
pub enum Racy {}

/// A typed place (field lens) inside a live engine object, projected at a
/// layout-dependent byte offset from a zero-sized [`ue_struct!`] view.
///
/// The pointer carries provenance from the object's base pointer and no
/// reference-level assertion is ever made about the field bytes; the access
/// regime `A` ([`Excl`] or [`Racy`]) decides which operations exist.
pub struct Place<'o, T, A> {
    ptr: *mut T,
    _ph: std::marker::PhantomData<(&'o (), fn() -> A)>,
}
impl<T, A> Clone for Place<'_, T, A> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T, A> Copy for Place<'_, T, A> {}

impl<T: Copy> Place<'_, T, Excl> {
    pub fn get(self) -> T {
        unsafe { self.ptr.read() }
    }
    pub fn set(self, v: T) {
        unsafe { self.ptr.write(v) }
    }
}

impl<T: AtomicBits> Place<'_, T, Racy> {
    /// Acquire load — pairs with [`store`](Self::store)'s Release, so a thread
    /// that reads a just-published value here also sees everything written
    /// before the store (hook stub bytes, header fields, …).
    pub fn load(self) -> T {
        unsafe { T::atomic_load(self.ptr) }
    }
    /// Release store.
    ///
    /// # Safety
    /// Engine threads may observe the old or the new value in any interleaving
    /// around the store, and `v` must satisfy the field's semantic contract
    /// (see the field's accessor docs) — the engine acts on what it reads.
    pub unsafe fn store(self, v: T) {
        unsafe { T::atomic_store(self.ptr, v) }
    }
}

/// Bit-level round-trip through an in-place atomic of the same size, for
/// [`Racy`] places.
///
/// # Safety
/// Implementations must access memory through an atomic type of `Self`'s exact
/// size, and every bit pattern they can load must be a valid `Self`.
pub unsafe trait AtomicBits: Copy {
    /// # Safety
    /// `p` must be valid for reads and aligned for the backing atomic type.
    unsafe fn atomic_load(p: *const Self) -> Self;
    /// # Safety
    /// `p` must be valid for writes and aligned for the backing atomic type.
    unsafe fn atomic_store(p: *mut Self, v: Self);
}

macro_rules! atomic_bits {
    ($ty:ty, $atom:ty, $to:expr, $from:expr) => {
        unsafe impl AtomicBits for $ty {
            unsafe fn atomic_load(p: *const Self) -> Self {
                const _: () = assert!(std::mem::size_of::<$ty>() == std::mem::size_of::<$atom>());
                ($from)(unsafe { &*(p as *const $atom) }.load(std::sync::atomic::Ordering::Acquire))
            }
            unsafe fn atomic_store(p: *mut Self, v: Self) {
                unsafe { &*(p as *const $atom) }
                    .store(($to)(v), std::sync::atomic::Ordering::Release)
            }
        }
    };
}
atomic_bits!(u32, std::sync::atomic::AtomicU32, |v| v, |v| v);
atomic_bits!(u64, std::sync::atomic::AtomicU64, |v| v, |v| v);
atomic_bits!(usize, std::sync::atomic::AtomicUsize, |v| v, |v| v);
atomic_bits!(
    EFunctionFlags,
    std::sync::atomic::AtomicU32,
    |v: EFunctionFlags| v.bits(),
    EFunctionFlags::from_bits_retain
);
unsafe impl<T> AtomicBits for *const T {
    unsafe fn atomic_load(p: *const Self) -> Self {
        unsafe { &*(p as *const std::sync::atomic::AtomicUsize) }
            .load(std::sync::atomic::Ordering::Acquire) as Self
    }
    unsafe fn atomic_store(p: *mut Self, v: Self) {
        unsafe { &*(p as *const std::sync::atomic::AtomicUsize) }
            .store(v as usize, std::sync::atomic::Ordering::Release)
    }
}

macro_rules! ue_struct {
    // Derived type
    (
        $(#[$meta:meta])*
        struct $name:ident: $parent:ty;
        $($body:tt)*
    ) => {
        $(#[$meta])*
        #[derive(Debug)]
        #[repr(C)]
        pub struct $name {
            _opaque: [u8; 0],
        }
        impl_deref!($name => $parent);
        impl $crate::LayoutView for $name {}
        ue_struct!(@handle_mut $name);
        ue_struct!(@body $name; $($body)*);
    };

    // Root type
    (
        $(#[$meta:meta])*
        struct $name:ident;
        $($body:tt)*
    ) => {
        $(#[$meta])*
        #[derive(Debug)]
        #[repr(C)]
        pub struct $name {
            _opaque: [u8; 0],
        }
        impl $crate::LayoutView for $name {}
        ue_struct!(@handle_mut $name);
        ue_struct!(@body $name; $($body)*);
    };

    (@handle_mut $name:ident) => {
        impl $name {
            /// Assert exclusive access to this live object and mint the `&mut`
            /// handle that the `*_mut` field places and other `&mut self` APIs
            /// require.
            ///
            /// # Safety
            /// For the whole borrow the caller must guarantee **exclusive**
            /// access to every field touched through it: game thread only, no
            /// other live Rust borrow of those fields, and no concurrent
            /// engine access. (`Self` is a zero-sized view, so this is a
            /// raw-pointer round-trip, not a `&`→`&mut` cast.) The two
            /// established uses: setup writes on a freshly-constructed object
            /// nothing else can reach yet, and game-thread-only FuncMap
            /// mutation on a private class.
            #[allow(clippy::mut_from_ref)]
            pub unsafe fn handle_mut(&self) -> &mut Self {
                unsafe { &mut *($crate::LayoutView::base(self) as *mut Self) }
            }
        }
    };

    // ── body clauses ──────────────────────────────────────────────────────
    // Clauses are consumed one at a time. `cast`/`value` may appear in any
    // order; a `layout`/`extern_layout` clause (which carries the field list)
    // must come last.

    (@body $name:ident;) => {};

    // `ObjTrait`/`FieldTrait` cast-flags impl (the discriminator behind `cast`).
    (@body $name:ident; cast $tr:path = $flags:expr; $($rest:tt)*) => {
        unsafe impl $tr for $name {
            const CAST_FLAGS: EClassCastFlags = $flags;
        }
        ue_struct!(@body $name; $($rest)*);
    };

    // Standard by-value `PropTrait` — the `FPropertyData<T>` read/write wrapper
    // for a plain-scalar property value. Only valid where `PropTrait` is in
    // scope (the `properties` module).
    (@body $name:ident; value $value:ty; $($rest:tt)*) => {
        impl PropTrait for $name {
            type PropValue<'o> = FPropertyData<'o, $value>;
            type PropValueMut<'o> = FPropertyDataMut<'o, $value>;
            unsafe fn value<'o>(&'o self, data: *const ()) -> Self::PropValue<'o> {
                unsafe { FPropertyData(&*data.cast()) }
            }
            unsafe fn value_mut<'o>(&'o self, data: *mut ()) -> Self::PropValueMut<'o> {
                unsafe { FPropertyDataMut(&mut *data.cast()) }
            }
        }
        ue_struct!(@body $name; $($rest)*);
    };

    // Body with a macro-generated layout struct (all-`usize` offsets).
    (@body $name:ident;
        layout $tbl:ident: $layout_ty:ident;
        $( $(#[$fmeta:meta])* $kind:ident $fname:ident $(as $acc:ident)? : $fty:ty ),* $(,)?
    ) => {
        #[doc = concat!("Byte offsets of the `", stringify!($name), "` members libue reads.")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $layout_ty {
            $( pub $fname: usize, )*
        }

        impl $name {
            $( ue_struct!(@accessor $tbl, $(#[$fmeta])* $kind $fname $(as $acc)? : $fty); )*
        }
    };

    // Body with an externally-defined layout struct — for structs whose layout
    // carries more than plain offsets (format discriminants, extra fields) and
    // so is hand-written in `layout.rs`. Only the accessors are generated.
    (@body $name:ident;
        extern_layout $tbl:ident: $layout_ty:ty;
        $( $(#[$fmeta:meta])* $kind:ident $fname:ident $(as $acc:ident)? : $fty:ty ),* $(,)?
    ) => {
        impl $name {
            $( ue_struct!(@accessor $tbl, $(#[$fmeta])* $kind $fname $(as $acc)? : $fty); )*
        }
    };

    // ── accessor kinds ────────────────────────────────────────────────────

    // Read a member by value.
    (@accessor $tbl:ident, $(#[$fmeta:meta])* scalar $fname:ident : $fty:ty) => {
        $(#[$fmeta])*
        pub fn $fname(&self) -> $fty {
            unsafe { $crate::LayoutView::read::<$fty>(self, globals().layout.$tbl.$fname) }
        }
    };
    (@accessor $tbl:ident, $(#[$fmeta:meta])* scalar $fname:ident as $acc:ident : $fty:ty) => {
        $(#[$fmeta])*
        pub fn $acc(&self) -> $fty {
            unsafe { $crate::LayoutView::read::<$fty>(self, globals().layout.$tbl.$fname) }
        }
    };
    // Read a member by value, plus a `<field>_mut` exclusive-write `Place`
    // behind `&mut` (mint the `&mut` via `handle_mut`).
    (@accessor $tbl:ident, $(#[$fmeta:meta])* excl $fname:ident : $fty:ty) => {
        $(#[$fmeta])*
        pub fn $fname(&self) -> $fty {
            unsafe { $crate::LayoutView::read::<$fty>(self, globals().layout.$tbl.$fname) }
        }
        paste::paste! {
            $(#[$fmeta])*
            pub fn [<$fname _mut>](&mut self) -> $crate::Place<'_, $fty, $crate::Excl> {
                unsafe { $crate::LayoutView::place_mut::<$fty>(self, globals().layout.$tbl.$fname) }
            }
        }
    };
    (@accessor $tbl:ident, $(#[$fmeta:meta])* excl $fname:ident as $acc:ident : $fty:ty) => {
        $(#[$fmeta])*
        pub fn $acc(&self) -> $fty {
            unsafe { $crate::LayoutView::read::<$fty>(self, globals().layout.$tbl.$fname) }
        }
        paste::paste! {
            $(#[$fmeta])*
            pub fn [<$acc _mut>](&mut self) -> $crate::Place<'_, $fty, $crate::Excl> {
                unsafe { $crate::LayoutView::place_mut::<$fty>(self, globals().layout.$tbl.$fname) }
            }
        }
    };
    // In-place-atomic member: Acquire-load getter plus a `<field>_atomic`
    // `Racy` `Place` whose Release `store` is the only writer.
    (@accessor $tbl:ident, $(#[$fmeta:meta])* racy $fname:ident : $fty:ty) => {
        paste::paste! {
            $(#[$fmeta])*
            pub fn $fname(&self) -> $fty {
                self.[<$fname _atomic>]().load()
            }
            $(#[$fmeta])*
            pub fn [<$fname _atomic>](&self) -> $crate::Place<'_, $fty, $crate::Racy> {
                unsafe { $crate::LayoutView::place::<$fty>(self, globals().layout.$tbl.$fname) }
            }
        }
    };
    (@accessor $tbl:ident, $(#[$fmeta:meta])* racy $fname:ident as $acc:ident : $fty:ty) => {
        paste::paste! {
            $(#[$fmeta])*
            pub fn $acc(&self) -> $fty {
                self.[<$acc _atomic>]().load()
            }
            $(#[$fmeta])*
            pub fn [<$acc _atomic>](&self) -> $crate::Place<'_, $fty, $crate::Racy> {
                unsafe { $crate::LayoutView::place::<$fty>(self, globals().layout.$tbl.$fname) }
            }
        }
    };
    // Borrow a member in place.
    (@accessor $tbl:ident, $(#[$fmeta:meta])* view $fname:ident : $fty:ty) => {
        $(#[$fmeta])*
        pub fn $fname(&self) -> &$fty {
            unsafe { $crate::LayoutView::read_ref::<$fty>(self, globals().layout.$tbl.$fname) }
        }
    };
    (@accessor $tbl:ident, $(#[$fmeta:meta])* view $fname:ident as $acc:ident : $fty:ty) => {
        $(#[$fmeta])*
        pub fn $acc(&self) -> &$fty {
            unsafe { $crate::LayoutView::read_ref::<$fty>(self, globals().layout.$tbl.$fname) }
        }
    };
    // Borrow a member in place, plus a `<acc>_mut` exclusive `&mut` behind
    // `&mut` (mint the `&mut` via `handle_mut`). For non-`Copy` members that a
    // plain-old `excl` place can't hand back.
    (@accessor $tbl:ident, $(#[$fmeta:meta])* view_mut $fname:ident as $acc:ident : $fty:ty) => {
        $(#[$fmeta])*
        pub fn $acc(&self) -> &$fty {
            unsafe { $crate::LayoutView::read_ref::<$fty>(self, globals().layout.$tbl.$fname) }
        }
        paste::paste! {
            $(#[$fmeta])*
            pub fn [<$acc _mut>](&mut self) -> &mut $fty {
                unsafe { $crate::LayoutView::read_ref_mut::<$fty>(self, globals().layout.$tbl.$fname) }
            }
        }
    };
    // Pointer member → `Option<&T>` (null becomes `None`).
    (@accessor $tbl:ident, $(#[$fmeta:meta])* ptr $fname:ident : $fty:ty) => {
        $(#[$fmeta])*
        pub fn $fname(&self) -> Option<&$fty> {
            unsafe {
                $crate::LayoutView::read::<*const $fty>(self, globals().layout.$tbl.$fname).as_ref()
            }
        }
    };
    (@accessor $tbl:ident, $(#[$fmeta:meta])* ptr $fname:ident as $acc:ident : $fty:ty) => {
        $(#[$fmeta])*
        pub fn $acc(&self) -> Option<&$fty> {
            unsafe {
                $crate::LayoutView::read::<*const $fty>(self, globals().layout.$tbl.$fname).as_ref()
            }
        }
    };
    // Pointer member that must be non-null → `&T` (panics on null).
    (@accessor $tbl:ident, $(#[$fmeta:meta])* ptr_req $fname:ident : $fty:ty) => {
        $(#[$fmeta])*
        pub fn $fname(&self) -> &$fty {
            unsafe {
                $crate::LayoutView::read::<*const $fty>(self, globals().layout.$tbl.$fname)
                    .as_ref()
                    .expect(concat!(stringify!($fname), " is null"))
            }
        }
    };
    (@accessor $tbl:ident, $(#[$fmeta:meta])* ptr_req $fname:ident as $acc:ident : $fty:ty) => {
        $(#[$fmeta])*
        pub fn $acc(&self) -> &$fty {
            unsafe {
                $crate::LayoutView::read::<*const $fty>(self, globals().layout.$tbl.$fname)
                    .as_ref()
                    .expect(concat!(stringify!($fname), " is null"))
            }
        }
    };
}
pub(crate) use ue_struct;

pub type GNatives = [Option<FNativesFuncPtr>; 0xff];
pub type FNativesFuncPtr =
    unsafe extern "system" fn(*mut UObject, stack: &mut kismet::FFrame, result: *mut c_void);

pub type FnFNameToString = unsafe extern "system" fn(&FName, &mut FString);
impl Display for FName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut string = FString::new();
        unsafe {
            (globals().fname_to_string)(self, &mut string);
        };
        write!(f, "{string}")
    }
}

pub type FnUObjectBaseUtilityGetPathName =
    unsafe extern "system" fn(&UObjectBase, Option<&UObject>, &mut FString);
impl UObjectBase {
    // pub fn get_path_name(&self, stop_outer: Option<&UObject>) -> String {
    //     let mut string = FString::new();
    //     unsafe {
    //         (globals().uobject_base_utility_get_path_name())(self, stop_outer, &mut string);
    //     }
    //     string.to_string()
    // }
}

#[derive(Debug)]
#[repr(C)]
pub struct FMalloc {
    vtable: *const FMallocVTable,
}
unsafe impl Sync for FMalloc {}
unsafe impl Send for FMalloc {}
impl FMalloc {
    /// Read the vtable entry at `slot` (in pointer-sized units). The slot of
    /// each allocator method shifts across engine versions (UE5's `FExec` added
    /// virtuals ahead of `Malloc`), so the indices come from the active layout.
    #[inline]
    fn vtable_fn(&self, slot: usize) -> *const c_void {
        unsafe { *(self.vtable as *const *const c_void).add(slot) }
    }
    pub fn malloc(&self, count: usize, alignment: u32) -> *mut c_void {
        let malloc: unsafe extern "system" fn(&FMalloc, usize, u32) -> *mut c_void =
            unsafe { std::mem::transmute(self.vtable_fn(globals().layout.fmalloc.malloc)) };
        unsafe { malloc(self, count, alignment) }
    }
    /// # Safety
    /// `original` must be null or a pointer previously returned by this allocator.
    pub unsafe fn realloc(
        &self,
        original: *mut c_void,
        count: usize,
        alignment: u32,
    ) -> *mut c_void {
        let realloc: unsafe extern "system" fn(&FMalloc, *mut c_void, usize, u32) -> *mut c_void =
            unsafe { std::mem::transmute(self.vtable_fn(globals().layout.fmalloc.realloc)) };
        unsafe { realloc(self, original, count, alignment) }
    }
    /// # Safety
    /// `original` must be null or a pointer previously returned by this allocator.
    pub unsafe fn free(&self, original: *mut c_void) {
        let free: unsafe extern "system" fn(&FMalloc, *mut c_void) =
            unsafe { std::mem::transmute(self.vtable_fn(globals().layout.fmalloc.free)) };
        unsafe { free(self, original) }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct FMallocVTable {
    pub __vec_del_dtor: *const (),
    pub exec: *const (),
    pub malloc:
        unsafe extern "system" fn(this: &FMalloc, count: usize, alignment: u32) -> *mut c_void,
    pub try_malloc:
        unsafe extern "system" fn(this: &FMalloc, count: usize, alignment: u32) -> *mut c_void,
    pub realloc: unsafe extern "system" fn(
        this: &FMalloc,
        original: *mut c_void,
        count: usize,
        alignment: u32,
    ) -> *mut c_void,
    pub try_realloc: unsafe extern "system" fn(
        this: &FMalloc,
        original: *mut c_void,
        count: usize,
        alignment: u32,
    ) -> *mut c_void,
    pub free: unsafe extern "system" fn(this: &FMalloc, original: *mut c_void),
    pub quantize_size: *const (),
    pub get_allocation_size: *const (),
    pub trim: *const (),
    pub setup_tls_caches_on_current_thread: *const (),
    pub clear_and_disable_tlscaches_on_current_thread: *const (),
    pub initialize_stats_metadata: *const (),
    pub update_stats: *const (),
    pub get_allocator_stats: *const (),
    pub dump_allocator_stats: *const (),
    pub is_internally_thread_safe: *const (),
    pub validate_heap: *const (),
    pub get_descriptive_name: *const (),
}
unsafe impl Sync for FMallocVTable {}
unsafe impl Send for FMallocVTable {}

#[derive(Debug)]
#[repr(C)]
pub struct FWindowsCriticalSection(UnsafeCell<CRITICAL_SECTION>);
impl FWindowsCriticalSection {
    fn crit_ptr_mut(&self) -> *mut CRITICAL_SECTION {
        // `UnsafeCell::get` is the provenance-correct way to obtain a `*mut` from a
        // shared borrow — no `&`-through-`*mut` laundering.
        self.0.get()
    }
    unsafe fn lock(&self) {
        unsafe {
            EnterCriticalSection(self.crit_ptr_mut());
        }
    }
    unsafe fn unlock(&self) {
        unsafe {
            LeaveCriticalSection(self.crit_ptr_mut());
        }
    }
}

pub struct CriticalSectionGuard<'crit, 'data, T: ?Sized + 'data> {
    critical_section: &'crit FWindowsCriticalSection,
    data: &'data UnsafeCell<T>,
}
impl<'crit, 'data, T: ?Sized> CriticalSectionGuard<'crit, 'data, T> {
    fn lock(critical_section: &'crit FWindowsCriticalSection, data: &'data UnsafeCell<T>) -> Self {
        unsafe {
            critical_section.lock();
        }
        Self {
            critical_section,
            data,
        }
    }
}
impl<T: ?Sized> Drop for CriticalSectionGuard<'_, '_, T> {
    fn drop(&mut self) {
        unsafe { self.critical_section.unlock() }
    }
}
impl<T: ?Sized> Deref for CriticalSectionGuard<'_, '_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.data.get() }
    }
}
impl<T: ?Sized> DerefMut for CriticalSectionGuard<'_, '_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.data.get() }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct FUObjectCreateListener;

#[derive(Debug)]
#[repr(C)]
pub struct FUObjectDeleteListener;

type ObjectIndex = i32;

#[derive(Debug)]
#[repr(C)]
pub struct FUObjectArray {
    obj_first_gcindex: i32,
    obj_last_non_gcindex: i32,
    max_objects_not_considered_by_gc: i32,
    open_for_disregard_for_gc: bool,

    obj_objects: UnsafeCell<FChunkedFixedUObjectArray>,
    obj_objects_critical: FWindowsCriticalSection,
    obj_available_list: TArray<i32>,
    uobject_create_listeners: TArray<*const FUObjectCreateListener>,
    uobject_delete_listeners: TArray<*const FUObjectDeleteListener>,
    uobject_delete_listeners_critical: FWindowsCriticalSection,
    master_serial_number: std::sync::atomic::AtomicI32,
}
const _: () = assert!(std::mem::offset_of!(FUObjectArray, master_serial_number) == 0x0b0);
impl FUObjectArray {
    pub fn objects(&self) -> CriticalSectionGuard<'_, '_, FChunkedFixedUObjectArray> {
        CriticalSectionGuard::lock(&self.obj_objects_critical, &self.obj_objects)
    }
    /// Resolve an object by its internal index (`GUObjectArray` slot), the form
    /// UE5 stores in the object hash tables. Returns `None` for an out-of-range
    /// index or an empty slot.
    pub fn index_to_object(&self, index: ObjectIndex) -> Option<&UObjectBase> {
        if index < 0 {
            return None;
        }
        let objects = unsafe { &*self.obj_objects.get() };
        if index >= objects.num_elements {
            return None;
        }
        unsafe { objects.item(index).object().as_ref() }
    }
    pub fn allocate_serial_number(&self, index: ObjectIndex) -> i32 {
        use std::sync::atomic::Ordering;

        let objects = unsafe { &*self.obj_objects.get() };
        let serial = objects.item(index).serial_atomic();

        let current = serial.load(Ordering::SeqCst);
        if current != 0 {
            current
        } else {
            let new = self.master_serial_number.fetch_add(1, Ordering::SeqCst);

            match serial.compare_exchange(0, new, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => new,
                Err(old) => old,
            }
        }
    }
}

pub struct ObjectIterator<'a> {
    array: &'a FChunkedFixedUObjectArray,
    index: i32,
}
impl<'a> Iterator for ObjectIterator<'a> {
    type Item = Option<&'a UObjectBase>;
    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = self.array.num_elements as usize;
        (size, Some(size))
    }
    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        let n = n as i32;
        if self.index < n {
            self.index = n;
        }
        self.next()
    }
    fn next(&mut self) -> Option<Option<&'a UObjectBase>> {
        if self.index >= self.array.num_elements {
            None
        } else {
            let obj = unsafe { self.array.item(self.index).object().as_ref() };

            self.index += 1;
            Some(obj)
        }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct FChunkedFixedUObjectArray {
    pub objects: *const *const FUObjectItem,
    pub pre_allocated_objects: *const FUObjectItem,
    pub max_elements: i32,
    pub num_elements: i32,
    pub max_chunks: i32,
    pub num_chunks: i32,
}
impl FChunkedFixedUObjectArray {
    pub fn iter(&self) -> ObjectIterator<'_> {
        ObjectIterator {
            array: self,
            index: 0,
        }
    }
    fn item_ptr(&self, index: ObjectIndex) -> *const FUObjectItem {
        let per_chunk = self.max_elements / self.max_chunks;
        let stride = globals().layout.fuobjectitem.stride;

        unsafe {
            let chunk = *self.objects.add((index / per_chunk) as usize) as *const u8;
            chunk.add((index % per_chunk) as usize * stride) as *const FUObjectItem
        }
    }
    fn item(&self, index: ObjectIndex) -> &FUObjectItem {
        unsafe { &*self.item_ptr(index) }
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct EInternalObjectFlags: i32 {
        const None = 0;
        const ReachableInCluster = 1 << 23;
        const ClusterRoot = 1 << 24;
        const Native = 1 << 25;
        const Async = 1 << 26;
        const AsyncLoading = 1 << 27;
        const Unreachable = 1 << 28;
        const PendingKill = 1 << 29;
        const RootSet = 1 << 30;
        const PendingConstruction = 1 << 31;
    }
}

/// `FUObjectItem` — one entry in the global object array. A zero-sized,
/// layout-dynamic view: its members (and even the encoding of the flags word)
/// shift across engine builds — notably UE 5.7 prepended an 8-byte
/// `FlagsAndRefCount`, moving `Object` to 0x8 — so every field is read through
/// [`FUObjectItemLayout`] rather than a fixed struct position.
#[repr(C)]
pub struct FUObjectItem {
    _opaque: [u8; 0],
}
// Its accessors are atomic / flag-format specific rather than plain offset
// reads, so it isn't a `ue_struct!`, but it reuses the shared `base()` primitive.
impl LayoutView for FUObjectItem {}
impl std::fmt::Debug for FUObjectItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FUObjectItem")
            .field("object", &self.object())
            .field("serial_number", &self.get_serial_number())
            .finish()
    }
}
impl FUObjectItem {
    #[inline]
    fn layout() -> &'static FUObjectItemLayout {
        &globals().layout.fuobjectitem
    }

    /// The object pointer for this slot (null for an empty slot).
    pub fn object(&self) -> *const UObjectBase {
        unsafe { (self.base().add(Self::layout().object) as *const *const UObjectBase).read() }
    }

    #[inline]
    fn serial_atomic(&self) -> &std::sync::atomic::AtomicI32 {
        unsafe {
            &*(self.base().add(Self::layout().serial_number) as *const std::sync::atomic::AtomicI32)
        }
    }
    pub fn get_serial_number(&self) -> i32 {
        self.serial_atomic()
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Read the raw `EInternalObjectFlags` bits, decoding the per-version
    /// storage (a standalone `int32`, or the high half of `FlagsAndRefCount`).
    fn raw_flags(&self) -> i32 {
        use std::sync::atomic::{AtomicI32, AtomicI64, Ordering::Relaxed};
        let l = Self::layout();
        let ptr = unsafe { self.base().add(l.flags) };
        match l.flags_format {
            FUObjectItemFlagsFormat::Int32 => unsafe { &*(ptr as *const AtomicI32) }.load(Relaxed),
            FUObjectItemFlagsFormat::HighHalfOfI64 => {
                (unsafe { &*(ptr as *const AtomicI64) }.load(Relaxed) >> 32) as i32
            }
        }
    }
    pub fn get_flags(&self) -> EInternalObjectFlags {
        EInternalObjectFlags::from_bits_truncate(self.raw_flags())
    }
    /// Atomically OR `flags` into this object's internal flag bits, honoring the
    /// per-version storage of the flags word.
    pub fn set_flags(&self, flags: EInternalObjectFlags) {
        use std::sync::atomic::{AtomicI32, AtomicI64, Ordering::Relaxed};
        let l = Self::layout();
        let ptr = unsafe { self.base().add(l.flags) };
        match l.flags_format {
            FUObjectItemFlagsFormat::Int32 => {
                unsafe { &*(ptr as *const AtomicI32) }.fetch_or(flags.bits(), Relaxed);
            }
            FUObjectItemFlagsFormat::HighHalfOfI64 => {
                let mask = ((flags.bits() as u32 as u64) << 32) as i64;
                unsafe { &*(ptr as *const AtomicI64) }.fetch_or(mask, Relaxed);
            }
        }
    }
    /// Atomically clear `flags` from this object's internal flag bits, honoring
    /// the per-version storage of the flags word.
    pub fn clear_flags(&self, flags: EInternalObjectFlags) {
        use std::sync::atomic::{AtomicI32, AtomicI64, Ordering::Relaxed};
        let l = Self::layout();
        let ptr = unsafe { self.base().add(l.flags) };
        match l.flags_format {
            FUObjectItemFlagsFormat::Int32 => {
                unsafe { &*(ptr as *const AtomicI32) }.fetch_and(!flags.bits(), Relaxed);
            }
            FUObjectItemFlagsFormat::HighHalfOfI64 => {
                let mask = ((flags.bits() as u32 as u64) << 32) as i64;
                unsafe { &*(ptr as *const AtomicI64) }.fetch_and(!mask, Relaxed);
            }
        }
    }
    pub fn is_unreachable(&self) -> bool {
        // `Unreachable` (1<<28) is stable across all supported builds.
        self.raw_flags() & EInternalObjectFlags::Unreachable.bits() != 0
    }
    pub fn is_pending_kill(&self) -> bool {
        // The garbage / pending-kill bit moved across versions; take it from the layout.
        (self.raw_flags() as u32) & Self::layout().garbage != 0
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FWeakObjectPtr {
    object_index: i32,
    object_serial_number: i32,
}
impl Default for FWeakObjectPtr {
    fn default() -> Self {
        Self {
            object_index: -1, // INDEX_NONE
            object_serial_number: 0,
        }
    }
}
impl PartialEq for FWeakObjectPtr {
    fn eq(&self, other: &Self) -> bool {
        (self.object_index == other.object_index
            && self.object_serial_number == other.object_serial_number)
            || (!self.is_valid() && !other.is_valid())
    }
}
impl Eq for FWeakObjectPtr {}
impl std::hash::Hash for FWeakObjectPtr {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.object_index ^ self.object_serial_number).hash(state);
    }
}
impl FWeakObjectPtr {
    pub fn new(object: &UObjectBase) -> Self {
        Self::new_from_index(object.internal_index())
    }
    pub fn new_from_index(index: ObjectIndex) -> Self {
        Self {
            object_index: index,
            // serial allocation performs only atomic operations
            object_serial_number: unsafe {
                (*globals().guobject_array).allocate_serial_number(index)
            },
        }
    }
    pub fn object_index(&self) -> i32 {
        self.object_index
    }
    pub fn object_serial_number(&self) -> i32 {
        self.object_serial_number
    }
    pub fn reset(&mut self) {
        self.object_index = -1;
        self.object_serial_number = 0;
    }
    pub fn is_explicitly_null(&self) -> bool {
        self.object_index == -1
    }
    pub fn has_same_index_and_serial_number(&self, other: &Self) -> bool {
        self.object_index == other.object_index
            && self.object_serial_number == other.object_serial_number
    }

    fn serial_numbers_match(&self, item: &FUObjectItem) -> bool {
        item.get_serial_number() == self.object_serial_number
    }

    /// Internal resolution: get the FUObjectItem if serial numbers still match.
    fn internal_get_object_item(&self) -> Option<&FUObjectItem> {
        if self.object_serial_number == 0 {
            return None;
        }
        if self.object_index < 0 {
            return None;
        }
        unsafe {
            let objects = &*(*globals().guobject_array).obj_objects.get();
            let item = objects.item(self.object_index);
            if !self.serial_numbers_match(item) {
                return None;
            }
            Some(item)
        }
    }

    fn is_valid_item(item: &FUObjectItem, even_if_pending_kill: bool) -> bool {
        if even_if_pending_kill {
            !item.is_unreachable()
        } else {
            !item.is_unreachable() && !item.is_pending_kill()
        }
    }

    /// Resolve the weak pointer, returning `None` if the object has been destroyed or is pending kill.
    pub fn get(&self) -> Option<&UObjectBase> {
        self.internal_get(false)
    }

    /// Resolve the weak pointer, returning the object even if it is pending kill
    /// (but not if it is unreachable/destroyed).
    pub fn get_even_if_pending_kill(&self) -> Option<&UObjectBase> {
        self.internal_get(true)
    }

    /// Resolve the weak pointer, bypassing all flag checks (unreachable, pending kill).
    /// Only checks that the serial number still matches. Suitable for cross-thread access.
    pub fn get_even_if_unreachable(&self) -> Option<&UObjectBase> {
        let item = self.internal_get_object_item()?;
        unsafe { item.object().as_ref() }
    }

    fn internal_get(&self, even_if_pending_kill: bool) -> Option<&UObjectBase> {
        let item = self.internal_get_object_item()?;
        if !Self::is_valid_item(item, even_if_pending_kill) {
            return None;
        }
        unsafe { item.object().as_ref() }
    }

    /// Check whether this weak pointer still points to a live object.
    pub fn is_valid(&self) -> bool {
        self.internal_is_valid(false, false)
    }

    pub fn is_valid_even_if_pending_kill(&self) -> bool {
        self.internal_is_valid(true, false)
    }

    fn internal_is_valid(&self, even_if_pending_kill: bool, threadsafe_test: bool) -> bool {
        match self.internal_get_object_item() {
            None => false,
            Some(item) => {
                if threadsafe_test {
                    true
                } else {
                    Self::is_valid_item(item, even_if_pending_kill)
                }
            }
        }
    }

    /// Returns true if this pointer once pointed to an object that has since been destroyed.
    /// A null/default pointer is NOT stale. A pointer whose target was GC'd IS stale.
    pub fn is_stale(&self) -> bool {
        self.internal_is_stale(false, false)
    }

    fn internal_is_stale(&self, even_if_pending_kill: bool, threadsafe_test: bool) -> bool {
        if self.object_serial_number == 0 {
            return false; // null is not stale
        }
        if self.object_index < 0 {
            return true;
        }
        unsafe {
            let objects = &*(*globals().guobject_array).obj_objects.get();
            let item = objects.item(self.object_index);
            if !self.serial_numbers_match(item) {
                return true;
            }
            if threadsafe_test {
                return false;
            }
            // IsStale in UE: bEvenIfPendingKill ? (PendingKill || Unreachable) : Unreachable
            if even_if_pending_kill {
                item.is_pending_kill() || item.is_unreachable()
            } else {
                item.is_unreachable()
            }
        }
    }
}

#[repr(C)]
pub struct TWeakObjectPtr<T: ObjTrait> {
    inner: FWeakObjectPtr,
    _marker: std::marker::PhantomData<*const T>,
}
impl<T: ObjTrait> Clone for TWeakObjectPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: ObjTrait> Copy for TWeakObjectPtr<T> {}
impl<T: ObjTrait> std::fmt::Debug for TWeakObjectPtr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}
impl<T: ObjTrait> Default for TWeakObjectPtr<T> {
    fn default() -> Self {
        Self {
            inner: FWeakObjectPtr::default(),
            _marker: std::marker::PhantomData,
        }
    }
}
impl<T: ObjTrait> PartialEq for TWeakObjectPtr<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}
impl<T: ObjTrait> Eq for TWeakObjectPtr<T> {}
impl<T: ObjTrait> std::hash::Hash for TWeakObjectPtr<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}
impl<T: ObjTrait> TWeakObjectPtr<T> {
    pub fn new(object: &UObjectBase) -> Self {
        Self {
            inner: FWeakObjectPtr::new(object),
            _marker: std::marker::PhantomData,
        }
    }
    pub fn new_from_index(index: ObjectIndex) -> Self {
        Self {
            inner: FWeakObjectPtr::new_from_index(index),
            _marker: std::marker::PhantomData,
        }
    }
    pub fn from_raw(raw: FWeakObjectPtr) -> Self {
        Self {
            inner: raw,
            _marker: std::marker::PhantomData,
        }
    }
    pub fn into_raw(self) -> FWeakObjectPtr {
        self.inner
    }
    pub fn object_index(&self) -> i32 {
        self.inner.object_index()
    }
    pub fn object_serial_number(&self) -> i32 {
        self.inner.object_serial_number()
    }
    pub fn reset(&mut self) {
        self.inner.reset();
    }
    pub fn is_explicitly_null(&self) -> bool {
        self.inner.is_explicitly_null()
    }
    pub fn is_valid(&self) -> bool {
        self.inner.is_valid()
    }
    pub fn is_stale(&self) -> bool {
        self.inner.is_stale()
    }
    pub fn get(&self) -> Option<&T> {
        self.inner.get().and_then(|obj| obj.cast::<T>())
    }
    pub fn get_even_if_pending_kill(&self) -> Option<&T> {
        self.inner
            .get_even_if_pending_kill()
            .and_then(|obj| obj.cast::<T>())
    }
    pub fn get_even_if_unreachable(&self) -> Option<&T> {
        self.inner
            .get_even_if_unreachable()
            .and_then(|obj| obj.cast::<T>())
    }
}

// ─── FGuid ─────────────────────────────────────────────────────────
//
// UE's FGuid: four u32 words (Microsoft Variant 4 format internally,
// but exposed as 4 packed ints).

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct FGuid {
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub d: u32,
}

impl FGuid {
    pub fn is_valid(&self) -> bool {
        (self.a | self.b | self.c | self.d) != 0
    }
}

impl Display for FGuid {
    /// Canonical UE "Digits" format: `00000000000000000000000000000000`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:08X}{:08X}{:08X}{:08X}",
            self.a, self.b, self.c, self.d
        )
    }
}

// ─── FSoftObjectPath ───────────────────────────────────────────────
//
// UE 4.x layout: `FName AssetPathName; FString SubPathString;`
// (UE 5.x replaced AssetPathName with FTopLevelAssetPath = {FName, FName}
// — not implemented here; would change size from 24 → 32 bytes.)

#[derive(Debug)]
#[repr(C)]
pub struct FSoftObjectPath {
    pub asset_path_name: FName,
    pub sub_path_string: FString,
}

impl Default for FSoftObjectPath {
    fn default() -> Self {
        Self {
            asset_path_name: FName {
                comparison_index: FNameEntryId { value: 0 },
                number: 0,
            },
            sub_path_string: FString::new(),
        }
    }
}

impl FSoftObjectPath {
    pub fn is_empty(&self) -> bool {
        self.asset_path_name.comparison_index.value == 0 && self.sub_path_string.num == 0
    }

    /// Parse a path string of the form `Package.Asset[:SubPath]` and
    /// build an `FSoftObjectPath` whose `AssetPathName` is added to the
    /// FName table (FNAME_Add). The cached weak pointer / tag fields
    /// living *outside* this struct (see `FSoftObjectPtr`) are caller's
    /// responsibility to reset.
    pub fn from_string(s: &str) -> Self {
        let (asset_path, sub_path) = match s.find(':') {
            Some(i) => (&s[..i], &s[i + 1..]),
            None => (s, ""),
        };
        let asset_path_name = if asset_path.is_empty() {
            FName {
                comparison_index: FNameEntryId { value: 0 },
                number: 0,
            }
        } else {
            FName::from_string(asset_path, 1) // FNAME_Add
        };
        let sub_path_string = if sub_path.is_empty() {
            FString::new()
        } else {
            FString::from(sub_path)
        };
        Self {
            asset_path_name,
            sub_path_string,
        }
    }
}

impl Display for FSoftObjectPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_empty() {
            return Ok(());
        }
        write!(f, "{}", self.asset_path_name)?;
        if self.sub_path_string.num > 0 {
            write!(f, ":{}", self.sub_path_string)?;
        }
        Ok(())
    }
}

// ─── FSoftObjectPtr ────────────────────────────────────────────────
//
// What FSoftObjectProperty actually stores. UE 4.x:
//   FPersistentObjectPtr<FSoftObjectPath> {
//     mutable FWeakObjectPtr WeakPtr;       // 8 bytes
//     mutable int32 TagAtLastTest;          // 4 bytes
//     FSoftObjectPath ObjectID;             // 24 bytes (8-byte aligned)
//   }
// Padding 4 bytes between TagAtLastTest and ObjectID to satisfy
// FSoftObjectPath's 8-byte alignment (FString contains a pointer).

#[derive(Debug)]
#[repr(C)]
pub struct FSoftObjectPtr {
    pub weak_ptr: FWeakObjectPtr,
    pub tag_at_last_test: i32,
    _pad: u32,
    pub object_id: FSoftObjectPath,
}
const _: () = assert!(std::mem::size_of::<FSoftObjectPtr>() == 40);
const _: () = assert!(std::mem::offset_of!(FSoftObjectPtr, object_id) == 16);

impl Default for FSoftObjectPtr {
    fn default() -> Self {
        Self {
            weak_ptr: FWeakObjectPtr::default(),
            tag_at_last_test: 0,
            _pad: 0,
            object_id: FSoftObjectPath::default(),
        }
    }
}

impl FSoftObjectPtr {
    pub fn from_path(path: FSoftObjectPath) -> Self {
        Self {
            weak_ptr: FWeakObjectPtr::default(),
            tag_at_last_test: 0,
            _pad: 0,
            object_id: path,
        }
    }
    pub fn from_string(s: &str) -> Self {
        Self::from_path(FSoftObjectPath::from_string(s))
    }
}

impl Display for FSoftObjectPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.object_id, f)
    }
}

// ─── FLazyObjectPtr ────────────────────────────────────────────────
//
// What FLazyObjectProperty stores. Same FPersistentObjectPtr shape,
// parameterized by FUniqueObjectGuid (which wraps FGuid).
//   {
//     mutable FWeakObjectPtr WeakPtr;       // 8
//     mutable int32 TagAtLastTest;          // 4
//     FUniqueObjectGuid ObjectID;           // 16 (just FGuid)
//   }

#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct FUniqueObjectGuid {
    pub guid: FGuid,
}

#[derive(Debug, Default)]
#[repr(C)]
pub struct FLazyObjectPtr {
    pub weak_ptr: FWeakObjectPtr,
    pub tag_at_last_test: i32,
    pub object_id: FUniqueObjectGuid,
}
const _: () = assert!(std::mem::size_of::<FLazyObjectPtr>() == 28);

impl FLazyObjectPtr {
    pub fn from_guid(guid: FGuid) -> Self {
        Self {
            weak_ptr: FWeakObjectPtr::default(),
            tag_at_last_test: 0,
            object_id: FUniqueObjectGuid { guid },
        }
    }
}

impl Display for FLazyObjectPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.object_id.guid, f)
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone)]
    pub struct EObjectFlags: u32 {
        const RF_NoFlags = 0x0000;
        const RF_Public = 0x0001;
        const RF_Standalone = 0x0002;
        const RF_MarkAsNative = 0x0004;
        const RF_Transactional = 0x0008;
        const RF_ClassDefaultObject = 0x0010;
        const RF_ArchetypeObject = 0x0020;
        const RF_Transient = 0x0040;
        const RF_MarkAsRootSet = 0x0080;
        const RF_TagGarbageTemp = 0x0100;
        const RF_NeedInitialization = 0x0200;
        const RF_NeedLoad = 0x0400;
        const RF_KeepForCooker = 0x0800;
        const RF_NeedPostLoad = 0x1000;
        const RF_NeedPostLoadSubobjects = 0x2000;
        const RF_NewerVersionExists = 0x4000;
        const RF_BeginDestroyed = 0x8000;
        const RF_FinishDestroyed = 0x00010000;
        const RF_BeingRegenerated = 0x00020000;
        const RF_DefaultSubObject = 0x00040000;
        const RF_WasLoaded = 0x00080000;
        const RF_TextExportTransient = 0x00100000;
        const RF_LoadCompleted = 0x00200000;
        const RF_InheritableComponentTemplate = 0x00400000;
        const RF_DuplicateTransient = 0x00800000;
        const RF_StrongRefOnFrame = 0x01000000;
        const RF_NonPIEDuplicateTransient = 0x02000000;
        const RF_Dynamic = 0x04000000;
        const RF_WillBeLoaded = 0x08000000;
    }
}
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy)]
    pub struct EFunctionFlags: u32 {
        const FUNC_None = 0x0000;
        const FUNC_Final = 0x0001;
        const FUNC_RequiredAPI = 0x0002;
        const FUNC_BlueprintAuthorityOnly = 0x0004;
        const FUNC_BlueprintCosmetic = 0x0008;
        const FUNC_Net = 0x0040;
        const FUNC_NetReliable = 0x0080;
        const FUNC_NetRequest = 0x0100;
        const FUNC_Exec = 0x0200;
        const FUNC_Native = 0x0400;
        const FUNC_Event = 0x0800;
        const FUNC_NetResponse = 0x1000;
        const FUNC_Static = 0x2000;
        const FUNC_NetMulticast = 0x4000;
        const FUNC_UbergraphFunction = 0x8000;
        const FUNC_MulticastDelegate = 0x00010000;
        const FUNC_Public = 0x00020000;
        const FUNC_Private = 0x00040000;
        const FUNC_Protected = 0x00080000;
        const FUNC_Delegate = 0x00100000;
        const FUNC_NetServer = 0x00200000;
        const FUNC_HasOutParms = 0x00400000;
        const FUNC_HasDefaults = 0x00800000;
        const FUNC_NetClient = 0x01000000;
        const FUNC_DLLImport = 0x02000000;
        const FUNC_BlueprintCallable = 0x04000000;
        const FUNC_BlueprintEvent = 0x08000000;
        const FUNC_BlueprintPure = 0x10000000;
        const FUNC_EditorOnly = 0x20000000;
        const FUNC_Const = 0x40000000;
        const FUNC_NetValidate = 0x80000000;
        const FUNC_AllFlags = 0xffffffff;
    }

    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct EClassFlags: i32 {
        const CLASS_None = 0x0000;
        const CLASS_Abstract = 0x0001;
        const CLASS_DefaultConfig = 0x0002;
        const CLASS_Config = 0x0004;
        const CLASS_Transient = 0x0008;
        const CLASS_Parsed = 0x0010;
        const CLASS_MatchedSerializers = 0x0020;
        const CLASS_ProjectUserConfig = 0x0040;
        const CLASS_Native = 0x0080;
        const CLASS_NoExport = 0x0100;
        const CLASS_NotPlaceable = 0x0200;
        const CLASS_PerObjectConfig = 0x0400;
        const CLASS_ReplicationDataIsSetUp = 0x0800;
        const CLASS_EditInlineNew = 0x1000;
        const CLASS_CollapseCategories = 0x2000;
        const CLASS_Interface = 0x4000;
        const CLASS_CustomConstructor = 0x8000;
        const CLASS_Const = 0x00010000;
        const CLASS_LayoutChanging = 0x00020000;
        const CLASS_CompiledFromBlueprint = 0x00040000;
        const CLASS_MinimalAPI = 0x00080000;
        const CLASS_RequiredAPI = 0x00100000;
        const CLASS_DefaultToInstanced = 0x00200000;
        const CLASS_TokenStreamAssembled = 0x00400000;
        const CLASS_HasInstancedReference = 0x00800000;
        const CLASS_Hidden = 0x01000000;
        const CLASS_Deprecated = 0x02000000;
        const CLASS_HideDropDown = 0x04000000;
        const CLASS_GlobalUserConfig = 0x08000000;
        const CLASS_Intrinsic = 0x10000000;
        const CLASS_Constructed = 0x20000000;
        const CLASS_ConfigDoNotCheckDefaults = 0x40000000;
        const CLASS_NewerVersionExists = i32::MIN;
    }


    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct EClassCastFlags : u64 {
        const CASTCLASS_None = 0x0000000000000000;
        const CASTCLASS_UField = 0x0000000000000001;
        const CASTCLASS_FInt8Property = 0x0000000000000002;
        const CASTCLASS_UEnum = 0x0000000000000004;
        const CASTCLASS_UStruct = 0x0000000000000008;
        const CASTCLASS_UScriptStruct = 0x0000000000000010;
        const CASTCLASS_UClass = 0x0000000000000020;
        const CASTCLASS_FByteProperty = 0x0000000000000040;
        const CASTCLASS_FIntProperty = 0x0000000000000080;
        const CASTCLASS_FFloatProperty = 0x0000000000000100;
        const CASTCLASS_FUInt64Property = 0x0000000000000200;
        const CASTCLASS_FClassProperty = 0x0000000000000400;
        const CASTCLASS_FUInt32Property = 0x0000000000000800;
        const CASTCLASS_FInterfaceProperty = 0x0000000000001000;
        const CASTCLASS_FNameProperty = 0x0000000000002000;
        const CASTCLASS_FStrProperty = 0x0000000000004000;
        const CASTCLASS_FProperty = 0x0000000000008000;
        const CASTCLASS_FObjectProperty = 0x0000000000010000;
        const CASTCLASS_FBoolProperty = 0x0000000000020000;
        const CASTCLASS_FUInt16Property = 0x0000000000040000;
        const CASTCLASS_UFunction = 0x0000000000080000;
        const CASTCLASS_FStructProperty = 0x0000000000100000;
        const CASTCLASS_FArrayProperty = 0x0000000000200000;
        const CASTCLASS_FInt64Property = 0x0000000000400000;
        const CASTCLASS_FDelegateProperty = 0x0000000000800000;
        const CASTCLASS_FNumericProperty = 0x0000000001000000;
        const CASTCLASS_FMulticastDelegateProperty = 0x0000000002000000;
        const CASTCLASS_FObjectPropertyBase = 0x0000000004000000;
        const CASTCLASS_FWeakObjectProperty = 0x0000000008000000;
        const CASTCLASS_FLazyObjectProperty = 0x0000000010000000;
        const CASTCLASS_FSoftObjectProperty = 0x0000000020000000;
        const CASTCLASS_FTextProperty = 0x0000000040000000;
        const CASTCLASS_FInt16Property = 0x0000000080000000;
        const CASTCLASS_FDoubleProperty = 0x0000000100000000;
        const CASTCLASS_FSoftClassProperty = 0x0000000200000000;
        const CASTCLASS_UPackage = 0x0000000400000000;
        const CASTCLASS_ULevel = 0x0000000800000000;
        const CASTCLASS_AActor = 0x0000001000000000;
        const CASTCLASS_APlayerController = 0x0000002000000000;
        const CASTCLASS_APawn = 0x0000004000000000;
        const CASTCLASS_USceneComponent = 0x0000008000000000;
        const CASTCLASS_UPrimitiveComponent = 0x0000010000000000;
        const CASTCLASS_USkinnedMeshComponent = 0x0000020000000000;
        const CASTCLASS_USkeletalMeshComponent = 0x0000040000000000;
        const CASTCLASS_UBlueprint = 0x0000080000000000;
        const CASTCLASS_UDelegateFunction = 0x0000100000000000;
        const CASTCLASS_UStaticMeshComponent = 0x0000200000000000;
        const CASTCLASS_FMapProperty = 0x0000400000000000;
        const CASTCLASS_FSetProperty = 0x0000800000000000;
        const CASTCLASS_FEnumProperty = 0x0001000000000000;
        const CASTCLASS_USparseDelegateFunction = 0x0002000000000000;
        const CASTCLASS_FMulticastInlineDelegateProperty = 0x0004000000000000;
        const CASTCLASS_FMulticastSparseDelegateProperty = 0x0008000000000000;
        const CASTCLASS_FFieldPathProperty = 0x0010000000000000;
        const CASTCLASS_FLargeWorldCoordinatesRealProperty = 0x0080000000000000;
        const CASTCLASS_FOptionalProperty = 0x0100000000000000;
        const CASTCLASS_FVerseValueProperty = 0x0200000000000000;
        const CASTCLASS_UVerseVMClass = 0x0400000000000000;
    }

    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct  EPropertyFlags: u64 {
        const CPF_None = 0x0000;
        const CPF_Edit = 0x0001;
        const CPF_ConstParm = 0x0002;
        const CPF_BlueprintVisible = 0x0004;
        const CPF_ExportObject = 0x0008;
        const CPF_BlueprintReadOnly = 0x0010;
        const CPF_Net = 0x0020;
        const CPF_EditFixedSize = 0x0040;
        const CPF_Parm = 0x0080;
        const CPF_OutParm = 0x0100;
        const CPF_ZeroConstructor = 0x0200;
        const CPF_ReturnParm = 0x0400;
        const CPF_DisableEditOnTemplate = 0x0800;
        const CPF_NonNullable = 0x1000;
        const CPF_Transient = 0x2000;
        const CPF_Config = 0x4000;
        const CPF_RequiredParm = 0x8000;
        const CPF_DisableEditOnInstance = 0x00010000;
        const CPF_EditConst = 0x00020000;
        const CPF_GlobalConfig = 0x00040000;
        const CPF_InstancedReference = 0x00080000;
        const CPF_ExperimentalExternalObjects = 0x00100000;
        const CPF_DuplicateTransient = 0x00200000;
        const CPF_SaveGame = 0x01000000;
        const CPF_NoClear = 0x02000000;
        const CPF_Virtual = 0x04000000;
        const CPF_ReferenceParm = 0x08000000;
        const CPF_BlueprintAssignable = 0x10000000;
        const CPF_Deprecated = 0x20000000;
        const CPF_IsPlainOldData = 0x40000000;
        const CPF_RepSkip = 0x80000000;
        const CPF_RepNotify = 0x100000000;
        const CPF_Interp = 0x200000000;
        const CPF_NonTransactional = 0x400000000;
        const CPF_EditorOnly = 0x800000000;
        const CPF_NoDestructor = 0x1000000000;
        const CPF_AutoWeak = 0x4000000000;
        const CPF_ContainsInstancedReference = 0x8000000000;
        const CPF_AssetRegistrySearchable = 0x10000000000;
        const CPF_SimpleDisplay = 0x20000000000;
        const CPF_AdvancedDisplay = 0x40000000000;
        const CPF_Protected = 0x80000000000;
        const CPF_BlueprintCallable = 0x100000000000;
        const CPF_BlueprintAuthorityOnly = 0x200000000000;
        const CPF_TextExportTransient = 0x400000000000;
        const CPF_NonPIEDuplicateTransient = 0x800000000000;
        const CPF_ExposeOnSpawn = 0x1000000000000;
        const CPF_PersistentInstance = 0x2000000000000;
        const CPF_UObjectWrapper = 0x4000000000000;
        const CPF_HasGetValueTypeHash = 0x8000000000000;
        const CPF_NativeAccessSpecifierPublic = 0x10000000000000;
        const CPF_NativeAccessSpecifierProtected = 0x20000000000000;
        const CPF_NativeAccessSpecifierPrivate = 0x40000000000000;
        const CPF_SkipSerialization = 0x80000000000000;
        const CPF_TObjectPtr = 0x100000000000000;
        const CPF_ExperimentalOverridableLogic = 0x200000000000000;
        const CPF_ExperimentalAlwaysOverriden = 0x400000000000000;
        const CPF_ExperimentalNeverOverriden = 0x800000000000000;
        const CPF_AllowSelfReference = 0x1000000000000000;
    }
}

/// # Safety
/// `CAST_FLAGS` must uniquely identify the implementing type; wrong flags
/// allow safe `cast::<T>` to produce a misaligned reference (UB).
pub unsafe trait ObjTrait {
    const CAST_FLAGS: EClassCastFlags;
}
/// # Safety
/// `CAST_FLAGS` must uniquely identify the implementing type; wrong flags
/// allow safe `cast::<T>` to produce a misaligned reference (UB).
pub unsafe trait FieldTrait {
    const CAST_FLAGS: EClassCastFlags;
    fn cast<T: FieldTrait>(&self) -> Option<&T>
    where
        Self: std::ops::Deref<Target = FFieldBase>,
    {
        FFieldBase::cast(self)
    }
}

ue_struct! {
    /// `UObjectBase` — the root of the UObject hierarchy and a layout-dynamic
    /// view into a live engine object. This is the type whose member order shifts
    /// most across engine versions, so all access is offset-driven. See
    /// the `UE_4_27` offsets for the reference 4.27 layout (`vtable` sits
    /// at offset 0 and is never read by name).
    struct UObjectBase;
    layout uobjectbase: UObjectBaseLayout;
    scalar object_flags: EObjectFlags,
    scalar internal_index: i32,
    /// This object's `ClassPrivate`. Setting it through the `class_private_mut`
    /// place re-parents the object: afterwards `GetClass()` (and thus
    /// `FindFunction`, `IsA`, delegate dispatch) resolves against the new
    /// class. Only sane for an object with no reflected properties whose new
    /// and old class are layout-compatible (e.g. a bare `UObject` re-parented
    /// onto a host class whose `SuperStruct` is that same `UObject` class),
    /// on the game thread, before the object is published to any concurrent
    /// lookup — which is exactly the exclusivity the `&mut` pedigree proves.
    excl class_private: *const UClass,
    scalar name_private: FName,
    scalar outer_private: *const UObject,
}

impl UObjectBase {
    pub fn class(&self) -> &UClass {
        // TODO thread safe class access? i guess if we have a reference to the containing object it's fine?
        unsafe { self.class_private().as_ref().unwrap() }
    }

    pub fn is<T: ObjTrait>(&self) -> bool {
        self.class().class_cast_flags().contains(T::CAST_FLAGS)
    }
    pub fn cast<T: ObjTrait>(&self) -> Option<&T> {
        self.is::<T>()
            .then(|| unsafe { std::mem::transmute::<&Self, &T>(self) })
    }
    pub fn cast_mut<T: ObjTrait>(&mut self) -> Option<&mut T> {
        self.is::<T>()
            .then(|| unsafe { std::mem::transmute::<&mut Self, &mut T>(self) })
    }

    /// UE's `UObjectBaseUtility::GetInterfaceAddress`. Returns the address of the
    /// interface subobject for `iface` within this instance, or `None` if this
    /// object doesn't implement it. For a non-native (Blueprint) interface the
    /// address is the object itself; for a native interface it's `this +
    /// PointerOffset`, looked up in the class's (and supers') interface table.
    ///
    /// `iface` may be null (returns `None`); a non-null `iface` must be a valid
    /// `UClass` pointer.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn get_interface_address(&self, iface: *const UClass) -> Option<*const c_void> {
        let iface_ref = unsafe { iface.as_ref() }?;
        if !iface_ref
            .class_flags()
            .contains(EClassFlags::CLASS_Interface)
        {
            return None;
        }
        let class = self.class();
        if !iface_ref.class_flags().contains(EClassFlags::CLASS_Native) {
            // Script/BP interface: the subobject isn't laid out separately, so the
            // address is just the object — provided the class actually implements it.
            return class
                .implements_interface(iface)
                .then(|| self as *const Self as *const c_void);
        }
        // Native interface: find the implementing entry and apply its vtable offset.
        let mut cur: *const UClass = class;
        while let Some(c) = unsafe { cur.as_ref() } {
            for entry in c.interfaces() {
                if entry.b_implemented_by_k2 {
                    continue;
                }
                if let Some(ic) = unsafe { entry.class.as_ref() } {
                    if ic.is_child_of(iface as *const UStruct) {
                        let base = self as *const Self as *const u8;
                        return Some(
                            unsafe { base.add(entry.pointer_offset as usize) } as *const c_void
                        );
                    }
                }
            }
            cur = c.super_struct() as *const UClass;
        }
        None
    }

    /// Resolve this object's `FUObjectItem` slot in the global object array,
    /// or `None` if `internal_index` isn't populated.
    pub fn object_item(&self) -> Option<&FUObjectItem> {
        if self.internal_index() < 0 {
            return None;
        }
        unsafe {
            let objects = &*(*globals().guobject_array).obj_objects.get();
            Some(objects.item(self.internal_index()))
        }
    }

    /// Mark this object as a GC root by setting `EInternalObjectFlags::RootSet`
    /// on its `FUObjectItem`. Idempotent.
    ///
    /// NOTE: this only ORs the flag — unlike the engine's `UObject::AddToRoot`
    /// it does **not** insert the object into the `GRoots` `TSet` that UE5.4+
    /// iterates during reachability analysis, because that path (and its lock)
    /// isn't resolved. On those builds the flag alone does not keep the object
    /// alive; to root a UObject you construct, pass `RootSet` as
    /// `internal_set_flags` to [`static_construct_object`] instead, which lets
    /// the engine populate `GRoots` for you. On pre-5.4 builds the flag is all
    /// GC checks, so this is fully equivalent to `AddToRoot` there.
    pub fn add_to_root(&self) {
        if let Some(item) = self.object_item() {
            item.set_flags(EInternalObjectFlags::RootSet);
        }
    }
    /// Clear `EInternalObjectFlags::RootSet`, making the object eligible for GC
    /// again. Idempotent.
    ///
    /// NOTE: flag-only, mirroring [`add_to_root`](Self::add_to_root) — it does
    /// not remove the object from the UE5.4+ `GRoots` `TSet`. An object that was
    /// rooted into `GRoots` (via `static_construct_object`'s `RootSet`
    /// `internal_set_flags`) stays a root here until `FUObjectItem::ClearRootFlags`
    /// is resolved and wired in; on pre-5.4 builds this is equivalent to
    /// `RemoveFromRoot`.
    pub fn remove_from_root(&self) {
        if let Some(item) = self.object_item() {
            item.clear_flags(EInternalObjectFlags::RootSet);
        }
    }
    /// Whether the object is currently a GC root (`RootSet` set on its item).
    pub fn is_rooted(&self) -> bool {
        self.object_item()
            .is_some_and(|item| item.get_flags().contains(EInternalObjectFlags::RootSet))
    }
    /// UObjectBaseUtility::GetFullName — "ClassName PathName".
    pub fn path(&self) -> String {
        // Objects pulled straight from GUObjectArray can be mid-registration with
        // a null ClassPrivate; don't unwrap (a panic here unwinds across the FFI
        // boundary and crashes the host).
        let mut path = String::new();
        if let Some(class) = unsafe { self.class_private().as_ref() } {
            path.push_str(&class.name_private().to_string());
            path.push(' ');
        }
        self.append_path_name(std::ptr::null(), &mut path);
        path
    }
    /// UObjectBaseUtility::GetPathName — matches UE's implementation.
    pub fn object_path(&self) -> String {
        let mut path = String::new();
        self.append_path_name(std::ptr::null(), &mut path);
        path
    }

    fn append_path_name(&self, stop_outer: *const UObject, path: &mut String) {
        let self_ptr = self as *const UObjectBase as *const UObject;
        if self_ptr == stop_outer || self_ptr.is_null() {
            return;
        }

        let outer = unsafe { self.outer_private().as_ref() };
        if let Some(outer) = outer {
            // &UObject coerces to &UObjectBase through Deref.
            let outer_base: &UObjectBase = outer;

            if !std::ptr::eq(outer, stop_outer) {
                outer_base.append_path_name(stop_outer, path);

                // SUBOBJECT_DELIMITER_CHAR is used when outer is not a UPackage
                // but outer's outer is a UPackage
                let outer_is_package = unsafe {
                    outer_base.class_private().as_ref().is_some_and(|cls| {
                        cls.class_cast_flags()
                            .contains(EClassCastFlags::CASTCLASS_UPackage)
                    })
                };
                let outers_outer_is_package = unsafe {
                    outer_base
                        .outer_private()
                        .as_ref()
                        .and_then(|oo| oo.class_private().as_ref())
                        .is_some_and(|cls| {
                            cls.class_cast_flags()
                                .contains(EClassCastFlags::CASTCLASS_UPackage)
                        })
                };

                if !outer_is_package && outers_outer_is_package {
                    path.push(':');
                } else {
                    path.push('.');
                }
            }
        }

        path.push_str(&self.name_private().to_string());
    }
    pub fn props<'o>(&'o self) -> IterFieldsBound<'o> {
        let inner = self.class().iter_props();
        IterFieldsBound {
            object: std::ptr::from_ref(self).cast(),
            inner,
        }
    }
    pub fn props_mut<'o>(&'o mut self) -> IterFieldsBoundMut<'o> {
        let inner = unsafe { &*std::ptr::from_ref(self.class()) }.iter_props();
        IterFieldsBoundMut {
            object: std::ptr::from_mut(self).cast(),
            inner,
        }
    }
}

pub struct IterFieldsBound<'o> {
    object: *const (),
    inner: IterProps<'o>,
}
impl<'o> Iterator for IterFieldsBound<'o> {
    type Item = BoundField<'o>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|f| BoundField {
            object: self.object,
            field: f,
        })
    }
}
pub trait PropertyAccess<'o> {
    fn try_get<P: PropTrait + 'o>(&self) -> Option<P::PropValue<'o>>;
    fn try_get_mut<P: PropTrait + 'o>(&mut self) -> Option<P::PropValueMut<'o>>;
    fn field(&self) -> &FField;
}

pub struct BoundField<'o> {
    pub object: *const (),
    pub field: &'o FField,
}
impl<'o> BoundField<'o> {
    pub fn get<P: PropTrait + 'o>(&self) -> Option<P::PropValue<'o>> {
        FieldTrait::cast::<P>(self.field).map(|f| unsafe { f.value_obj(self.object) })
    }
}

impl<'o> PropertyAccess<'o> for BoundField<'o> {
    fn try_get<P: PropTrait + 'o>(&self) -> Option<P::PropValue<'o>> {
        self.get::<P>()
    }

    fn try_get_mut<P: PropTrait + 'o>(&mut self) -> Option<P::PropValueMut<'o>> {
        // BoundField doesn't support mutable access directly since it's for immutable object references
        // This could be implemented if needed by converting to BoundFieldMut
        None
    }

    fn field(&self) -> &FField {
        self.field
    }
}

pub struct BoundArrayElement<'o> {
    pub data_ptr: *const (),
    pub property: &'o FProperty,
}
impl<'o> BoundArrayElement<'o> {
    pub fn get<P: PropTrait + 'o>(&self) -> Option<P::PropValue<'o>> {
        self.property
            .cast_field::<P>()
            .map(|f| unsafe { f.value(self.data_ptr) })
    }

    /// # Safety
    /// Caller must guarantee the underlying element has the layout of `T`.
    pub unsafe fn cast<T>(&self) -> &'o T {
        unsafe { &*(self.data_ptr as *const T) }
    }
}

impl<'o> PropertyAccess<'o> for BoundArrayElement<'o> {
    fn try_get<P: PropTrait + 'o>(&self) -> Option<P::PropValue<'o>> {
        self.get::<P>()
    }

    fn try_get_mut<P: PropTrait + 'o>(&mut self) -> Option<P::PropValueMut<'o>> {
        // BoundArrayElement is immutable, return None for mutable access
        None
    }

    fn field(&self) -> &FField {
        self.property
    }
}

pub struct BoundArrayElementMut<'o> {
    data_ptr: *mut (),
    pub property: &'o FProperty,
}
impl<'o> BoundArrayElementMut<'o> {
    pub fn get<P: PropTrait>(&self) -> Option<P::PropValueMut<'o>> {
        self.property
            .cast_field::<P>()
            .map(|f| unsafe { f.value_mut(self.data_ptr) })
    }

    pub fn cast<T>(&self) -> &'o T {
        unsafe { &*(self.data_ptr as *const T) }
    }

    pub fn cast_mut<T>(&mut self) -> &'o mut T {
        unsafe { &mut *(self.data_ptr as *mut T) }
    }
}

impl<'o> PropertyAccess<'o> for BoundArrayElementMut<'o> {
    fn try_get<P: PropTrait + 'o>(&self) -> Option<P::PropValue<'o>> {
        // For immutable access, we can cast the property but need to be careful with lifetimes
        self.property
            .cast_field::<P>()
            .map(|f| unsafe { f.value(self.data_ptr as *const ()) })
    }

    fn try_get_mut<P: PropTrait + 'o>(&mut self) -> Option<P::PropValueMut<'o>> {
        self.get::<P>()
    }

    fn field(&self) -> &FField {
        self.property
    }
}

pub struct IterFieldsBoundMut<'o> {
    object: *mut (),
    inner: IterProps<'o>,
}
impl<'o> Iterator for IterFieldsBoundMut<'o> {
    type Item = BoundFieldMut<'o>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|f| BoundFieldMut {
            object: self.object,
            field: f,
        })
    }
}
pub struct BoundFieldMut<'o> {
    object: *mut (),
    pub field: &'o FField,
}
impl<'o> BoundFieldMut<'o> {
    pub fn get<P: PropTrait + 'o>(&mut self) -> Option<P::PropValueMut<'o>> {
        FieldTrait::cast::<P>(self.field).map(|f| unsafe { f.value_obj_mut(self.object) })
    }
}

impl<'o> PropertyAccess<'o> for BoundFieldMut<'o> {
    fn try_get<P: PropTrait + 'o>(&self) -> Option<P::PropValue<'o>> {
        // For immutable access through BoundFieldMut
        FieldTrait::cast::<P>(self.field).map(|f| unsafe { f.value_obj(self.object as *const _) })
    }

    fn try_get_mut<P: PropTrait + 'o>(&mut self) -> Option<P::PropValueMut<'o>> {
        FieldTrait::cast::<P>(self.field).map(|f| unsafe { f.value_obj_mut(self.object) })
    }

    fn field(&self) -> &FField {
        self.field
    }
}

ue_struct! {
    /// `UObjectBaseUtility` — a field-less offset-0 wrapper over `UObjectBase`.
    struct UObjectBaseUtility: UObjectBase;
}

ue_struct! {
    /// `UObject` — a field-less offset-0 wrapper over `UObjectBaseUtility`.
    struct UObject: UObjectBaseUtility;
    // TODO not sure if this works? special case UObject?
    cast ObjTrait = EClassCastFlags::CASTCLASS_None;
}

#[derive(Debug)]
#[repr(C)]
pub struct UPackage {
    pub uobject: UObject,
}

#[derive(Debug)]
#[repr(C)]
pub struct FOutputDevice {
    vtable: *const c_void,
    b_suppress_event_tag: bool,
    b_auto_emit_line_terminator: bool,
}

ue_struct! {
    /// `UField` — a layout-dynamic view into a live engine object. See
    /// the `UE_4_27` offsets for the reference 4.27 layout.
    struct UField: UObject;
    cast ObjTrait = EClassCastFlags::CASTCLASS_UField;
    layout ufield: UFieldLayout;
    scalar next: *const UField,
}

/// One entry in `UEnum::Names`. UE's `TPair<FName, int64>`: `FName` (8B,
/// 4-byte aligned), `int64` (8B, 8-byte aligned). With FName at offset 0
/// and int64 needing 8-byte alignment, the pair ends up 16 bytes.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FEnumNameEntry {
    pub name: FName,
    pub value: i64,
}
const _: () = assert!(std::mem::size_of::<FEnumNameEntry>() == 16);

/// UE 5.7 `UEnum::FNameData` — the struct-of-arrays that replaced the old
/// `TArray<TPair<FName,int64>>`. `tagged_names`/`tagged_values` are pointers to
/// parallel `FName[]` / `int64[]` arrays with an allocation tag in bit 0 (mask
/// with `& !1`). See [`EnumNamesFormat::NameData`].
#[repr(C)]
struct FNameData {
    tagged_names: usize,
    tagged_values: usize,
    num_values: i32,
}

#[derive(Debug)]
#[repr(u8)]
pub enum ECppForm {
    Regular = 0,
    Namespaced = 1,
    EnumClass = 2,
}

ue_struct! {
    /// `UEnum` — a layout-dynamic view into a live engine object. Carries no
    /// `#[repr(C)]` layout of its own: it is a zero-sized handle that is only ever
    /// held behind a pointer into engine memory, and its members are read at the
    /// offsets in [`layout().uenum`](UEnumLayout). See the `UE_4_27` offsets for
    /// the reference 4.27 layout (further fields — `EnumFlags`,
    /// `EnumDisplayNameFn`, … — are unused).
    struct UEnum: UField;
    cast ObjTrait = EClassCastFlags::CASTCLASS_UEnum;
    layout uenum: UEnumLayout;
    view cpp_type: FString,
    view names as names_array: TArray<FEnumNameEntry>,
    scalar cpp_form: ECppForm,
}

impl UEnum {
    /// The `(FName, i64)` entries. Note: the *last* entry is always the
    /// auto-generated `_MAX` sentinel — callers usually want to ignore it.
    ///
    /// Returns owned entries because UE 5.7 changed `UEnum::Names` from a
    /// contiguous `TArray<TPair<FName,int64>>` to `FNameData`, a struct of two
    /// *separate* tagged arrays (names and values) — there's no in-memory slice
    /// of pairs to borrow, so we materialize one.
    pub fn entries(&self) -> Vec<FEnumNameEntry> {
        match globals().layout.enum_names {
            EnumNamesFormat::PairArray => self.names_array().as_slice().to_vec(),
            EnumNamesFormat::NameData => {
                let nd = unsafe {
                    &*(self.base().add(globals().layout.uenum.names) as *const FNameData)
                };
                // Low bit of each pointer is an allocation tag — mask it off.
                let names = (nd.tagged_names & !1) as *const FName;
                let values = (nd.tagged_values & !1) as *const i64;
                let num = nd.num_values.max(0) as usize;
                if num == 0 || names.is_null() || values.is_null() {
                    return Vec::new();
                }
                (0..num)
                    .map(|i| unsafe {
                        FEnumNameEntry {
                            name: names.add(i).read(),
                            value: values.add(i).read(),
                        }
                    })
                    .collect()
            }
        }
    }

    /// Resolve an underlying numeric value to its declared FName, if any.
    /// For `enum class` and namespaced enums this returns the
    /// `EnumType::Entry` qualified form (that's how UE stores it).
    pub fn name_by_value(&self, value: i64) -> Option<FName> {
        self.entries()
            .iter()
            .find(|e| e.value == value)
            .map(|e| e.name)
    }

    /// Resolve a name to its underlying numeric value. Tries an exact
    /// match first (`EnumType::Entry` or `Entry`), then a short-form
    /// match against the `::Entry` suffix for enum-class enums.
    pub fn value_by_name_str(&self, name: &str) -> Option<i64> {
        // Exact-FName match (fast path).
        let exact = FName::find(name);
        if let Some(e) = self
            .entries()
            .iter()
            .find(|e| e.name == exact && exact.comparison_index.value != 0)
        {
            return Some(e.value);
        }
        // Fallback: compare by string. Handles "Entry" matching
        // "EnumType::Entry" for enum-class enums.
        for e in self.entries() {
            let n = e.name.to_string();
            if n == name {
                return Some(e.value);
            }
            if let Some((_, short)) = n.rsplit_once("::") {
                if short == name {
                    return Some(e.value);
                }
            }
        }
        None
    }

    /// Largest declared value (excluding the auto `_MAX` sentinel UE
    /// appends). Useful for bounds-checking writes from integers.
    pub fn max_enum_value(&self) -> i64 {
        let mut max = i64::MIN;
        // The trailing _MAX entry has the largest value but we skip it.
        let entries = self.entries();
        let usable = if entries
            .last()
            .map(|e| e.name.to_string().ends_with("_MAX"))
            .unwrap_or(false)
        {
            &entries[..entries.len() - 1]
        } else {
            &entries[..]
        };
        for e in usable {
            if e.value > max {
                max = e.value;
            }
        }
        max
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct FStructBaseChain {
    pub struct_base_chain_array: *const *const FStructBaseChain,
    pub num_struct_bases_in_chain_minus_one: i32,
}

/// `FFieldClass` — the runtime class of an `FField`/`FProperty`. `Name` is at
/// offset 0 in every engine build and kept as a real field, but the members
/// after it (`Id`, `ClassFlags`, `CastFlags`, …) were reordered in UE5, so
/// `CastFlags` is read through the active layout rather than a fixed position.
#[derive(Debug)]
#[repr(C)]
pub struct FFieldClass {
    pub name: FName,
    // Remaining members (Id / ClassFlags / CastFlags / SuperClass / …) live past
    // `name` at version-dependent offsets and are read via accessor methods.
}
impl FFieldClass {
    /// `EClassCastFlags` for this field class — the discriminator behind
    /// [`FieldTrait::cast`](FField). Read at the layout offset because its
    /// position shifted between engine generations (0x10 in UE4, 0x18 by 5.7).
    pub fn cast_flags(&self) -> EClassCastFlags {
        unsafe {
            ((self as *const Self as *const u8).add(globals().layout.ffieldclass.cast_flags)
                as *const EClassCastFlags)
                .read()
        }
    }
}

pub enum FFieldOwner<'a> {
    UObject(&'a UObjectBase),
    FField(&'a FFieldBase),
}

/// `FField::Owner` — an `FFieldVariant` pointing at either the owning `UObject`
/// or a parent `FField`. UE4 stores `{ void* Container; bool bIsUObject; }`;
/// UE5 packs the flag into the pointer's low bit (see [`FieldVariantFormat`]).
/// Resolved through the active layout, so this is a view, not a stored struct.
#[derive(Debug, Clone, Copy)]
pub struct FFieldVariant<'a> {
    ptr: *const c_void,
    is_uobject: bool,
    _marker: std::marker::PhantomData<&'a ()>,
}
impl<'a> FFieldVariant<'a> {
    pub fn is_valid(&self) -> bool {
        !self.ptr.is_null()
    }
    pub fn get(&self) -> Option<FFieldOwner<'a>> {
        if self.ptr.is_null() {
            return None;
        }
        Some(if self.is_uobject {
            FFieldOwner::UObject(unsafe { &*(self.ptr as *const UObjectBase) })
        } else {
            FFieldOwner::FField(unsafe { &*(self.ptr as *const FFieldBase) })
        })
    }
    pub fn to_uobject(&self) -> Option<&'a UObjectBase> {
        (self.is_uobject && !self.ptr.is_null())
            .then(|| unsafe { &*(self.ptr as *const UObjectBase) })
    }
    pub fn to_field(&self) -> Option<&'a FFieldBase> {
        (!self.is_uobject && !self.ptr.is_null())
            .then(|| unsafe { &*(self.ptr as *const FFieldBase) })
    }
}

ue_struct! {
    /// `FFieldBase` — root of the FField/FProperty reflection tree. A zero-sized,
    /// layout-dynamic view into engine memory (like the UObject types): all access
    /// is offset-driven through [`FFieldLayout`], so it works across engine versions.
    struct FFieldBase;
    cast FieldTrait = EClassCastFlags::CASTCLASS_None;
    extern_layout ffield: FFieldLayout;
    scalar next: *const FField,
    scalar name_private: FName,
    scalar flags_private as flags: EObjectFlags,
}
impl FFieldBase {
    pub fn class(&self) -> &FFieldClass {
        let p: *const FFieldClass = unsafe { self.read(globals().layout.ffield.class_private) };
        unsafe { p.as_ref().unwrap() }
    }
    pub fn name(&self) -> FName {
        self.name_private()
    }
    pub fn owner(&self) -> FFieldVariant<'_> {
        let l = &globals().layout.ffield;
        let raw: usize = unsafe { self.read(l.owner) };
        let (ptr, is_uobject) = match l.variant {
            FieldVariantFormat::SeparateBool => {
                let is_uobject: bool = unsafe { self.read(l.owner + std::mem::size_of::<usize>()) };
                (raw, is_uobject)
            }
            FieldVariantFormat::TaggedPointer => (raw & !1usize, raw & 1 != 0),
        };
        FFieldVariant {
            ptr: ptr as *const c_void,
            is_uobject,
            _marker: std::marker::PhantomData,
        }
    }
    pub fn is<T: FieldTrait>(&self) -> bool {
        self.class().cast_flags().contains(T::CAST_FLAGS)
    }
    pub fn cast<T: FieldTrait>(&self) -> Option<&T> {
        self.is::<T>()
            .then(|| unsafe { std::mem::transmute::<&Self, &T>(self) })
    }
}
ue_struct! {
    /// `FField` — the concrete leaf of the FField reflection tree; an offset-0
    /// view over [`FFieldBase`].
    struct FField: FFieldBase;
    cast FieldTrait = EClassCastFlags::CASTCLASS_None;
}

ue_struct! {
    /// `UStruct` — a layout-dynamic view into a live engine object. See
    /// the `UE_4_27` offsets for the reference 4.27 layout; only the members
    /// libue reads get accessors.
    struct UStruct: UField;
    cast ObjTrait = EClassCastFlags::CASTCLASS_UStruct;
    layout ustruct: UStructLayout;
    /// `SuperStruct`. Written (via the `super_struct_mut` place) when
    /// manufacturing a private host `UClass` to parent it onto an existing
    /// engine class (e.g. `UObject`) so cast/`IsA` walks terminate correctly
    /// and reflection has a valid super chain. Write only on a
    /// freshly-manufactured struct, before publish, with no concurrent
    /// reflection walks — the exclusivity the `&mut` pedigree proves.
    excl super_struct: *const UStruct,
    scalar children: *const UField,
    scalar child_properties: *const FField,
    excl properties_size: i32,
    /// `MinAlignment`. Copied from a template class (e.g. `UObject`) onto a
    /// manufactured host so `StaticAllocateObject` aligns instances of it
    /// correctly (`Align = max(4, MinAlignment)`). Write only on a fresh,
    /// unpublished struct.
    excl min_alignment: i32,
    /// The compiled Kismet bytecode. Mutable via `script_mut` for callers that
    /// rewrite a function body in place (e.g. bytecode patching); the writer
    /// must hold the owning struct exclusively.
    view_mut script as script: TArray<u8>,
}

impl UStruct {
    pub fn iter_props<'o>(&'o self) -> IterProps<'o> {
        IterProps {
            next_super: self,
            next_field: std::ptr::null(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Iterate only *this* struct's own `ChildProperties` (params + locals),
    /// NOT super-struct properties. [`iter_props`] walks the super chain, which
    /// is right for reflection but wrong for building an execution frame: a
    /// frame is sized to this struct's own `PropertiesSize`, and a super's
    /// property offsets don't index into it. Use this when initializing or
    /// destroying frame locals so an overriding UFunction can't double-visit.
    pub fn own_props(&self) -> impl Iterator<Item = &FProperty> + '_ {
        let mut field = self.child_properties();
        std::iter::from_fn(move || {
            while let Some(f) = unsafe { field.as_ref() } {
                field = f.next();
                if let Some(p) = f.cast::<FProperty>() {
                    return Some(p);
                }
            }
            None
        })
    }

    /// Iterate `UField` children (UFunctions, nested UStructs, UEnums) on
    /// this struct *only*. Use [`UStruct::find_function`] for super-chain
    /// walks.
    pub fn iter_children(&self) -> impl Iterator<Item = &UField> + '_ {
        let mut cur = self.children();
        std::iter::from_fn(move || {
            let f = unsafe { cur.as_ref() }?;
            cur = f.next();
            Some(f)
        })
    }

    /// Walk this struct's `children` chain (and the super chain) looking for
    /// a UFunction with the given FName.
    pub fn find_function(&self, name: FName) -> Option<&UFunction> {
        let mut cur: *const UStruct = self;
        while let Some(s) = unsafe { cur.as_ref() } {
            for field in s.iter_children() {
                if field.name_private() == name {
                    if let Some(func) = field.cast::<UFunction>() {
                        return Some(func);
                    }
                }
            }
            cur = s.super_struct();
        }
        None
    }

    /// Iterate this struct and its ancestors via the super chain, nearest first
    /// (self included).
    pub fn iter_super(&self) -> impl Iterator<Item = &UStruct> + '_ {
        let mut cur: *const UStruct = self;
        std::iter::from_fn(move || {
            let s = unsafe { cur.as_ref() }?;
            cur = s.super_struct();
            Some(s)
        })
    }

    /// Iterate every UFunction on this struct and its ancestors.
    pub fn iter_functions(&self) -> impl Iterator<Item = &UFunction> + '_ {
        self.iter_super()
            .flat_map(|s| s.iter_children())
            .filter_map(|f| f.cast::<UFunction>())
    }

    /// True if `self` is `other` or descends from it through the super chain
    /// (UE's `UStruct::IsChildOf`).
    pub fn is_child_of(&self, other: *const UStruct) -> bool {
        self.iter_super().any(|s| std::ptr::eq(s, other))
    }
}

pub struct IterProps<'o> {
    next_super: *const UStruct,
    next_field: *const FField,
    _marker: std::marker::PhantomData<&'o ()>,
}
impl<'o> Iterator for IterProps<'o> {
    type Item = &'o FField;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(f) = unsafe { self.next_field.as_ref() } {
                self.next_field = f.next();
                return Some(f);
            } else if let Some(s) = unsafe { self.next_super.as_ref() } {
                self.next_field = s.child_properties();
                self.next_super = s.super_struct();
            } else {
                return None;
            }
        }
    }
}

ue_struct! {
    /// `UFunction` — a layout-dynamic view into a live engine object. See
    /// the `UE_4_27` offsets for the reference 4.27 layout; only the members
    /// libue reads get accessors. `func` and `function_flags` are `racy`: we
    /// mutate them on live functions while other threads (the BP VM on worker
    /// threads) read them, so their only idiom is the in-place-atomic
    /// [`Place`] behind the `*_atomic` accessors.
    struct UFunction: UStruct;
    cast ObjTrait = EClassCastFlags::CASTCLASS_UFunction;
    layout ufunction: UFunctionLayout;
    excl parms_size: u16,
    /// `FunctionFlags`. A hook toggling `FUNC_Native` reroutes Blueprint-VM
    /// dispatch of this function through `func` (so a `Func`-swap hook can
    /// intercept it) instead of executing the script body inline via
    /// `ProcessLocalScriptFunction`. A `store` must keep every interleaving
    /// with the current `func` value valid (see the hook install/teardown
    /// orderings in the caller), and the original flags must be restored on
    /// unhook.
    racy function_flags: EFunctionFlags,
    /// The current native entry point (`UFunction::Func`) as a raw address —
    /// for script (BP) functions `UObject::ProcessInternal`, for native
    /// functions the generated `execXxx` thunk. Swapped on live functions to
    /// install/remove hook trampolines; the place's Acquire/Release pairing
    /// means a thread that loads a hook stub's address here also sees the
    /// stub's code bytes and its fully-populated `SinkHeader`. A stored
    /// address must be a valid [`FNativeFuncPtr`] (or the saved original when
    /// un-hooking), and engine threads may invoke either the old or the new
    /// target around the swap — the caller must keep both valid across it.
    racy func as native_func_addr: usize,
}

pub type FNativeFuncPtr = unsafe extern "system" fn(*mut UObject, *mut kismet::FFrame, *mut c_void);
// `func` is declared `usize` above and transits in-place atomics; it must be
// pointer-sized for the address round-trip to be lossless.
const _: () = assert!(std::mem::size_of::<FNativeFuncPtr>() == std::mem::size_of::<usize>());

impl UFunction {
    /// Iterate this function's input/output parameters (`CPF_Parm`), excluding
    /// the return value, in declaration order.
    ///
    /// Own `ChildProperties` only, NOT the super chain: a UFunction that
    /// overrides another has `SuperStruct` set to the overridden function, which
    /// owns its *own duplicate* set of param properties — walking the chain
    /// would yield every parameter twice (and mis-step call-site bytecode when
    /// used to build an execution frame). This matches the engine, whose param
    /// loops iterate `Function->ChildProperties` directly.
    pub fn params(&self) -> impl Iterator<Item = &FProperty> + '_ {
        self.own_props().filter(|p| {
            p.property_flags().contains(EPropertyFlags::CPF_Parm)
                && !p.property_flags().contains(EPropertyFlags::CPF_ReturnParm)
        })
    }

    /// This function's return-value property (`CPF_ReturnParm`), if any. Own
    /// properties only, like [`params`](Self::params).
    pub fn return_property(&self) -> Option<&FProperty> {
        self.own_props()
            .find(|p| p.property_flags().contains(EPropertyFlags::CPF_ReturnParm))
    }

    /// Copy `sig`'s frame *sizes* (`ParmsSize`, `PropertiesSize`) onto this
    /// function so `ProcessEvent` allocates a frame of the right size and memcpys
    /// the broadcast's argument block into it. Deliberately does **not** touch
    /// `ChildProperties`: a manufactured sink must keep a null property chain so
    /// its `UStruct` destructor never frees properties it doesn't own (they
    /// belong to `sig`). Arg *reading* is done against `sig`'s own properties, so
    /// the sink never needs its own chain.
    pub fn copy_param_sizes(&mut self, sig: &UFunction) {
        let props = sig.properties_size();
        let parms = sig.parms_size();
        self.properties_size_mut().set(props);
        self.parms_size_mut().set(parms);
    }
}

ue_struct! {
    /// `UClass` — a layout-dynamic view into a live engine object. Carries no
    /// `#[repr(C)]` layout of its own: it is a zero-sized handle held behind a
    /// pointer into engine memory, with members read at the offsets in
    /// [`layout().uclass`](UClassLayout). See the `UE_4_27` offsets for the
    /// reference 4.27 layout; the opaque members (FuncMap, SuperFuncMap,
    /// editor-only fields, …) don't need offsets because libue never reads them.
    struct UClass: UStruct;
    cast ObjTrait = EClassCastFlags::CASTCLASS_UClass;
    layout uclass: UClassLayout;
    /// `ClassFlags`. Written on a manufactured host class (via the
    /// `class_flags_mut` place, game thread, before publish) to OR in
    /// `CLASS_TokenStreamAssembled` so the engine's pre-GC
    /// `AssembleReferenceTokenStreams()` pass skips it — an unlinked
    /// frankenclass has no property chain to assemble, and its deep-copied
    /// stream (see [`UClass::copy_reference_token_stream_from`]) must never be
    /// rebuilt or freed.
    excl class_flags: EClassFlags,
    /// `ClassCastFlags`. A manufactured host class copies these from its
    /// parent (e.g. `UObject`) via the `class_cast_flags_mut` place so
    /// `IsA`/`Cast<>` checks on its instances still classify correctly. Write on
    /// the game thread, before publish.
    excl class_cast_flags: EClassCastFlags,
    /// `ClassAddReferencedObjects`. A manufactured host class copies this
    /// function pointer from its template (`UObject`'s class) so that if any
    /// GC path dispatches through an instance's `Class->
    /// CallAddReferencedObjects`, it lands in a real, safe collector rather
    /// than a null pointer. Write a valid `ClassAddReferencedObjects`-
    /// signature function only (e.g. copied verbatim from an existing class),
    /// game thread, before publish.
    excl class_add_referenced_objects: usize,
    /// `ClassWithin`. A manufactured host class copies this from its template
    /// (`UObject`'s class) so GC's `UClass::AddReferencedObjects` and any
    /// within-checks see a valid pointer rather than null. Write on the game
    /// thread, before publish.
    excl class_within: *const UClass,
    /// `ClassConstructor`. A manufactured host class copies this function
    /// pointer from its template (`UObject`'s class) so the engine can actually
    /// *instantiate* the class — `StaticConstructObject`/`CreateDefaultObject`
    /// invoke it to placement-construct each instance (and the CDO). Write a
    /// valid constructor (copied from an existing class whose instance layout
    /// matches) only, game thread, before publish.
    excl class_constructor: usize,
    scalar class_default_object: *const UObject,
    view interfaces as interfaces_array: TArray<FImplementedInterface>,
    view func_map as func_map_view: TMap<FName, *const UFunction>,
    view_mut reference_token_stream as reference_token_stream_view: TArray<u32>,
}

/// `FImplementedInterface` — one entry in `UClass::Interfaces`. For a native
/// interface, `pointer_offset` is the byte offset of the interface's vtable
/// subobject within an instance; for a K2 (Blueprint) interface it's 0 and
/// `b_implemented_by_k2` is set.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FImplementedInterface {
    pub class: *const UClass,
    pub pointer_offset: i32,
    pub b_implemented_by_k2: bool,
}
const _: () = assert!(std::mem::size_of::<FImplementedInterface>() == 16);

impl UClass {
    /// The interfaces declared directly on this class (not its supers).
    pub fn interfaces(&self) -> &[FImplementedInterface] {
        self.interfaces_array().as_slice()
    }

    /// Give this (manufactured) class a valid GC reference-token stream by
    /// **deep-copying** `template`'s into a fresh `gmalloc` buffer that `self`
    /// owns. The parallel garbage collector dereferences `Tokens.GetData()[0]`
    /// **unconditionally** (`AccessReferenceInfo(0)` with no empty-check), so a
    /// manufactured class must have a non-empty, `GCRT_EndOfStream`-terminated
    /// stream before any instance of it is traced — an empty stream (even one
    /// flagged `CLASS_TokenStreamAssembled`) is a null read that crashes GC.
    /// `template`'s stream (e.g. `UObject`'s: ClassPrivate + OuterPrivate +
    /// package + EndOfStream) is valid for any instance laid out like a
    /// `template` instance — which our sink is: a direct instance of a host
    /// class that derives from `UObject` and adds no properties, so it shares
    /// `UObject`'s exact instance layout.
    ///
    /// The copy is built with [`TArray::from`], which allocates through
    /// `gmalloc` (the engine allocator), so the resulting `TArray<uint32>` is
    /// byte-for-byte what the engine expects and, unlike a shared copy of
    /// `template`'s buffer, cannot be invalidated by anything that touches
    /// `template`'s stream. Returns `false` (writing nothing) if `template`'s
    /// stream is empty/unassembled, so the caller can fail loudly rather than
    /// install a null stream. The caller should still set
    /// `CLASS_TokenStreamAssembled` on `self` so the engine never re-assembles
    /// (and frees) it.
    ///
    /// The `&mut` receiver is the exclusivity proof (game thread, freshly-
    /// manufactured class — see [`handle_mut`](Self::handle_mut)). `template`
    /// must be a live class with an
    /// assembled stream and an instance layout compatible with `self`'s
    /// instances, or GC will mis-trace those instances later.
    #[must_use]
    pub fn copy_reference_token_stream_from(&mut self, template: &UClass) -> bool {
        // Only the UE4 `TArray<uint32>` token stream is copyable. On UE5 the
        // member is a differently-typed schema owner (see [`GcReferenceFormat`]);
        // treating it as a token array would corrupt memory, so refuse.
        if globals().layout.gc_reference_format != crate::GcReferenceFormat::Ue4TokenStream {
            return false;
        }
        // Read the template's stream as the typed container it is.
        let src = template.reference_token_stream_view().as_slice();
        if src.is_empty() {
            return false;
        }
        // Install an owning, `gmalloc`-backed copy. `TArray::from` allocates
        // through the engine allocator, so the result is byte-for-byte what the
        // engine expects and — unlike a shared copy of `template`'s buffer —
        // can't be invalidated by anything that later touches the template's
        // stream. Assigning through the typed field accessor drops our host's
        // current stream first; it's still default-empty here, so that's a
        // no-op `free(null)`.
        *self.reference_token_stream_view_mut() = TArray::from(src);
        true
    }

    /// UE5 counterpart of [`copy_reference_token_stream_from`]: there is nothing
    /// to install. UE5 replaced the `TArray<uint32>` token stream with
    /// `UClass::ReferenceSchema` (`UE::GC::FSchemaOwner`, a ref-counted
    /// `FSchemaView`), but the collector's object loop traces the base references
    /// (`Class`, `Outer`) as *memberless* references handled directly, and guards
    /// member iteration with `if (!Schema.IsEmpty())`. Our sink is a direct
    /// instance of a host class that adds no properties to `UObject`, so it has
    /// no reflected references and an **empty** schema is exactly correct: GC
    /// still traces the sink's class and outer and skips member iteration — no
    /// null-deref (that guard is the UE5 difference from UE4, where an empty
    /// token stream is dereferenced unconditionally).
    ///
    /// A freshly-constructed host already has an empty (`SchemaView == 0`) schema,
    /// so there is nothing to do. The caller still sets `CLASS_TokenStreamAssembled`
    /// so the engine never autogenerates a schema for the unlinked class. Returns
    /// `true` on `Ue5Schema` builds (the empty schema is valid) and `false`
    /// otherwise, mirroring the UE4 gate — this only *validates*, it doesn't copy.
    #[allow(clippy::unused_self)]
    pub fn reference_schema_already_valid(&self) -> bool {
        globals().layout.gc_reference_format == crate::GcReferenceFormat::Ue5Schema
    }

    /// Mutable view over `UClass::FuncMap` (`TMap<FName, UFunction*>`), the map
    /// the engine's `UObject::FindFunction`/`FindFunctionChecked` (and thus
    /// dynamic-delegate dispatch) resolves through. Growth routes through
    /// `gmalloc()` — the engine's own allocator — so entries inserted here are
    /// laid out exactly as the engine expects and are found by `FindRef`.
    ///
    /// The `&mut` receiver is the exclusivity proof (via
    /// [`handle_mut`](Self::handle_mut)); its contract must cover the map for
    /// the whole borrow: game thread only, no other Rust reference to the map
    /// live (including [`func_map_view`](Self::func_map_view)), and no
    /// concurrent engine lookup on this class — a `FindFunction` racing a
    /// rehash reads freed buckets. In practice that means mutating only
    /// classes whose functions the engine resolves on the game thread.
    pub fn func_map_mut(&mut self) -> &mut TMap<FName, *const UFunction> {
        let p = self.base().wrapping_add(globals().layout.uclass.func_map)
            as *mut TMap<FName, *const UFunction>;
        unsafe { &mut *p }
    }

    /// Register `func` under `name` in this class's function map — the engine
    /// equivalent of `UClass::AddFunctionToFunctionMap` (`FuncMap.Add`). After
    /// this, `FindFunction(name)` on instances of this class (own-map lookup, so
    /// no `SuperFuncMap` cache interaction) resolves to `func`. Exclusivity
    /// contract: see [`func_map_mut`](Self::func_map_mut).
    pub fn add_function_to_func_map(&mut self, name: FName, func: *const UFunction) {
        self.func_map_mut().insert(name, func);
    }

    /// Remove a previously-added function-map entry (`FuncMap.Remove`).
    /// Exclusivity contract: see [`func_map_mut`](Self::func_map_mut).
    pub fn remove_function_from_func_map(&mut self, name: &FName) {
        self.func_map_mut().remove(name);
    }

    /// Find a UFunction reachable through one of this class's implemented
    /// interfaces. Native interface functions (e.g. the `IAssetRegistry` query
    /// methods) are declared on the interface UClass, not copied into the
    /// implementing class's function map, so a plain super-chain lookup misses
    /// them. Walks this class's super chain and each interface's super chain.
    pub fn find_interface_function(&self, name: FName) -> Option<&UFunction> {
        let mut cur: *const UClass = self;
        while let Some(c) = unsafe { cur.as_ref() } {
            for entry in c.interfaces() {
                if let Some(ic) = unsafe { entry.class.as_ref() } {
                    // `ic`/`c` deref `UClass -> UStruct` (offset 0).
                    if let Some(f) = ic.find_function(name) {
                        return Some(f);
                    }
                }
            }
            cur = c.super_struct() as *const UClass;
        }
        None
    }

    /// UE's `UClass::ImplementsInterface` — walks this class and its supers
    /// looking for an implemented interface that is `iface` or a child of it.
    pub fn implements_interface(&self, iface: *const UClass) -> bool {
        let mut cur: *const UClass = self;
        while let Some(c) = unsafe { cur.as_ref() } {
            for entry in c.interfaces() {
                if let Some(ic) = unsafe { entry.class.as_ref() } {
                    if ic.is_child_of(iface as *const UStruct) {
                        return true;
                    }
                }
            }
            cur = c.super_struct() as *const UClass;
        }
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct FName {
    pub comparison_index: FNameEntryId,
    pub number: u32,
}

impl FName {
    /// Construct an FName from a string by calling into UE's FName constructor.
    /// `find_type` corresponds to EFindName: 0 = FNAME_Find, 1 = FNAME_Add.
    /// Use FNAME_Find (0) to look up without creating, FNAME_Add (1) to create if missing.
    pub fn from_string(s: &str, find_type: i32) -> Self {
        let mut name = FName {
            comparison_index: FNameEntryId { value: 0 },
            number: 0,
        };
        let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            (globals().fname_ctor_wchar)(&mut name, wide.as_ptr(), find_type);
        }
        name
    }

    /// Look up an existing FName without creating it. Returns None-equivalent
    /// (comparison_index 0, number 0) if the name doesn't exist in the name table.
    pub fn find(s: &str) -> Self {
        Self::from_string(s, 0) // FNAME_Find
    }

    /// `NAME_None` — the null name (comparison index 0). Engine APIs treat this
    /// as "unset" (e.g. `StaticConstructObject_Internal` assigns a unique name).
    pub fn none() -> Self {
        FName {
            comparison_index: FNameEntryId { value: 0 },
            number: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct FNameEntryId {
    pub value: u32,
}

impl GetTypeHash for FNameEntryId {
    fn get_type_hash(&self) -> u32 {
        // Mirrors FNameEntryHandle hash from UnrealNames.cpp
        const BLOCK_OFFSET_BITS: u32 = 16;
        const MAX_BLOCK_BITS: u32 = 13;
        let block = self.value >> BLOCK_OFFSET_BITS;
        let offset = self.value & ((1 << BLOCK_OFFSET_BITS) - 1);
        (block << (32 - MAX_BLOCK_BITS))
            .wrapping_add(block)
            .wrapping_add(offset << BLOCK_OFFSET_BITS)
            .wrapping_add(offset)
            .wrapping_add(offset >> 4)
    }
}

impl GetTypeHash for FName {
    fn get_type_hash(&self) -> u32 {
        self.comparison_index
            .get_type_hash()
            .wrapping_add(self.number)
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct TSharedPtr<T> {
    pub object: *const T,
    pub reference_controller: *const FReferenceControllerBase,
}

#[derive(Debug)]
#[repr(C)]
pub struct FReferenceControllerBase {
    pub shared_reference_count: i32,
    pub weak_reference_count: i32,
}

pub type FString = TArray<u16>;
impl From<&str> for FString {
    fn from(value: &str) -> Self {
        Self::from(
            widestring::U16CString::from_str(value)
                .unwrap()
                .as_slice_with_nul(),
        )
    }
}

impl Display for FString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let slice = self.as_slice();
        let last = slice.len()
            - slice
                .iter()
                .cloned()
                .rev()
                .position(|c| c != 0)
                .unwrap_or_default();
        write!(
            f,
            "{}",
            widestring::U16Str::from_slice(&slice[..last])
                .to_string()
                .unwrap()
        )
    }
}

#[derive(Debug, Default)]
#[repr(C)]
pub struct FVector {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Default)]
#[repr(C)]
pub struct FLinearColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

pub mod kismet {
    use super::*;

    /// `FOutParmRec` — a node in `FFrame::OutParms`, the linked list of output
    /// parameters for the executing function. `prop_addr` is the *caller's*
    /// address for the value (e.g. into the ProcessEvent `Parms` buffer), which
    /// is where out-param results must be written to reach the caller.
    #[derive(Debug)]
    #[repr(C)]
    pub struct FOutParmRec {
        pub property: *const FProperty,
        pub prop_addr: *mut u8,
        pub next_out_parm: *const FOutParmRec,
    }

    ue_struct! {
        /// `FFrame` — a Kismet VM execution frame. A zero-sized, layout-dynamic
        /// view (UE5 inserted members, shifting `OutParms`/`CurrentNativeFunction`
        /// and growing the frame; see [`FFrameLayout`]). Build a synthetic frame
        /// with [`FFrameBuf`]. The fields are `excl`: a frame is either an
        /// owned buffer ([`FFrameBuf`]) or a live frame the executing thread
        /// exclusively drives, so `&mut` is always honestly available.
        struct FFrame;
        extern_layout fframe: FFrameLayout;
        excl node: *const c_void,
        excl object: *mut UObject,
        excl code: *const c_void,
        excl locals: *const c_void,
        excl most_recent_property: *const FProperty,
        excl most_recent_property_address: *const c_void,
        excl out_parms: *const c_void,
        excl property_chain_for_compiled_in: *const FField,
        excl current_native_function: *const c_void,
    }

    /// An owned, engine-sized `FFrame` buffer for building a synthetic execution
    /// frame: copy a live frame, then overwrite fields through [`FFrame`]'s
    /// `*_mut` places. The buffer is sized from the active layout so the engine
    /// reads the fields at the offsets it expects.
    pub struct FFrameBuf {
        buf: Vec<u8>,
    }
    impl FFrameBuf {
        /// # Safety
        /// `frame` must be a live execution frame at least `layout.fframe.size` bytes.
        pub unsafe fn from_frame(frame: &FFrame) -> Self {
            let size = globals().layout.fframe.size;
            let mut buf = vec![0u8; size];
            unsafe {
                std::ptr::copy_nonoverlapping(frame.base(), buf.as_mut_ptr(), size);
            }
            Self { buf }
        }
        pub fn frame_mut(&mut self) -> &mut FFrame {
            unsafe { &mut *(self.buf.as_mut_ptr() as *mut FFrame) }
        }
    }

    /// `FFrame::Step(Context, Result)` — evaluate the next bytecode expression
    /// into `result`, advancing `stack.code` and updating `MostRecentProperty*`.
    ///
    /// Reimplemented from the engine's one-liner rather than resolved by address
    /// (its tiny body doesn't pattern-match reliably across builds):
    ///
    /// ```cpp
    /// void FFrame::Step(UObject* Context, RESULT_DECL) {
    ///     int32 B = *Code++;
    ///     (GNatives[B])(Context, *this, RESULT_PARAM);
    /// }
    /// ```
    ///
    /// The opcode byte indexes [`GNatives`] (the VM dispatch table we already
    /// hold), and every effect on the frame — `MostRecentProperty*`, further
    /// `Code` advancement — happens inside the native handler we call.
    ///
    /// # Safety
    /// `stack` must be a live execution frame positioned on an expression, and
    /// `result` must point at writable storage for the expression's property type.
    pub unsafe fn step(stack: &mut FFrame, context: *mut UObject, result: *mut c_void) {
        unsafe {
            let code = stack.code() as *const u8;
            let opcode = *code;
            stack.code_mut().set(code.add(1) as *const c_void);

            // Index the table by raw pointer arithmetic to mirror `GNatives[B]`
            // exactly — the engine array is `EX_Max` (0x100) entries, one past
            // our `[_; 0xff]` view, so a 0xff opcode must not bounds-panic.
            let table = globals().gnatives as *const Option<FNativesFuncPtr>;
            let handler = (*table.add(opcode as usize)).expect("GNatives dispatch entry is null");
            handler(context, stack, result);
        }
    }
}
