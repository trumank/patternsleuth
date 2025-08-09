use super::*;
use std::marker::PhantomData;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FScriptSetLayout {
    // Element is always at offset 0 in TSetElement - not stored here
    pub hash_next_id_offset: i32,
    pub hash_index_offset: i32,
    pub size: i32,
    pub sparse_array_layout: FScriptSparseArrayLayout,
}

impl FScriptSetLayout {
    pub fn get_layout(element_size: i32, element_alignment: i32) -> Self {
        // Calculate TSetElement<T> layout
        let mut offset = 0i32;

        // Element comes first (aligned to element_alignment)
        let _element_offset = 0; // Always at start
        offset += element_size;

        // FSetElementId hash_next_id (4 bytes, aligned to 4)
        offset = align_up(offset, 4);
        let hash_next_id_offset = offset;
        offset += 4;

        // int32 hash_index (4 bytes, aligned to 4)
        offset = align_up(offset, 4);
        let hash_index_offset = offset;
        offset += 4;

        // Total size of TSetElement
        let total_size = align_up(offset, element_alignment);

        Self {
            hash_next_id_offset,
            hash_index_offset,
            size: total_size,
            sparse_array_layout: FScriptSparseArrayLayout::get_layout(
                total_size,
                element_alignment,
            ),
        }
    }
}

fn align_up(value: i32, alignment: i32) -> i32 {
    (value + alignment - 1) & !(alignment - 1)
}

// Untyped set type for accessing TSet data, like FScriptArray for TArray
// Must have the same memory representation as a TSet
#[repr(C)]
pub struct TScriptSet<Allocator = FDefaultSetAllocator, DerivedType = ()>
where
    Allocator: self::Allocator,
{
    // Elements stored in sparse array
    pub elements: TScriptSparseArray<TSparseArrayAllocator32, ()>,

    // Hash table - inline allocator with 1 element inline, then heap
    pub hash: TInlineAllocatorForElementType<FSetElementId, 1>,

    // Number of hash buckets
    pub hash_size: i32,

    _phantom: PhantomData<(Allocator, DerivedType)>,
}

impl<A, D> Default for TScriptSet<A, D>
where
    A: self::Allocator,
{
    fn default() -> Self {
        Self {
            elements: TScriptSparseArray::new(),
            hash: TInlineAllocatorForElementType::new(),
            hash_size: 0,
            _phantom: PhantomData,
        }
    }
}

