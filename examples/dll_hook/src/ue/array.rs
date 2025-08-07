use super::*;
use std::ffi::c_void;

#[derive(Debug)]
#[repr(C)]
pub struct TArray<T> {
    data: *const T,
    num: i32,
    max: i32,
}
impl<T> Drop for TArray<T> {
    fn drop(&mut self) {
        unsafe {
            std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(
                self.data.cast_mut(),
                self.num as usize,
            ));
            gmalloc().free(self.data as *mut c_void);
        }
    }
}
impl<T> Default for TArray<T> {
    fn default() -> Self {
        Self {
            data: std::ptr::null(),
            num: 0,
            max: 0,
        }
    }
}
impl<T> TArray<T> {
    pub fn new() -> Self {
        Self {
            data: std::ptr::null(),
            num: 0,
            max: 0,
        }
    }
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: unsafe {
                gmalloc().malloc(
                    capacity * std::mem::size_of::<T>(),
                    std::mem::align_of::<T>() as u32,
                ) as *const T
            },
            num: 0,
            max: capacity as i32,
        }
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
            unsafe { std::slice::from_raw_parts(self.data, self.num as usize) }
        }
    }
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        if self.num == 0 {
            &mut []
        } else {
            unsafe { std::slice::from_raw_parts_mut(self.data as *mut _, self.num as usize) }
        }
    }
    pub fn clear(&mut self) {
        let elems: *mut [T] = self.as_mut_slice();

        unsafe {
            self.num = 0;
            std::ptr::drop_in_place(elems);
        }
    }
    pub fn push(&mut self, new_value: T) {
        if self.num >= self.max {
            self.max = u32::next_power_of_two((self.max + 1) as u32) as i32;
            let new = unsafe {
                gmalloc().realloc(
                    self.data as *mut c_void,
                    self.max as usize * std::mem::size_of::<T>(),
                    std::mem::align_of::<T>() as u32,
                ) as *const T
            };
            self.data = new;
        }
        unsafe {
            std::ptr::write(self.data.add(self.num as usize).cast_mut(), new_value);
        }
        self.num += 1;
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
        self.data
    }
}

impl<T> From<&[T]> for TArray<T>
where
    T: Copy,
{
    fn from(value: &[T]) -> Self {
        let mut new = Self::with_capacity(value.len());
        // TODO this is probably unsound
        new.num = value.len() as i32;
        new.as_mut_slice().copy_from_slice(value);
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
}
