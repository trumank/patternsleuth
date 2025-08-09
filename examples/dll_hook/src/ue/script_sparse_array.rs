use super::*;
use std::ffi::c_void;
use std::marker::PhantomData;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FScriptSparseArrayLayout {
    pub element_offset: i32, // Always 0 for sparse array elements
    pub element_size: i32,
    pub first_free_index_offset: i32,
    pub num_free_indices_offset: i32,
}

impl FScriptSparseArrayLayout {
    pub fn get_layout(element_size: i32, _element_alignment: i32) -> Self {
        Self {
            element_offset: 0,
            element_size,
            first_free_index_offset: 48, // After TArray and TBitArray
            num_free_indices_offset: 52, // 4 bytes after first_free_index
        }
    }
}

// Untyped sparse array type for accessing TSparseArray data
// Must have the same memory representation as TSparseArray
#[repr(C)]
pub struct TScriptSparseArray<Allocator = TSparseArrayAllocator32, DerivedType = ()>
where
    Allocator: self::Allocator,
{
    // Data storage: TArray of element union
    pub data: FScriptArray,
    // Allocation flags: TBitArray
    pub allocation_flags: TBitArray<TInlineAllocator<4>>,
    // Free list management
    pub first_free_index: i32,
    pub num_free_indices: i32,
    _phantom: PhantomData<(Allocator, DerivedType)>,
}

impl<A, D> Default for TScriptSparseArray<A, D>
where
    A: self::Allocator,
{
    fn default() -> Self {
        Self {
            data: FScriptArray::new(),
            allocation_flags: TBitArray::new(),
            first_free_index: -1,
            num_free_indices: 0,
            _phantom: PhantomData,
        }
    }
}