impl<A, D> TScriptSet<A, D>
where
    A: self::Allocator,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_valid_index(&self, index: i32) -> bool {
        self.elements.is_valid_index(index)
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn num(&self) -> i32 {
        self.elements.num()
    }

    pub fn num_unchecked(&self) -> i32 {
        self.elements.num_unchecked()
    }

    pub fn get_max_index(&self) -> i32 {
        self.elements.get_max_index()
    }

    pub fn get_data(&self, index: i32, layout: &FScriptSetLayout) -> *const c_void {
        self.elements.get_data(index, &layout.sparse_array_layout)
    }

    pub fn get_data_mut(&mut self, index: i32, layout: &FScriptSetLayout) -> *mut c_void {
        self.elements
            .get_data_mut(index, &layout.sparse_array_layout)
    }

    pub fn move_assign(&mut self, mut other: Self, layout: &FScriptSetLayout) {
        if std::ptr::eq(self, &other) {
            return;
        }

        self.empty(0, layout);

        // Move elements
        std::mem::swap(&mut self.elements, &mut other.elements);

        // Move hash table
        std::mem::swap(&mut self.hash, &mut other.hash);
        self.hash_size = other.hash_size;
    }

    pub fn empty(&mut self, slack: i32, layout: &FScriptSetLayout) {
        // Empty elements
        self.elements.empty(slack, &layout.sparse_array_layout);

        // Calculate desired hash size
        let desired_hash_size = if slack > 0 {
            get_number_of_hash_buckets(slack)
        } else {
            0
        };

        // Resize hash if needed
        if slack != 0 && (self.hash_size == 0 || self.hash_size != desired_hash_size) {
            self.hash_size = desired_hash_size;

            // Allocate and initialize hash table
            let hash_ptr = if desired_hash_size <= 1 {
                // Use inline storage
                self.hash.inline_data.as_mut_ptr() as *mut FSetElementId
            } else {
                // Use secondary storage
                self.hash.reallocate(desired_hash_size as usize)
            };

            // Initialize all hash entries to invalid
            if !hash_ptr.is_null() {
                unsafe {
                    for i in 0..desired_hash_size {
                        std::ptr::write(hash_ptr.add(i as usize), FSetElementId::INVALID);
                    }
                }
            }
        }
    }

    pub fn remove_at(&mut self, index: i32, layout: &FScriptSetLayout) {
        if !self.is_valid_index(index) {
            return;
        }

        let element_ptr = self.get_data(index, layout);
        if element_ptr.is_null() {
            return;
        }

        // Remove element from hash table
        let hash_index = get_hash_index_ref(element_ptr, layout);
        let hash_ptr = self.get_typed_hash_mut(0); // Get base pointer

        if !hash_ptr.is_null() {
            unsafe {
                let mut next_element_id_ptr = hash_ptr.add(hash_index as usize);

                loop {
                    let next_element_id = *next_element_id_ptr;
                    if !next_element_id.is_valid() {
                        break; // Corrupt hash
                    }

                    if next_element_id.index == index {
                        // Found the element, remove it from chain
                        let hash_next_id = get_hash_next_id_ref(element_ptr, layout);
                        *next_element_id_ptr = hash_next_id;
                        break;
                    }

                    // Move to next element in chain
                    let next_element_ptr = self.get_data_mut(next_element_id.index, layout);
                    if next_element_ptr.is_null() {
                        break;
                    }
                    next_element_id_ptr = get_hash_next_id_ref_mut(next_element_ptr, layout);
                }
            }
        }

        // Remove from sparse array
        self.elements
            .remove_at_uninitialized(&layout.sparse_array_layout, index);
    }

    pub fn add_uninitialized(&mut self, layout: &FScriptSetLayout) -> i32 {
        let index = self.elements.add_uninitialized(&layout.sparse_array_layout);

        // Initialize hash metadata fields to avoid undefined behavior
        let element_ptr = self.get_data_mut(index, layout);
        if !element_ptr.is_null() {
            unsafe {
                let element_bytes = element_ptr as *mut u8;

                // Initialize hash_next_id field
                let hash_next_id_ptr =
                    element_bytes.add(layout.hash_next_id_offset as usize) as *mut FSetElementId;
                std::ptr::write(hash_next_id_ptr, FSetElementId::INVALID);

                // Initialize hash_index field
                let hash_index_ptr =
                    element_bytes.add(layout.hash_index_offset as usize) as *mut i32;
                std::ptr::write(hash_index_ptr, -1);
            }
        }

        index
    }

    pub fn rehash(
        &mut self,
        layout: &FScriptSetLayout,
        get_key_hash: impl Fn(*const c_void) -> u32,
    ) {
        // Calculate new hash size based on current number of elements
        let new_hash_size = get_number_of_hash_buckets(self.elements.num());
        self.hash_size = new_hash_size;

        if new_hash_size == 0 {
            self.hash.deallocate();
            return;
        }

        // Allocate new hash table
        let hash_ptr = if new_hash_size <= 1 {
            self.hash.inline_data.as_mut_ptr() as *mut FSetElementId
        } else {
            self.hash.reallocate(new_hash_size as usize)
        };

        if hash_ptr.is_null() {
            return;
        }

        // Initialize hash table
        unsafe {
            for i in 0..new_hash_size {
                std::ptr::write(hash_ptr.add(i as usize), FSetElementId::INVALID);
            }
        }

        // Rehash all existing elements
        let mut index = 0;
        let mut count = self.elements.num();

        while count > 0 {
            if self.elements.is_valid_index(index) {
                let element_ptr = self.get_data_mut(index, layout);
                if !element_ptr.is_null() {
                    let key_hash = get_key_hash(element_ptr);
                    let hash_bucket = (key_hash as i32) & (new_hash_size - 1);

                    // Set hash index for this element
                    *get_hash_index_ref_mut(element_ptr, layout) = hash_bucket;

                    // Link into hash bucket
                    unsafe {
                        let bucket_ptr = hash_ptr.add(hash_bucket as usize);
                        *get_hash_next_id_ref_mut(element_ptr, layout) = *bucket_ptr;
                        *bucket_ptr = FSetElementId { index };
                    }
                }
                count -= 1;
            }
            index += 1;
        }
    }

    pub fn find_index(
        &self,
        element: *const c_void,
        layout: &FScriptSetLayout,
        get_key_hash: impl Fn(*const c_void) -> u32,
        equality_fn: impl Fn(*const c_void, *const c_void) -> bool,
    ) -> i32 {
        if self.elements.num() == 0 {
            return -1; // INDEX_NONE
        }

        self.find_index_by_hash(element, layout, get_key_hash(element), equality_fn)
    }

    pub fn find_index_by_hash(
        &self,
        element: *const c_void,
        layout: &FScriptSetLayout,
        key_hash: u32,
        equality_fn: impl Fn(*const c_void, *const c_void) -> bool,
    ) -> i32 {
        if self.elements.num() == 0 || self.hash_size == 0 {
            return -1; // INDEX_NONE
        }

        let hash_index = (key_hash as i32) & (self.hash_size - 1);
        let hash_ptr = self.get_typed_hash(0); // Get base pointer

        if hash_ptr.is_null() {
            return -1;
        }

        unsafe {
            let mut element_id = *hash_ptr.add(hash_index as usize);

            while element_id.is_valid() {
                let current_element = self.get_data(element_id.index, layout);
                if !current_element.is_null() && equality_fn(element, current_element) {
                    return element_id.index;
                }

                // Move to next element in hash chain
                element_id = get_hash_next_id_ref(current_element, layout);
            }
        }

        -1 // INDEX_NONE
    }

    pub fn find_or_add(
        &mut self,
        element: *const c_void,
        layout: &FScriptSetLayout,
        get_key_hash: impl Fn(*const c_void) -> u32,
        equality_fn: impl Fn(*const c_void, *const c_void) -> bool,
        construct_fn: impl Fn(*mut c_void),
    ) -> i32 {
        let key_hash = get_key_hash(element);
        let existing_index = self.find_index_by_hash(element, layout, key_hash, equality_fn);

        if existing_index != -1 {
            return existing_index;
        }

        self.add_new_element(layout, get_key_hash, key_hash, construct_fn)
    }

    pub fn add(
        &mut self,
        element: *const c_void,
        layout: &FScriptSetLayout,
        get_key_hash: impl Fn(*const c_void) -> u32,
        equality_fn: impl Fn(*const c_void, *const c_void) -> bool,
        construct_fn: impl Fn(*mut c_void),
        destruct_fn: impl Fn(*mut c_void),
    ) {
        let key_hash = get_key_hash(element);
        let existing_index = self.find_index_by_hash(element, layout, key_hash, &equality_fn);

        if existing_index != -1 {
            // Replace existing element
            let element_ptr = self.get_data_mut(existing_index, layout);
            if !element_ptr.is_null() {
                destruct_fn(element_ptr);
                construct_fn(element_ptr);
            }
        } else {
            // Add new element
            self.add_new_element(layout, get_key_hash, key_hash, construct_fn);
        }
    }

    fn add_new_element(
        &mut self,
        layout: &FScriptSetLayout,
        get_key_hash: impl Fn(*const c_void) -> u32,
        key_hash: u32,
        construct_fn: impl Fn(*mut c_void),
    ) -> i32 {
        let new_element_index = self.elements.add_uninitialized(&layout.sparse_array_layout);
        let element_ptr = self.get_data_mut(new_element_index, layout);

        if element_ptr.is_null() {
            return -1;
        }

        // Construct the element
        construct_fn(element_ptr);

        // Check if we need to rehash
        let desired_hash_size = get_number_of_hash_buckets(self.num());
        if self.hash_size == 0 || self.hash_size < desired_hash_size {
            // Rehash will handle linking the new element
            self.rehash(layout, get_key_hash);
        } else {
            // Link the new element into existing hash
            let hash_bucket = (key_hash as i32) & (self.hash_size - 1);
            let hash_ptr = self.get_typed_hash_mut(hash_bucket);

            if !hash_ptr.is_null() {
                unsafe {
                    *get_hash_index_ref_mut(element_ptr, layout) = hash_bucket;
                    *get_hash_next_id_ref_mut(element_ptr, layout) = *hash_ptr;
                    *hash_ptr = FSetElementId {
                        index: new_element_index,
                    };
                }
            }
        }

        new_element_index
    }

    fn get_typed_hash(&self, hash_index: i32) -> *const FSetElementId {
        if self.hash_size == 0 {
            return std::ptr::null();
        }

        let adjusted_index = (hash_index & (self.hash_size - 1)) as usize;

        if self.hash_size <= 1 {
            // Using inline storage
            unsafe { (self.hash.inline_data.as_ptr() as *const FSetElementId).add(adjusted_index) }
        } else {
            // Using secondary storage
            let secondary_ptr = self.hash.secondary_data.data_ptr();
            if secondary_ptr.is_null() {
                std::ptr::null()
            } else {
                unsafe { secondary_ptr.add(adjusted_index) }
            }
        }
    }

    fn get_typed_hash_mut(&mut self, hash_index: i32) -> *mut FSetElementId {
        if self.hash_size == 0 {
            return std::ptr::null_mut();
        }

        let adjusted_index = (hash_index & (self.hash_size - 1)) as usize;

        if self.hash_size <= 1 {
            // Using inline storage
            unsafe {
                (self.hash.inline_data.as_mut_ptr() as *mut FSetElementId).add(adjusted_index)
            }
        } else {
            // Using secondary storage
            let secondary_ptr = self.hash.secondary_data.data_ptr_mut();
            if secondary_ptr.is_null() {
                std::ptr::null_mut()
            } else {
                unsafe { secondary_ptr.add(adjusted_index) }
            }
        }
    }
}

