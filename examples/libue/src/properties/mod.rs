mod array;
mod map;
mod set;
mod r#struct;

pub use array::*;
pub use map::*;
pub use set::*;
pub use r#struct::*;

use super::*;

pub trait PropTrait: FieldTrait + Deref<Target = FProperty> {
    type PropValue<'o>
    where
        Self: 'o;
    type PropValueMut<'o>
    where
        Self: 'o;

    /// # Safety
    /// `object` must point to a valid instance containing this property.
    unsafe fn value_obj<'o>(&'o self, object: *const ()) -> Self::PropValue<'o> {
        unsafe { self.value(object.byte_offset(self.offset_internal() as isize).cast()) }
    }
    /// # Safety
    /// `object` must point to a valid instance containing this property.
    unsafe fn value_obj_mut<'o>(&'o self, object: *mut ()) -> Self::PropValueMut<'o> {
        unsafe { self.value_mut(object.byte_offset(self.offset_internal() as isize).cast()) }
    }

    /// # Safety
    /// `data` must point to a valid value of this property's underlying type.
    unsafe fn value<'o>(&'o self, data: *const ()) -> Self::PropValue<'o>;
    /// # Safety
    /// `data` must point to a valid value of this property's underlying type.
    unsafe fn value_mut<'o>(&'o self, data: *mut ()) -> Self::PropValueMut<'o>;

    //     unsafe fn value_mut<'o>(&self, object: &'o mut UObjectBase) -> &'o mut Self::PropValue {
    //         &mut *std::ptr::from_mut(object)
    //             .byte_offset(self.offset_internal() as isize)
    //             .cast()
    //     }
    //     unsafe fn value_mut_ptr<'o>(&self, object: *mut UObjectBase) -> &'o mut Self::PropValue {
    //         &mut *object.byte_offset(self.offset_internal() as isize).cast()
    //     }
}

/// Test-only mock of an `FSetProperty`: a backing byte buffer with the fields
/// the container ops read (`element_size`, the self-referential element
/// property pointer, and `set_layout`) written at the active layout's offsets.
/// Replaces the old by-value struct construction now that properties are ZSTs.
#[cfg(test)]
pub(crate) struct MockSetProperty {
    buf: Box<[u8; 0x100]>,
}
#[cfg(test)]
impl MockSetProperty {
    pub fn new(element_size: i32, set_layout: FScriptSetLayout) -> Self {
        let mut buf = Box::new([0u8; 0x100]);
        let l = &globals().layout;
        let base = buf.as_mut_ptr();
        unsafe {
            (base.add(l.fproperty.element_size) as *mut i32).write(element_size);
            // Element property points back at this buffer's FProperty base.
            (base.add(l.fsetproperty.element_prop) as *mut *const FProperty)
                .write(base as *const FProperty);
            (base.add(l.fsetproperty.set_layout) as *mut FScriptSetLayout).write(set_layout);
        }
        Self { buf }
    }
    pub fn get(&self) -> &FSetProperty {
        unsafe { &*(self.buf.as_ptr() as *const FSetProperty) }
    }
}

ue_struct! {
    /// `FProperty` — a layout-dynamic view. UE5 both shifted its members (smaller
    /// `FFieldVariant`) and reordered them, so all access is offset-driven via
    /// [`FPropertyLayout`]. The offset primitives (`base`/`read`/`read_ref`) come
    /// from [`LayoutView`] and are reused by every `FProperty` subtype via `Deref`.
    struct FProperty: FField;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FProperty;
    extern_layout fproperty: FPropertyLayout;
    scalar array_dim: i32,
    scalar element_size: i32,
    scalar property_flags: EPropertyFlags,
    scalar offset_internal: i32,
}

impl FProperty {
    pub fn offset(&self) -> i32 {
        self.offset_internal()
    }

