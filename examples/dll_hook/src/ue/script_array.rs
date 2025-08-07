use super::*;
use std::ffi::c_void;
use std::{ptr, slice};

#[repr(C)]
pub struct FScriptArray {
    allocator_instance: <TSizedHeapAllocator32 as Allocator>::ForAnyElementType<u8>,
    array_num: i32,
    array_max: i32,
}

impl FScriptArray {
    pub fn new() -> Self {
        Self {
            allocator_instance: <TSizedHeapAllocator32 as Allocator>::ForAnyElementType::default(),
            array_num: 0,
            array_max: 0,
        }
    }

    pub fn get_data(&self) -> *const c_void {
        self.allocator_instance.data_ptr() as *const c_void
    }

    pub fn get_data_mut(&mut self) -> *mut c_void {
        self.allocator_instance.data_ptr_mut() as *mut c_void
    }

    pub fn is_valid_index(&self, index: i32) -> bool {
        index >= 0 && index < self.array_num
    }

    pub fn is_empty(&self) -> bool {
        self.array_num == 0
    }

    pub fn num(&self) -> i32 {
        debug_assert!(self.array_num >= 0);
        debug_assert!(self.array_max >= self.array_num);
        self.array_num
    }

    pub fn num_unchecked(&self) -> i32 {
        self.array_num
    }

    pub fn get_slack(&self) -> i32 {
        self.array_max - self.array_num
    }

    pub fn get_allocated_size(&self, num_bytes_per_element: i32) -> usize {
        (self.array_max as usize) * (num_bytes_per_element as usize)
    }

    pub fn empty(&mut self, slack: i32, num_bytes_per_element: i32) {
        debug_assert!(slack >= 0);
        self.array_num = 0;

        if slack != self.array_max {
            self.resize_to(slack, num_bytes_per_element);
        }
    }

    pub fn reset(&mut self, new_size: i32, num_bytes_per_element: i32) {
        if new_size <= self.array_max {
            self.array_num = 0;
        } else {
            self.empty(new_size, num_bytes_per_element);
        }
    }

    pub fn shrink(&mut self, num_bytes_per_element: i32) {
        debug_assert!(self.array_num >= 0);
        debug_assert!(self.array_max >= self.array_num);
        if self.array_num != self.array_max {
            self.resize_to(self.array_num, num_bytes_per_element);
        }
    }

    pub fn insert_zeroed(&mut self, index: i32, count: i32, num_bytes_per_element: i32) {
        self.insert(index, count, num_bytes_per_element);
        if count > 0 {
            let ptr = self.get_data_mut() as *mut u8;
            let offset = (index as usize) * (num_bytes_per_element as usize);
            let size = (count as usize) * (num_bytes_per_element as usize);
            unsafe {
                ptr::write_bytes(ptr.add(offset), 0, size);
            }
        }
    }

    pub fn insert(&mut self, index: i32, count: i32, num_bytes_per_element: i32) {
        debug_assert!(count >= 0);
        debug_assert!(self.array_num >= 0);
        debug_assert!(self.array_max >= self.array_num);
        debug_assert!(index >= 0);
        debug_assert!(index <= self.array_num);

        let old_num = self.array_num;
        self.array_num += count;

        if self.array_num > self.array_max {
            self.resize_grow(old_num, num_bytes_per_element);
        }

        if count > 0 && index < old_num {
            let ptr = self.get_data_mut() as *mut u8;
            let element_size = num_bytes_per_element as usize;
            unsafe {
                ptr::copy(
                    ptr.add(index as usize * element_size),
                    ptr.add((index + count) as usize * element_size),
                    (old_num - index) as usize * element_size,
                );
            }
        }
    }

    pub fn add(&mut self, count: i32, num_bytes_per_element: i32) -> i32 {
        debug_assert!(count >= 0);
        debug_assert!(self.array_num >= 0);
        debug_assert!(self.array_max >= self.array_num);

        let old_num = self.array_num;
        self.array_num += count;

        if self.array_num > self.array_max {
            self.resize_grow(old_num, num_bytes_per_element);
        }

        old_num
    }