// Helper functions for accessing TSetElement fields
fn get_hash_next_id_ref(element: *const c_void, layout: &FScriptSetLayout) -> FSetElementId {
    unsafe {
        let ptr =
            (element as *const u8).add(layout.hash_next_id_offset as usize) as *const FSetElementId;
        *ptr
    }
}

fn get_hash_next_id_ref_mut(element: *mut c_void, layout: &FScriptSetLayout) -> &mut FSetElementId {
    unsafe {
        let ptr =
            (element as *mut u8).add(layout.hash_next_id_offset as usize) as *mut FSetElementId;
        &mut *ptr
    }
}

fn get_hash_index_ref(element: *const c_void, layout: &FScriptSetLayout) -> i32 {
    unsafe {
        let ptr = (element as *const u8).add(layout.hash_index_offset as usize) as *const i32;
        *ptr
    }
}

fn get_hash_index_ref_mut(element: *mut c_void, layout: &FScriptSetLayout) -> &mut i32 {
    unsafe {
        let ptr = (element as *mut u8).add(layout.hash_index_offset as usize) as *mut i32;
        &mut *ptr
    }
}

// Helper function to calculate number of hash buckets (power of 2)
fn get_number_of_hash_buckets(num_elements: i32) -> i32 {
    if num_elements <= 0 {
        return 0;
    }

    // UE typically uses load factor around 0.75, so we want about 1.33x elements in hash size
    let target_buckets = ((num_elements as f32 * 1.33) as u32).next_power_of_two();
    std::cmp::max(target_buckets as i32, 8) // Minimum 8 buckets
}

