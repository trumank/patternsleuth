use std::ffi::c_void;
use std::sync::atomic::AtomicU64;

use super::*;

/// Hash bucket that conserves memory by storing up to 2 elements inline
/// before spilling to a heap-allocated TSet.
///
/// Encoding of `elements_or_set_ptr`:
/// - Both null: empty bucket
/// - `[0]` non-null, `[1]` null: 1 element (`[0]` is `*mut UObjectBase`)
/// - `[0]` non-null, `[1]` non-null: 2 elements (both are `*mut UObjectBase`)
/// - `[0]` null, `[1]` non-null: `[1]` is `*mut TSet<*mut UObjectBase>` (overflow)
///
/// This is the shipping build layout (no `ReadOnlyLock` field).
#[repr(C)]
pub struct FHashBucket {
    pub elements_or_set_ptr: [*mut c_void; 2],
}

impl Clone for FHashBucket {
    fn clone(&self) -> Self {
        Self {
            elements_or_set_ptr: self.elements_or_set_ptr,
        }
    }
}

impl FHashBucket {
    pub fn is_empty(&self) -> bool {
        self.elements_or_set_ptr[0].is_null() && self.elements_or_set_ptr[1].is_null()
    }

    fn get_set(&self) -> Option<&TSet<*mut UObjectBase>> {
        if self.elements_or_set_ptr[0].is_null() && !self.elements_or_set_ptr[1].is_null() {
            Some(unsafe { &*(self.elements_or_set_ptr[1] as *const TSet<*mut UObjectBase>) })
        } else {
            None
        }
    }

    pub fn num(&self) -> usize {
        if let Some(set) = self.get_set() {
            set.len()
        } else {
            !self.elements_or_set_ptr[0].is_null() as usize
                + !self.elements_or_set_ptr[1].is_null() as usize
        }
    }

    pub fn contains(&self, object: *mut UObjectBase) -> bool {
        if let Some(set) = self.get_set() {
            set.contains(&object)
        } else {
            self.elements_or_set_ptr[0] as *mut UObjectBase == object
                || self.elements_or_set_ptr[1] as *mut UObjectBase == object
        }
    }

    pub fn iter(&self) -> FHashBucketIter<'_> {
        if let Some(set) = self.get_set() {
            FHashBucketIter::Set(set.iter())
        } else {
            FHashBucketIter::Inline {
                bucket: self,
                index: 0,
            }
        }
    }
}

pub enum FHashBucketIter<'a> {
    Inline {
        bucket: &'a FHashBucket,
        index: usize,
    },
    Set(
        SetIterator<
            'a,
            *mut UObjectBase,
            <<FDefaultSetAllocator as SetAllocator>::SparseArrayAllocator as SparseArrayAllocator>::BitArrayAllocator,
        >,
    ),
}

impl<'a> Iterator for FHashBucketIter<'a> {
    type Item = *mut UObjectBase;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            FHashBucketIter::Inline { bucket, index } => {
                while *index < 2 {
                    let ptr = bucket.elements_or_set_ptr[*index] as *mut UObjectBase;
                    *index += 1;
                    if !ptr.is_null() {
                        return Some(ptr);
                    }
                }
                None
            }
            FHashBucketIter::Set(iter) => iter.next().copied(),
        }
    }
}

/// The global UObject hash tables singleton.
///
/// Provides multiple hash indices for fast object lookups by name, outer, class, and package.
#[repr(C)]
pub struct FUObjectHashTables {
    pub critical_section: FWindowsCriticalSection,
    pub hash: TMap<i32, FHashBucket>,
    pub hash_outer: TMultiMap<i32, *mut UObjectBase>,
    pub object_outer_map: TMap<*mut UObjectBase, FHashBucket>,
    pub class_to_object_list_map: TMap<*mut UClass, FHashBucket>,
    pub class_to_child_list_map: TMap<*mut UClass, TSet<*mut UClass>>,
    pub class_to_child_list_map_version: AtomicU64,
    pub package_to_object_list_map: TMap<*mut UPackage, FHashBucket>,
    pub object_to_package_map: TMap<*mut UObjectBase, *mut UPackage>,
}

/// Computes the hash for a name-only object lookup.
pub fn get_object_hash(name: FName) -> i32 {
    name.get_type_hash() as i32
}

/// Computes the hash for a (name, outer) object lookup.
pub fn get_object_outer_hash(name: FName, outer: usize) -> i32 {
    (name.get_type_hash() as usize).wrapping_add(outer >> 6) as i32
}