    pub fn cast_field<T: FieldTrait>(&self) -> Option<&T> {
        FFieldBase::cast(self)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Per-property virtual ops dispatched through FProperty's vtable. The slot
// offsets are engine-build specific and live in `layout().vtable` (the
// non-virtual public wrappers below replicate UE's `ArrayDim` looping and the
// `CPF_ZeroConstructor`/`CPF_NoDestructor` short circuits around them).
// ──────────────────────────────────────────────────────────────────────────────

impl FProperty {
    /// Read the function pointer at `offset` of this FProperty's vtable.
    /// The vtable pointer is the FField base's first word.
    unsafe fn vtable_slot(&self, offset: usize) -> usize {
        unsafe {
            let vt = (self as *const Self as *const *const u8).read();
            (vt.byte_add(offset) as *const usize).read()
        }
    }

    /// `FProperty::InitializeValue(Dest)` — initializes `ArrayDim`
    /// consecutive elements at `Dest`. For `CPF_ZeroConstructor` types the
    /// default state is all-zeros, so we memzero the slot (mirroring UE's
    /// `FScriptMapHelper::ConstructItem`). Callers like `add_uninitialized`
    /// hand us genuinely uninitialized memory, so we must zero it ourselves
    /// rather than assume it already is — otherwise a later drop-on-assign
    /// (e.g. FString) frees a garbage pointer.
    ///
    /// # Safety
    /// `dest` must point at `ArrayDim * ElementSize` writable bytes that are
    /// either freshly allocated or have already been destroyed.
    pub unsafe fn initialize_value(&self, dest: *mut std::ffi::c_void) {
        if self
            .property_flags()
            .contains(EPropertyFlags::CPF_ZeroConstructor)
        {
            unsafe {
                std::ptr::write_bytes(
                    dest as *mut u8,
                    0,
                    self.array_dim() as usize * self.element_size() as usize,
                );
            }
            return;
        }
        unsafe {
            let f: unsafe extern "system" fn(*const Self, *mut std::ffi::c_void) =
                std::mem::transmute(
                    self.vtable_slot(globals().layout.vtable.prop_initialize_value_internal),
                );
            for i in 0..self.array_dim() as usize {
                let slot = (dest as *mut u8).add(i * self.element_size() as usize)
                    as *mut std::ffi::c_void;
                f(self, slot);
            }
        }
    }

    /// `FProperty::DestroyValue(Dest)` — destroys `ArrayDim` elements.
    /// No-op for `CPF_NoDestructor` types.
    ///
    /// # Safety
    /// `dest` must point at `ArrayDim * ElementSize` initialized bytes.
    pub unsafe fn destroy_value(&self, dest: *mut std::ffi::c_void) {
        if self
            .property_flags()
            .contains(EPropertyFlags::CPF_NoDestructor)
        {
            return;
        }
        unsafe {
            let f: unsafe extern "system" fn(*const Self, *mut std::ffi::c_void) =
                std::mem::transmute(
                    self.vtable_slot(globals().layout.vtable.prop_destroy_value_internal),
                );
            for i in 0..self.array_dim() as usize {
                let slot = (dest as *mut u8).add(i * self.element_size() as usize)
                    as *mut std::ffi::c_void;
                f(self, slot);
            }
        }
    }

    /// `FProperty::ClearValue(Dest)` — resets to default initial state for
    /// each of the `ArrayDim` elements (e.g. FString → empty, TArray → empty).
    /// Unlike `destroy_value`, the slot remains initialized.
    ///
    /// # Safety
    /// `dest` must point at `ArrayDim * ElementSize` initialized bytes.
    pub unsafe fn clear_value(&self, dest: *mut std::ffi::c_void) {
        unsafe {
            let f: unsafe extern "system" fn(*const Self, *mut std::ffi::c_void) =
                std::mem::transmute(
                    self.vtable_slot(globals().layout.vtable.prop_clear_value_internal),
                );
            for i in 0..self.array_dim() as usize {
                let slot = (dest as *mut u8).add(i * self.element_size() as usize)
                    as *mut std::ffi::c_void;
                f(self, slot);
            }
        }
    }

    /// `FProperty::CopyCompleteValue(Dest, Src)` — deep-copies `ArrayDim`
    /// elements. Handles all property types correctly (FString allocates a
    /// fresh buffer for `Dest`, TArray clones storage, structs recurse).
    ///
    /// # Safety
    /// `dst` must be initialized (e.g. via [`initialize_value`]) and writable;
    /// `src` must point at a valid value of this property's type.
    pub unsafe fn copy_complete_value(
        &self,
        dst: *mut std::ffi::c_void,
        src: *const std::ffi::c_void,
    ) {
        unsafe {
            let f: unsafe extern "system" fn(
                *const Self,
                *mut std::ffi::c_void,
                *const std::ffi::c_void,
                i32,
            ) = std::mem::transmute(
                self.vtable_slot(globals().layout.vtable.prop_copy_values_internal),
            );
            f(self, dst, src, self.array_dim());
        }
    }

    /// `FProperty::GetValueTypeHash(Src)` — single-value hash matching UE's
    /// hashing for TMap/TSet bucketing.
    ///
    /// # Safety
    /// `src` must point at a valid value of this property's type.
    pub unsafe fn get_value_type_hash(&self, src: *const std::ffi::c_void) -> u32 {
        unsafe {
            let f: unsafe extern "system" fn(*const Self, *const std::ffi::c_void) -> u32 =
                std::mem::transmute(
                    self.vtable_slot(globals().layout.vtable.prop_get_value_type_hash_internal),
                );
            f(self, src)
        }
    }

    /// `FProperty::Identical(A, B, PortFlags=0)` — per-property equality.
    ///
    /// # Safety
    /// `a` and `b` must point at valid values of this property's type.
    pub unsafe fn identical(&self, a: *const std::ffi::c_void, b: *const std::ffi::c_void) -> bool {
        unsafe {
            let f: unsafe extern "system" fn(
                *const Self,
                *const std::ffi::c_void,
                *const std::ffi::c_void,
                u32,
            ) -> bool =
                std::mem::transmute(self.vtable_slot(globals().layout.vtable.prop_identical));
            f(self, a, b, 0)
        }
    }
}

#[derive(Debug)]
pub struct FPropertyData<'o, T>(&'o T);
impl<T> Deref for FPropertyData<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

#[derive(Debug)]
pub struct FPropertyDataMut<'o, T>(&'o mut T);
impl<T> Deref for FPropertyDataMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}
impl<T> DerefMut for FPropertyDataMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
    }
}

