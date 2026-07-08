use super::set::GetTypeHash;
use super::*;
use std::marker::PhantomData;

// UE's TPair equivalent - key-value pair
#[repr(C)]
#[derive(Debug, Clone, PartialEq)]
pub struct TPair<K, V> {
    pub key: K,
    pub value: V,
}

impl<K, V> TPair<K, V> {
    pub fn new(key: K, value: V) -> Self {
        Self { key, value }
    }
}

impl<K: GetTypeHash, V: GetTypeHash> GetTypeHash for TPair<K, V> {
    fn get_type_hash(&self) -> u32 {
        // Use the same hash combining as TTuple since TPair serves similar purpose
        let key_hash = self.key.get_type_hash();
        let value_hash = self.value.get_type_hash();

        // UE's hash combine formula
        key_hash
            ^ (value_hash
                .wrapping_add(0x9e3779b9)
                .wrapping_add(key_hash << 6)
                .wrapping_add(key_hash >> 2))
    }
}

// TPairInitializer - used for adding pairs to maps
#[derive(Debug, Clone)]
pub struct TPairInitializer<K, V> {
    pub key: K,
    pub value: V,
}

impl<K, V> TPairInitializer<K, V> {
    pub fn new(key: K, value: V) -> Self {
        Self { key, value }
    }
}

impl<K, V> From<TPair<K, V>> for TPairInitializer<K, V> {
    fn from(pair: TPair<K, V>) -> Self {
        Self {
            key: pair.key,
            value: pair.value,
        }
    }
}

impl<K, V> From<TPairInitializer<K, V>> for TPair<K, V> {
    fn from(init: TPairInitializer<K, V>) -> Self {
        Self {
            key: init.key,
            value: init.value,
        }
    }
}

// Default map key funcs - keys are extracted from TPair elements
pub struct TDefaultMapKeyFuncs<K, V> {
    _phantom: PhantomData<(K, V)>,
}

impl<K, V> Default for TDefaultMapKeyFuncs<K, V> {
    fn default() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<K, V> KeyFuncs<TPair<K, V>> for TDefaultMapKeyFuncs<K, V>
where
    K: PartialEq + Clone + GetTypeHash,
    V: Clone,
{
    type KeyType = K;

    fn get_set_key(element: &TPair<K, V>) -> &Self::KeyType {
        &element.key
    }

    fn get_key_hash(key: &Self::KeyType) -> u32 {
        key.get_type_hash()
    }

    fn matches(a: &Self::KeyType, b: &Self::KeyType) -> bool {
        a == b
    }

    const ALLOW_DUPLICATE_KEYS: bool = false; // TMap doesn't allow duplicates
}

// TMultiMap KeyFuncs - allows duplicate keys
pub struct TMultiMapKeyFuncs<K, V> {
    _phantom: PhantomData<(K, V)>,
}

impl<K, V> Default for TMultiMapKeyFuncs<K, V> {
    fn default() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<K, V> KeyFuncs<TPair<K, V>> for TMultiMapKeyFuncs<K, V>
where
    K: PartialEq + Clone + GetTypeHash,
    V: Clone,
{
    type KeyType = K;

    fn get_set_key(element: &TPair<K, V>) -> &Self::KeyType {
        &element.key
    }

    fn get_key_hash(key: &Self::KeyType) -> u32 {
        key.get_type_hash()
    }

    fn matches(a: &Self::KeyType, b: &Self::KeyType) -> bool {
        a == b
    }

    const ALLOW_DUPLICATE_KEYS: bool = true; // TMultiMap allows duplicates
}

// TMap - single value per key
#[repr(C)]
pub struct TMap<
    KeyType,
    ValueType,
    SetAllocator = FDefaultSetAllocator,
    KeyFuncs = TDefaultMapKeyFuncs<KeyType, ValueType>,
> where
    SetAllocator: super::set::SetAllocator,
    KeyFuncs: super::set::KeyFuncs<TPair<KeyType, ValueType>, KeyType = KeyType>,
    KeyType: Clone + GetTypeHash + PartialEq,
    ValueType: Clone,
{
    pub pairs: TSet<TPair<KeyType, ValueType>, KeyFuncs, SetAllocator>,
}

impl<K, V, A, KF> std::fmt::Debug for TMap<K, V, A, KF>
where
    A: SetAllocator,
    KF: super::set::KeyFuncs<TPair<K, V>, KeyType = K>,
    K: Clone + GetTypeHash + PartialEq + std::fmt::Debug,
    V: Clone + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TMap")
            .field("pairs", &self.pairs.len())
            .finish()
    }
}

