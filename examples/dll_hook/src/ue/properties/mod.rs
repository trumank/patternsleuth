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

// FStructProperty contains nested properties
impl_deref!(FStructProperty, fproperty: FProperty);
unsafe impl FieldTrait for FStructProperty {
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_FStructProperty;
}
#[derive(Debug)]
#[repr(C)]
pub struct FStructProperty {
    fproperty: FProperty,
    r#struct: *const UStruct, // Using UStruct since UScriptStruct inherits from it
}

impl FStructProperty {
    pub fn get_struct(&self) -> &UStruct {
        unsafe { self.r#struct.as_ref().unwrap() }
    }
}

pub struct FStructPropertyData<'o> {
    property: &'o FStructProperty,
    data_ptr: *const (),
}

impl<'o> FStructPropertyData<'o> {
    pub fn get_struct(&self) -> &UStruct {
        self.property.get_struct()
    }

    pub fn data_ptr(&self) -> *const () {
        self.data_ptr
    }
    pub fn props(&'o self) -> IterFieldsBound<'o> {
        let inner = self.property.get_struct().iter_props();
        IterFieldsBound {
            object: self.data_ptr,
            inner,
        }
    }
}

pub struct FStructPropertyDataMut<'o> {
    property: &'o FStructProperty,
    data_ptr: *mut (),
}

impl<'o> FStructPropertyDataMut<'o> {
    pub fn get_struct(&self) -> &UStruct {
        self.property.get_struct()
    }

    pub fn data_ptr(&self) -> *const () {
        self.data_ptr as *const ()
    }

    pub fn data_ptr_mut(&mut self) -> *mut () {
        self.data_ptr
    }

    pub fn props(&'o self) -> IterFieldsBoundMut<'o> {
        let inner = self.property.get_struct().iter_props();
        IterFieldsBoundMut {
            object: self.data_ptr,
            inner,
        }
    }
}

impl PropTrait for FStructProperty {
    type PropValue<'o> = FStructPropertyData<'o>;
    type PropValueMut<'o> = FStructPropertyDataMut<'o>;

    unsafe fn value<'o>(&'o self, data: *const ()) -> Self::PropValue<'o> {
        FStructPropertyData {
            property: self,
            data_ptr: data,
        }
    }

    unsafe fn value_mut<'o>(&'o self, data: *mut ()) -> Self::PropValueMut<'o> {
        FStructPropertyDataMut {
            property: self,
            data_ptr: data,
        }
    }
}

pub struct FArrayPropertyData<'o> {
    array_property: &'o FArrayProperty,
    array: &'o FScriptArray,
}

impl<'o> FArrayPropertyData<'o> {
    pub fn len(&self) -> i32 {
        self.array.num()
    }

    pub fn is_empty(&self) -> bool {
        self.array.is_empty()
    }

    pub fn element_size(&self) -> i32 {
        self.array_property.inner().element_size
    }

    pub fn inner_property(&self) -> &'o FProperty {
        self.array_property.inner()
    }

    pub fn get_element(&self, index: i32) -> Option<BoundArrayElement<'o>> {
        if self.array.is_valid_index(index) {
            let base_ptr = self.array.get_data() as *const u8;
            let element_size = self.element_size() as usize;
            unsafe {
                let ptr = base_ptr.add((index as usize) * element_size) as *const ();
                Some(BoundArrayElement {
                    data_ptr: ptr,
                    property: self.inner_property(),
                })
            }
        } else {
            None
        }
    }

    pub fn get_element_cast<T>(&self, index: i32) -> Option<&'o T> {
        if self.array.is_valid_index(index) {
            let base_ptr = self.array.get_data() as *const u8;
            let element_size = self.element_size() as usize;
            unsafe {
                let ptr = base_ptr.add((index as usize) * element_size) as *const T;
                Some(&*ptr)
            }
        } else {
            None
        }
    }
}

pub struct FArrayPropertyDataMut<'o> {
    array_property: &'o FArrayProperty,
    array: &'o mut FScriptArray,
}

impl<'o> FArrayPropertyDataMut<'o> {
    pub fn len(&self) -> i32 {
        self.array.num()
    }

    pub fn is_empty(&self) -> bool {
        self.array.is_empty()
    }

    pub fn element_size(&self) -> i32 {
        self.array_property.inner().element_size
    }

