use super::*;

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