impl<K, V, A, KF> Default for TMap<K, V, A, KF>
where
    A: SetAllocator,
    KF: super::set::KeyFuncs<TPair<K, V>, KeyType = K>,
    K: Clone + GetTypeHash + PartialEq,
    V: Clone,
{
    fn default() -> Self {
        Self { pairs: TSet::new() }
    }
}

impl<K, V, A, KF> TMap<K, V, A, KF>
where
    A: SetAllocator,
    KF: super::set::KeyFuncs<TPair<K, V>, KeyType = K>,
    K: Clone + GetTypeHash + PartialEq,
    V: Clone,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            pairs: TSet::with_capacity(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.pairs.capacity()
    }

    pub fn reserve(&mut self, capacity: usize) {
        // Since TSet::reserve is private, we'll use with_capacity when creating
        // or just document that capacity management is handled internally
        // For now, this is a no-op that satisfies the interface
        let _ = capacity; // Acknowledge the capacity hint
    }

    pub fn clear(&mut self) {
        self.pairs.clear();
    }

    pub fn add(&mut self, key: K, value: V) -> &mut V {
        let pair = TPair::new(key, value);
        let pair_id = self.pairs.insert(pair);
        &mut self.pairs.get_mut(pair_id).unwrap().value
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        // Check if key exists first
        if let Some(existing_pair) = self.find_pair_mut(&key) {
            let old_value = std::mem::replace(&mut existing_pair.value, value);
            Some(old_value)
        } else {
            self.pairs.insert(TPair::new(key, value));
            None
        }
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        if let Some(pair_id) = self.find_id(key)
            && let Some(pair) = self.pairs.get(pair_id).cloned()
        {
            self.pairs.remove(pair_id);
            return Some(pair.value);
        }
        None
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.find_pair(key).map(|pair| &pair.value)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.find_pair_mut(key).map(|pair| &mut pair.value)
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.pairs.contains(key)
    }

    pub fn find_or_add(&mut self, key: K) -> &mut V
    where
        V: Default,
    {
        // Check if key exists first
        if let Some(pair_id) = self.find_id(&key) {
            return &mut self.pairs.get_mut(pair_id).unwrap().value;
        }

        // If not found, add with default value
        let pair = TPair::new(key, V::default());
        let pair_id = self.pairs.insert(pair);
        &mut self.pairs.get_mut(pair_id).unwrap().value
    }

    pub fn find_or_insert(&mut self, key: K, value: V) -> &mut V {
        // Check if key exists first
        if let Some(pair_id) = self.find_id(&key) {
            return &mut self.pairs.get_mut(pair_id).unwrap().value;
        }

        // If not found, insert the new pair
        let pair = TPair::new(key, value);
        let pair_id = self.pairs.insert(pair);
        &mut self.pairs.get_mut(pair_id).unwrap().value
    }

    fn find_pair(&self, key: &K) -> Option<&TPair<K, V>> {
        if let Some(pair_id) = self.find_id(key) {
            self.pairs.get(pair_id)
        } else {
            None
        }
    }

    fn find_pair_mut(&mut self, key: &K) -> Option<&mut TPair<K, V>> {
        if let Some(pair_id) = self.find_id(key) {
            self.pairs.get_mut(pair_id)
        } else {
            None
        }
    }

    fn find_id(&self, key: &K) -> Option<FSetElementId> {
        self.pairs.find(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.pairs.iter().map(|pair| &pair.key)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.pairs.iter().map(|pair| &pair.value)
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.pairs.iter_mut().map(|pair| &mut pair.value)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.pairs.iter().map(|pair| (&pair.key, &pair.value))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        self.pairs
            .iter_mut()
            .map(|pair| (&pair.key, &mut pair.value))
    }
}