    pub fn inner_property(&self) -> &'o FProperty {
        self.array_property.inner()
    }

    pub fn get_element(&self, index: i32) -> BoundArrayElement<'o> {
        if self.array.is_valid_index(index) {
            let base_ptr = self.array.get_data() as *const u8;
            let element_size = self.element_size() as usize;
            unsafe {
                let ptr = base_ptr.add((index as usize) * element_size) as *const ();
                BoundArrayElement {
                    data_ptr: ptr,
                    property: self.inner_property(),
                }
            }
        } else {
            panic!("Out of bounds FScriptArray access");
        }
    }

    pub fn get_element_mut(&mut self, index: i32) -> BoundArrayElementMut<'o> {
        if self.array.is_valid_index(index) {
            let base_ptr = self.array.get_data_mut() as *mut u8;
            let element_size = self.element_size() as usize;
            unsafe {
                let ptr = base_ptr.add((index as usize) * element_size) as *mut ();
                BoundArrayElementMut {
                    data_ptr: ptr,
                    property: self.inner_property(),
                }
            }
        } else {
            panic!("Out of bounds FScriptArray access");
        }
    }

    pub fn add_element(&mut self, count: i32) -> i32 {
        let element_size = self.element_size();
        self.array.add(count, element_size)
    }

    pub fn add_zeroed_element(&mut self, count: i32) -> i32 {
        let element_size = self.element_size();
        self.array.add_zeroed(count, element_size)
    }

    pub fn insert_element(&mut self, index: i32, count: i32) {
        let element_size = self.element_size();
        self.array.insert(index, count, element_size)
    }

    pub fn insert_zeroed_element(&mut self, index: i32, count: i32) {
        let element_size = self.element_size();
        self.array.insert_zeroed(index, count, element_size)
    }

    pub fn remove_element(&mut self, index: i32, count: i32) {
        let element_size = self.element_size();
        self.array.remove(index, count, element_size)
    }

    pub fn set_num_elements(&mut self, new_num: i32) {
        let element_size = self.element_size();
        self.array.set_num_uninitialized(new_num, element_size)
    }

    pub fn empty(&mut self, slack: i32) {
        let element_size = self.element_size();
        self.array.empty(slack, element_size)
    }
}

impl_deref!(FArrayProperty, fproperty: FProperty);
unsafe impl FieldTrait for FArrayProperty {
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_FArrayProperty;
}
impl PropTrait for FArrayProperty {
    type PropValue<'o> = FArrayPropertyData<'o>;
    type PropValueMut<'o> = FArrayPropertyDataMut<'o>;

    unsafe fn value<'o>(&'o self, data: *const ()) -> Self::PropValue<'o> {
        let array = &*data.cast::<FScriptArray>();
        FArrayPropertyData {
            array_property: self,
            array,
        }
    }
    unsafe fn value_mut<'o>(&'o self, data: *mut ()) -> Self::PropValueMut<'o> {
        let array = &mut *data.cast::<FScriptArray>();
        FArrayPropertyDataMut {
            array_property: self,
            array,
        }
    }
}
#[derive(Debug)]
#[repr(C)]
pub struct FArrayProperty {
    fproperty: FProperty,
    inner: *const FProperty,
}

impl FArrayProperty {
    pub fn inner(&self) -> &FProperty {
        unsafe {
            self.inner
                .as_ref()
                .expect("FArrayProperty inner property is null")
        }
    }
}

// pub struct ArrayPropertyIterator<'o, T: 'o> {
//     array_property: &'o FArrayProperty,
//     object: &'o UObjectBase,
//     index: i32,
//     _phantom: std::marker::PhantomData<T>,
// }

// impl<'o, T: 'o> Iterator for ArrayPropertyIterator<'o, T> {
//     type Item = Option<&'o T>;

//     fn next(&mut self) -> Option<Self::Item> {
//         let num_elements = self.array_property.num_elements(self.object);
//         if self.index >= num_elements {
//             None
//         } else {
//             let element = self
//                 .array_property
//                 .get_element::<T>(self.object, self.index);
//             self.index += 1;
//             Some(element)
//         }
//     }

//     fn size_hint(&self) -> (usize, Option<usize>) {
//         let remaining =
//             (self.array_property.num_elements(self.object) - self.index).max(0) as usize;
//         (remaining, Some(remaining))
//     }
// }

// impl<'o, T: 'o> ExactSizeIterator for ArrayPropertyIterator<'o, T> {}

// pub struct ArrayPropertyIteratorMut<'o, T: 'o> {
//     array_property: &'o FArrayProperty,
//     object: *mut UObjectBase,
//     index: i32,
//     _phantom: std::marker::PhantomData<&'o mut T>,
// }

// impl<'o, T: 'o> Iterator for ArrayPropertyIteratorMut<'o, T> {
//     type Item = Option<&'o mut T>;

//     fn next(&mut self) -> Option<Self::Item> {
//         let num_elements = unsafe { self.array_property.num_elements(&*self.object) };
//         if self.index >= num_elements {
//             None
//         } else {
//             let element = unsafe {
//                 self.array_property
//                     .get_element_mut::<T>(&mut *self.object, self.index)
//             };
//             self.index += 1;
//             Some(element)
//         }
//     }

//     fn size_hint(&self) -> (usize, Option<usize>) {
//         let num_elements = unsafe { self.array_property.num_elements(&*self.object) };
//         let remaining = (num_elements - self.index).max(0) as usize;
//         (remaining, Some(remaining))
//     }
// }

// impl<'o, T: 'o> ExactSizeIterator for ArrayPropertyIteratorMut<'o, T> {}
