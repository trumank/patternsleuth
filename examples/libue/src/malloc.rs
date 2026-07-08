use super::*;

/// # Safety
/// [`init`] must have been called with a `gmalloc` pointing to a valid `FMalloc*`.
pub unsafe fn gmalloc() -> &'static FMalloc {
    unsafe { &**globals().gmalloc }
}

#[cfg(test)]
pub(crate) mod test {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout};

    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MiriAllocator {
        allocations: Mutex<HashMap<usize, Layout>>,
    }

    impl MiriAllocator {
        fn new() -> Self {
            Self {
                allocations: Mutex::new(HashMap::new()),
            }
        }

        unsafe fn malloc(&self, size: usize, alignment: u32) -> *mut c_void {
            let layout = Layout::from_size_align(size, alignment as usize).unwrap();
            let ptr = unsafe { std::alloc::System.alloc(layout) } as *mut c_void;

            if !ptr.is_null() {
                self.allocations
                    .lock()
                    .unwrap()
                    .insert(ptr as usize, layout);
            }

            ptr
        }

        unsafe fn realloc(
            &self,
            original: *mut c_void,
            size: usize,
            alignment: u32,
        ) -> *mut c_void {
            if original.is_null() {
                return unsafe { self.malloc(size, alignment) };
            }

            let new_layout = Layout::from_size_align(size, alignment as usize).unwrap();

            let old_layout = {
                let mut allocations = self.allocations.lock().unwrap();
                allocations
                    .remove(&(original as usize))
                    .expect("existing alloc not found")
            };

            let new_ptr =
                unsafe { std::alloc::System.realloc(original as *mut u8, old_layout, size) }
                    as *mut c_void;

            if !new_ptr.is_null() {
                self.allocations
                    .lock()
                    .unwrap()
                    .insert(new_ptr as usize, new_layout);
            }

            new_ptr
        }

        unsafe fn free(&self, ptr: *mut c_void) {
            if !ptr.is_null() {
                let layout = {
                    let mut allocations = self.allocations.lock().unwrap();
                    allocations.remove(&(ptr as usize))
                };

                if let Some(layout) = layout {
                    unsafe { std::alloc::System.dealloc(ptr as *mut u8, layout) };
                }
            }
        }
    }

    use std::sync::{LazyLock, Once};
    static MIRI_ALLOCATOR: LazyLock<MiriAllocator> = LazyLock::new(MiriAllocator::new);

    unsafe extern "system" fn miri_malloc(
        _this: &FMalloc,
        count: usize,
        alignment: u32,
    ) -> *mut c_void {
        unsafe { MIRI_ALLOCATOR.malloc(count, alignment) }
    }

    unsafe extern "system" fn miri_realloc(
        _this: &FMalloc,
        original: *mut c_void,
        count: usize,
        alignment: u32,
    ) -> *mut c_void {
        unsafe { MIRI_ALLOCATOR.realloc(original, count, alignment) }
    }

    unsafe extern "system" fn miri_free(_this: &FMalloc, original: *mut c_void) {
        unsafe { MIRI_ALLOCATOR.free(original) };
    }

    unsafe extern "system" fn stub() {}

    static MIRI_VTABLE: FMallocVTable = FMallocVTable {
        __vec_del_dtor: stub as *const (),
        exec: stub as *const (),
        malloc: miri_malloc,
        try_malloc: miri_malloc,
        realloc: miri_realloc,
        try_realloc: miri_realloc,
        free: miri_free,
        quantize_size: stub as *const (),
        get_allocation_size: stub as *const (),
        trim: stub as *const (),
        setup_tls_caches_on_current_thread: stub as *const (),
        clear_and_disable_tlscaches_on_current_thread: stub as *const (),
        initialize_stats_metadata: stub as *const (),
        update_stats: stub as *const (),
        get_allocator_stats: stub as *const (),
        dump_allocator_stats: stub as *const (),
        is_internally_thread_safe: stub as *const (),
        validate_heap: stub as *const (),
        get_descriptive_name: stub as *const (),
    };

    static MIRI_MALLOC: &&FMalloc = &&FMalloc {
        vtable: &MIRI_VTABLE,
    };

    static INIT: Once = Once::new();

    unsafe extern "system" fn stub_fname_to_string(_: &FName, _: &mut FString) {
        unreachable!("stub fname_to_string called in test")
    }
    unsafe extern "system" fn stub_fname_ctor_wchar(_: *mut FName, _: *const u16, _: i32) {
        unreachable!("stub fname_ctor_wchar called in test")
    }
    extern "system" fn stub_fuobject_hash_tables_get() -> &'static FUObjectHashTables {
        unreachable!("stub fuobject_hash_tables_get called in test")
    }
    unsafe extern "system" fn stub_static_load_object(
        _: *const UClass,
        _: *const UObject,
        _: *const u16,
        _: *const u16,
        _: u32,
        _: *const c_void,
        _: bool,
        _: *const c_void,
    ) -> *const UObjectBase {
        unreachable!("stub static_load_object called in test")
    }

    unsafe extern "system" fn stub_static_construct_object(_: *const c_void) -> *const UObjectBase {
        unreachable!("stub static_construct_object called in test")
    }

    pub fn setup_test_globals() {
        INIT.call_once(|| unsafe {
            init(UeGlobals {
                // god raw pointers in rust suck
                gmalloc: (MIRI_MALLOC as *const &'static _) as *const *const _,
                fname_to_string: stub_fname_to_string,
                fname_ctor_wchar: stub_fname_ctor_wchar,
                guobject_array: std::ptr::null(),
                fuobject_hash_tables_get: stub_fuobject_hash_tables_get,
                static_load_object: stub_static_load_object,
                static_construct_object: stub_static_construct_object,
                gnatives: std::ptr::null_mut(),
                layout: UeLayout::UE_4_27,
            });
        });
    }
}
