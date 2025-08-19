use super::*;
use std::ffi::c_void;

#[derive(Debug)]
#[repr(C)]
pub struct FSetProperty {
    fproperty: FProperty,
    element_prop: *const FProperty,
    set_layout: FScriptSetLayout,
}

impl FSetProperty {
    pub fn element_property(&self) -> &FProperty {
        unsafe {
            self.element_prop
                .as_ref()
                .expect("FSetProperty element property is null")
        }
    }
}

impl_deref!(FSetProperty, fproperty: FProperty);
unsafe impl FieldTrait for FSetProperty {
    const CAST_FLAGS: EClassCastFlags = EClassCastFlags::CASTCLASS_FSetProperty;
}

impl PropTrait for FSetProperty {
    type PropValue<'o> = FSetPropertyData<'o>;
    type PropValueMut<'o> = FSetPropertyDataMut<'o>;

    unsafe fn value<'o>(&'o self, data: *const ()) -> Self::PropValue<'o> {
        let set = &*data.cast::<FScriptSet>();
        FSetPropertyData {
            set_property: self,
            set,
        }
    }

    unsafe fn value_mut<'o>(&'o self, data: *mut ()) -> Self::PropValueMut<'o> {
        let set = &mut *data.cast::<FScriptSet>();
        FSetPropertyDataMut {
            set_property: self,
            set,
        }
    }
}

pub struct BoundSetElement<'o> {
    data_ptr: *const (),
    pub property: &'o FProperty,
}

impl<'o> BoundSetElement<'o> {
    pub fn get<P: PropTrait + 'o>(&self) -> Option<P::PropValue<'o>> {
        self.property
            .base
            .cast::<P>()
            .map(|f| unsafe { f.value(self.data_ptr) })
    }

    pub unsafe fn cast<T>(&self) -> &'o T {
        unsafe { &*(self.data_ptr as *const T) }
    }
}

impl<'o> PropertyAccess<'o> for BoundSetElement<'o> {
    fn try_get<P: PropTrait + 'o>(&self) -> Option<P::PropValue<'o>> {
        self.get::<P>()
    }

    fn try_get_mut<P: PropTrait + 'o>(&mut self) -> Option<P::PropValueMut<'o>> {
        // BoundSetElement is immutable, return None for mutable access
        None
    }

    fn field(&self) -> &FField {
        &self.property
    }
}

pub struct BoundSetElementMut<'o> {
    data_ptr: *mut (),
    pub property: &'o FProperty,
}

impl<'o> BoundSetElementMut<'o> {
    pub fn get<P: PropTrait>(&self) -> Option<P::PropValueMut<'o>> {
        self.property
            .base
            .cast::<P>()
            .map(|f| unsafe { f.value_mut(self.data_ptr) })
    }

    pub fn cast<T>(&self) -> &'o T {
        unsafe { &*(self.data_ptr as *const T) }
    }

    pub fn cast_mut<T>(&mut self) -> &'o mut T {
        unsafe { &mut *(self.data_ptr as *mut T) }
    }
}

impl<'o> PropertyAccess<'o> for BoundSetElementMut<'o> {
    fn try_get<P: PropTrait + 'o>(&self) -> Option<P::PropValue<'o>> {
        // For immutable access, we can cast the property but need to be careful with lifetimes
        self.property
            .base
            .cast::<P>()
            .map(|f| unsafe { f.value(self.data_ptr as *const ()) })
    }

    fn try_get_mut<P: PropTrait + 'o>(&mut self) -> Option<P::PropValueMut<'o>> {
        self.get::<P>()
    }

    fn field(&self) -> &FField {
        &self.property
    }
}

pub struct FSetPropertyData<'o> {
    set_property: &'o FSetProperty,
    set: &'o FScriptSet,
}

impl<'o> FSetPropertyData<'o> {
    pub fn len(&self) -> i32 {
        self.set.num()
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    pub fn element_size(&self) -> i32 {
        self.set_property.element_property().element_size
    }

    pub fn element_property(&self) -> &'o FProperty {
        self.set_property.element_property()
    }

