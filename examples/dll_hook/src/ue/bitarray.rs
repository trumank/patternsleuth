use super::*;

#[repr(C)]
#[derive(Debug)]
pub struct TBitArray<Allocator = TInlineAllocator<4>>
where
    Allocator: self::Allocator,
{
    pub allocator_instance: Allocator::ForAnyElementType<u32>,
    pub num_bits: i32,
    pub max_bits: i32,
}

impl<A> Default for TBitArray<A>
where
    A: self::Allocator,
{
    fn default() -> Self {
        Self {
            allocator_instance: A::ForAnyElementType::default(),
            num_bits: 0,
            max_bits: 0,
        }
    }
}

impl<A> TBitArray<A>
where
    A: self::Allocator,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(num_bits: usize) -> Self {
        let mut array = Self::new();
        array.reserve(num_bits);
        array
    }

    pub fn len(&self) -> usize {
        self.num_bits as usize
    }

    pub fn capacity(&self) -> usize {
        self.max_bits as usize
    }

    pub fn is_empty(&self) -> bool {
        self.num_bits == 0
    }

    fn reserve(&mut self, num_bits: usize) {
        let words_needed = (num_bits + 31) / 32; // Round up to nearest u32
        if words_needed * 32 > self.max_bits as usize {
            if self.max_bits == 0 {
                self.allocator_instance.allocate(words_needed);
            } else {
                self.allocator_instance.reallocate(words_needed);
            }
            self.max_bits = (words_needed * 32) as i32;

            // Initialize new bits to 0
            if !self.allocator_instance.data_ptr().is_null() {
                let data = self.allocator_instance.data_ptr_mut();
                unsafe {
                    let old_words = (self.num_bits + 31) / 32;
                    for i in old_words as usize..words_needed {
                        std::ptr::write(data.add(i), 0);
                    }
                }
            }
        }
    }

    pub fn set_bit(&mut self, index: usize, value: bool) {
        if index >= self.len() {
            self.reserve(index + 1);
            self.num_bits = (index + 1) as i32;
        }

        let word_index = index / 32;
        let bit_index = index % 32;
        let data = self.allocator_instance.data_ptr_mut();

        if !data.is_null() {
            unsafe {
                let word = data.add(word_index);
                if value {
                    *word |= 1u32 << bit_index;
                } else {
                    *word &= !(1u32 << bit_index);
                }
            }
        }
    }

    pub fn get_bit(&self, index: usize) -> bool {
        if index >= self.len() {
            return false;
        }

        let word_index = index / 32;
        let bit_index = index % 32;
        let data = self.allocator_instance.data_ptr();

        if !data.is_null() {
            unsafe {
                let word = *data.add(word_index);
                (word & (1u32 << bit_index)) != 0
            }
        } else {
            false
        }
    }

    pub fn push(&mut self, value: bool) {
        let index = self.num_bits as usize;
        self.set_bit(index, value);
    }

    pub fn clear(&mut self) {
        self.num_bits = 0;
        // Clear all bits
        let words = (self.max_bits + 31) / 32;
        let data = self.allocator_instance.data_ptr_mut();
        if !data.is_null() {
            unsafe {
                for i in 0..words as usize {
                    std::ptr::write(data.add(i), 0);
                }
            }
        }
    }

    pub fn resize(&mut self, new_size: usize, value: bool) {
        let old_size = self.len();
        if new_size > old_size {
            self.reserve(new_size);
            for i in old_size..new_size {
                self.set_bit(i, value);
            }
        }
        self.num_bits = new_size as i32;
    }
}

impl<A> Drop for TBitArray<A>
where
    A: self::Allocator,
{
    fn drop(&mut self) {
        self.allocator_instance.deallocate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use malloc::test::setup_test_globals;

    #[test]
    fn test_bitarray_basic_operations() {
        setup_test_globals();

        let mut bitarray: TBitArray = TBitArray::new();

        assert_eq!(bitarray.len(), 0);
        assert!(bitarray.is_empty());

        // Set some bits
        bitarray.set_bit(0, true);
        bitarray.set_bit(1, false);
        bitarray.set_bit(2, true);
        bitarray.set_bit(5, true);

        assert_eq!(bitarray.len(), 6);
        assert!(!bitarray.is_empty());

        // Check bits
        assert!(bitarray.get_bit(0));
        assert!(!bitarray.get_bit(1));
        assert!(bitarray.get_bit(2));
        assert!(!bitarray.get_bit(3));
        assert!(!bitarray.get_bit(4));
        assert!(bitarray.get_bit(5));
    }

    #[test]
    fn test_bitarray_push() {
        setup_test_globals();

        let mut bitarray: TBitArray = TBitArray::new();

        bitarray.push(true);
        bitarray.push(false);
        bitarray.push(true);

        assert_eq!(bitarray.len(), 3);
        assert!(bitarray.get_bit(0));
        assert!(!bitarray.get_bit(1));
        assert!(bitarray.get_bit(2));
    }

    #[test]
    fn test_bitarray_clear() {
        setup_test_globals();

        let mut bitarray: TBitArray = TBitArray::new();

        bitarray.set_bit(10, true);
        bitarray.set_bit(20, true);
        assert_eq!(bitarray.len(), 21);

        bitarray.clear();
        assert_eq!(bitarray.len(), 0);
        assert!(bitarray.is_empty());
    }

    #[test]
    fn test_bitarray_resize() {
        setup_test_globals();

        let mut bitarray: TBitArray = TBitArray::new();

        bitarray.resize(10, true);
        assert_eq!(bitarray.len(), 10);

        for i in 0..10 {
            assert!(bitarray.get_bit(i));
        }

        bitarray.resize(5, false);
        assert_eq!(bitarray.len(), 5);

        for i in 0..5 {
            assert!(bitarray.get_bit(i));
        }
    }
}
