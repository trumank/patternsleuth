use super::*;
use std::mem::MaybeUninit;
use std::{ffi::c_void, marker::PhantomData};

pub trait AllocatorInstance<T> {
    fn data_ptr(&self) -> *const T;
    fn data_ptr_mut(&mut self) -> *mut T;
    fn allocate(&mut self, count: usize) -> *mut T;
    fn reallocate(&mut self, count: usize) -> *mut T;
    fn deallocate(&mut self);
}

pub trait Allocator {
    type ForAnyElementType<T>: Default + AllocatorInstance<T>;
}

pub struct TSizedHeapAllocator32;

impl Allocator for TSizedHeapAllocator32 {
    type ForAnyElementType<T> = TSizedHeapAllocatorForAnyElementType<T>;
}

pub struct TInlineAllocator<const INLINE_ELEMENTS: usize>;

impl<const INLINE_ELEMENTS: usize> Allocator for TInlineAllocator<INLINE_ELEMENTS> {
    type ForAnyElementType<T> = TInlineAllocatorForElementType<T, INLINE_ELEMENTS>;
}

#[repr(C)]
pub struct TSizedHeapAllocatorForAnyElementType<T> {
    pub data: *mut T,
}

impl<T> Default for TSizedHeapAllocatorForAnyElementType<T> {
    fn default() -> Self {
        Self {
            data: std::ptr::null_mut(),
        }
    }
}

impl<T> AllocatorInstance<T> for TSizedHeapAllocatorForAnyElementType<T> {
    fn data_ptr(&self) -> *const T {
        self.data
    }

    fn data_ptr_mut(&mut self) -> *mut T {
        self.data
    }

    fn allocate(&mut self, count: usize) -> *mut T {
        if count == 0 {
            std::ptr::null_mut()
        } else {
            let size = count * std::mem::size_of::<T>();
            let alignment = std::mem::align_of::<T>() as u32;
            let ptr = unsafe { gmalloc().malloc(size, alignment) } as *mut T;
            self.data = ptr;
            ptr
        }
    }

    fn reallocate(&mut self, count: usize) -> *mut T {
        let size = count * std::mem::size_of::<T>();
        let alignment = std::mem::align_of::<T>() as u32;
        let ptr = unsafe { gmalloc().realloc(self.data as *mut c_void, size, alignment) } as *mut T;
        self.data = ptr;
        ptr
    }

    fn deallocate(&mut self) {
        if !self.data.is_null() {
            unsafe { gmalloc().free(self.data as *mut c_void) };
            self.data = std::ptr::null_mut();
        }
    }
}

impl<T> TSizedHeapAllocatorForAnyElementType<T> {
    pub fn new() -> Self {
        Self::default()
    }
}

#[repr(C)]
pub struct TInlineAllocatorForElementType<T, const INLINE_ELEMENTS: usize> {
    pub inline_data: [MaybeUninit<T>; INLINE_ELEMENTS],
    pub secondary_data: TSizedHeapAllocatorForAnyElementType<T>,
}

impl<T, const INLINE_ELEMENTS: usize> Default
    for TInlineAllocatorForElementType<T, INLINE_ELEMENTS>
{
    fn default() -> Self {
        Self {
            inline_data: std::array::from_fn(|_| MaybeUninit::uninit()),
            secondary_data: TSizedHeapAllocatorForAnyElementType::default(),
        }
    }
}

impl<T, const INLINE_ELEMENTS: usize> AllocatorInstance<T>
    for TInlineAllocatorForElementType<T, INLINE_ELEMENTS>
{
    fn data_ptr(&self) -> *const T {
        let mut ptr = self.secondary_data.data_ptr();
        if ptr.is_null() {
            ptr = self.inline_data.as_ptr() as *const T
        }
        ptr
    }

    fn data_ptr_mut(&mut self) -> *mut T {
        let mut ptr = self.secondary_data.data_ptr_mut();
        if ptr.is_null() {
            ptr = self.inline_data.as_mut_ptr() as *mut T
        }
        ptr
    }

    fn allocate(&mut self, count: usize) -> *mut T {
        if count == 0 {
            std::ptr::null_mut()
        } else if count <= INLINE_ELEMENTS {
            self.inline_data.as_mut_ptr() as *mut T
        } else {
            self.secondary_data.allocate(count)
        }
    }

    fn reallocate(&mut self, count: usize) -> *mut T {
        if count == 0 {
            self.secondary_data.deallocate();
            std::ptr::null_mut()
        } else if count <= INLINE_ELEMENTS {
            if !self.secondary_data.data_ptr().is_null() {
                // Copy data from secondary back to inline
                let inline_ptr = self.inline_data.as_mut_ptr() as *mut T;
                let secondary_ptr = self.secondary_data.data_ptr();
                if !secondary_ptr.is_null() {
                    let copy_count = std::cmp::min(count, INLINE_ELEMENTS);
                    unsafe {
                        std::ptr::copy_nonoverlapping(secondary_ptr, inline_ptr, copy_count);
                    }
                    self.secondary_data.deallocate();
                }
                inline_ptr
            } else {
                self.inline_data.as_mut_ptr() as *mut T
            }
        } else {
            if self.secondary_data.data_ptr().is_null() {
                // Moving from inline to secondary
                let new_ptr = self.secondary_data.allocate(count);
                if !new_ptr.is_null() {
                    let inline_ptr = self.inline_data.as_ptr() as *const T;
                    let copy_count = std::cmp::min(INLINE_ELEMENTS, count);
                    unsafe {
                        std::ptr::copy_nonoverlapping(inline_ptr, new_ptr, copy_count);
                    }
                }
                new_ptr
            } else {
                self.secondary_data.reallocate(count)
            }
        }
    }

    fn deallocate(&mut self) {
        self.secondary_data.deallocate();
    }
}

