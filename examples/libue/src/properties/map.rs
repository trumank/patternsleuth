use super::*;

ue_struct! {
    /// `FMapProperty` — a `TMap` value; `key_property`/`value_property` describe
    /// the element types and `map_layout` the `FScriptMap` memory layout.
    struct FMapProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FMapProperty;
    extern_layout fmapproperty: FMapPropertyLayout;
    ptr_req key_prop as key_property: FProperty,
    ptr_req value_prop as value_property: FProperty,
    view map_layout: FScriptMapLayout,
}

pub struct FMapPropertyData<'o> {
    map_property: &'o FMapProperty,
    map: &'o FScriptMap,
}

impl<'o> FMapPropertyData<'o> {
    pub fn map_property(&self) -> &'o FMapProperty {
        self.map_property
    }

    pub fn script_map_ptr(&self) -> *const FScriptMap {
        self.map as *const FScriptMap
    }

    pub fn len(&self) -> i32 {
        self.map.num()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn max_index(&self) -> i32 {
        self.map.get_max_index()
    }

    pub fn is_valid_index(&self, index: i32) -> bool {
        self.map.is_valid_index(index)
    }

    /// Get the key data pointer for a valid index
    pub fn key_data(&self, index: i32) -> *const () {
        self.map
            .get_data(index, self.map_property.map_layout())
            .cast()
    }

    /// Get the value data pointer for a valid index
    pub fn value_data(&self, index: i32) -> *const () {
        unsafe {
            (self.map.get_data(index, self.map_property.map_layout()) as *const u8)
                .add(self.map_property.map_layout().value_offset as usize)
                .cast()
        }
    }

    pub fn key_property(&self) -> &'o FProperty {
        self.map_property.key_property()
    }

    pub fn value_property(&self) -> &'o FProperty {
        self.map_property.value_property()
    }
}

pub struct FMapPropertyDataMut<'o> {
    map: &'o mut FScriptMap,
}

impl<'o> FMapPropertyDataMut<'o> {
    pub fn len(&self) -> i32 {
        self.map.num()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl PropTrait for FMapProperty {
    type PropValue<'o> = FMapPropertyData<'o>;
    type PropValueMut<'o> = FMapPropertyDataMut<'o>;

    unsafe fn value<'o>(&'o self, data: *const ()) -> Self::PropValue<'o> {
        unsafe {
            let map = &*data.cast::<FScriptMap>();
            FMapPropertyData {
                map_property: self,
                map,
            }
        }
    }

    unsafe fn value_mut<'o>(&'o self, data: *mut ()) -> Self::PropValueMut<'o> {
        unsafe {
            let map = &mut *data.cast::<FScriptMap>();
            FMapPropertyDataMut { map }
        }
    }
}