impl<A, D> TScriptSparseArray<A, D>
where
    A: self::Allocator,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_valid_index(&self, index: i32) -> bool {
        index >= 0 && index < self.data.num() && self.allocation_flags.get_bit(index as usize)
    }

    pub fn is_empty(&self) -> bool {
        self.num() == 0
    }

    pub fn num(&self) -> i32 {
        self.data.num() - self.num_free_indices
    }

    pub fn num_unchecked(&self) -> i32 {
        self.data.num_unchecked() - self.num_free_indices
    }

    pub fn get_max_index(&self) -> i32 {
        self.data.num()
    }

    pub fn get_data(&self, index: i32, layout: &FScriptSparseArrayLayout) -> *const c_void {
        if !self.is_valid_index(index) {
            return std::ptr::null();
        }

        let element_size = layout.element_size as usize;
        let data_ptr = self.data.get_data() as *const u8;
        unsafe { data_ptr.add((index as usize) * element_size) as *const c_void }
    }

    pub fn get_data_mut(&mut self, index: i32, layout: &FScriptSparseArrayLayout) -> *mut c_void {
        if !self.is_valid_index(index) {
            return std::ptr::null_mut();
        }

        let element_size = layout.element_size as usize;
        let data_ptr = self.data.get_data_mut() as *mut u8;
        unsafe { data_ptr.add((index as usize) * element_size) as *mut c_void }
    }

    pub fn add_uninitialized(&mut self, layout: &FScriptSparseArrayLayout) -> i32 {
        let element_size = layout.element_size;

        if self.first_free_index >= 0 {
            // Reuse a free slot
            let index = self.first_free_index;

            // Update free list by reading the next free index from the slot
            let data_ptr = self.data.get_data_mut() as *mut i32;
            unsafe {
                self.first_free_index = *data_ptr.add(index as usize * (element_size as usize / 4));
            }

            self.allocation_flags.set_bit(index as usize, true);
            self.num_free_indices -= 1;
            index
        } else {
            // Add to end
            let index = self.data.add(1, element_size);

            // Ensure bit array has enough capacity
            let required_capacity = (index + 1) as usize;
            if self.allocation_flags.capacity() < required_capacity {
                let new_capacity =
                    std::cmp::max(required_capacity, self.allocation_flags.capacity() * 2);
                self.allocation_flags.resize(new_capacity, false);
            }

            self.allocation_flags.set_bit(index as usize, true);
            index
        }
    }

    pub fn remove_at_uninitialized(&mut self, layout: &FScriptSparseArrayLayout, index: i32) {
        if !self.is_valid_index(index) {
            return;
        }

        // Mark as free in allocation flags
        self.allocation_flags.set_bit(index as usize, false);

        // Add to free list by storing the current first_free_index at this slot
        let element_size = layout.element_size;
        let data_ptr = self.data.get_data_mut() as *mut i32;
        unsafe {
            *data_ptr.add(index as usize * (element_size as usize / 4)) = self.first_free_index;
        }

        self.first_free_index = index;
        self.num_free_indices += 1;
    }

    pub fn empty(&mut self, slack: i32, layout: &FScriptSparseArrayLayout) {
        let element_size = layout.element_size;

        // Clear data array
        self.data.empty(slack, element_size);

        // Clear bit array
        self.allocation_flags.clear();
        if slack > 0 {
            self.allocation_flags.resize(slack as usize, false);
        }

        // Reset free list
        self.first_free_index = -1;
        self.num_free_indices = 0;
    }

    pub fn shrink(&mut self, layout: &FScriptSparseArrayLayout) {
        let element_size = layout.element_size;

        // Shrink underlying data array
        self.data.shrink(element_size);

        // Shrink bit array
        let max_index = self.data.num() as usize;
        if self.allocation_flags.len() > max_index {
            self.allocation_flags.resize(max_index, false);
        }
    }

    pub fn compact(&mut self, layout: &FScriptSparseArrayLayout) -> bool {
        if self.num_free_indices == 0 {
            return false; // Nothing to compact
        }

        let element_size = layout.element_size as usize;
        let data_ptr = self.data.get_data_mut() as *mut u8;
        let mut write_index = 0i32;
        let max_index = self.data.num();

        // Compact by moving all valid elements to the front
        for read_index in 0..max_index {
            if self.allocation_flags.get_bit(read_index as usize) {
                if write_index != read_index {
                    // Move element from read_index to write_index
                    unsafe {
                        let src = data_ptr.add((read_index as usize) * element_size);
                        let dst = data_ptr.add((write_index as usize) * element_size);
                        std::ptr::copy(src, dst, element_size);
                    }

                    // Update allocation flags
                    self.allocation_flags.set_bit(write_index as usize, true);
                    self.allocation_flags.set_bit(read_index as usize, false);
                }
                write_index += 1;
            }
        }

        // Update array size
        self.data
            .set_num_uninitialized(write_index, layout.element_size);

        // Update bit array
        self.allocation_flags.resize(write_index as usize, false);

        // Reset free list
        self.first_free_index = -1;
        self.num_free_indices = 0;

        true
    }

    pub fn compact_stable(&mut self, layout: &FScriptSparseArrayLayout) -> bool {
        // For stable compact, we maintain the relative order of elements
        // This is more complex but preserves iteration order
        // For now, just delegate to regular compact
        self.compact(layout)
    }

    pub fn move_assign(&mut self, mut other: Self, layout: &FScriptSparseArrayLayout) {
        if std::ptr::eq(self, &other) {
            return;
        }

        // Clear current contents
        self.empty(0, layout);

        // Move data from other
        std::mem::swap(&mut self.data, &mut other.data);
        std::mem::swap(&mut self.allocation_flags, &mut other.allocation_flags);
        self.first_free_index = other.first_free_index;
        self.num_free_indices = other.num_free_indices;
    }

    pub fn reserve(&mut self, expected_num_elements: i32, layout: &FScriptSparseArrayLayout) {
        if expected_num_elements <= 0 {
            return;
        }

        let element_size = layout.element_size;

        // Reserve space in data array
        let current_capacity = self.data.get_allocated_size(element_size) / element_size as usize;
        if (expected_num_elements as usize) > current_capacity {
            // Calculate new capacity - round up to power of 2 for efficiency
            let new_capacity = (expected_num_elements as usize).next_power_of_two();

            // Resize data array
            let old_num = self.data.num();
            self.data
                .set_num_uninitialized((new_capacity as i32).max(old_num), element_size);
            self.data.set_num_uninitialized(old_num, element_size); // Reset to original size

            // Resize bit array
            self.allocation_flags.resize(new_capacity, false);
        }
    }

    pub fn get_allocated_size(&self, layout: &FScriptSparseArrayLayout) -> usize {
        let data_size = self.data.get_allocated_size(layout.element_size);
        let flags_size = (self.allocation_flags.max_bits as usize + 7) / 8; // Approximate bit array size
        data_size + flags_size
    }

    // Iterator support would go here, but requires more complex lifetime management
    // for now, access elements directly using get_data/get_data_mut
}