// Plain by-value properties — an `FProperty` view whose value is a single
// scalar; the `value` clause emits the standard `FPropertyData<T>` wrapper.
ue_struct! {
    /// `FStrProperty` — an `FString` value.
    struct FStrProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FStrProperty;
    value FString;
}
ue_struct! {
    /// `FNameProperty` — an `FName` value.
    struct FNameProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FNameProperty;
    value FName;
}
// `FText` is a refcounted `TSharedRef<ITextData>`, not a plain-copy value, so
// there is no `value` clause (and thus no `FPropertyData<FText>`): its lifetime
// is managed exclusively through the inherited `FProperty` vtable ops
// (`copy_complete_value` = `FText::operator=`, `destroy_value` = `~FText`).
// Callers marshal it as an opaque owning handle — see py_repl's `ue.Text`.
ue_struct! {
    /// `FTextProperty` — an `FText` value.
    struct FTextProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FTextProperty;
}
ue_struct! {
    /// `FInt8Property` — an `i8` value.
    struct FInt8Property: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FInt8Property;
    value i8;
}
ue_struct! {
    /// `FInt16Property` — an `i16` value.
    struct FInt16Property: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FInt16Property;
    value i16;
}
ue_struct! {
    /// `FIntProperty` — an `i32` value.
    struct FIntProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FIntProperty;
    value i32;
}
ue_struct! {
    /// `FInt64Property` — an `i64` value.
    struct FInt64Property: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FInt64Property;
    value i64;
}
// FByteProperty: u8 storage, plus an optional UEnum pointer for the
// legacy TEnumAsByte<E> pattern.
ue_struct! {
    /// `FByteProperty` — a `u8` value, optionally a `TEnumAsByte<E>`.
    struct FByteProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FByteProperty;
    value u8;
    extern_layout fbyteproperty: FBytePropertyLayout;
    /// `UEnum*` for a `TEnumAsByte<E>` property, or None for a plain `uint8`.
    ptr enum_ as enum_ptr: UEnum,
}
ue_struct! {
    /// `FUInt16Property` — a `u16` value.
    struct FUInt16Property: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FUInt16Property;
    value u16;
}
ue_struct! {
    /// `FUInt32Property` — a `u32` value.
    struct FUInt32Property: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FUInt32Property;
    value u32;
}
ue_struct! {
    /// `FUInt64Property` — a `u64` value.
    struct FUInt64Property: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FUInt64Property;
    value u64;
}
ue_struct! {
    /// `FFloatProperty` — an `f32` value.
    struct FFloatProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FFloatProperty;
    value f32;
}
ue_struct! {
    /// `FDoubleProperty` — an `f64` value.
    struct FDoubleProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FDoubleProperty;
    value f64;
}
// FObjectPropertyBase: has PropertyClass pointer
ue_struct! {
    /// `FObjectPropertyBase` — base of the object-pointer properties; holds the
    /// `UClass*` the referenced object must be.
    struct FObjectPropertyBase: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FObjectPropertyBase;
    extern_layout fobjectproperty: FObjectPropertyLayout;
    ptr property_class: UClass,
}