    pub fn add_zeroed(&mut self, count: i32, num_bytes_per_element: i32) -> i32 {
        let index = self.add(count, num_bytes_per_element);
        if count > 0 {
            let ptr = self.get_data_mut() as *mut u8;
            let offset = (index as usize) * (num_bytes_per_element as usize);
            let size = (count as usize) * (num_bytes_per_element as usize);
            unsafe {
                ptr::write_bytes(ptr.add(offset), 0, size);
            }
        }
        index
    }

    pub fn remove(&mut self, index: i32, count: i32, num_bytes_per_element: i32) {
        if count > 0 {
            debug_assert!(count >= 0);
            debug_assert!(index >= 0);
            debug_assert!(index <= self.array_num);
            debug_assert!(index + count <= self.array_num);

            let num_to_move = self.array_num - index - count;
            if num_to_move > 0 {
                let ptr = self.get_data_mut() as *mut u8;
                let element_size = num_bytes_per_element as usize;
                unsafe {
                    ptr::copy(
                        ptr.add((index + count) as usize * element_size),
                        ptr.add(index as usize * element_size),
                        num_to_move as usize * element_size,
                    );
                }
            }
            self.array_num -= count;
            debug_assert!(self.array_num >= 0);
            debug_assert!(self.array_max >= self.array_num);
        }
    }

    pub fn set_num_uninitialized(&mut self, new_num: i32, num_bytes_per_element: i32) {
        debug_assert!(new_num >= 0);
        let old_num = self.num();
        if new_num > old_num {
            self.add(new_num - old_num, num_bytes_per_element);
        } else if new_num < old_num {
            self.remove(new_num, old_num - new_num, num_bytes_per_element);
        }
    }

    pub fn swap_memory(&mut self, a: i32, b: i32, num_bytes_per_element: i32) {
        if a != b {
            let ptr = self.get_data_mut() as *mut u8;
            let element_size = num_bytes_per_element as usize;
            unsafe {
                let ptr_a = ptr.add(a as usize * element_size);
                let ptr_b = ptr.add(b as usize * element_size);
                ptr::swap_nonoverlapping(ptr_a, ptr_b, element_size);
            }
        }
    }

    pub fn as_slice<T>(&self) -> &[T] {
        if self.array_num == 0 {
            &[]
        } else {
            let ptr = self.get_data() as *const T;
            if ptr.is_null() {
                &[]
            } else {
                unsafe { slice::from_raw_parts(ptr, self.array_num as usize) }
            }
        }
    }

    pub fn as_mut_slice<T>(&mut self) -> &mut [T] {
        if self.array_num == 0 {
            &mut []
        } else {
            let ptr = self.get_data_mut() as *mut T;
            if ptr.is_null() {
                &mut []
            } else {
                unsafe { slice::from_raw_parts_mut(ptr, self.array_num as usize) }
            }
        }
    }

    pub fn check_address(&self, addr: *const c_void, num_bytes_per_element: i32) {
        let data_start = self.get_data() as *const u8;
        let data_end =
            unsafe { data_start.add((self.array_max as usize) * (num_bytes_per_element as usize)) };
        let addr = addr as *const u8;
        debug_assert!(
            addr < data_start || addr >= data_end,
            "Attempting to use a container element which already comes from the container being modified"
        );
    }

    fn resize_grow(&mut self, old_num: i32, num_bytes_per_element: i32) {
        let new_capacity = self.calculate_slack_grow(num_bytes_per_element);
        self.array_max = new_capacity;
        let byte_capacity = (new_capacity as usize) * (num_bytes_per_element as usize);
        self.allocator_instance.reallocate(byte_capacity);
    }

    fn resize_to(&mut self, new_max: i32, num_bytes_per_element: i32) {
        let new_max = if new_max != 0 {
            self.calculate_slack_reserve(new_max, num_bytes_per_element)
        } else {
            0
        };

        if new_max != self.array_max {
            self.array_max = new_max;
            if new_max == 0 {
                self.allocator_instance.deallocate();
            } else {
                let byte_capacity = (new_max as usize) * (num_bytes_per_element as usize);
                self.allocator_instance.reallocate(byte_capacity);
            }
        }
    }

    fn calculate_slack_grow(&self, _num_bytes_per_element: i32) -> i32 {
        let current_capacity = self.array_max;
        if current_capacity == 0 {
            let required = self.array_num.max(4);
            let next_power_of_two = (required as u32).next_power_of_two();
            next_power_of_two as i32
        } else {
            (current_capacity * 2).max(self.array_num)
        }
    }

