use super::*;
use std::marker::PhantomData;

#[repr(C)]
#[derive(Debug, Clone, PartialEq)]
pub struct TTuple<K, V> {
    pub key: K,
    pub value: V,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FSetElementId {
    pub index: i32,
}

impl FSetElementId {
    pub const INVALID: FSetElementId = FSetElementId { index: -1 };

    pub fn is_valid(&self) -> bool {
        self.index >= 0
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct TSetElement<T> {
    pub value: T,
    pub hash_next_id: FSetElementId,
    pub hash_index: FSetElementId,
}

impl<T> TSetElement<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            hash_next_id: FSetElementId::INVALID,
            hash_index: FSetElementId::INVALID,
        }
    }
}

// Default set allocator type
#[derive(Default)]
pub struct FDefaultSetAllocator;

impl Allocator for FDefaultSetAllocator {
    type ForAnyElementType<T> = TSizedHeapAllocatorForAnyElementType<T>;
}

// Key functions trait - mirrors UE's KeyFuncs template parameter
pub trait KeyFuncs<T> {
    type KeyType: PartialEq + Clone;

    fn get_set_key(element: &T) -> &Self::KeyType;
    fn get_key_hash(key: &Self::KeyType) -> u32;
    fn matches(a: &Self::KeyType, b: &Self::KeyType) -> bool;

    const ALLOW_DUPLICATE_KEYS: bool = false;
}

// Default key funcs type - element is its own key
#[derive(Default)]
pub struct DefaultKeyFuncs<T>(PhantomData<T>);

impl<T> KeyFuncs<T> for DefaultKeyFuncs<T>
where
    T: PartialEq + Clone + GetTypeHash,
{
    type KeyType = T;

    fn get_set_key(element: &T) -> &Self::KeyType {
        element
    }

    fn get_key_hash(key: &Self::KeyType) -> u32 {
        key.get_type_hash()
    }

    fn matches(a: &Self::KeyType, b: &Self::KeyType) -> bool {
        a == b
    }
}

// UE-compatible hashing trait - equivalent to GetTypeHash<T>()
pub trait GetTypeHash {
    fn get_type_hash(&self) -> u32;
}

// Implementations for common types
impl GetTypeHash for u32 {
    fn get_type_hash(&self) -> u32 {
        *self
    }
}

impl GetTypeHash for i32 {
    fn get_type_hash(&self) -> u32 {
        *self as u32
    }
}

impl GetTypeHash for u64 {
    fn get_type_hash(&self) -> u32 {
        let bytes = self.to_le_bytes();
        cityhasher::hash(&bytes)
    }
}

impl GetTypeHash for i64 {
    fn get_type_hash(&self) -> u32 {
        (*self as u64).get_type_hash()
    }
}

impl<T> GetTypeHash for *const T {
    fn get_type_hash(&self) -> u32 {
        (*self as usize as u64).get_type_hash()
    }
}

impl<T> GetTypeHash for *mut T {
    fn get_type_hash(&self) -> u32 {
        (*self as *const T).get_type_hash()
    }
}

impl<K: GetTypeHash, V: GetTypeHash> GetTypeHash for TTuple<K, V> {
    fn get_type_hash(&self) -> u32 {
        // Combine hashes like UE does for pairs/tuples
        let key_hash = self.key.get_type_hash();
        let value_hash = self.value.get_type_hash();

        // UE's hash combine formula
        key_hash
            ^ (value_hash
                .wrapping_add(0x9e3779b9)
                .wrapping_add(key_hash << 6)
                .wrapping_add(key_hash >> 2))
    }
}

#[repr(C)]
pub struct TSet<
    ElementType,
    KeyFuncsType = DefaultKeyFuncs<ElementType>,
    SetAllocator = FDefaultSetAllocator,
> where
    SetAllocator: Allocator,
    KeyFuncsType: KeyFuncs<ElementType>,
    ElementType: Clone,
{
    // Use TSparseArray for element storage like real UE
    pub elements: TSparseArray<TSetElement<ElementType>, TSparseArrayAllocator32>,
    // Hash table using inline allocator like real UE
    pub hash: TInlineAllocatorForElementType<FSetElementId, 1>,
    pub hash_size: i32,
    _phantom: PhantomData<(KeyFuncsType, SetAllocator)>,
}