impl<A, D> Drop for TScriptSet<A, D>
where
    A: self::Allocator,
{
    fn drop(&mut self) {
        // The hash allocator will be dropped automatically via TInlineAllocatorForElementType's Drop
        // The elements sparse array will be dropped automatically via TScriptSparseArray's Drop
        // This explicit Drop implementation ensures proper cleanup ordering
    }
}

// FScriptSet is a concrete type alias using the default allocator
pub type FScriptSet = TScriptSet<FDefaultSetAllocator, ()>;

// Implementation that makes FScriptSet compatible with the expected interface
impl FScriptSet {
    // Additional methods specific to FScriptSet can be added here
    // This matches the interface expected by UE reflection system
}

// Static assertions to ensure layout compatibility with UE
const _: () = {
    // Verify that TScriptSet has the same size as the expected UE layout
    // Based on fscript_set.h and mod.rs: FScriptSet should be 80 bytes
    assert!(std::mem::size_of::<TScriptSet<FDefaultSetAllocator, ()>>() == 80);

    // Verify FSetElementId size
    assert!(std::mem::size_of::<FSetElementId>() == 4);
};

#[cfg(test)]
mod tests {
    use super::*;
    use malloc::test::setup_test_globals;

    #[test]
    fn test_script_set_layout() {
        let layout = FScriptSetLayout::get_layout(4, 4); // i32 element

        // Element should be at offset 0
        assert_eq!(layout.hash_next_id_offset, 4); // After 4-byte element
        assert_eq!(layout.hash_index_offset, 8); // After hash_next_id
        assert_eq!(layout.size, 12); // Total TSetElement<i32> size
    }