// Implement indexing for TMap
impl<K, V, A, KF> std::ops::Index<&K> for TMap<K, V, A, KF>
where
    A: SetAllocator,
    KF: super::set::KeyFuncs<TPair<K, V>, KeyType = K>,
    K: Clone + GetTypeHash + PartialEq,
    V: Clone,
{
    type Output = V;

    fn index(&self, key: &K) -> &Self::Output {
        self.get(key).expect("Key not found in map")
    }
}

impl<K, V, A, KF> std::ops::IndexMut<&K> for TMap<K, V, A, KF>
where
    A: SetAllocator,
    KF: super::set::KeyFuncs<TPair<K, V>, KeyType = K>,
    K: Clone + GetTypeHash + PartialEq,
    V: Clone,
{
    fn index_mut(&mut self, key: &K) -> &mut Self::Output {
        self.get_mut(key).expect("Key not found in map")
    }
}

// Support for for-in loops over references
impl<'a, K, V, A, KF> IntoIterator for &'a TMap<K, V, A, KF>
where
    A: SetAllocator,
    KF: super::set::KeyFuncs<TPair<K, V>, KeyType = K>,
    K: Clone + GetTypeHash + PartialEq,
    V: Clone,
{
    type Item = (&'a K, &'a V);
    type IntoIter = std::iter::Map<
        super::set::SetIterator<
            'a,
            TPair<K, V>,
            <<A as SetAllocator>::SparseArrayAllocator as SparseArrayAllocator>::BitArrayAllocator,
        >,
        fn(&'a TPair<K, V>) -> (&'a K, &'a V),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.pairs.iter().map(|pair| (&pair.key, &pair.value))
    }
}

impl<'a, K, V, A, KF> IntoIterator for &'a mut TMap<K, V, A, KF>
where
    A: SetAllocator,
    KF: super::set::KeyFuncs<TPair<K, V>, KeyType = K>,
    K: Clone + GetTypeHash + PartialEq,
    V: Clone,
{
    type Item = (&'a K, &'a mut V);
    type IntoIter = std::iter::Map<
        super::set::SetIteratorMut<
            'a,
            TPair<K, V>,
            <<A as SetAllocator>::SparseArrayAllocator as SparseArrayAllocator>::BitArrayAllocator,
        >,
        fn(&'a mut TPair<K, V>) -> (&'a K, &'a mut V),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.pairs
            .iter_mut()
            .map(|pair| (&pair.key, &mut pair.value))
    }
}

// TMultiMap - allows multiple values per key
#[repr(C)]
pub struct TMultiMap<
    KeyType,
    ValueType,
    SetAllocator = FDefaultSetAllocator,
    KeyFuncs = TMultiMapKeyFuncs<KeyType, ValueType>,
> where
    SetAllocator: super::set::SetAllocator,
    KeyFuncs: super::set::KeyFuncs<TPair<KeyType, ValueType>, KeyType = KeyType>,
    KeyType: Clone + GetTypeHash + PartialEq,
    ValueType: Clone,
{
    pub pairs: TSet<TPair<KeyType, ValueType>, KeyFuncs, SetAllocator>,
}

impl<K, V, A, KF> Default for TMultiMap<K, V, A, KF>
where
    A: SetAllocator,
    KF: super::set::KeyFuncs<TPair<K, V>, KeyType = K>,
    K: Clone + GetTypeHash + PartialEq,
    V: Clone,
{
    fn default() -> Self {
        Self { pairs: TSet::new() }
    }
}

