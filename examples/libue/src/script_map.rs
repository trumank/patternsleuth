use super::*;
use std::ffi::c_void;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FScriptMapLayout {
    // Key is always at offset 0 in TPair - not stored here
    pub value_offset: i32,
    pub set_layout: FScriptSetLayout,
}

impl FScriptMapLayout {
    pub fn for_types<K, V>() -> Self {
        Self::get_layout(
            std::mem::size_of::<K>() as i32,
            std::mem::align_of::<K>() as i32,
            std::mem::size_of::<V>() as i32,
            std::mem::align_of::<V>() as i32,
        )
    }

    pub fn get_layout(
        key_size: i32,
        key_alignment: i32,
        value_size: i32,
        value_alignment: i32,
    ) -> Self {
        // Calculate TPair<Key, Value> layout
        let mut offset = 0i32;

        // Key comes first at offset 0
        offset += key_size;

        // Value follows, aligned to value_alignment
        offset = align_up(offset, value_alignment);
        let value_offset = offset;
        offset += value_size;

        // Pair alignment is max of key and value alignment
        let pair_alignment = key_alignment.max(value_alignment);
        let pair_size = align_up(offset, pair_alignment);

        Self {
            value_offset,
            set_layout: FScriptSetLayout::get_layout(pair_size, pair_alignment),
        }
    }
}

fn align_up(value: i32, alignment: i32) -> i32 {
    (value + alignment - 1) & !(alignment - 1)
}

/// Untyped map type for accessing TMap data, like FScriptArray for TArray.
/// Must have the same memory representation as a TMap.
#[repr(C)]
pub struct TScriptMap<Allocator = FDefaultSetAllocator, DerivedType = ()>
where
    Allocator: self::SetAllocator,
{
    pairs: TScriptSet<Allocator, DerivedType>,
}

impl<A, D> Default for TScriptMap<A, D>
where
    A: self::SetAllocator,
{
    fn default() -> Self {
        Self {
            pairs: TScriptSet::new(),
        }
    }
}

impl<A, D> TScriptMap<A, D>
where
    A: self::SetAllocator,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_valid_index(&self, index: i32) -> bool {
        self.pairs.is_valid_index(index)
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    pub fn num(&self) -> i32 {
        self.pairs.num()
    }

    pub fn get_max_index(&self) -> i32 {
        self.pairs.get_max_index()
    }

    pub fn get_data(&self, index: i32, layout: &FScriptMapLayout) -> *const c_void {
        self.pairs.get_data(index, &layout.set_layout)
    }

    pub fn get_data_mut(&mut self, index: i32, layout: &FScriptMapLayout) -> *mut c_void {
        self.pairs.get_data_mut(index, &layout.set_layout)
    }

    pub fn move_assign(&mut self, other: Self, layout: &FScriptMapLayout) {
        self.pairs.move_assign(other.pairs, &layout.set_layout);
    }

    pub fn empty(&mut self, slack: i32, layout: &FScriptMapLayout) {
        self.pairs.empty(slack, &layout.set_layout);
    }

    pub fn remove_at(&mut self, index: i32, layout: &FScriptMapLayout) {
        self.pairs.remove_at(index, &layout.set_layout);
    }

    pub fn add_uninitialized(&mut self, layout: &FScriptMapLayout) -> i32 {
        self.pairs.add_uninitialized(&layout.set_layout)
    }

    pub fn rehash(
        &mut self,
        layout: &FScriptMapLayout,
        get_key_hash: impl Fn(*const c_void) -> u32,
    ) {
        self.pairs.rehash(&layout.set_layout, get_key_hash);
    }

    /// Find a pair's index by key hash and equality.
    /// The equality function receives (key, pair_ptr) where the key within the
    /// pair is at offset 0, so pair_ptr can be compared directly against key.
    pub fn find_pair_index(
        &self,
        key: *const c_void,
        layout: &FScriptMapLayout,
        get_key_hash: impl Fn(*const c_void) -> u32,
        key_equality_fn: impl Fn(*const c_void, *const c_void) -> bool,
    ) -> i32 {
        if self.pairs.num() == 0 {
            return -1;
        }

        self.pairs
            .find_index(key, &layout.set_layout, get_key_hash, key_equality_fn)
    }

    /// Find the value pointer for a given key, or null if not found.
    pub fn find_value(
        &self,
        key: *const c_void,
        layout: &FScriptMapLayout,
        get_key_hash: impl Fn(*const c_void) -> u32,
        key_equality_fn: impl Fn(*const c_void, *const c_void) -> bool,
    ) -> *const c_void {
        let index = self.find_pair_index(key, layout, get_key_hash, key_equality_fn);
        if index == -1 {
            return std::ptr::null();
        }

        let pair_ptr = self.get_data(index, layout) as *const u8;
        if pair_ptr.is_null() {
            return std::ptr::null();
        }

        unsafe { pair_ptr.add(layout.value_offset as usize) as *const c_void }
    }

    /// Find the mutable value pointer for a given key, or null if not found.
    pub fn find_value_mut(
        &mut self,
        key: *const c_void,
        layout: &FScriptMapLayout,
        get_key_hash: impl Fn(*const c_void) -> u32,
        key_equality_fn: impl Fn(*const c_void, *const c_void) -> bool,
    ) -> *mut c_void {
        let index = self.find_pair_index(key, layout, get_key_hash, key_equality_fn);
        if index == -1 {
            return std::ptr::null_mut();
        }

        let pair_ptr = self.get_data_mut(index, layout) as *mut u8;
        if pair_ptr.is_null() {
            return std::ptr::null_mut();
        }

        unsafe { pair_ptr.add(layout.value_offset as usize) as *mut c_void }
    }

    /// Find or add a pair by key, returning a pointer to the value.
    pub fn find_or_add(
        &mut self,
        key: *const c_void,
        layout: &FScriptMapLayout,
        get_key_hash: impl Fn(*const c_void) -> u32,
        key_equality_fn: impl Fn(*const c_void, *const c_void) -> bool,
        construct_pair_fn: impl Fn(*mut c_void, *mut c_void), // (key_ptr, value_ptr)
    ) -> *mut c_void {
        let value_offset = layout.value_offset;
        let pair_index = self.pairs.find_or_add(
            key,
            &layout.set_layout,
            get_key_hash,
            key_equality_fn,
            move |new_pair: *mut c_void| unsafe {
                let value_ptr = (new_pair as *mut u8).add(value_offset as usize) as *mut c_void;
                construct_pair_fn(new_pair, value_ptr);
            },
        );

        let pair_ptr = self.pairs.get_data_mut(pair_index, &layout.set_layout) as *mut u8;
        if pair_ptr.is_null() {
            return std::ptr::null_mut();
        }
        unsafe { pair_ptr.add(value_offset as usize) as *mut c_void }
    }
}

