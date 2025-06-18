use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use crate::ue;

/// Opaque pointer type representing UObject*
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UObjectPtr(pub *mut ue::UObjectBase);
unsafe impl Send for UObjectPtr {}
unsafe impl Sync for UObjectPtr {}

/// Unique identifier for object handles
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectId(u64);

/// Events that mods can process during engine tick
#[derive(Debug, Clone)]
pub enum ObjectEvent {
    Created { id: ObjectId },
    Deleted { id: ObjectId },
}

/// Internal object state
#[derive(Debug)]
struct ObjectState {
    ptr: UObjectPtr,
    valid: bool,
}

/// Thread-safe registry for managing object lifecycle
/// This is a singleton that's always available
pub struct ObjectRegistry {
    /// Map from ObjectId to object state
    objects: RwLock<HashMap<ObjectId, ObjectState>>,
    /// Map from raw pointer to ObjectId for fast lookup
    ptr_to_id: RwLock<HashMap<UObjectPtr, ObjectId>>,
    /// Queue of events to be processed in main thread
    pending_events: Mutex<VecDeque<ObjectEvent>>,
    /// Counter for generating unique IDs
    next_id: AtomicU64,
}

// Global singleton instance

impl ObjectRegistry {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            objects: RwLock::new(HashMap::new()),
            ptr_to_id: RwLock::new(HashMap::new()),
            pending_events: Mutex::new(VecDeque::new()),
            next_id: AtomicU64::new(1),
        })
    }

    /// Get the global registry instance
    pub fn instance() -> &'static Arc<ObjectRegistry> {
        static REGISTRY: LazyLock<Arc<ObjectRegistry>> = LazyLock::new(ObjectRegistry::new);
        &REGISTRY
    }

    /// Called from ObjectCreated hook (arbitrary thread)
    pub fn on_object_created(object_ptr: UObjectPtr) {
        let reg = Self::instance();

        let id = ObjectId(reg.next_id.fetch_add(1, Ordering::SeqCst));

        // Update internal state
        {
            let mut objects = reg.objects.write().unwrap();
            objects.insert(
                id,
                ObjectState {
                    ptr: object_ptr,
                    valid: true,
                },
            );
        }

        {
            let mut ptr_to_id = reg.ptr_to_id.write().unwrap();
            ptr_to_id.insert(object_ptr, id);
        }

        // Queue event for main thread processing
        {
            let mut events = reg.pending_events.lock().unwrap();
            events.push_back(ObjectEvent::Created { id });
        }
    }

    /// Called from ObjectDeleted hook (arbitrary thread)
    pub fn on_object_deleted(object_ptr: UObjectPtr) {
        let reg = Self::instance();

        let id = {
            let ptr_to_id = reg.ptr_to_id.read().unwrap();
            match ptr_to_id.get(&object_ptr) {
                Some(&id) => id,
                None => {
                    // Object wasn't tracked, ignore
                    return;
                }
            }
        };

        // Mark object as invalid
        {
            let mut objects = reg.objects.write().unwrap();
            if let Some(state) = objects.get_mut(&id) {
                state.valid = false;
            }
        }

        // Remove from pointer mapping
        {
            let mut ptr_to_id = reg.ptr_to_id.write().unwrap();
            ptr_to_id.remove(&object_ptr);
        }

        // Queue event for main thread processing
        {
            let mut events = reg.pending_events.lock().unwrap();
            events.push_back(ObjectEvent::Deleted { id });
        }
    }

    /// Called from EngineTick (main thread only)
    /// Returns all pending events and clears the queue
    pub unsafe fn drain_events() -> Vec<ObjectEvent> {
        let reg = Self::instance();

        static mut CLEANUP_COUNTER: u32 = 0;
        unsafe {
            CLEANUP_COUNTER += 1;
            if CLEANUP_COUNTER % 60 == 0 {
                // Cleanup every 60 ticks
                let mut objects = reg.objects.write().unwrap();
                objects.retain(|_, state| state.valid);
            }
        }

        let mut events = reg.pending_events.lock().unwrap();
        events.drain(..).collect()
    }

    /// Get the raw pointer for a handle (only safe to call from main thread during tick)
    pub fn get_object_ptr(id: &ObjectId) -> Option<UObjectPtr> {
        let reg = Self::instance();
        let objects = reg.objects.read().unwrap();
        objects
            .get(&id)
            .filter(|state| state.valid)
            .map(|state| state.ptr)
    }

    /// Check if a handle is still valid
    pub fn is_handle_valid(id: &ObjectId) -> bool {
        let reg = Self::instance();
        let objects = reg.objects.read().unwrap();
        objects.get(&id).map(|state| state.valid).unwrap_or(false)
    }
}

impl ObjectId {
    /// Check if this handle is still valid
    pub fn is_valid(&self) -> bool {
        ObjectRegistry::is_handle_valid(self)
    }

    /// Get the raw UObject pointer (only safe during main thread tick)
    /// Returns None if object was deleted
    pub fn get_ptr(&self) -> Option<UObjectPtr> {
        ObjectRegistry::get_object_ptr(self)
    }
}