impl<T, K, A> std::fmt::Debug for TSet<T, K, A>
where
    A: Allocator,
    K: KeyFuncs<T>,
    T: std::fmt::Debug + Clone,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TSet")
            // .field("elements", &self.elements)
            .field("hash_size", &self.hash_size)
            .finish()
    }
}

impl<T, K, A> Default for TSet<T, K, A>
where
    A: Allocator,
    K: KeyFuncs<T>,
    T: Clone,
{
    fn default() -> Self {
        Self {
            elements: TSparseArray::default(),
            hash: TInlineAllocatorForElementType::default(),
            hash_size: 0,
            _phantom: PhantomData,
        }
    }
}

impl<T, K, A> Drop for TSet<T, K, A>
where
    A: Allocator,
    K: KeyFuncs<T>,
    T: Clone,
{
    fn drop(&mut self) {
        self.hash.deallocate();
    }
}

impl<T, K, A> TSet<T, K, A>
where
    A: Allocator,
    K: KeyFuncs<T>,
    T: Clone,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let mut set = Self::new();
        set.reserve(capacity);
        set
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn capacity(&self) -> usize {
        self.elements.capacity()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    fn reserve(&mut self, capacity: usize) {
        self.elements.reserve(capacity);
        self.conditional_rehash(capacity, false);
    }

    fn get_number_of_hash_buckets(num_elements: usize) -> i32 {
        if num_elements == 0 {
            return 0;
        }
        // UE uses a minimum of 8 buckets and ensures power of 2
        let min_buckets = std::cmp::max(num_elements * 4 / 3, 8); // ~0.75 load factor
        min_buckets.next_power_of_two() as i32
    }

    fn should_rehash(
        &self,
        num_elements: usize,
        desired_hash_size: i32,
        allow_shrinking: bool,
    ) -> bool {
        (num_elements > 0 && self.hash_size < desired_hash_size)
            || (allow_shrinking && self.hash_size > desired_hash_size)
    }

    fn conditional_rehash(&mut self, num_elements: usize, allow_shrinking: bool) -> bool {
        let desired_hash_size = Self::get_number_of_hash_buckets(num_elements);

        if self.should_rehash(num_elements, desired_hash_size, allow_shrinking) {
            self.hash_size = desired_hash_size;
            self.rehash();
            return true;
        }
        false
    }

    fn rehash(&mut self) {
        // Free old hash
        self.hash.deallocate();

        if self.hash_size > 0 {
            // Allocate new hash table
            self.hash.allocate(self.hash_size as usize);

            // Initialize all hash buckets to invalid
            let ptr = self.hash.data_ptr_mut();
            if !ptr.is_null() {
                unsafe {
                    for i in 0..self.hash_size as usize {
                        std::ptr::write(ptr.add(i), FSetElementId::INVALID);
                    }
                }
            }

            // Re-hash all existing elements
            let elements: Vec<(i32, T)> = self
                .elements
                .iter()
                .map(|(idx, elem)| (idx, elem.value.clone()))
                .collect();

            for (sparse_index, _) in elements {
                if let Some(element) = self.elements.get(sparse_index) {
                    let key_hash = K::get_key_hash(K::get_set_key(&element.value));
                    let hash_index = (key_hash as i32) & (self.hash_size - 1);

                    // Update element's hash info
                    if let Some(element_mut) = self.elements.get_mut(sparse_index) {
                        element_mut.hash_index = FSetElementId { index: hash_index };

                        // Link into hash chain
                        let ptr = self.hash.data_ptr_mut();
                        if !ptr.is_null() {
                            unsafe {
                                let hash_bucket = ptr.add(hash_index as usize);
                                element_mut.hash_next_id = *hash_bucket;
                                *hash_bucket = FSetElementId {
                                    index: sparse_index,
                                };
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn insert(&mut self, element: T) -> FSetElementId {
        let key_hash = K::get_key_hash(K::get_set_key(&element));

        // Check for existing element if duplicates not allowed
        if !K::ALLOW_DUPLICATE_KEYS {
            if let Some(existing_id) = self.find_by_hash(key_hash, K::get_set_key(&element)) {
                // Replace existing element
                if let Some(existing_element) = self.elements.get_mut(existing_id.index) {
                    existing_element.value = element;
                }
                return existing_id;
            }
        }

        // Add new element
        let sparse_index = self.elements.add(TSetElement::new(element.clone()));
        let element_id = FSetElementId {
            index: sparse_index,
        };

        // Check if rehash is needed
        if !self.conditional_rehash(self.elements.len(), false) {
            // If no rehash, manually link the element
            let hash_index = (key_hash as i32) & (self.hash_size - 1);

            if let Some(element_mut) = self.elements.get_mut(sparse_index) {
                element_mut.hash_index = FSetElementId { index: hash_index };

                // Link into hash chain
                let ptr = self.hash.data_ptr_mut();
                if !ptr.is_null() {
                    unsafe {
                        let hash_bucket = ptr.add(hash_index as usize);
                        element_mut.hash_next_id = *hash_bucket;
                        *hash_bucket = FSetElementId {
                            index: sparse_index,
                        };
                    }
                }
            }
        }

        element_id
    }

    pub fn remove(&mut self, element_id: FSetElementId) -> bool {
        if !element_id.is_valid() {
            return false;
        }

        self.remove_by_index(element_id.index)
    }

    pub fn remove_by_key(&mut self, key: &K::KeyType) -> bool {
        if let Some(element_id) = self.find(key) {
            self.remove_by_index(element_id.index)
        } else {
            false
        }
    }

    fn remove_by_index(&mut self, element_index: i32) -> bool {
        if !self.elements.is_valid_index(element_index) {
            return false;
        }

        // Get element hash info before removal
        let (hash_index, hash_next_id) = {
            let element = match self.elements.get(element_index) {
                Some(e) => e,
                None => return false,
            };
            (element.hash_index.index, element.hash_next_id)
        };

        // Remove from hash chain - we'll rebuild the chain without this element
        if self.hash_size > 0 && hash_index >= 0 && hash_index < self.hash_size {
            let ptr = self.hash.data_ptr_mut();
            if !ptr.is_null() {
                unsafe {
                    let hash_bucket = ptr.add(hash_index as usize);
                    let mut current_id = *hash_bucket;

                    if current_id.index == element_index {
                        // Element is first in chain
                        *hash_bucket = hash_next_id;
                    } else {
                        // Find and unlink element from chain
                        while current_id.is_valid() {
                            if let Some(current_elem) = self.elements.get_mut(current_id.index) {
                                if current_elem.hash_next_id.index == element_index {
                                    // Found the element pointing to our target
                                    current_elem.hash_next_id = hash_next_id;
                                    break;
                                }
                                current_id = current_elem.hash_next_id;
                            } else {
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Remove from sparse array
        self.elements.remove(element_index)
    }

    pub fn find(&self, key: &K::KeyType) -> Option<FSetElementId> {
        let key_hash = K::get_key_hash(key);
        self.find_by_hash(key_hash, key)
    }

    fn find_by_hash(&self, key_hash: u32, key: &K::KeyType) -> Option<FSetElementId> {
        if self.elements.len() == 0 || self.hash_size == 0 {
            return None;
        }

        let hash_index = (key_hash as i32) & (self.hash_size - 1);
        let ptr = self.hash.data_ptr();

        if ptr.is_null() {
            return None;
        }

        unsafe {
            let mut element_id = *ptr.add(hash_index as usize);

            while element_id.is_valid() {
                if let Some(element) = self.elements.get(element_id.index) {
                    if K::matches(K::get_set_key(&element.value), key) {
                        return Some(element_id);
                    }
                    element_id = element.hash_next_id;
                } else {
                    break;
                }
            }
        }

        None
    }

    pub fn contains(&self, key: &K::KeyType) -> bool {
        self.find(key).is_some()
    }

    pub fn get(&self, element_id: FSetElementId) -> Option<&T> {
        self.elements.get(element_id.index).map(|e| &e.value)
    }

    pub fn get_mut(&mut self, element_id: FSetElementId) -> Option<&mut T> {
        self.elements
            .get_mut(element_id.index)
            .map(|e| &mut e.value)
    }

    pub fn clear(&mut self) {
        self.elements.clear();
        self.unhash_elements();
    }

    fn unhash_elements(&mut self) {
        let ptr = self.hash.data_ptr_mut();
        if !ptr.is_null() && self.hash_size > 0 {
            unsafe {
                // Reset all hash buckets to invalid
                for i in 0..self.hash_size as usize {
                    std::ptr::write(ptr.add(i), FSetElementId::INVALID);
                }
            }
        }
    }

    pub fn iter(&self) -> SetIterator<'_, T> {
        SetIterator {
            sparse_iter: self.elements.iter(),
        }
    }

    pub fn iter_mut(&mut self) -> SetIteratorMut<'_, T> {
        SetIteratorMut {
            sparse_iter: self.elements.iter_mut(),
        }
    }
}

pub struct SetIterator<'a, T> {
    sparse_iter: SparseArrayIterator<'a, TSetElement<T>>,
}

impl<'a, T> Iterator for SetIterator<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.sparse_iter.next().map(|(_, element)| &element.value)
    }
}

pub struct SetIteratorMut<'a, T> {
    sparse_iter: SparseArrayIteratorMut<'a, TSetElement<T>>,
}

impl<'a, T> Iterator for SetIteratorMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        self.sparse_iter
            .next()
            .map(|(_, element)| &mut element.value)
    }
}

// Support for for-in loops
impl<'a, T, K, A> IntoIterator for &'a TSet<T, K, A>
where
    A: Allocator,
    K: KeyFuncs<T>,
    T: Clone,
{
    type Item = &'a T;
    type IntoIter = SetIterator<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, K, A> IntoIterator for &'a mut TSet<T, K, A>
where
    A: Allocator,
    K: KeyFuncs<T>,
    T: Clone,
{
    type Item = &'a mut T;
    type IntoIter = SetIteratorMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use malloc::test::setup_test_globals;

    #[test]
    fn test_tset_basic_operations() {
        setup_test_globals();

        let mut set: TSet<i32> = TSet::new();

        assert_eq!(set.len(), 0);
        assert!(set.is_empty());

        // Insert some elements
        let id1 = set.insert(42);
        let id2 = set.insert(100);
        let id3 = set.insert(200);

        assert_eq!(set.len(), 3);
        assert!(!set.is_empty());

        // Test get
        assert_eq!(set.get(id1), Some(&42));
        assert_eq!(set.get(id2), Some(&100));
        assert_eq!(set.get(id3), Some(&200));

        // Test invalid id
        assert_eq!(set.get(FSetElementId::INVALID), None);
    }

    #[test]
    fn test_tset_find_and_contains() {
        setup_test_globals();

        let mut set: TSet<i32> = TSet::new();

        set.insert(10);
        set.insert(20);
        set.insert(30);

        assert!(set.contains(&10));
        assert!(set.contains(&20));
        assert!(set.contains(&30));
        assert!(!set.contains(&40));

        if let Some(id) = set.find(&20) {
            assert_eq!(set.get(id), Some(&20));
        } else {
            panic!("Should have found element 20");
        }

        let invalid_id = set.find(&99);
        assert!(invalid_id.is_none());
    }

    #[test]
    fn test_tset_remove() {
        setup_test_globals();

        let mut set: TSet<i32> = TSet::new();

        let id1 = set.insert(10);
        let id2 = set.insert(20);
        let id3 = set.insert(30);

        assert_eq!(set.len(), 3);

        // Remove middle element
        assert!(set.remove(id2));
        assert_eq!(set.len(), 2);

        // Check elements
        assert_eq!(set.get(id1), Some(&10));
        assert_eq!(set.get(id2), None);
        assert_eq!(set.get(id3), Some(&30));
    }

    #[test]
    fn test_tset_iteration() {
        setup_test_globals();

        let mut set: TSet<i32> = TSet::new();

        set.insert(1);
        set.insert(2);
        set.insert(3);

        // Test immutable iteration
        let mut values: Vec<i32> = set.iter().cloned().collect();
        values.sort(); // Sort since set order is not guaranteed
        assert_eq!(values, vec![1, 2, 3]);

        // Test mutable iteration
        for value in &mut set {
            *value += 10;
        }

        let mut values: Vec<i32> = set.iter().cloned().collect();
        values.sort();
        assert_eq!(values, vec![11, 12, 13]);
    }

    #[test]
    fn test_tset_clear() {
        setup_test_globals();

        let mut set: TSet<i32> = TSet::new();

        set.insert(1);
        set.insert(2);
        set.insert(3);

        assert_eq!(set.len(), 3);

        set.clear();
        assert_eq!(set.len(), 0);
        assert!(set.is_empty());

        // Should still work after clearing
        set.insert(42);
        assert_eq!(set.len(), 1);
        assert!(set.contains(&42));
    }

    #[test]
    fn test_tset_with_capacity() {
        setup_test_globals();

        let mut set: TSet<i32> = TSet::with_capacity(10);
        assert_eq!(set.len(), 0);

        for i in 0..5 {
            set.insert(i * 10);
        }

        assert_eq!(set.len(), 5);
        for i in 0..5 {
            assert!(set.contains(&(i * 10)));
        }
    }

    #[test]
    fn test_tset_sparse_array_behavior() {
        setup_test_globals();

        let mut set: TSet<i32> = TSet::new();

        let id1 = set.insert(10);
        let id2 = set.insert(20);
        let id3 = set.insert(30);

        // Remove middle element
        set.remove(id2);

        // Add new element - should reuse freed slot in sparse array
        let id4 = set.insert(40);
        assert_eq!(set.len(), 3);
        assert_eq!(set.get(id4), Some(&40));

        // Verify sparse array behavior
        assert_eq!(set.get(id1), Some(&10));
        assert_eq!(set.get(id3), Some(&30));
        assert_eq!(set.get(id4), Some(&40));

        // Note: In a real UE-compatible implementation, old IDs pointing to
        // reused slots may see new data. This matches UE behavior where
        // FSetElementId doesn't have generation counters.
    }

    #[test]
    fn test_tset_hash_collision_handling() {
        setup_test_globals();

        let mut set: TSet<u32> = TSet::new();

        // Insert values that might collide in hash table
        let values = vec![1, 2, 3, 4, 5, 16, 17, 32, 33];
        let mut ids = Vec::new();

        for &val in &values {
            ids.push(set.insert(val));
        }

        assert_eq!(set.len(), values.len());

        // Verify all values can be found
        for &val in &values {
            assert!(set.contains(&val), "Set should contain {}", val);
        }

        // Verify all IDs still point to correct values
        for (i, &val) in values.iter().enumerate() {
            assert_eq!(set.get(ids[i]), Some(&val));
        }

        // Remove some elements and verify others remain
        assert!(set.remove_by_key(&16));
        assert!(set.remove_by_key(&32));
        assert_eq!(set.len(), values.len() - 2);

        assert!(!set.contains(&16));
        assert!(!set.contains(&32));
        assert!(set.contains(&17));
        assert!(set.contains(&33));
    }

    #[test]
    fn test_tset_rehashing() {
        setup_test_globals();

        let mut set: TSet<u32> = TSet::new();

        // Insert many elements to force rehashing
        let mut values = Vec::new();
        for i in 0..100 {
            values.push(i * 7); // Use non-sequential values
            set.insert(i * 7);
        }

        assert_eq!(set.len(), 100);

        // Verify all elements are still findable after rehashing
        for &val in &values {
            assert!(set.contains(&val), "Should contain {} after rehashing", val);
        }

        // Test removal after rehashing
        for i in 0..50 {
            let val = i * 7;
            assert!(set.remove_by_key(&val));
        }

        assert_eq!(set.len(), 50);

        // Verify remaining elements
        for i in 50..100 {
            let val = i * 7;
            assert!(set.contains(&val), "Should still contain {}", val);
        }
    }

    #[test]
    fn test_tset_ttuple_support() {
        setup_test_globals();

        let mut set: TSet<TTuple<u32, u32>> = TSet::new();

        let tuple1 = TTuple { key: 1, value: 10 };
        let tuple2 = TTuple { key: 2, value: 20 };
        let tuple3 = TTuple { key: 1, value: 30 }; // Same key, different value

        let id1 = set.insert(tuple1.clone());
        let id2 = set.insert(tuple2.clone());
        let id3 = set.insert(tuple3.clone());

        assert_eq!(set.len(), 3);

        // Verify tuples can be found
        assert!(set.contains(&tuple1));
        assert!(set.contains(&tuple2));
        assert!(set.contains(&tuple3));

        // Verify IDs point to correct tuples
        assert_eq!(set.get(id1), Some(&tuple1));
        assert_eq!(set.get(id2), Some(&tuple2));
        assert_eq!(set.get(id3), Some(&tuple3));

        // Test removal
        assert!(set.remove_by_key(&tuple2));
        assert_eq!(set.len(), 2);
        assert!(!set.contains(&tuple2));
        assert!(set.contains(&tuple1));
        assert!(set.contains(&tuple3));
    }

    #[test]
    fn test_tset_pointer_types() {
        setup_test_globals();

        let mut set: TSet<*mut u32> = TSet::new();

        // Create some dummy values to get pointers
        let mut val1 = 42u32;
        let mut val2 = 84u32;
        let mut val3 = 126u32;

        let ptr1 = &mut val1 as *mut u32;
        let ptr2 = &mut val2 as *mut u32;
        let ptr3 = &mut val3 as *mut u32;

        // Insert pointers
        let id1 = set.insert(ptr1);
        let id2 = set.insert(ptr2);
        let id3 = set.insert(ptr3);

        assert_eq!(set.len(), 3);

        // Verify pointers can be found
        assert!(set.contains(&ptr1));
        assert!(set.contains(&ptr2));
        assert!(set.contains(&ptr3));

        // Verify IDs point to correct pointers
        assert_eq!(set.get(id1), Some(&ptr1));
        assert_eq!(set.get(id2), Some(&ptr2));
        assert_eq!(set.get(id3), Some(&ptr3));

        // Test removal
        assert!(set.remove_by_key(&ptr2));
        assert_eq!(set.len(), 2);
        assert!(!set.contains(&ptr2));
    }

    #[test]
    fn test_tset_edge_cases() {
        setup_test_globals();

        let mut set: TSet<i32> = TSet::new();

        // Test empty set operations
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
        assert!(!set.contains(&42));
        assert!(set.find(&42).is_none());
        assert!(!set.remove_by_key(&42));

        // Test single element
        let id = set.insert(42);
        assert!(!set.is_empty());
        assert_eq!(set.len(), 1);
        assert!(set.contains(&42));
        assert_eq!(set.get(id), Some(&42));

        // Test duplicate insertion (should replace)
        let id2 = set.insert(42);
        assert_eq!(set.len(), 1); // Should still be 1 if duplicates not allowed

        // Test clear
        set.clear();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
        assert!(!set.contains(&42));
        assert_eq!(set.get(id), None); // Old ID should be invalid
    }

    #[test]
    fn test_tset_iteration_after_modifications() {
        setup_test_globals();

        let mut set: TSet<u32> = TSet::new();

        // Insert initial values
        for i in 0..10 {
            set.insert(i * 10);
        }

        // Remove some values
        set.remove_by_key(&20);
        set.remove_by_key(&50);
        set.remove_by_key(&80);

        // Collect remaining values
        let mut remaining: Vec<u32> = set.iter().cloned().collect();
        remaining.sort();

        let expected = vec![0, 10, 30, 40, 60, 70, 90];
        assert_eq!(remaining, expected);
        assert_eq!(set.len(), 7);

        // Test iteration is stable after more modifications
        set.insert(25);
        set.insert(35);

        let mut updated: Vec<u32> = set.iter().cloned().collect();
        updated.sort();

        let expected_updated = vec![0, 10, 25, 30, 35, 40, 60, 70, 90];
        assert_eq!(updated, expected_updated);
    }

    #[test]
    fn test_tset_reserve_and_capacity() {
        setup_test_globals();

        let mut set: TSet<u32> = TSet::new();

        // Test initial capacity
        let initial_capacity = set.capacity();

        // Reserve space
        set.reserve(100);
        assert!(set.capacity() >= 100);
        assert!(set.capacity() >= initial_capacity);

        // Insert elements and verify capacity doesn't shrink unexpectedly
        for i in 0..50 {
            set.insert(i);
        }

        assert!(set.capacity() >= 50);
        assert_eq!(set.len(), 50);

        // Test with_capacity constructor
        let set2: TSet<u32> = TSet::with_capacity(200);
        assert!(set2.capacity() >= 200);
        assert_eq!(set2.len(), 0);
    }

    #[test]
    fn test_tset_hash_distribution() {
        setup_test_globals();

        let mut set: TSet<u64> = TSet::new();

        // Test with larger numbers to ensure hash distribution
        let large_values = vec![
            0x123456789ABCDEF0,
            0xFEDCBA9876543210,
            0x0F0F0F0F0F0F0F0F,
            0xF0F0F0F0F0F0F0F0,
            0x5555555555555555,
            0xAAAAAAAAAAAAAAAA,
        ];

        for &val in &large_values {
            set.insert(val);
        }

        assert_eq!(set.len(), large_values.len());

        // All should be findable
        for &val in &large_values {
            assert!(
                set.contains(&val),
                "Should contain large value 0x{:016X}",
                val
            );
        }
    }
}