    #[test]
    fn test_script_set_basic() {
        setup_test_globals();

        let mut set = FScriptSet::new();
        let layout = FScriptSetLayout::get_layout(4, 4);

        assert!(set.is_empty());
        assert_eq!(set.num(), 0);

        // Add an element
        let index = set.add_uninitialized(&layout);
        assert!(!set.is_empty());
        assert_eq!(set.num(), 1);
        assert!(set.is_valid_index(index));

        // Get element data
        let element_ptr = set.get_data_mut(index, &layout);
        assert!(!element_ptr.is_null());

        // Write a value to the element
        unsafe {
            *(element_ptr as *mut i32) = 42;
        }

        // Read it back
        let read_ptr = set.get_data(index, &layout);
        assert!(!read_ptr.is_null());
        unsafe {
            assert_eq!(*(read_ptr as *const i32), 42);
        }
    }

    #[test]
    fn test_script_set_empty() {
        setup_test_globals();

        let mut set = FScriptSet::new();
        let layout = FScriptSetLayout::get_layout(4, 4);

        // Add some elements
        set.add_uninitialized(&layout);
        set.add_uninitialized(&layout);
        assert_eq!(set.num(), 2);

        // Empty with slack
        set.empty(10, &layout);
        assert!(set.is_empty());
        assert_eq!(set.num(), 0);
    }

    #[test]
    fn test_script_set_remove() {
        setup_test_globals();

        let mut set = FScriptSet::new();
        let layout = FScriptSetLayout::get_layout(4, 4);

        let index1 = set.add_uninitialized(&layout);
        let index2 = set.add_uninitialized(&layout);
        assert_eq!(set.num(), 2);

        // Remove one element
        set.remove_at(index1, &layout);
        assert_eq!(set.num(), 1);
        assert!(!set.is_valid_index(index1));
        assert!(set.is_valid_index(index2));
    }

    #[test]
    fn test_get_number_of_hash_buckets() {
        assert_eq!(get_number_of_hash_buckets(0), 0);
        assert_eq!(get_number_of_hash_buckets(1), 8); // Minimum
        assert_eq!(get_number_of_hash_buckets(5), 8);
        assert_eq!(get_number_of_hash_buckets(10), 16);
        assert_eq!(get_number_of_hash_buckets(20), 32);
    }
}
