use super::*;
use std::{ffi::c_void, marker::PhantomData};

#[repr(C)]
pub struct TArray<T, Allocator = TSizedHeapAllocator32>
where
    Allocator: self::Allocator,
{
    pub allocator_instance: Allocator::ForAnyElementType<T>,
    pub num: i32,
    pub max: i32,
}
impl<T, A> std::fmt::Debug for TArray<T, A>
where
    A: self::Allocator,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO elements
        f.debug_struct("TArray")
            .field("num", &self.num)
            .field("max", &self.max)
            .finish()
    }
}
impl<T, A> Drop for TArray<T, A>
where
    A: self::Allocator,
{
    fn drop(&mut self) {
        self.clear();
        self.allocator_instance.deallocate();
    }
}
impl<T, A> Default for TArray<T, A>
where
    A: self::Allocator,
{
    fn default() -> Self {
        Self {
            allocator_instance: A::ForAnyElementType::default(),
            num: 0,
            max: 0,
        }
    }
}
impl<T, A> TArray<T, A>
where
    A: self::Allocator,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let mut array = Self::new();
        if capacity > 0 {
            array.allocator_instance.allocate(capacity);
            array.max = capacity as i32;
        }
        array
    }
    pub fn len(&self) -> usize {
        self.num as usize
    }
    pub fn capacity(&self) -> usize {
        self.max as usize
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn as_slice(&self) -> &[T] {
        if self.num == 0 {
            &[]
        } else {
            let ptr = self.allocator_instance.data_ptr();
            if ptr.is_null() {
                &[]
            } else {
                unsafe { std::slice::from_raw_parts(ptr, self.num as usize) }
            }
        }
    }
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        if self.num == 0 {
            &mut []
        } else {
            let ptr = self.allocator_instance.data_ptr_mut();
            if ptr.is_null() {
                &mut []
            } else {
                unsafe { std::slice::from_raw_parts_mut(ptr, self.num as usize) }
            }
        }
    }
    pub fn clear(&mut self) {
        if self.num > 0 {
            let ptr = self.allocator_instance.data_ptr_mut();
            if !ptr.is_null() {
                unsafe {
                    std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(
                        ptr,
                        self.num as usize,
                    ));
                }
            }
            self.num = 0;
        }
    }
    pub fn push(&mut self, new_value: T) {
        if self.num >= self.max {
            let new_capacity = u32::next_power_of_two((self.max + 1) as u32) as usize;
            self.allocator_instance.reallocate(new_capacity);
            self.max = new_capacity as i32;
        }
        let ptr = self.allocator_instance.data_ptr_mut();
        if !ptr.is_null() {
            unsafe {
                std::ptr::write(ptr.add(self.num as usize), new_value);
            }
            self.num += 1;
        }
    }
    pub fn extend(&mut self, other: &[T])
    where
        T: Copy,
    {
        for o in other {
            self.push(*o);
        }
    }
    pub fn as_ptr(&self) -> *const T {
        self.allocator_instance.data_ptr()
    }

    pub fn reserve_capacity(&mut self, capacity: usize) {
        if capacity > self.capacity() {
            if self.max == 0 {
                self.allocator_instance.allocate(capacity);
            } else {
                self.allocator_instance.reallocate(capacity);
            }
            self.max = capacity as i32;
        }
    }
}