// FScriptMap is a concrete type alias using the default allocator
pub type FScriptMap = TScriptMap<FDefaultSetAllocator, ()>;

// Typed API on the concrete FScriptMap type
impl FScriptMap {
    /// Add or replace a key-value pair using typed values.
    pub fn add_element<K, V>(
        &mut self,
        key: K,
        value: V,
        get_key_hash: impl Fn(&K) -> u32,
        key_equality_fn: impl Fn(&K, &K) -> bool,
    ) -> (i32, bool) {
        let pair = TPair::new(key, value);

        self.pairs.add_element(
            pair,
            |p| get_key_hash(&p.key),
            move |a, b| key_equality_fn(&a.key, &b.key),
        )
    }

    /// Find a key's index in the map.
    fn find_key_index<K: Clone, V>(
        &self,
        key: &K,
        get_key_hash: impl Fn(&K) -> u32,
        key_equality_fn: impl Fn(&K, &K) -> bool,
    ) -> i32 {
        let layout = FScriptSetLayout::for_type::<TPair<K, V>>();
        let key_hash = get_key_hash(key);

        // We need a TPair to pass to find_element_index, but we only have a key.
        // Since TPair is repr(C) with key at offset 0, and the equality function
        // only compares keys, we can construct a search pair with an uninitialized value.
        let mut search_pair = std::mem::MaybeUninit::<TPair<K, V>>::uninit();
        unsafe {
            std::ptr::write(
                std::ptr::addr_of_mut!((*search_pair.as_mut_ptr()).key),
                key.clone(),
            );
        }

        let result = self.pairs.find_element_index(
            unsafe { search_pair.assume_init_ref() },
            &layout,
            key_hash,
            |a, b| key_equality_fn(&a.key, &b.key),
        );

        // Drop the key we wrote (don't drop the whole pair since value is uninit)
        unsafe {
            std::ptr::drop_in_place(std::ptr::addr_of_mut!((*search_pair.as_mut_ptr()).key));
        }

        result
    }

