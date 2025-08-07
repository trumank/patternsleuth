use super::*;
use std::marker::PhantomData;

#[repr(C)]
#[derive(Debug)]
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

// Default key funcs type
#[derive(Default)]
pub struct DefaultKeyFuncs;

#[repr(C)]
pub struct TSet<ElementType, KeyFuncs = DefaultKeyFuncs, SetAllocator = FDefaultSetAllocator>
where
    SetAllocator: Allocator,
{
    // Use TSparseArray for element storage like real UE
    pub elements: TSparseArray<TSetElement<ElementType>, TSparseArrayAllocator32>,
    // Hash table using inline allocator like real UE
    pub hash: TInlineAllocatorForElementType<FSetElementId, 1>,
    pub hash_size: i32,
    _phantom: PhantomData<(KeyFuncs, SetAllocator)>,
}

impl<T, K, A> std::fmt::Debug for TSet<T, K, A>
where
    A: Allocator,
    T: std::fmt::Debug,
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
{
    fn drop(&mut self) {
        self.hash.deallocate();
    }
}

impl<T, K, A> TSet<T, K, A>
where
    A: Allocator,
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

        // Initialize hash table with reasonable size
        let hash_size = std::cmp::max(capacity.next_power_of_two(), 16);
        if self.hash_size < hash_size as i32 {
            // Allocate hash table space
            if hash_size <= 1 {
                // Use inline storage
                self.hash_size = 1;
            } else {
                // Use secondary storage
                self.hash.allocate(hash_size);
                // Initialize hash table to invalid entries
                let ptr = self.hash.data_ptr_mut();
                if !ptr.is_null() {
                    unsafe {
                        for i in 0..hash_size {
                            std::ptr::write(ptr.add(i), FSetElementId::INVALID);
                        }
                    }
                }
                self.hash_size = hash_size as i32;
            }
        }
    }

    // Stubbed insert - doesn't actually hash or check for duplicates
    pub fn insert(&mut self, element: T) -> FSetElementId {
        if self.elements.capacity() == 0 {
            self.reserve(16);
        }

        let sparse_index = self.elements.add(TSetElement::new(element));
        let element_id = FSetElementId {
            index: sparse_index,
        };

        // TODO: Implement proper hashing and collision handling
        element_id
    }

    // Stubbed remove - uses sparse array removal
    pub fn remove(&mut self, element_id: FSetElementId) -> bool {
        if !element_id.is_valid() {
            return false;
        }

        // Use sparse array's removal which handles free list management
        self.elements.remove(element_id.index)
    }

    // Stubbed find - linear search through sparse array
    pub fn find(&self, key: &T) -> FSetElementId
    where
        T: PartialEq,
    {
        // TODO: Implement proper hash lookup
        // For now, do linear search through sparse array
        for (sparse_index, element) in self.elements.iter() {
            if &element.value == key {
                return FSetElementId {
                    index: sparse_index,
                };
            }
        }
        FSetElementId::INVALID
    }

    pub fn contains(&self, key: &T) -> bool
    where
        T: PartialEq,
    {
        self.find(key).is_valid()
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
        // Reset hash table
        let ptr = self.hash.data_ptr_mut();
        if !ptr.is_null() {
            unsafe {
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

        let id = set.find(&20);
        assert!(id.is_valid());
        assert_eq!(set.get(id), Some(&20));

        let invalid_id = set.find(&99);
        assert!(!invalid_id.is_valid());
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

        // Note: id2 may now point to the reused slot containing 40
        // This is expected behavior for sparse arrays - once a slot is reused,
        // old IDs that pointed to that slot will see the new data
        // In a real implementation, you'd need generation counters or similar
        // to make old IDs truly invalid
    }
}