impl<K, V, A, KF> TMultiMap<K, V, A, KF>
where
    A: SetAllocator,
    KF: super::set::KeyFuncs<TPair<K, V>, KeyType = K>,
    K: Clone + GetTypeHash + PartialEq,
    V: Clone,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    pub fn add(&mut self, key: K, value: V) {
        self.pairs.insert(TPair::new(key, value));
    }

    pub fn remove_all(&mut self, key: &K) -> usize {
        let mut removed_count = 0;
        // Keep removing until no more pairs with this key exist
        while self.pairs.remove_by_key(key) {
            removed_count += 1;
        }
        removed_count
    }

    pub fn find_all(&self, key: &K) -> Vec<&V> {
        // Linear search through pairs since we allow duplicates
        self.pairs
            .iter()
            .filter(|pair| &pair.key == key)
            .map(|pair| &pair.value)
            .collect()
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.pairs.contains(key)
    }

    pub fn clear(&mut self) {
        self.pairs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use malloc::test::setup_test_globals;

    #[test]
    fn test_tmap_basic_operations() {
        setup_test_globals();

        let mut map: TMap<i32, String> = TMap::new();

        assert!(map.is_empty());
        assert_eq!(map.len(), 0);

        // Test add
        map.add(1, "one".to_string());
        map.add(2, "two".to_string());
        map.add(3, "three".to_string());

        assert!(!map.is_empty());
        assert_eq!(map.len(), 3);

        // Test get
        assert_eq!(map.get(&1), Some(&"one".to_string()));
        assert_eq!(map.get(&2), Some(&"two".to_string()));
        assert_eq!(map.get(&3), Some(&"three".to_string()));
        assert_eq!(map.get(&4), None);

        // Test contains
        assert!(map.contains_key(&1));
        assert!(map.contains_key(&2));
        assert!(!map.contains_key(&4));
    }

    #[test]
    fn test_tmap_insert_replace() {
        setup_test_globals();

        let mut map: TMap<i32, String> = TMap::new();

        // Insert new key
        assert_eq!(map.insert(1, "one".to_string()), None);
        assert_eq!(map.len(), 1);

        // Replace existing key
        assert_eq!(map.insert(1, "ONE".to_string()), Some("one".to_string()));
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&1), Some(&"ONE".to_string()));
    }

    #[test]
    fn test_tmap_remove() {
        setup_test_globals();

        let mut map: TMap<i32, String> = TMap::new();

        map.add(1, "one".to_string());
        map.add(2, "two".to_string());

        // Remove existing key
        assert_eq!(map.remove(&1), Some("one".to_string()));
        assert_eq!(map.len(), 1);
        assert!(!map.contains_key(&1));

        // Remove non-existent key
        assert_eq!(map.remove(&3), None);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_tmap_indexing() {
        setup_test_globals();

        let mut map: TMap<i32, String> = TMap::new();

        map.add(1, "one".to_string());
        map.add(2, "two".to_string());

        // Test index access
        assert_eq!(&map[&1], "one");
        assert_eq!(&map[&2], "two");

        // Test mutable index access
        map[&1] = "ONE".to_string();
        assert_eq!(&map[&1], "ONE");
    }

    #[test]
    fn test_tmap_iteration() {
        setup_test_globals();

        let mut map: TMap<i32, String> = TMap::new();

        map.add(1, "one".to_string());
        map.add(2, "two".to_string());
        map.add(3, "three".to_string());

        // Test key iteration
        let keys: Vec<i32> = map.keys().cloned().collect();
        assert_eq!(keys.len(), 3);
        assert!(keys.contains(&1));
        assert!(keys.contains(&2));
        assert!(keys.contains(&3));

        // Test value iteration
        let values: Vec<String> = map.values().cloned().collect();
        assert_eq!(values.len(), 3);
        assert!(values.contains(&"one".to_string()));
        assert!(values.contains(&"two".to_string()));
        assert!(values.contains(&"three".to_string()));

        // Test pair iteration
        let pairs: Vec<(i32, String)> = map.iter().map(|(k, v)| (*k, v.clone())).collect();
        assert_eq!(pairs.len(), 3);
    }

    #[test]
    fn test_tmultimap_basic_operations() {
        setup_test_globals();

        let mut map: TMultiMap<i32, String> = TMultiMap::new();

        assert!(map.is_empty());

        // Add multiple values for same key
        map.add(1, "one".to_string());
        map.add(1, "uno".to_string());
        map.add(2, "two".to_string());

        assert_eq!(map.len(), 3);
        assert!(map.contains_key(&1));
        assert!(map.contains_key(&2));

        // Find all values for a key
        let values = map.find_all(&1);
        assert_eq!(values.len(), 2);
        assert!(values.contains(&&"one".to_string()));
        assert!(values.contains(&&"uno".to_string()));

        let values_2 = map.find_all(&2);
        assert_eq!(values_2.len(), 1);
        assert_eq!(values_2[0], &"two".to_string());
    }

    #[test]
    fn test_tmap_u64_keys() {
        setup_test_globals();

        let mut map: TMap<u64, i32> = TMap::new();

        // u64 uses cityhasher for GetTypeHash
        let keys: Vec<u64> = vec![0, 1, u64::MAX, 0x123456789ABCDEF0, 0xFEDCBA9876543210];

        for (i, &key) in keys.iter().enumerate() {
            map.add(key, i as i32);
        }

        assert_eq!(map.len(), keys.len());

        for (i, &key) in keys.iter().enumerate() {
            assert_eq!(
                map.get(&key),
                Some(&(i as i32)),
                "Failed for key 0x{:X}",
                key
            );
        }

        // Remove and verify
        assert!(map.remove(&0xFEDCBA9876543210).is_some());
        assert_eq!(map.len(), keys.len() - 1);
        assert!(!map.contains_key(&0xFEDCBA9876543210));
    }

    #[test]
    fn test_tmap_i64_keys() {
        setup_test_globals();

        let mut map: TMap<i64, String> = TMap::new();

        map.add(-1, "neg_one".to_string());
        map.add(0, "zero".to_string());
        map.add(i64::MAX, "max".to_string());
        map.add(i64::MIN, "min".to_string());

        assert_eq!(map.len(), 4);
        assert_eq!(map.get(&-1), Some(&"neg_one".to_string()));
        assert_eq!(map.get(&0), Some(&"zero".to_string()));
        assert_eq!(map.get(&i64::MAX), Some(&"max".to_string()));
        assert_eq!(map.get(&i64::MIN), Some(&"min".to_string()));
    }

    #[test]
    fn test_tmap_pointer_keys() {
        setup_test_globals();

        let mut map: TMap<*mut u32, i32> = TMap::new();

        let mut vals = [10u32, 20, 30];
        let ptrs: Vec<*mut u32> = vals.iter_mut().map(|v| v as *mut u32).collect();

        for (i, &ptr) in ptrs.iter().enumerate() {
            map.add(ptr, i as i32);
        }

        assert_eq!(map.len(), 3);

        for (i, &ptr) in ptrs.iter().enumerate() {
            assert_eq!(map.get(&ptr), Some(&(i as i32)));
        }

        // Null pointer as key
        map.add(std::ptr::null_mut(), 99);
        assert_eq!(map.get(&std::ptr::null_mut()), Some(&99));
    }

    #[test]
    fn test_tmap_find_or_add() {
        setup_test_globals();

        let mut map: TMap<i32, String> = TMap::new();

        // find_or_add on empty map creates default
        let val = map.find_or_add(1);
        assert_eq!(val, &""); // String::default() is ""
        *val = "one".to_string();

        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&1), Some(&"one".to_string()));

        // find_or_add on existing key returns existing value
        let val = map.find_or_add(1);
        assert_eq!(val, &"one");
    }

    #[test]
    fn test_tmap_find_or_insert() {
        setup_test_globals();

        let mut map: TMap<i32, String> = TMap::new();

        // find_or_insert on empty map creates with given value
        let val = map.find_or_insert(1, "one".to_string());
        assert_eq!(val, &"one");

        // find_or_insert on existing key returns existing, doesn't replace
        let val = map.find_or_insert(1, "ONE".to_string());
        assert_eq!(val, &"one"); // still the original
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_tmap_many_elements_rehash() {
        setup_test_globals();

        let mut map: TMap<u32, u32> = TMap::new();

        // Insert enough to trigger multiple rehashes
        for i in 0..200 {
            map.add(i * 7, i * 100);
        }

        assert_eq!(map.len(), 200);

        // All should be findable after rehashing
        for i in 0..200 {
            assert_eq!(
                map.get(&(i * 7)),
                Some(&(i * 100)),
                "Missing key {} after rehash",
                i * 7
            );
        }

        // Remove half
        for i in 0..100 {
            assert!(map.remove(&(i * 7)).is_some());
        }
        assert_eq!(map.len(), 100);

        // Remaining half still accessible
        for i in 100..200 {
            assert!(map.contains_key(&(i * 7)));
        }

        // Removed half gone
        for i in 0..100 {
            assert!(!map.contains_key(&(i * 7)));
        }
    }

    #[test]
    fn test_tmap_hash_collision_stress() {
        setup_test_globals();

        let mut map: TMap<u32, u32> = TMap::new();

        // Insert values that will collide in small hash tables
        // (powers of 2 and multiples of hash_size)
        let values: Vec<u32> = (0..50).map(|i| i * 8).collect();

        for &v in &values {
            map.add(v, v * 10);
        }

        assert_eq!(map.len(), values.len());

        for &v in &values {
            assert_eq!(map.get(&v), Some(&(v * 10)));
        }

        // Remove every other element
        for &v in values.iter().step_by(2) {
            assert!(map.remove(&v).is_some());
        }

        // Verify remaining
        for (i, &v) in values.iter().enumerate() {
            if i % 2 == 0 {
                assert!(!map.contains_key(&v));
            } else {
                assert_eq!(map.get(&v), Some(&(v * 10)));
            }
        }
    }

    #[test]
    fn test_tmap_values_mut_iteration() {
        setup_test_globals();

        let mut map: TMap<i32, i32> = TMap::new();

        map.add(1, 10);
        map.add(2, 20);
        map.add(3, 30);

        // Mutate all values
        for v in map.values_mut() {
            *v *= 2;
        }

        assert_eq!(map.get(&1), Some(&20));
        assert_eq!(map.get(&2), Some(&40));
        assert_eq!(map.get(&3), Some(&60));
    }

    #[test]
    fn test_tmultimap_remove_all() {
        setup_test_globals();

        let mut map: TMultiMap<i32, String> = TMultiMap::new();

        map.add(1, "a".to_string());
        map.add(1, "b".to_string());
        map.add(1, "c".to_string());
        map.add(2, "x".to_string());

        assert_eq!(map.len(), 4);

        let removed = map.remove_all(&1);
        assert_eq!(removed, 3);
        assert_eq!(map.len(), 1);
        assert!(!map.contains_key(&1));
        assert!(map.contains_key(&2));
    }

    #[test]
    fn test_tmultimap_many_duplicates() {
        setup_test_globals();

        let mut map: TMultiMap<u32, u32> = TMultiMap::new();

        // Many values for same key
        for i in 0..50 {
            map.add(1, i);
        }
        // A few other keys
        for i in 0..10 {
            map.add(i + 100, i);
        }

        assert_eq!(map.len(), 60);

        let values = map.find_all(&1);
        assert_eq!(values.len(), 50);

        // Remove all duplicates
        let removed = map.remove_all(&1);
        assert_eq!(removed, 50);
        assert_eq!(map.len(), 10);

        // Other keys still intact
        for i in 0..10 {
            assert!(map.contains_key(&(i + 100)));
        }
    }

    #[test]
    fn test_tmap_with_capacity() {
        setup_test_globals();

        let mut map: TMap<u32, u32> = TMap::with_capacity(100);

        assert!(map.is_empty());
        assert!(map.capacity() >= 100);

        for i in 0..50 {
            map.add(i, i * 10);
        }

        assert_eq!(map.len(), 50);
        for i in 0..50 {
            assert_eq!(map.get(&i), Some(&(i * 10)));
        }
    }

    #[test]
    fn test_tmap_inline_allocator() {
        setup_test_globals();

        // TMap with TInlineSetAllocator<4> as set allocator
        let mut map: TMap<i32, i32, TInlineSetAllocator<4>> = TMap::new();

        assert!(map.is_empty());

        map.add(1, 10);
        map.add(2, 20);
        map.add(3, 30);

        assert_eq!(map.len(), 3);
        assert_eq!(map.get(&1), Some(&10));
        assert_eq!(map.get(&2), Some(&20));
        assert_eq!(map.get(&3), Some(&30));

        // Replace
        map.add(2, 200);
        assert_eq!(map.get(&2), Some(&200));
        assert_eq!(map.len(), 3);

        // Remove
        assert!(map.remove(&1).is_some());
        assert_eq!(map.len(), 2);
        assert!(!map.contains_key(&1));

        // Clear
        map.clear();
        assert!(map.is_empty());
    }

    #[test]
    fn test_tmap_inline_allocator_grow_beyond_inline() {
        setup_test_globals();

        // Small inline allocator that will be exceeded
        let mut map: TMap<u32, u32, TInlineSetAllocator<1>> = TMap::new();

        // Insert more elements than inline capacity to force heap allocation
        for i in 0..50 {
            map.add(i, i * 100);
        }

        assert_eq!(map.len(), 50);

        // All should be accessible after spilling to heap
        for i in 0..50 {
            assert_eq!(map.get(&i), Some(&(i * 100)), "Missing key {}", i);
        }

        // Remove and re-add to test free list with heap storage
        for i in 0..25 {
            assert!(map.remove(&i).is_some());
        }
        assert_eq!(map.len(), 25);

        for i in 0..25 {
            map.add(i + 1000, i);
        }
        assert_eq!(map.len(), 50);
    }

    #[test]
    fn test_tmap_inline_allocator_with_strings() {
        setup_test_globals();

        let mut map: TMap<i32, String, TInlineSetAllocator<2>> = TMap::new();

        map.add(1, "hello".to_string());
        map.add(2, "world".to_string());
        map.add(3, "foo".to_string());
        map.add(4, "bar".to_string());

        assert_eq!(map.len(), 4);
        assert_eq!(map.get(&1), Some(&"hello".to_string()));
        assert_eq!(map.get(&4), Some(&"bar".to_string()));

        // Iteration
        let mut pairs: Vec<(i32, String)> = map.iter().map(|(k, v)| (*k, v.clone())).collect();
        pairs.sort_by_key(|(k, _)| *k);
        assert_eq!(
            pairs,
            vec![
                (1, "hello".to_string()),
                (2, "world".to_string()),
                (3, "foo".to_string()),
                (4, "bar".to_string()),
            ]
        );
    }

    #[test]
    fn test_tset_inline_allocator() {
        setup_test_globals();

        let mut set: TSet<i32, DefaultKeyFuncs<i32>, TInlineSetAllocator<4>> = TSet::new();

        set.insert(10);
        set.insert(20);
        set.insert(30);

        assert_eq!(set.len(), 3);
        assert!(set.contains(&10));
        assert!(set.contains(&20));
        assert!(set.contains(&30));
        assert!(!set.contains(&40));

        assert!(set.remove_by_key(&20));
        assert_eq!(set.len(), 2);
        assert!(!set.contains(&20));
    }

    #[test]
    fn test_tset_inline_allocator_stress() {
        setup_test_globals();

        let mut set: TSet<u32, DefaultKeyFuncs<u32>, TInlineSetAllocator<1>> = TSet::new();

        // Many elements to exceed inline capacity
        for i in 0..100 {
            set.insert(i);
        }

        assert_eq!(set.len(), 100);

        for i in 0..100 {
            assert!(set.contains(&i), "Missing element {}", i);
        }

        // Remove half
        for i in (0..100).step_by(2) {
            assert!(set.remove_by_key(&i));
        }
        assert_eq!(set.len(), 50);

        for i in 0..100 {
            if i % 2 == 0 {
                assert!(!set.contains(&i));
            } else {
                assert!(set.contains(&i));
            }
        }
    }

    #[test]
    fn test_tpair_hashing() {
        setup_test_globals();

        let pair1 = TPair::new(1u32, 100u32);
        let pair2 = TPair::new(1u32, 200u32);
        let pair3 = TPair::new(2u32, 100u32);

        // Same key, different value should have different hash
        assert_ne!(pair1.get_type_hash(), pair2.get_type_hash());

        // Different key, same value should have different hash
        assert_ne!(pair1.get_type_hash(), pair3.get_type_hash());

        // Same pair should have same hash
        let pair1_copy = TPair::new(1u32, 100u32);
        assert_eq!(pair1.get_type_hash(), pair1_copy.get_type_hash());
    }
}