impl<T, A> From<&[T]> for TArray<T, A>
where
    T: Copy,
    A: self::Allocator,
{
    fn from(value: &[T]) -> Self {
        let mut new = Self::with_capacity(value.len());
        new.extend(value);
        new
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use malloc::test::setup_test_globals;

    #[test]
    fn test_tarray_basic_operations() {
        setup_test_globals();

        let mut array: TArray<i32> = TArray::new();

        assert_eq!(array.len(), 0);
        assert!(array.is_empty());
        assert_eq!(array.as_slice(), &[]);

        array.push(42);
        assert_eq!(array.len(), 1);
        assert!(!array.is_empty());
        assert_eq!(array.as_slice(), &[42]);

        array.push(100);
        array.push(200);
        assert_eq!(array.len(), 3);
        assert_eq!(array.as_slice(), &[42, 100, 200]);
    }

    #[test]
    fn test_tarray_with_capacity() {
        setup_test_globals();

        let mut array: TArray<u32> = TArray::with_capacity(10);
        assert_eq!(array.len(), 0);
        assert_eq!(array.capacity(), 10);

        for i in 0..5 {
            array.push(i * 2);
        }

        assert_eq!(array.len(), 5);
        assert_eq!(array.capacity(), 10);
        assert_eq!(array.as_slice(), &[0, 2, 4, 6, 8]);
    }

    #[test]
    fn test_tarray_growth() {
        setup_test_globals();

        let mut array: TArray<u8> = TArray::new();

        for i in 0..20u8 {
            array.push(i);
        }

        assert_eq!(array.len(), 20);
        let expected: Vec<u8> = (0..20).collect();
        assert_eq!(array.as_slice(), expected.as_slice());
    }

    #[test]
    fn test_tarray_clear() {
        setup_test_globals();

        let mut array: TArray<i32> = TArray::new();
        for i in 0..10 {
            array.push(i);
        }

        assert_eq!(array.len(), 10);

        array.clear();
        assert_eq!(array.len(), 0);
        assert!(array.is_empty());
        assert_eq!(array.as_slice(), &[]);
    }

    #[test]
    fn test_tarray_extend() {
        setup_test_globals();

        let mut array: TArray<i32> = TArray::new();
        array.push(1);
        array.push(2);

        let more_data = &[3, 4, 5, 6];
        array.extend(more_data);

        assert_eq!(array.len(), 6);
        assert_eq!(array.as_slice(), &[1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn test_tarray_with_inline_allocator_small() {
        setup_test_globals();

        let mut array: TArray<i32, TInlineAllocator<4>> = TArray::new();

        // Test operations within inline capacity
        array.push(10);
        array.push(20);
        array.push(30);

        assert_eq!(array.len(), 3);
        assert_eq!(array.as_slice(), &[10, 20, 30]);

        // Still within inline capacity
        array.push(40);
        assert_eq!(array.len(), 4);
        assert_eq!(array.as_slice(), &[10, 20, 30, 40]);
    }

    #[test]
    fn test_tarray_with_inline_allocator_overflow() {
        setup_test_globals();

        let mut array: TArray<u8, TInlineAllocator<2>> = TArray::new();

        // Fill inline capacity
        array.push(1);
        array.push(2);
        assert_eq!(array.len(), 2);
        assert_eq!(array.as_slice(), &[1, 2]);

        // This should cause transition to heap allocation
        array.push(3);
        array.push(4);
        array.push(5);

        assert_eq!(array.len(), 5);
        assert_eq!(array.as_slice(), &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_tarray_inline_allocator_clear() {
        setup_test_globals();

        let mut array: TArray<i32, TInlineAllocator<3>> = TArray::new();

        // Add some elements
        for i in 0..5 {
            array.push(i * 10);
        }

        assert_eq!(array.len(), 5);

        array.clear();
        assert_eq!(array.len(), 0);
        assert!(array.is_empty());
        assert_eq!(array.as_slice(), &[]);

        // Should still work after clearing
        array.push(100);
        assert_eq!(array.len(), 1);
        assert_eq!(array.as_slice(), &[100]);
    }

    // #[test]
    // fn test_tarray_inline_allocator_from_slice() {
    //     setup_test_globals();

    //     let source = &[10, 20, 30, 40, 50];
    //     let array: TArray<i32, TInlineAllocator<3>> = TArray::from(source);

    //     assert_eq!(array.len(), 5);
    //     assert_eq!(array.as_slice(), source);
    // }
}