    /// Get a reference to the value for a given key.
    pub fn get<'a, K: Clone + 'a, V: 'a>(
        &'a self,
        key: &K,
        get_key_hash: impl Fn(&K) -> u32,
        key_equality_fn: impl Fn(&K, &K) -> bool,
    ) -> Option<&'a V> {
        let index = self.find_key_index::<K, V>(key, get_key_hash, key_equality_fn);
        self.pairs.get::<TPair<K, V>>(index).map(|pair| &pair.value)
    }

    /// Get a mutable reference to the value for a given key.
    pub fn get_mut<'a, K: Clone + 'a, V: 'a>(
        &'a mut self,
        key: &K,
        get_key_hash: impl Fn(&K) -> u32,
        key_equality_fn: impl Fn(&K, &K) -> bool,
    ) -> Option<&'a mut V> {
        let index = self.find_key_index::<K, V>(key, get_key_hash, key_equality_fn);
        self.pairs
            .get_mut::<TPair<K, V>>(index)
            .map(|pair| &mut pair.value)
    }

    /// Check if the map contains a given key.
    pub fn contains_key<K, V>(
        &self,
        key: &K,
        get_key_hash: impl Fn(&K) -> u32,
        key_equality_fn: impl Fn(&K, &K) -> bool,
    ) -> bool
    where
        K: Clone,
    {
        self.find_key_index::<K, V>(key, get_key_hash, key_equality_fn) != -1
    }

    /// Remove a key-value pair by key. Returns true if found and removed.
    pub fn remove<K, V>(
        &mut self,
        key: &K,
        get_key_hash: impl Fn(&K) -> u32,
        key_equality_fn: impl Fn(&K, &K) -> bool,
    ) -> bool
    where
        K: Clone,
    {
        let index = self.find_key_index::<K, V>(key, get_key_hash, key_equality_fn);
        if index == -1 {
            return false;
        }

        self.pairs.remove::<TPair<K, V>>(index)
    }

    /// Clear all elements from the map.
    pub fn clear<K, V>(&mut self) {
        self.pairs.clear::<TPair<K, V>>();
    }

    /// Get an iterator over valid element indices.
    pub fn indices(&self) -> ScriptMapIndexIterator<'_> {
        ScriptMapIndexIterator {
            set_iter: self.pairs.indices(),
        }
    }

    /// Get a key-value pair reference at a given index.
    pub fn get_pair<K, V>(&self, index: i32) -> Option<(&K, &V)> {
        self.pairs
            .get::<TPair<K, V>>(index)
            .map(|pair| (&pair.key, &pair.value))
    }

    /// Get a mutable value reference at a given index (key remains immutable).
    pub fn get_pair_mut<K, V>(&mut self, index: i32) -> Option<(&K, &mut V)> {
        self.pairs
            .get_mut::<TPair<K, V>>(index)
            .map(|pair| (&pair.key, &mut pair.value))
    }
}

/// Iterator over valid indices in a FScriptMap
pub struct ScriptMapIndexIterator<'a> {
    set_iter: ScriptSetIndexIterator<'a>,
}

impl<'a> Iterator for ScriptMapIndexIterator<'a> {
    type Item = i32;

    fn next(&mut self) -> Option<Self::Item> {
        self.set_iter.next()
    }
}

// Static assertions to ensure layout compatibility with UE
const _: () = {
    // FScriptMap should be the same size as FScriptSet (80 bytes)
    assert!(std::mem::size_of::<TScriptMap<FDefaultSetAllocator, ()>>() == 80);
};

#[cfg(test)]
mod tests {
    use super::*;
    use malloc::test::setup_test_globals;

    #[test]
    fn test_script_map_layout() {
        // i32 key, i32 value
        let layout = FScriptMapLayout::get_layout(4, 4, 4, 4);
        assert_eq!(layout.value_offset, 4);

        // u8 key, i32 value (value needs alignment)
        let layout = FScriptMapLayout::get_layout(1, 1, 4, 4);
        assert_eq!(layout.value_offset, 4);

        // i32 key, u8 value
        let layout = FScriptMapLayout::get_layout(4, 4, 1, 1);
        assert_eq!(layout.value_offset, 4);
    }

    #[test]
    fn test_script_map_layout_for_types() {
        let layout = FScriptMapLayout::for_types::<i32, i32>();
        assert_eq!(layout.value_offset, 4);

        let layout = FScriptMapLayout::for_types::<u8, i32>();
        assert_eq!(layout.value_offset, 4);

        let layout = FScriptMapLayout::for_types::<i32, u8>();
        assert_eq!(layout.value_offset, 4);
    }

    #[test]
    fn test_script_map_basic() {
        setup_test_globals();

        let mut map = FScriptMap::new();
        assert!(map.is_empty());
        assert_eq!(map.num(), 0);

        let hash_fn = |x: &i32| *x as u32;
        let eq_fn = |a: &i32, b: &i32| a == b;

        // Add elements
        map.add_element(10i32, 100i32, hash_fn, eq_fn);
        map.add_element(20i32, 200i32, hash_fn, eq_fn);
        map.add_element(30i32, 300i32, hash_fn, eq_fn);

        assert_eq!(map.num(), 3);
        assert!(!map.is_empty());

        // Lookup by key
        assert_eq!(map.get::<i32, i32>(&10, hash_fn, eq_fn), Some(&100));
        assert_eq!(map.get::<i32, i32>(&20, hash_fn, eq_fn), Some(&200));
        assert_eq!(map.get::<i32, i32>(&30, hash_fn, eq_fn), Some(&300));
        assert_eq!(map.get::<i32, i32>(&40, hash_fn, eq_fn), None);

        // Contains
        assert!(map.contains_key::<i32, i32>(&10, hash_fn, eq_fn));
        assert!(map.contains_key::<i32, i32>(&20, hash_fn, eq_fn));
        assert!(!map.contains_key::<i32, i32>(&40, hash_fn, eq_fn));
    }