// FObjectProperty: same layout as FObjectPropertyBase
ue_struct! {
    /// `FObjectProperty` — a `UObject*` value.
    struct FObjectProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FObjectProperty;
    extern_layout fobjectproperty: FObjectPropertyLayout;
    ptr property_class: UClass,
}
impl PropTrait for FObjectProperty {
    type PropValue<'o> = FPropertyData<'o, Option<&'o UObject>>;
    type PropValueMut<'o> = FPropertyDataMut<'o, Option<&'o UObject>>;

    unsafe fn value<'o>(&'o self, data: *const ()) -> Self::PropValue<'o> {
        unsafe { FPropertyData(&*data.cast()) }
    }
    unsafe fn value_mut<'o>(&'o self, data: *mut ()) -> Self::PropValueMut<'o> {
        unsafe { FPropertyDataMut(&mut *data.cast()) }
    }
}

// FClassProperty: adds MetaClass (second trailing pointer)
ue_struct! {
    /// `FClassProperty` — a `UClass*` value constrained to `meta_class`.
    struct FClassProperty: FObjectProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FClassProperty;
    extern_layout fclassproperty: FClassPropertyLayout;
    ptr meta_class: UClass,
}

// FSoftObjectProperty: same layout as FObjectPropertyBase
ue_struct! {
    /// `FSoftObjectProperty` — a soft (path-based) object reference.
    struct FSoftObjectProperty: FObjectPropertyBase;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FSoftObjectProperty;
}

// FSoftClassProperty: adds MetaClass
ue_struct! {
    /// `FSoftClassProperty` — a soft class reference constrained to `meta_class`.
    struct FSoftClassProperty: FSoftObjectProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FSoftClassProperty;
    extern_layout fclassproperty: FClassPropertyLayout;
    ptr meta_class: UClass,
}

// FWeakObjectProperty: no extra fields
ue_struct! {
    /// `FWeakObjectProperty` — a weak object reference.
    struct FWeakObjectProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FWeakObjectProperty;
}

// FInterfaceProperty: has InterfaceClass
ue_struct! {
    /// `FInterfaceProperty` — a `TScriptInterface` value; `interface_class` is
    /// the required interface `UClass`.
    struct FInterfaceProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FInterfaceProperty;
    extern_layout finterfaceproperty: FInterfacePropertyLayout;
    ptr interface_class: UClass,
}