    fn calculate_slack_reserve(&self, num_elements: i32, _num_bytes_per_element: i32) -> i32 {
        if num_elements <= 0 {
            0
        } else {
            let next_power_of_two = (num_elements as u32).next_power_of_two();
            next_power_of_two as i32
        }
    }
}

impl Default for FScriptArray {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FScriptArray {
    fn drop(&mut self) {
        self.array_num = 0;
        self.allocator_instance.deallocate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use malloc::test::setup_test_globals;

    #[test]
    fn test_fscriptarray_basic_operations() {
        setup_test_globals();

        let mut script_array = FScriptArray::new();
        assert!(script_array.is_empty());
        assert_eq!(script_array.num(), 0);
        assert_eq!(script_array.get_slack(), 0);
        assert!(script_array.get_data().is_null());

        let element_size = std::mem::size_of::<i32>() as i32;
        script_array.add_zeroed(5, element_size);
        assert!(!script_array.is_empty());
        assert_eq!(script_array.num(), 5);
        assert!(script_array.get_slack() > 0);

        let slice: &[i32] = script_array.as_slice();
        assert_eq!(slice.len(), 5);
        assert_eq!(slice, &[0, 0, 0, 0, 0]);
    }

    #[test]
    fn test_fscriptarray_insert_and_remove() {
        setup_test_globals();

        let mut script_array = FScriptArray::new();
        let element_size = std::mem::size_of::<u32>() as i32;

        script_array.add(3, element_size);
        {
            let slice: &mut [u32] = script_array.as_mut_slice();
            slice[0] = 10;
            slice[1] = 20;
            slice[2] = 30;
        }

        script_array.insert_zeroed(1, 2, element_size);
        assert_eq!(script_array.num(), 5);

        let slice: &[u32] = script_array.as_slice();
        assert_eq!(slice, &[10, 0, 0, 20, 30]);

        script_array.remove(1, 2, element_size);
        assert_eq!(script_array.num(), 3);

        let slice: &[u32] = script_array.as_slice();
        assert_eq!(slice, &[10, 20, 30]);
    }

    #[test]
    fn test_fscriptarray_set_num_and_swap() {
        setup_test_globals();

        let mut script_array = FScriptArray::new();
        let element_size = std::mem::size_of::<i16>() as i32;

        script_array.set_num_uninitialized(4, element_size);
        assert_eq!(script_array.num(), 4);

        {
            let slice: &mut [i16] = script_array.as_mut_slice();
            slice[0] = 100;
            slice[1] = 200;
            slice[2] = 300;
            slice[3] = 400;
        }

        script_array.swap_memory(0, 3, element_size);
        let slice: &[i16] = script_array.as_slice();
        assert_eq!(slice, &[400, 200, 300, 100]);

        script_array.set_num_uninitialized(2, element_size);
        assert_eq!(script_array.num(), 2);
        let slice: &[i16] = script_array.as_slice();
        assert_eq!(slice, &[400, 200]);
    }

    #[test]
    fn test_fscriptarray_empty_and_reset() {
        setup_test_globals();

        let mut script_array = FScriptArray::new();
        let element_size = std::mem::size_of::<u8>() as i32;

        script_array.add(10, element_size);
        assert_eq!(script_array.num(), 10);

        script_array.empty(0, element_size);
        assert!(script_array.is_empty());
        assert_eq!(script_array.num(), 0);

        script_array.add(5, element_size);
        assert_eq!(script_array.num(), 5);

        script_array.reset(0, element_size);
        assert!(script_array.is_empty());
        assert_eq!(script_array.num(), 0);
    }

    #[test]
    fn test_fscriptarray_shrink() {
        setup_test_globals();

        let mut script_array = FScriptArray::new();
        let element_size = std::mem::size_of::<u64>() as i32;

        script_array.add(20, element_size);
        let capacity_before = script_array.get_slack() + script_array.num();

        script_array.remove(10, 10, element_size);
        assert_eq!(script_array.num(), 10);

        script_array.shrink(element_size);
        let capacity_after = script_array.get_slack() + script_array.num();

        assert!(capacity_after <= capacity_before);
    }
}
