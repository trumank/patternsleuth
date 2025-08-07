use super::*;

static mut GMALLOC: *const *const FMalloc = std::ptr::null();

pub unsafe fn gmalloc() -> &'static FMalloc {
    unsafe { &**GMALLOC }
}

pub unsafe fn init_gmalloc(malloc: *const *const FMalloc) {
    GMALLOC = malloc;
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
            let ptr = std::alloc::System.alloc(layout) as *mut c_void;

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
                return self.malloc(size, alignment);
            }

            let new_layout = Layout::from_size_align(size, alignment as usize).unwrap();

            let old_layout = {
                let mut allocations = self.allocations.lock().unwrap();
                allocations
                    .remove(&(original as usize))
                    .expect("existing alloc not found")
            };

            let new_ptr =
                std::alloc::System.realloc(original as *mut u8, old_layout, size) as *mut c_void;

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
                    std::alloc::System.dealloc(ptr as *mut u8, layout);
                }
            }
        }
    }

    use std::sync::{LazyLock, Once};
    static MIRI_ALLOCATOR: LazyLock<MiriAllocator> = LazyLock::new(|| MiriAllocator::new());

    unsafe extern "system" fn miri_malloc(
        _this: &FMalloc,
        count: usize,
        alignment: u32,
    ) -> *mut c_void {
        MIRI_ALLOCATOR.malloc(count, alignment)
    }

    unsafe extern "system" fn miri_realloc(
        _this: &FMalloc,
        original: *mut c_void,
        count: usize,
        alignment: u32,
    ) -> *mut c_void {
        MIRI_ALLOCATOR.realloc(original, count, alignment)
    }

    unsafe extern "system" fn miri_free(_this: &FMalloc, original: *mut c_void) {
        MIRI_ALLOCATOR.free(original);
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

    static MIRI_MALLOC: &'static &'static FMalloc = &&FMalloc {
        vtable: &MIRI_VTABLE,
    };

    static INIT: Once = Once::new();

    pub fn setup_test_globals() {
        INIT.call_once(|| unsafe {
            // god raw pointers in rust suck
            init_gmalloc((MIRI_MALLOC as *const &'static _) as *const *const _);
        });
    }
}