    pub fn get_element(&self, index: i32) -> Option<BoundSetElement<'o>> {
        if self.set.is_valid_index(index) {
            let element_ptr = self.set.get_data(index, &self.set_property.set_layout);
            if !element_ptr.is_null() {
                Some(BoundSetElement {
                    data_ptr: element_ptr as *const (),
                    property: self.element_property(),
                })
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn iter(&'o self) -> FSetPropertyDataIterator<'o> {
        FSetPropertyDataIterator {
            set_data: self,
            index: 0,
        }
    }

    /// Check if the set contains an element
    pub fn contains<T>(
        &self,
        element: &T,
        get_key_hash: impl Fn(&T) -> u32,
        equality_fn: impl Fn(&T, &T) -> bool,
    ) -> bool {
        self.set.contains(element, get_key_hash, equality_fn)
    }

    /// Find an element in the set, returning its index or -1 if not found
    pub fn find<T>(
        &self,
        element: &T,
        get_key_hash: impl Fn(&T) -> u32,
        equality_fn: impl Fn(&T, &T) -> bool,
    ) -> i32 {
        self.set.find(element, get_key_hash, equality_fn)
    }

    /// Get a typed reference to an element at the given index
    pub fn get_typed<T>(&self, index: i32) -> Option<&T> {
        self.set.get::<T>(index)
    }
}

pub struct FSetPropertyDataMut<'o> {
    set_property: &'o FSetProperty,
    set: &'o mut FScriptSet,
}

impl<'o> FSetPropertyDataMut<'o> {
    pub fn len(&self) -> i32 {
        self.set.num()
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    pub fn element_size(&self) -> i32 {
        self.set_property.element_property().element_size
    }

    pub fn element_property(&self) -> &'o FProperty {
        self.set_property.element_property()
    }

    pub fn get_element(&self, index: i32) -> Option<BoundSetElement<'o>> {
        if self.set.is_valid_index(index) {
            let element_ptr = self.set.get_data(index, &self.set_property.set_layout);
            if !element_ptr.is_null() {
                Some(BoundSetElement {
                    data_ptr: element_ptr as *const (),
                    property: self.element_property(),
                })
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn get_element_mut(&mut self, index: i32) -> Option<BoundSetElementMut<'o>> {
        if self.set.is_valid_index(index) {
            let element_ptr = self.set.get_data_mut(index, &self.set_property.set_layout);
            if !element_ptr.is_null() {
                Some(BoundSetElementMut {
                    data_ptr: element_ptr as *mut (),
                    property: self.element_property(),
                })
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn add_uninitialized(&mut self) -> i32 {
        self.set.add_uninitialized(&self.set_property.set_layout)
    }

    pub fn remove_at(&mut self, index: i32) {
        self.set.remove_at(index, &self.set_property.set_layout)
    }

    pub fn empty(&mut self, slack: i32) {
        self.set.empty(slack, &self.set_property.set_layout)
    }

    pub fn iter(&self) -> FSetPropertyIteratorMut<'o> {
        FSetPropertyIteratorMut {
            set_data: self as *const _ as *mut _,
            index: 0,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn iter_mut(&mut self) -> FSetPropertyIteratorMut<'o> {
        FSetPropertyIteratorMut {
            set_data: self as *mut _,
            index: 0,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Check if the set contains an element
    pub fn contains<T>(
        &self,
        element: &T,
        get_key_hash: impl Fn(&T) -> u32,
        equality_fn: impl Fn(&T, &T) -> bool,
    ) -> bool {
        self.set.contains(element, get_key_hash, equality_fn)
    }

    /// Find an element in the set, returning its index or -1 if not found
    pub fn find<T>(
        &self,
        element: &T,
        get_key_hash: impl Fn(&T) -> u32,
        equality_fn: impl Fn(&T, &T) -> bool,
    ) -> i32 {
        self.set.find(element, get_key_hash, equality_fn)
    }

    /// Get a typed reference to an element at the given index
    pub fn get_typed<T>(&self, index: i32) -> Option<&T> {
        self.set.get::<T>(index)
    }

    /// Get a typed mutable reference to an element at the given index
    pub fn get_typed_mut<T>(&mut self, index: i32) -> Option<&mut T> {
        self.set.get_mut::<T>(index)
    }

    /// Insert an element into the set, returning its index
    pub fn insert<T>(
        &mut self,
        element: T,
        get_key_hash: impl Fn(&T) -> u32,
        equality_fn: impl Fn(&T, &T) -> bool,
    ) -> i32 {
        self.set.insert(element, get_key_hash, equality_fn)
    }

    /// Remove an element by value, returning true if found and removed
    pub fn remove_by_value<T>(
        &mut self,
        element: &T,
        get_key_hash: impl Fn(&T) -> u32,
        equality_fn: impl Fn(&T, &T) -> bool,
    ) -> bool {
        self.set.remove_by_value(element, get_key_hash, equality_fn)
    }

    /// Remove an element at the given typed index
    pub fn remove_typed<T>(&mut self, index: i32) -> bool {
        self.set.remove::<T>(index)
    }

    /// Clear all elements from the set with proper typed cleanup
    pub fn clear<T>(&mut self) {
        self.set.clear::<T>()
    }

    /// Reserve space for at least the given number of elements
    pub fn reserve<T>(&mut self, capacity: i32) {
        self.set.reserve::<T>(capacity)
    }
}

pub struct FSetPropertyDataIterator<'o> {
    set_data: &'o FSetPropertyData<'o>,
    index: i32,
}

impl<'o> Iterator for FSetPropertyDataIterator<'o> {
    type Item = BoundSetElement<'o>;

    fn next(&mut self) -> Option<Self::Item> {
        // Skip invalid indices and find the next valid element
        let max_index = self.set_data.set.get_max_index();
        while self.index <= max_index {
            if let Some(element) = self.set_data.get_element(self.index) {
                self.index += 1;
                return Some(element);
            }
            self.index += 1;
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.set_data.len() as usize;
        (0, Some(remaining)) // Lower bound is 0 since we don't know how many indices are valid
    }
}

pub struct FSetPropertyIteratorMut<'o> {
    set_data: *mut FSetPropertyDataMut<'o>,
    index: i32,
    _phantom: std::marker::PhantomData<&'o ()>,
}

impl<'o> Iterator for FSetPropertyIteratorMut<'o> {
    type Item = BoundSetElement<'o>; // For readonly iteration, even from mutable

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            let set_data = &*self.set_data;
            // Skip invalid indices and find the next valid element
            let max_index = set_data.set.get_max_index();
            while self.index <= max_index {
                if let Some(element) = set_data.get_element(self.index) {
                    self.index += 1;
                    return Some(element);
                }
                self.index += 1;
            }
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        unsafe {
            let set_data = &*self.set_data;
            let remaining = set_data.len() as usize;
            (0, Some(remaining)) // Lower bound is 0 since we don't know how many indices are valid
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ue::malloc::test::setup_test_globals;

    #[test]
    fn test_set_property_basic_operations() {
        setup_test_globals();

        // Create minimal mock structures for testing - we only need the sizes and basic fields
        // In practice, these would be created by UE's reflection system
        let mut element_prop: FProperty = unsafe { std::mem::zeroed() };
        element_prop.element_size = 4; // i32 sized elements
        let element_prop_ptr = &element_prop as *const _;

        let set_layout = FScriptSetLayout::get_layout(4, 4);
        let set_property = FSetProperty {
            fproperty: element_prop,
            element_prop: element_prop_ptr,
            set_layout,
        };

        let mut set = FScriptSet::new();

        // Test empty set
        let set_data = FSetPropertyData {
            set_property: &set_property,
            set: &set,
        };

        assert_eq!(set_data.len(), 0);
        assert!(set_data.is_empty());
        assert_eq!(set_data.element_size(), 4);

        // Test iterator on empty set
        let mut iter = set_data.iter();
        assert!(iter.next().is_none());

        // Test mutable operations
        let mut set_data_mut = FSetPropertyDataMut {
            set_property: &set_property,
            set: &mut set,
        };

        // Add some elements
        let idx1 = set_data_mut.add_uninitialized();
        let idx2 = set_data_mut.add_uninitialized();

        assert_eq!(set_data_mut.len(), 2);
        assert!(!set_data_mut.is_empty());

        // Test element access
        assert!(set_data_mut.get_element(idx1).is_some());
        assert!(set_data_mut.get_element(idx2).is_some());
        assert!(set_data_mut.get_element_mut(idx1).is_some());

        // Test iterator
        let element_count = set_data_mut.iter().count();
        assert_eq!(element_count, 2);

        // Test mutable iterator
        let mut_element_count = set_data_mut.iter_mut().count();
        assert_eq!(mut_element_count, 2);

        // Test removal
        set_data_mut.remove_at(idx1);
        assert_eq!(set_data_mut.len(), 1);

        // Test empty
        set_data_mut.empty(0);
        assert!(set_data_mut.is_empty());
        assert_eq!(set_data_mut.len(), 0);
    }

    #[test]
    fn test_bound_set_element() {
        setup_test_globals();

        let mut element_prop: FProperty = unsafe { std::mem::zeroed() };
        element_prop.element_size = 4;

        // Test BoundSetElement casting
        let test_value = 42i32;
        let element = BoundSetElement {
            data_ptr: &test_value as *const i32 as *const (),
            property: &element_prop,
        };

        unsafe {
            let cast_value: &i32 = element.cast();
            assert_eq!(*cast_value, 42);
        }
    }

    #[test]
    fn test_set_property_hash_based_api() {
        setup_test_globals();

        // Create mock structures
        let mut element_prop: FProperty = unsafe { std::mem::zeroed() };
        element_prop.element_size = 4; // i32 sized elements
        let element_prop_ptr = &element_prop as *const _;

        let set_layout = FScriptSetLayout::get_layout(4, 4);
        let set_property = FSetProperty {
            fproperty: element_prop,
            element_prop: element_prop_ptr,
            set_layout,
        };

        let mut set = FScriptSet::new();

        // Create mutable wrapper
        let mut set_data = FSetPropertyDataMut {
            set_property: &set_property,
            set: &mut set,
        };

        // Define hash and equality functions for i32
        let hash_fn = |x: &i32| *x as u32;
        let eq_fn = |a: &i32, b: &i32| a == b;

        // Test empty set
        assert!(!set_data.contains(&42, hash_fn, eq_fn));
        assert_eq!(set_data.find(&42, hash_fn, eq_fn), -1);
        assert_eq!(set_data.get_typed::<i32>(0), None);

        // Insert elements using hash-based API
        let index1 = set_data.insert(10, hash_fn, eq_fn);
        let index2 = set_data.insert(20, hash_fn, eq_fn);
        let index3 = set_data.insert(30, hash_fn, eq_fn);

        assert_eq!(set_data.len(), 3);

        // Test contains
        assert!(set_data.contains(&10, hash_fn, eq_fn));
        assert!(set_data.contains(&20, hash_fn, eq_fn));
        assert!(set_data.contains(&30, hash_fn, eq_fn));
        assert!(!set_data.contains(&40, hash_fn, eq_fn));

        // Test find
        assert!(set_data.find(&10, hash_fn, eq_fn) != -1);
        assert!(set_data.find(&20, hash_fn, eq_fn) != -1);
        assert!(set_data.find(&30, hash_fn, eq_fn) != -1);
        assert_eq!(set_data.find(&40, hash_fn, eq_fn), -1);

        // Test typed access
        assert_eq!(set_data.get_typed::<i32>(index1), Some(&10));
        assert_eq!(set_data.get_typed::<i32>(index2), Some(&20));
        assert_eq!(set_data.get_typed::<i32>(index3), Some(&30));

        // Test mutable typed access
        if let Some(element) = set_data.get_typed_mut::<i32>(index1) {
            *element = 100;
        }
        assert_eq!(set_data.get_typed::<i32>(index1), Some(&100));

        // Test remove by value
        assert!(set_data.remove_by_value(&20, hash_fn, eq_fn));
        assert_eq!(set_data.len(), 2);
        assert!(!set_data.contains(&20, hash_fn, eq_fn));

        // Test typed remove
        assert!(set_data.remove_typed::<i32>(index3));
        assert_eq!(set_data.len(), 1);
        assert!(!set_data.contains(&30, hash_fn, eq_fn));

        // Test clear
        set_data.clear::<i32>();
        assert_eq!(set_data.len(), 0);
        assert!(set_data.is_empty());
    }

    #[test]
    fn test_set_property_readonly_hash_api() {
        setup_test_globals();

        // Create mock structures
        let mut element_prop: FProperty = unsafe { std::mem::zeroed() };
        element_prop.element_size = 4;
        let element_prop_ptr = &element_prop as *const _;

        let set_layout = FScriptSetLayout::get_layout(4, 4);
        let set_property = FSetProperty {
            fproperty: element_prop,
            element_prop: element_prop_ptr,
            set_layout,
        };

        let mut set = FScriptSet::new();

        // Pre-populate the set
        let hash_fn = |x: &i32| *x as u32;
        let eq_fn = |a: &i32, b: &i32| a == b;

        let index1 = set.insert(10, hash_fn, eq_fn);
        let index2 = set.insert(20, hash_fn, eq_fn);

        // Create read-only wrapper
        let set_data = FSetPropertyData {
            set_property: &set_property,
            set: &set,
        };

        // Test read-only hash-based operations
        assert!(set_data.contains(&10, hash_fn, eq_fn));
        assert!(set_data.contains(&20, hash_fn, eq_fn));
        assert!(!set_data.contains(&30, hash_fn, eq_fn));

        assert!(set_data.find(&10, hash_fn, eq_fn) != -1);
        assert!(set_data.find(&20, hash_fn, eq_fn) != -1);
        assert_eq!(set_data.find(&30, hash_fn, eq_fn), -1);

        assert_eq!(set_data.get_typed::<i32>(index1), Some(&10));
        assert_eq!(set_data.get_typed::<i32>(index2), Some(&20));
        assert_eq!(set_data.get_typed::<i32>(-1), None);
    }

    #[test]
    fn test_set_property_complex_type() {
        setup_test_globals();

        #[derive(Debug, Clone, PartialEq)]
        struct TestStruct {
            id: u32,
            value: f32,
        }

        let element_size = std::mem::size_of::<TestStruct>() as i32;
        let element_align = std::mem::align_of::<TestStruct>() as i32;

        // Create mock structures
        let mut element_prop: FProperty = unsafe { std::mem::zeroed() };
        element_prop.element_size = element_size;
        let element_prop_ptr = &element_prop as *const _;

        let set_layout = FScriptSetLayout::get_layout(element_size, element_align);
        let set_property = FSetProperty {
            fproperty: element_prop,
            element_prop: element_prop_ptr,
            set_layout,
        };

        let mut set = FScriptSet::new();
        let mut set_data = FSetPropertyDataMut {
            set_property: &set_property,
            set: &mut set,
        };

        // Define hash and equality functions for TestStruct
        let hash_fn = |x: &TestStruct| x.id;
        let eq_fn = |a: &TestStruct, b: &TestStruct| a.id == b.id;

        let item1 = TestStruct { id: 1, value: 1.5 };
        let item2 = TestStruct { id: 2, value: 2.5 };

        // Insert complex objects
        let index1 = set_data.insert(item1.clone(), hash_fn, eq_fn);
        let index2 = set_data.insert(item2.clone(), hash_fn, eq_fn);

        assert_eq!(set_data.len(), 2);

        // Test complex type operations
        assert!(set_data.contains(&item1, hash_fn, eq_fn));
        assert!(set_data.contains(&item2, hash_fn, eq_fn));

        let item3 = TestStruct { id: 3, value: 3.5 };
        assert!(!set_data.contains(&item3, hash_fn, eq_fn));

        // Test typed access with complex type
        assert_eq!(set_data.get_typed::<TestStruct>(index1), Some(&item1));
        assert_eq!(set_data.get_typed::<TestStruct>(index2), Some(&item2));

        // Test mutable access and modification
        if let Some(element) = set_data.get_typed_mut::<TestStruct>(index1) {
            element.value = 10.0;
        }

        let modified_item = set_data.get_typed::<TestStruct>(index1).unwrap();
        assert_eq!(modified_item.id, 1);
        assert_eq!(modified_item.value, 10.0);

        // Clean up properly
        set_data.clear::<TestStruct>();
        assert_eq!(set_data.len(), 0);
    }
}