// FDelegateProperty: has SignatureFunction
ue_struct! {
    /// `FDelegateProperty` — a single-cast script delegate.
    struct FDelegateProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FDelegateProperty;
    extern_layout fdelegateproperty: FDelegatePropertyLayout;
    /// The UFunction describing this delegate's signature (its parameters).
    ptr signature_function: UFunction,
}

// FMulticastDelegateProperty: has SignatureFunction
ue_struct! {
    /// `FMulticastDelegateProperty` — a multicast script delegate (inline or sparse).
    struct FMulticastDelegateProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FMulticastDelegateProperty;
    extern_layout fdelegateproperty: FDelegatePropertyLayout;
    /// The UFunction describing this delegate's signature (its parameters).
    ptr signature_function: UFunction,
}
impl FMulticastDelegateProperty {
    /// `GetMulticastDelegate(PropertyValue)` — the current invocation list.
    /// For inline storage this is the `FMulticastScriptDelegate*` living at
    /// `property_value`; for sparse storage the engine resolves the owner and
    /// materializes the list from its global side table. Returns null when the
    /// delegate is unbound.
    ///
    /// # Safety
    /// `property_value` must point at this property's value (an
    /// `FMulticastScriptDelegate` for inline, an `FSparseDelegate` for sparse).
    pub unsafe fn get_multicast_delegate(
        &self,
        property_value: *const std::ffi::c_void,
    ) -> *const FMulticastScriptDelegate {
        // Inline storage keeps the `FMulticastScriptDelegate` right at the
        // property value, so read it directly and skip the vtable — this keeps
        // reads working on engine builds whose `mcast_*` vtable slots we haven't
        // mapped (they shift as `FProperty` gains virtuals across versions).
        if self.is_inline() {
            return property_value as *const FMulticastScriptDelegate;
        }
        unsafe {
            let f: unsafe extern "system" fn(
                *const Self,
                *const std::ffi::c_void,
            ) -> *const FMulticastScriptDelegate = std::mem::transmute(
                self.vtable_slot(globals().layout.vtable.mcast_get_multicast_delegate),
            );
            f(self, property_value)
        }
    }

    /// Whether this is an *inline* multicast delegate (value stored in place)
    /// rather than a *sparse* one (kept in the owner's side table). Inline
    /// operations are done directly on the `FMulticastScriptDelegate`; sparse
    /// ones still go through the engine's vtable methods.
    fn is_inline(&self) -> bool {
        self.class()
            .cast_flags()
            .contains(EClassCastFlags::CASTCLASS_FMulticastInlineDelegateProperty)
    }

    /// `AddDelegate(ScriptDelegate, Parent, PropertyValue)` — add a binding
    /// (deduplicated, like `AddUnique`). Pass both `parent` (the owning UObject)
    /// and `property_value`; with both supplied the engine's internal
    /// reference-resolution is a no-op for inline and sparse alike.
    ///
    /// # Safety
    /// `parent`/`property_value` must be the owning object and its value slot
    /// for this property.
    pub unsafe fn add_delegate(
        &self,
        delegate: FScriptDelegate,
        parent: *mut UObjectBase,
        property_value: *mut std::ffi::c_void,
    ) {
        // Inline: `AddUnique` directly on the in-place delegate (the engine's
        // inline `AddDelegate` does exactly this), avoiding the vtable slot.
        if self.is_inline() {
            unsafe { (*(property_value as *mut FMulticastScriptDelegate)).add_unique(delegate) };
            return;
        }
        unsafe {
            let f: unsafe extern "system" fn(
                *const Self,
                FScriptDelegate,
                *mut UObjectBase,
                *mut std::ffi::c_void,
            ) = std::mem::transmute(self.vtable_slot(globals().layout.vtable.mcast_add_delegate));
            f(self, delegate, parent, property_value);
        }
    }

