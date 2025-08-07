use super::*;
use std::{marker::PhantomData, mem::ManuallyDrop};

#[repr(C)]
pub union TSparseArrayElementOrFreeListLink<T> {
    pub element_data: ManuallyDrop<T>,
    pub free_list_link: (i32, i32), // (next_free_index, prev_free_index)
}

// Default sparse array allocator
#[derive(Default)]
pub struct TSparseArrayAllocator32;

impl Allocator for TSparseArrayAllocator32 {
    type ForAnyElementType<T> = TSizedHeapAllocatorForAnyElementType<T>;
}

#[repr(C)]
pub struct TSparseArray<T, Allocator = TSparseArrayAllocator32>
where
    Allocator: self::Allocator,
{
    pub data: TArray<TSparseArrayElementOrFreeListLink<T>, TSizedHeapAllocator32>,
    pub allocation_flags: TBitArray<TInlineAllocator<4>>,
    pub first_free_index: i32,
    pub num_free_indices: i32,
    _phantom: PhantomData<Allocator>,
}

impl<T, A> Default for TSparseArray<T, A>
where
    A: self::Allocator,
{
    fn default() -> Self {
        Self {
            data: TArray::default(),
            allocation_flags: TBitArray::default(),
            first_free_index: -1,
            num_free_indices: 0,
            _phantom: PhantomData,
        }
    }
}

impl<T, A> TSparseArray<T, A>
where
    A: self::Allocator,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let mut array = Self::new();
        array.reserve(capacity);
        array
    }

    pub fn len(&self) -> usize {
        (self.data.len() as i32 - self.num_free_indices) as usize
    }

    pub fn capacity(&self) -> usize {
        self.data.capacity()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn max_index(&self) -> usize {
        self.data.len()
    }

    pub fn reserve(&mut self, capacity: usize) {
        self.data.reserve_capacity(capacity);
        self.allocation_flags.resize(capacity, false);
    }

    pub fn add(&mut self, element: T) -> i32 {
        let index = if self.first_free_index >= 0 {
            // Use a free slot
            let index = self.first_free_index;

            unsafe {
                let slot = &mut self.data.as_mut_slice()[index as usize];
                // Update free list
                self.first_free_index = slot.free_list_link.0;

                // Place the element
                slot.element_data = ManuallyDrop::new(element);
            }

            self.allocation_flags.set_bit(index as usize, true);
            self.num_free_indices -= 1;
            index
        } else {
            // Add to end
            let index = self.data.len() as i32;

            // Ensure capacity
            if index >= self.allocation_flags.capacity() as i32 {
                let new_capacity = std::cmp::max(self.allocation_flags.capacity() * 2, 16);
                self.allocation_flags.resize(new_capacity, false);
            }

            let slot = TSparseArrayElementOrFreeListLink {
                element_data: ManuallyDrop::new(element),
            };
            self.data.push(slot);
            self.allocation_flags.set_bit(index as usize, true);
            index
        };

        index
    }

    pub fn remove(&mut self, index: i32) -> bool {
        if index < 0 || index as usize >= self.data.len() {
            return false;
        }

        if !self.allocation_flags.get_bit(index as usize) {
            return false; // Already removed
        }

        unsafe {
            let slot = &mut self.data.as_mut_slice()[index as usize];

            // Properly drop the element
            ManuallyDrop::drop(&mut slot.element_data);

            // Add to free list
            slot.free_list_link = (self.first_free_index, -1);
            self.first_free_index = index;
            self.num_free_indices += 1;
        }

        self.allocation_flags.set_bit(index as usize, false);
        true
    }

    pub fn is_valid_index(&self, index: i32) -> bool {
        index >= 0
            && (index as usize) < self.data.len()
            && self.allocation_flags.get_bit(index as usize)
    }

    pub fn get(&self, index: i32) -> Option<&T> {
        if self.is_valid_index(index) {
            unsafe {
                let slot = &self.data.as_slice()[index as usize];
                Some(&*slot.element_data)
            }
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, index: i32) -> Option<&mut T> {
        if self.is_valid_index(index) {
            unsafe {
                let slot = &mut self.data.as_mut_slice()[index as usize];
                Some(&mut *slot.element_data)
            }
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        // Properly drop all allocated elements
        for i in 0..self.data.len() {
            if self.allocation_flags.get_bit(i) {
                unsafe {
                    let slot = &mut self.data.as_mut_slice()[i];
                    ManuallyDrop::drop(&mut slot.element_data);
                }
            }
        }

        self.data.clear();
        self.allocation_flags.clear();
        self.first_free_index = -1;
        self.num_free_indices = 0;
    }

    pub fn iter(&self) -> SparseArrayIterator<'_, T> {
        SparseArrayIterator {
            data: self.data.as_slice(),
            allocation_flags: &self.allocation_flags,
            index: 0,
        }
    }

    pub fn iter_mut(&mut self) -> SparseArrayIteratorMut<'_, T> {
        SparseArrayIteratorMut {
            data: self.data.as_mut_slice(),
            allocation_flags: &self.allocation_flags,
            index: 0,
        }
    }

    pub fn indices(&self) -> SparseArrayIndexIterator<'_> {
        SparseArrayIndexIterator {
            allocation_flags: &self.allocation_flags,
            index: 0,
        }
    }
}