impl FUObjectHashTables {
    /// Find an object by its leaf FName and outer pointer.
    /// This is the fast path using `HashOuter` when the outer is known.
    ///
    /// `HashOuter`'s value encoding is engine-version dependent (see
    /// [`HashTableFormat`]): UE4 stores the object pointer directly, UE5 stores
    /// the object's internal index which we resolve through `GUObjectArray`.
    pub fn find_object(&self, name: FName, outer: *const UObject) -> Option<&UObjectBase> {
        let hash = get_object_outer_hash(name, outer as usize);
        let format = globals().layout.hash_table;
        let matches =
            |obj: &UObjectBase| obj.name_private() == name && obj.outer_private() == outer;
        match format {
            HashTableFormat::ObjectPointer => {
                let candidates = self.hash_outer.find_all(&hash);
                for &obj_ptr in candidates {
                    let obj = unsafe { &*obj_ptr };
                    if matches(obj) {
                        return Some(obj);
                    }
                }
            }
            HashTableFormat::InternalIndex => {
                // Same field, but the values are `uint32` indices rather than
                // pointers, so reinterpret the map's element type accordingly.
                let hash_outer =
                    unsafe { &*(&self.hash_outer as *const _ as *const TMultiMap<i32, u32>) };
                let array = unsafe { &*globals().guobject_array };
                let candidates = hash_outer.find_all(&hash);
                for &index in candidates {
                    if let Some(obj) = array.index_to_object(index as i32)
                        && matches(obj)
                    {
                        return Some(obj);
                    }
                }
            }
        }
        None
    }

    /// Find an object by its leaf FName only (no outer constraint).
    /// Scans the `Hash` name-only table. If multiple objects share the same name,
    /// returns the first match.
    pub fn find_object_by_name(&self, name: FName) -> Option<&UObjectBase> {
        let hash = get_object_hash(name);
        let bucket = self.hash.get(&hash)?;
        for obj_ptr in bucket.iter() {
            let obj = unsafe { &*obj_ptr };
            if obj.name_private() == name {
                return Some(obj);
            }
        }
        None
    }

    /// Collect all live instances of `class`, optionally including instances
    /// of its subclasses via `class_to_child_list_map`. This is the same
    /// index UE uses internally for `TObjectIterator<T>`-style lookups — far
    /// faster than walking `GUObjectArray` for a single class.
    pub fn objects_of_class(
        &self,
        class: *const UClass,
        include_subclasses: bool,
    ) -> Vec<&UObjectBase> {
        let class_mut = class as *mut UClass;
        let mut out = Vec::new();

        // BFS over the class → subclasses map.
        let mut frontier: Vec<*mut UClass> = vec![class_mut];
        let mut idx = 0;
        while idx < frontier.len() {
            let c = frontier[idx];
            idx += 1;

            if let Some(bucket) = self.class_to_object_list_map.get(&c) {
                for obj_ptr in bucket.iter() {
                    if let Some(obj) = unsafe { obj_ptr.as_ref() } {
                        out.push(obj);
                    }
                }
            }

            if include_subclasses {
                if let Some(children) = self.class_to_child_list_map.get(&c) {
                    for &child in children.iter() {
                        frontier.push(child);
                    }
                }
            }
        }

        out
    }

    /// Find an object by its full path string (e.g. `/Game/Maps/MyMap.MyMap:PersistentLevel`).
    ///
    /// Splits the path into segments, resolves each segment's FName, then walks the
    /// outer chain using hash lookups. Falls back to the name-only hash for the
    /// top-level package (null outer).
    pub fn find_object_by_path(&self, path: &str) -> Option<&UObjectBase> {
        // Split path on '.' and ':' into segments, preserving order
        let segments: Vec<&str> = path.split(&['.', ':'][..]).collect();
        if segments.is_empty() {
            return None;
        }

        // Resolve top-level package (null outer) via name-only hash
        let pkg_name = FName::find(segments[0]);
        let hash = get_object_hash(pkg_name);
        let bucket = self.hash.get(&hash)?;
        let mut current: Option<&UObjectBase> = None;
        for obj_ptr in bucket.iter() {
            let obj = unsafe { &*obj_ptr };
            if obj.name_private() == pkg_name && obj.outer_private().is_null() {
                current = Some(obj);
                break;
            }
        }
        let mut current = current?;

        // Walk remaining segments using HashOuter
        for &segment in &segments[1..] {
            let seg_name = FName::find(segment);
            let outer_ptr = current as *const UObjectBase as *const UObject;
            current = self.find_object(seg_name, outer_ptr)?;
        }

        Some(current)
    }
}