    /// `RemoveDelegate(ScriptDelegate, Parent, PropertyValue)` — remove a
    /// matching binding.
    ///
    /// # Safety
    /// See [`add_delegate`].
    pub unsafe fn remove_delegate(
        &self,
        delegate: &FScriptDelegate,
        parent: *mut UObjectBase,
        property_value: *mut std::ffi::c_void,
    ) {
        if self.is_inline() {
            unsafe {
                (*(property_value as *mut FMulticastScriptDelegate)).remove_matching(delegate)
            };
            return;
        }
        unsafe {
            let f: unsafe extern "system" fn(
                *const Self,
                *const FScriptDelegate,
                *mut UObjectBase,
                *mut std::ffi::c_void,
            ) = std::mem::transmute(
                self.vtable_slot(globals().layout.vtable.mcast_remove_delegate),
            );
            f(self, delegate, parent, property_value);
        }
    }

    /// `ClearDelegate(Parent, PropertyValue)` — remove all bindings.
    ///
    /// # Safety
    /// See [`add_delegate`].
    pub unsafe fn clear_delegate(
        &self,
        parent: *mut UObjectBase,
        property_value: *mut std::ffi::c_void,
    ) {
        if self.is_inline() {
            unsafe { (*(property_value as *mut FMulticastScriptDelegate)).clear() };
            return;
        }
        unsafe {
            let f: unsafe extern "system" fn(*const Self, *mut UObjectBase, *mut std::ffi::c_void) =
                std::mem::transmute(self.vtable_slot(globals().layout.vtable.mcast_clear_delegate));
            f(self, parent, property_value);
        }
    }
}

// FMulticastInlineDelegateProperty: same layout as FMulticastDelegateProperty
ue_struct! {
    /// `FMulticastInlineDelegateProperty` — inline-storage multicast delegate.
    struct FMulticastInlineDelegateProperty: FMulticastDelegateProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FMulticastInlineDelegateProperty;
}

// FMulticastSparseDelegateProperty: same layout as FMulticastDelegateProperty
ue_struct! {
    /// `FMulticastSparseDelegateProperty` — sparse (side-table) multicast delegate.
    struct FMulticastSparseDelegateProperty: FMulticastDelegateProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FMulticastSparseDelegateProperty;
}

// FEnumProperty: has UnderlyingProp and Enum
ue_struct! {
    /// `FEnumProperty` — an enum value stored via an inner numeric property.
    struct FEnumProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FEnumProperty;
    extern_layout fenumproperty: FEnumPropertyLayout;
    /// Inner numeric property (FByteProperty / FIntProperty / etc.) whose
    /// `element_size` and signedness tell us how to read the raw value.
    ptr_req underlying_prop: FProperty,
    ptr enum_ as enum_ptr: UEnum,
}
impl FEnumProperty {
    /// Read the underlying numeric at `data` (whatever its width), widened
    /// to i64. Mirrors `FNumericProperty::GetSignedIntPropertyValue`.
    ///
    /// # Safety
    /// `data` must point to a valid property slot of this enum's type.
    pub unsafe fn read_value(&self, data: *const ()) -> i64 {
        let inner = self.underlying_prop();
        let cast_flags = inner.class().cast_flags();
        use EClassCastFlags as F;
        unsafe {
            if cast_flags.contains(F::CASTCLASS_FByteProperty) {
                (data as *const u8).read() as i64
            } else if cast_flags.contains(F::CASTCLASS_FInt8Property) {
                (data as *const i8).read() as i64
            } else if cast_flags.contains(F::CASTCLASS_FInt16Property) {
                (data as *const i16).read() as i64
            } else if cast_flags.contains(F::CASTCLASS_FUInt16Property) {
                (data as *const u16).read() as i64
            } else if cast_flags.contains(F::CASTCLASS_FIntProperty) {
                (data as *const i32).read() as i64
            } else if cast_flags.contains(F::CASTCLASS_FUInt32Property) {
                (data as *const u32).read() as i64
            } else if cast_flags.contains(F::CASTCLASS_FInt64Property) {
                (data as *const i64).read()
            } else if cast_flags.contains(F::CASTCLASS_FUInt64Property) {
                (data as *const u64).read() as i64
            } else {
                // Shouldn't happen — UE only generates FEnumProperty over
                // FNumericProperty subclasses. Fall back to byte read.
                (data as *const u8).read() as i64
            }
        }
    }