impl<T, const INLINE_ELEMENTS: usize> TInlineAllocatorForElementType<T, INLINE_ELEMENTS> {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_using_inline(&self, count: usize) -> bool {
        count <= INLINE_ELEMENTS && self.secondary_data.data_ptr().is_null()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use malloc::test::setup_test_globals;

    #[test]
    fn test_default_allocator_basic() {
        setup_test_globals();

        let mut allocator = TSizedHeapAllocatorForAnyElementType::<i32>::new();
        assert!(allocator.data_ptr().is_null());

        let ptr = allocator.allocate(10);
        assert!(!ptr.is_null());
        assert!(!allocator.data_ptr().is_null());

        unsafe {
            std::ptr::write(ptr, 42);
            assert_eq!(std::ptr::read(ptr), 42);
        }

        allocator.deallocate();
        assert!(allocator.data_ptr().is_null());
    }

    #[test]
    fn test_default_allocator_reallocation() {
        setup_test_globals();

        let mut allocator = TSizedHeapAllocatorForAnyElementType::<u8>::new();

        let ptr1 = allocator.allocate(5);
        assert!(!ptr1.is_null());

        unsafe {
            for i in 0..5 {
                std::ptr::write(ptr1.add(i), i as u8);
            }
        }

        let ptr2 = allocator.reallocate(10);
        assert!(!ptr2.is_null());

        unsafe {
            for i in 0..5 {
                assert_eq!(std::ptr::read(ptr2.add(i)), i as u8);
            }
        }

        allocator.deallocate();
    }

    #[test]
    fn test_inline_allocator_inline_storage() {
        setup_test_globals();

        let mut allocator = TInlineAllocatorForElementType::<i32, 4>::new();

        let ptr = allocator.allocate(3);
        assert!(!ptr.is_null());
        assert!(allocator.is_using_inline(3));

        unsafe {
            std::ptr::write(ptr, 100);
            std::ptr::write(ptr.add(1), 200);
            std::ptr::write(ptr.add(2), 300);

            assert_eq!(std::ptr::read(ptr), 100);
            assert_eq!(std::ptr::read(ptr.add(1)), 200);
            assert_eq!(std::ptr::read(ptr.add(2)), 300);
        }

        allocator.deallocate();
    }

    #[test]
    fn test_inline_allocator_heap_fallback() {
        setup_test_globals();

        let mut allocator = TInlineAllocatorForElementType::<i32, 2>::new();

        let ptr = allocator.allocate(5);
        assert!(!ptr.is_null());
        assert!(!allocator.is_using_inline(5));

        unsafe {
            for i in 0..5 {
                std::ptr::write(ptr.add(i), (i * 10) as i32);
            }

            for i in 0..5 {
                assert_eq!(std::ptr::read(ptr.add(i)), (i * 10) as i32);
            }
        }

        allocator.deallocate();
    }

    #[test]
    fn test_inline_allocator_transition() {
        setup_test_globals();

        let mut allocator = TInlineAllocatorForElementType::<i32, 3>::new();

        // Start with inline allocation
        let ptr1 = allocator.allocate(2);
        assert!(allocator.is_using_inline(2));

        unsafe {
            std::ptr::write(ptr1, 10);
            std::ptr::write(ptr1.add(1), 20);
        }

        // Expand beyond inline capacity - should move to heap
        let ptr2 = allocator.reallocate(5);
        assert!(!allocator.is_using_inline(5));

        unsafe {
            assert_eq!(std::ptr::read(ptr2), 10);
            assert_eq!(std::ptr::read(ptr2.add(1)), 20);

            std::ptr::write(ptr2.add(2), 30);
            std::ptr::write(ptr2.add(3), 40);
            std::ptr::write(ptr2.add(4), 50);
        }

        // Shrink back to inline capacity - should move back to inline
        let ptr3 = allocator.reallocate(2);
        assert!(allocator.is_using_inline(2));

        unsafe {
            assert_eq!(std::ptr::read(ptr3), 10);
            assert_eq!(std::ptr::read(ptr3.add(1)), 20);
        }

        allocator.deallocate();
    }
}
