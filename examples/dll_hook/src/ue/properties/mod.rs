mod array;
mod set;
mod r#struct;

pub use array::*;
pub use r#struct::*;
pub use set::*;

use super::*;

pub trait PropTrait: FieldTrait + Deref<Target = FProperty> {
    type PropValue<'o>
    where
        Self: 'o;
    type PropValueMut<'o>
    where
        Self: 'o;

    unsafe fn value_obj<'o>(&'o self, object: *const ()) -> Self::PropValue<'o> {
        self.value(object.byte_offset(self.offset_internal as isize).cast())
    }
    unsafe fn value_obj_mut<'o>(&'o self, object: *mut ()) -> Self::PropValueMut<'o> {
        self.value_mut(object.byte_offset(self.offset_internal as isize).cast())
    }

    unsafe fn value<'o>(&'o self, data: *const ()) -> Self::PropValue<'o>;
    unsafe fn value_mut<'o>(&'o self, data: *mut ()) -> Self::PropValueMut<'o>;

    //     unsafe fn value_mut<'o>(&self, object: &'o mut UObjectBase) -> &'o mut Self::PropValue {
    //         &mut *std::ptr::from_mut(object)
    //             .byte_offset(self.offset_internal as isize)
    //             .cast()
    //     }
    //     unsafe fn value_mut_ptr<'o>(&self, object: *mut UObjectBase) -> &'o mut Self::PropValue {
    //         &mut *object.byte_offset(self.offset_internal as isize).cast()
    //     }
}

impl_deref!(FProperty, ffield: FField);
unsafe impl FieldTrait for FProperty {
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_FProperty;
}
#[derive(Debug)]
#[repr(C)]
pub struct FProperty {
    ffield: FField,
    array_dim: i32,
    element_size: i32,
    property_flags: EPropertyFlags,
    rep_index: u16,
    blueprint_replication_condition: u16, // TEnumAsByte<enum ELifetimeCondition>,
    offset_internal: i32,
    rep_notify_func: FName,
    property_link_next: *const FProperty,
    next_ref: *const FProperty,
    destructor_link_next: *const FProperty,
    post_construct_link_next: *const FProperty,
}

impl FProperty {
    pub fn offset(&self) -> i32 {
        self.offset_internal
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

macro_rules! impl_basic_prop {
    ($name:ident, $value:ty, $cast_flag:ident) => {
        impl_deref!($name, fproperty: FProperty);
        unsafe impl FieldTrait for $name {
            const CAST_FLAGS: EClassCastFlags = EClassCastFlags::$cast_flag;
        }
        #[derive(Debug)]
        #[repr(C)]
        pub struct $name {
            fproperty: FProperty,
        }
        impl PropTrait for $name {
            type PropValue<'o> = FPropertyData<'o, $value>;
            type PropValueMut<'o> = FPropertyDataMut<'o, $value>;

            unsafe fn value<'o>(&'o self, data: *const ()) -> Self::PropValue<'o> {
                FPropertyData(&*data.cast())
            }
            unsafe fn value_mut<'o>(&'o self, data: *mut ()) -> Self::PropValueMut<'o> {
                FPropertyDataMut(&mut *data.cast())
            }
        }
    };
}
impl_basic_prop!(FStrProperty, FString, CASTCLASS_FStrProperty);
impl_basic_prop!(FNameProperty, FName, CASTCLASS_FNameProperty);
impl_basic_prop!(FInt8Property, i8, CASTCLASS_FInt8Property);
impl_basic_prop!(FInt16Property, i16, CASTCLASS_FInt16Property);
impl_basic_prop!(FIntProperty, i32, CASTCLASS_FIntProperty);
impl_basic_prop!(FInt64Property, i64, CASTCLASS_FInt64Property);
impl_basic_prop!(FByteProperty, u8, CASTCLASS_FByteProperty); // TODO enum
impl_basic_prop!(FUInt16Property, u16, CASTCLASS_FUInt16Property);
impl_basic_prop!(FUInt32Property, u32, CASTCLASS_FUInt32Property);
impl_basic_prop!(FUInt64Property, u64, CASTCLASS_FUInt64Property);
impl_basic_prop!(FFloatProperty, f32, CASTCLASS_FFloatProperty);
impl_basic_prop!(FDoubleProperty, f64, CASTCLASS_FDoubleProperty);
impl_basic_prop!(
    FObjectProperty,
    Option<&'o UObject>,
    CASTCLASS_FObjectProperty
); // TODO figure out lifetimes

// FBoolProperty requires special bitfield handling
impl_deref!(FBoolProperty, fproperty: FProperty);
unsafe impl FieldTrait for FBoolProperty {
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_FBoolProperty;
}
#[derive(Debug)]
#[repr(C)]
pub struct FBoolProperty {
    fproperty: FProperty,
    field_size: u8,
    byte_offset: u8,
    byte_mask: u8,
    field_mask: u8,
}

pub struct FBoolPropertyData<'o> {
    property: &'o FBoolProperty,
    data_ptr: *const u8,
}

impl<'o> FBoolPropertyData<'o> {
    pub fn get(&self) -> bool {
        unsafe {
            let byte_value = *self.data_ptr.add(self.property.byte_offset as usize);
            (byte_value & self.property.byte_mask) != 0
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
            let byte_value = *self.data_ptr.add(self.property.byte_offset as usize);
            (byte_value & self.property.byte_mask) != 0
        }
    }

    pub fn set(&mut self, value: bool) {
        unsafe {
            let byte_ptr = self.data_ptr.add(self.property.byte_offset as usize);
            let mut byte_value = *byte_ptr;
            if value {
                byte_value |= self.property.byte_mask;
            } else {
                byte_value &= !self.property.byte_mask;
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