    /// Write `value` truncated to the underlying numeric width.
    ///
    /// # Safety
    /// `data` must point to a writable property slot of this enum's type.
    pub unsafe fn write_value(&self, data: *mut (), value: i64) {
        let inner = self.underlying_prop();
        let cast_flags = inner.class().cast_flags();
        use EClassCastFlags as F;
        unsafe {
            if cast_flags.contains(F::CASTCLASS_FByteProperty) {
                (data as *mut u8).write(value as u8);
            } else if cast_flags.contains(F::CASTCLASS_FInt8Property) {
                (data as *mut i8).write(value as i8);
            } else if cast_flags.contains(F::CASTCLASS_FInt16Property) {
                (data as *mut i16).write(value as i16);
            } else if cast_flags.contains(F::CASTCLASS_FUInt16Property) {
                (data as *mut u16).write(value as u16);
            } else if cast_flags.contains(F::CASTCLASS_FIntProperty) {
                (data as *mut i32).write(value as i32);
            } else if cast_flags.contains(F::CASTCLASS_FUInt32Property) {
                (data as *mut u32).write(value as u32);
            } else if cast_flags.contains(F::CASTCLASS_FInt64Property) {
                (data as *mut i64).write(value);
            } else if cast_flags.contains(F::CASTCLASS_FUInt64Property) {
                (data as *mut u64).write(value as u64);
            } else {
                (data as *mut u8).write(value as u8);
            }
        }
    }
}

// FBoolProperty requires special bitfield handling. Its four u8 bitfield
// members sit immediately after the FProperty base (`size + 0..4`).
ue_struct! {
    /// `FBoolProperty` — a bitfield/bool value described by four `u8` fields.
    struct FBoolProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FBoolProperty;
    extern_layout fboolproperty: FBoolPropertyLayout;
    scalar field_size: u8,
    scalar byte_offset: u8,
    scalar byte_mask: u8,
    scalar field_mask: u8,
}

pub struct FBoolPropertyData<'o> {
    property: &'o FBoolProperty,
    data_ptr: *const u8,
}

impl<'o> FBoolPropertyData<'o> {
    pub fn get(&self) -> bool {
        unsafe {
            let byte_value = *self.data_ptr.add(self.property.byte_offset() as usize);
            (byte_value & self.property.byte_mask()) != 0
        }
    }
}

pub struct FBoolPropertyDataMut<'o> {
    property: &'o FBoolProperty,
    data_ptr: *mut u8,
}

impl<'o> FBoolPropertyDataMut<'o> {
    pub fn get(&self) -> bool {
        unsafe {
            let byte_value = *self.data_ptr.add(self.property.byte_offset() as usize);
            (byte_value & self.property.byte_mask()) != 0
        }
    }

    pub fn set(&mut self, value: bool) {
        unsafe {
            let byte_ptr = self.data_ptr.add(self.property.byte_offset() as usize);
            let mut byte_value = *byte_ptr;
            if value {
                byte_value |= self.property.byte_mask();
            } else {
                byte_value &= !self.property.byte_mask();
            }
            *byte_ptr = byte_value;
        }
    }
}

impl PropTrait for FBoolProperty {
    type PropValue<'o> = FBoolPropertyData<'o>;
    type PropValueMut<'o> = FBoolPropertyDataMut<'o>;

    unsafe fn value<'o>(&'o self, data: *const ()) -> Self::PropValue<'o> {
        FBoolPropertyData {
            property: self,
            data_ptr: data.cast(),
        }
    }

    unsafe fn value_mut<'o>(&'o self, data: *mut ()) -> Self::PropValueMut<'o> {
        FBoolPropertyDataMut {
            property: self,
            data_ptr: data.cast(),
        }
    }
}