impl<T, A> Drop for TSparseArray<T, A>
where
    A: self::Allocator,
{
    fn drop(&mut self) {
        self.clear();
    }
}

pub struct SparseArrayIterator<'a, T> {
    data: &'a [TSparseArrayElementOrFreeListLink<T>],
    allocation_flags: &'a TBitArray<TInlineAllocator<4>>,
    index: usize,
}

impl<'a, T> Iterator for SparseArrayIterator<'a, T> {
    type Item = (i32, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.data.len() {
            let current_index = self.index;
            self.index += 1;

            if self.allocation_flags.get_bit(current_index) {
                unsafe {
                    let element = &*self.data[current_index].element_data;
                    return Some((current_index as i32, element));
                }
            }
        }
        None
    }
}

pub struct SparseArrayIteratorMut<'a, T> {
    data: &'a mut [TSparseArrayElementOrFreeListLink<T>],
    allocation_flags: &'a TBitArray<TInlineAllocator<4>>,
    index: usize,
}

impl<'a, T> Iterator for SparseArrayIteratorMut<'a, T> {
    type Item = (i32, &'a mut T);

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.data.len() {
            let current_index = self.index;
            self.index += 1;

            if self.allocation_flags.get_bit(current_index) {
                unsafe {
                    let ptr = self.data.as_mut_ptr().add(current_index);
                    let element = &mut *(*ptr).element_data;
                    return Some((current_index as i32, element));
                }
            }
        }
        None
    }
}

pub struct SparseArrayIndexIterator<'a> {
    allocation_flags: &'a TBitArray<TInlineAllocator<4>>,
    index: usize,
}

impl<'a> Iterator for SparseArrayIndexIterator<'a> {
    type Item = i32;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.allocation_flags.len() {
            let current_index = self.index;
            self.index += 1;

            if self.allocation_flags.get_bit(current_index) {
                return Some(current_index as i32);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use malloc::test::setup_test_globals;

    #[test]
    fn test_sparsearray_basic_operations() {
        setup_test_globals();

        let mut array: TSparseArray<i32> = TSparseArray::new();

        assert_eq!(array.len(), 0);
        assert!(array.is_empty());

        // Add some elements
        let id1 = array.add(42);
        let id2 = array.add(100);
        let id3 = array.add(200);

        assert_eq!(array.len(), 3);
        assert!(!array.is_empty());

        // Test get
        assert_eq!(array.get(id1), Some(&42));
        assert_eq!(array.get(id2), Some(&100));
        assert_eq!(array.get(id3), Some(&200));

        // Test invalid indices
        assert_eq!(array.get(-1), None);
        assert_eq!(array.get(999), None);
    }

    #[test]
    fn test_sparsearray_remove() {
        setup_test_globals();

        let mut array: TSparseArray<i32> = TSparseArray::new();

        let id1 = array.add(10);
        let id2 = array.add(20);
        let id3 = array.add(30);

        assert_eq!(array.len(), 3);

        // Remove middle element
        assert!(array.remove(id2));
        assert_eq!(array.len(), 2);

        // Check elements
        assert_eq!(array.get(id1), Some(&10));
        assert_eq!(array.get(id2), None);
        assert_eq!(array.get(id3), Some(&30));

        // Add new element - should reuse freed slot
        let id4 = array.add(40);
        assert_eq!(array.len(), 3);
        assert_eq!(array.get(id4), Some(&40));
    }

    #[test]
    fn test_sparsearray_iteration() {
        setup_test_globals();

        let mut array: TSparseArray<i32> = TSparseArray::new();

        let id1 = array.add(10);
        let id2 = array.add(20);
        let id3 = array.add(30);

        // Remove middle element
        array.remove(id2);

        // Test iteration - should only see valid elements
        let mut elements: Vec<(i32, i32)> = array.iter().map(|(i, &v)| (i, v)).collect();
        elements.sort_by_key(|&(i, _)| i);

        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0], (id1, 10));
        assert_eq!(elements[1], (id3, 30));

        // Test mutable iteration
        for (_, value) in array.iter_mut() {
            *value += 100;
        }

        assert_eq!(array.get(id1), Some(&110));
        assert_eq!(array.get(id3), Some(&130));
    }

    #[test]
    fn test_sparsearray_indices() {
        setup_test_globals();

        let mut array: TSparseArray<i32> = TSparseArray::new();

        let id1 = array.add(10);
        let id2 = array.add(20);
        let id3 = array.add(30);
        array.remove(id2);

        let indices: Vec<i32> = array.indices().collect();
        assert_eq!(indices, vec![id1, id3]);
    }

    #[test]
    fn test_sparsearray_clear() {
        setup_test_globals();

        let mut array: TSparseArray<i32> = TSparseArray::new();

        array.add(1);
        array.add(2);
        array.add(3);

        assert_eq!(array.len(), 3);

        array.clear();
        assert_eq!(array.len(), 0);
        assert!(array.is_empty());

        // Should work after clearing
        let id = array.add(42);
        assert_eq!(array.get(id), Some(&42));
    }
}
