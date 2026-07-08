use super::*;

ue_struct! {
    /// `FArrayProperty` — a `TArray` value; `inner` is the element property.
    struct FArrayProperty: FProperty;
    cast FieldTrait = EClassCastFlags::CASTCLASS_FArrayProperty;
    extern_layout fproperty: FPropertyLayout;
    /// `Inner` is reordered relative to `ArrayFlags` across engine versions, so
    /// it uses its own absolute offset (`array_inner`) rather than `size + n`.
    ptr_req array_inner as inner: FProperty,
}
impl PropTrait for FArrayProperty {
    type PropValue<'o> = FArrayPropertyData<'o>;
    type PropValueMut<'o> = FArrayPropertyDataMut<'o>;

    unsafe fn value<'o>(&'o self, data: *const ()) -> Self::PropValue<'o> {
        unsafe {
            let array = &*data.cast::<FScriptArray>();
            FArrayPropertyData {
                array_property: self,
                array,
            }
        }
    }
    unsafe fn value_mut<'o>(&'o self, data: *mut ()) -> Self::PropValueMut<'o> {
        unsafe {
            let array = &mut *data.cast::<FScriptArray>();
            FArrayPropertyDataMut {
                array_property: self,
                array,
            }
        }
    }
}

pub struct FArrayPropertyData<'o> {
    array_property: &'o FArrayProperty,
    array: &'o FScriptArray,
}

impl<'o> FArrayPropertyData<'o> {
    pub fn array_property(&self) -> &'o FArrayProperty {
        self.array_property
    }

    pub fn script_array_ptr(&self) -> *const FScriptArray {
        self.array as *const FScriptArray
    }

    pub fn len(&self) -> i32 {
        self.array.num()
    }

    pub fn is_empty(&self) -> bool {
        self.array.is_empty()
    }

    pub fn element_size(&self) -> i32 {
        self.array_property.inner().element_size()
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

    pub fn iter(&'o self) -> FArrayPropertyIterator<'o> {
        FArrayPropertyIterator {
            array_data: self,
            index: 0,
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
        self.array_property.inner().element_size()
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

    pub fn iter_mut(&mut self) -> FArrayPropertyIteratorMut<'o> {
        FArrayPropertyIteratorMut {
            array_data: self as *mut _,
            index: 0,
        }
    }
}

pub struct FArrayPropertyIterator<'o> {
    array_data: &'o FArrayPropertyData<'o>,
    index: i32,
}

impl<'o> Iterator for FArrayPropertyIterator<'o> {
    type Item = BoundArrayElement<'o>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.array_data.len() {
            let element = self.array_data.get_element(self.index)?;
            self.index += 1;
            Some(element)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.array_data.len() - self.index).max(0) as usize;
        (remaining, Some(remaining))
    }
}

impl<'o> ExactSizeIterator for FArrayPropertyIterator<'o> {}

pub struct FArrayPropertyIteratorMut<'o> {
    array_data: *mut FArrayPropertyDataMut<'o>,
    index: i32,
}

impl<'o> Iterator for FArrayPropertyIteratorMut<'o> {
    type Item = BoundArrayElementMut<'o>;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let array_data = &mut *self.array_data;
            if self.index < array_data.len() {
                let element = array_data.get_element_mut(self.index);
                self.index += 1;
                Some(element)
            } else {
                None
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        unsafe {
            let array_data = &*self.array_data;
            let remaining = (array_data.len() - self.index).max(0) as usize;
            (remaining, Some(remaining))
        }
    }
}

impl<'o> ExactSizeIterator for FArrayPropertyIteratorMut<'o> {}