// FScriptSparseArray is a concrete type alias for the default allocator
pub type FScriptSparseArray = TScriptSparseArray<TSparseArrayAllocator32, ()>;

// Static assertions to ensure layout compatibility
const _: () = {
    // Verify that TScriptSparseArray has the same size as the expected UE layout
    // Based on fscript_set.h: TScriptSparseArray should be 56 bytes
    assert!(std::mem::size_of::<TScriptSparseArray<TSparseArrayAllocator32, ()>>() == 56);
};

#[cfg(test)]
mod tests {
    use super::*;
    use malloc::test::setup_test_globals;

    #[test]
    fn test_script_sparse_array_basic() {
        setup_test_globals();

        let mut array = TScriptSparseArray::<TSparseArrayAllocator32, ()>::new();
        let layout = FScriptSparseArrayLayout::get_layout(4, 4); // i32 elements

        assert!(array.is_empty());
        assert_eq!(array.num(), 0);
        assert_eq!(array.get_max_index(), 0);

        // Add some elements
        let index1 = array.add_uninitialized(&layout);
        let index2 = array.add_uninitialized(&layout);
        let index3 = array.add_uninitialized(&layout);

        assert!(!array.is_empty());
        assert_eq!(array.num(), 3);
        assert_eq!(array.get_max_index(), 3);

        // Verify valid indices
        assert!(array.is_valid_index(index1));
        assert!(array.is_valid_index(index2));
        assert!(array.is_valid_index(index3));
        assert!(!array.is_valid_index(-1));
        assert!(!array.is_valid_index(100));
    }

    #[test]
    fn test_script_sparse_array_remove_and_reuse() {
        setup_test_globals();

        let mut array = TScriptSparseArray::<TSparseArrayAllocator32, ()>::new();
        let layout = FScriptSparseArrayLayout::get_layout(4, 4);

        let index1 = array.add_uninitialized(&layout);
        let index2 = array.add_uninitialized(&layout);
        let index3 = array.add_uninitialized(&layout);

        assert_eq!(array.num(), 3);

        // Remove middle element
        array.remove_at_uninitialized(&layout, index2);
        assert_eq!(array.num(), 2);
        assert!(!array.is_valid_index(index2));

        // Add new element - should reuse the freed slot
        let index4 = array.add_uninitialized(&layout);
        assert_eq!(array.num(), 3);

        // The reused index should be the same as the removed one
        assert_eq!(index4, index2);
        assert!(array.is_valid_index(index4));
    }

    #[test]
    fn test_script_sparse_array_empty_and_reserve() {
        setup_test_globals();

        let mut array = TScriptSparseArray::<TSparseArrayAllocator32, ()>::new();
        let layout = FScriptSparseArrayLayout::get_layout(8, 8);

        // Add some elements
        array.add_uninitialized(&layout);
        array.add_uninitialized(&layout);
        array.add_uninitialized(&layout);
        assert_eq!(array.num(), 3);

        // Empty with slack
        array.empty(10, &layout);
        assert!(array.is_empty());
        assert_eq!(array.num(), 0);

        // Reserve space
        array.reserve(20, &layout);

        // Should still be empty but have capacity
        assert!(array.is_empty());
        assert!(array.get_allocated_size(&layout) > 0);
    }
}