    #[test]
    fn test_script_map_replace() {
        setup_test_globals();

        let mut map = FScriptMap::new();
        let hash_fn = |x: &i32| *x as u32;
        let eq_fn = |a: &i32, b: &i32| a == b;

        let (_, was_existing) = map.add_element(10i32, 100i32, hash_fn, eq_fn);
        assert!(!was_existing);

        let (_, was_existing) = map.add_element(10i32, 999i32, hash_fn, eq_fn);
        assert!(was_existing);

        assert_eq!(map.num(), 1);
        assert_eq!(map.get::<i32, i32>(&10, hash_fn, eq_fn), Some(&999));
    }

    #[test]
    fn test_script_map_remove() {
        setup_test_globals();

        let mut map = FScriptMap::new();
        let hash_fn = |x: &i32| *x as u32;
        let eq_fn = |a: &i32, b: &i32| a == b;

        map.add_element(10i32, 100i32, hash_fn, eq_fn);
        map.add_element(20i32, 200i32, hash_fn, eq_fn);
        map.add_element(30i32, 300i32, hash_fn, eq_fn);

        assert!(map.remove::<i32, i32>(&20, hash_fn, eq_fn));
        assert_eq!(map.num(), 2);
        assert!(!map.contains_key::<i32, i32>(&20, hash_fn, eq_fn));
        assert!(map.contains_key::<i32, i32>(&10, hash_fn, eq_fn));
        assert!(map.contains_key::<i32, i32>(&30, hash_fn, eq_fn));

        assert!(!map.remove::<i32, i32>(&99, hash_fn, eq_fn));
        assert_eq!(map.num(), 2);
    }

    #[test]
    fn test_script_map_mutate() {
        setup_test_globals();

        let mut map = FScriptMap::new();
        let hash_fn = |x: &i32| *x as u32;
        let eq_fn = |a: &i32, b: &i32| a == b;

        map.add_element(10i32, 100i32, hash_fn, eq_fn);

        if let Some(value) = map.get_mut::<i32, i32>(&10, hash_fn, eq_fn) {
            *value = 999;
        }
        assert_eq!(map.get::<i32, i32>(&10, hash_fn, eq_fn), Some(&999));
    }

    #[test]
    fn test_script_map_clear() {
        setup_test_globals();

        let mut map = FScriptMap::new();
        let hash_fn = |x: &i32| *x as u32;
        let eq_fn = |a: &i32, b: &i32| a == b;

        map.add_element(1i32, 10i32, hash_fn, eq_fn);
        map.add_element(2i32, 20i32, hash_fn, eq_fn);
        map.add_element(3i32, 30i32, hash_fn, eq_fn);

        map.clear::<i32, i32>();
        assert_eq!(map.num(), 0);
        assert!(map.is_empty());

        map.add_element(42i32, 420i32, hash_fn, eq_fn);
        assert_eq!(map.num(), 1);
        assert_eq!(map.get::<i32, i32>(&42, hash_fn, eq_fn), Some(&420));
    }

    #[test]
    fn test_script_map_pair_access() {
        setup_test_globals();

        let mut map = FScriptMap::new();
        let hash_fn = |x: &i32| *x as u32;
        let eq_fn = |a: &i32, b: &i32| a == b;

        map.add_element(10i32, 100i32, hash_fn, eq_fn);
        map.add_element(20i32, 200i32, hash_fn, eq_fn);

        let mut pairs: Vec<(i32, i32)> = map
            .indices()
            .filter_map(|idx| map.get_pair::<i32, i32>(idx).map(|(&k, &v)| (k, v)))
            .collect();
        pairs.sort();
        assert_eq!(pairs, vec![(10, 100), (20, 200)]);
    }

    #[test]
    fn test_script_map_complex_types() {
        setup_test_globals();

        let mut map = FScriptMap::new();
        let hash_fn = |x: &u32| *x;
        let eq_fn = |a: &u32, b: &u32| a == b;

        map.add_element(1u32, String::from("one"), hash_fn, eq_fn);
        map.add_element(2u32, String::from("two"), hash_fn, eq_fn);

        assert_eq!(
            map.get::<u32, String>(&1, hash_fn, eq_fn),
            Some(&String::from("one"))
        );
        assert_eq!(
            map.get::<u32, String>(&2, hash_fn, eq_fn),
            Some(&String::from("two"))
        );

        map.clear::<u32, String>();
    }
}
