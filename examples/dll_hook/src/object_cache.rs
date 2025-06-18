//! This provides a thread-safe API for Unreal Engine that allows hooking into
//! object creation and deletion events. The API is designed to work with Unreal Engine's multi-threaded
//! architecture where objects can be created and destroyed on arbitrary threads.
//!
//! ## Architecture Overview
//!
//! The API is built around a singleton [`ObjectRegistry`] that tracks all UObject instances and provides
//! thread-safe handles ([`ObjectId`]) that mods can use to reference objects safely across engine ticks.
//!
//! ## Threading Model & Constraints
//!
//! ### Thread Safety Constraints
//! - **Object Creation/Deletion**: Can occur on any thread at any time
//! - **Engine Tick**: Always runs on the main thread
//! - **UObject Access**: Raw UObject pointers are only safe to dereference on the main thread during tick
//! - **Handle Usage**: [`ObjectId`]s can be created/stored on any thread but should only be dereferenced during tick
//!
//! ### Safety Guarantees
//! - Handles automatically become invalid when their associated object is deleted
//! - Raw pointer access returns `None` for deleted objects, preventing use-after-free
//!
//! ## Integration Points
//!
//! The API provides three main integration points that should be called from Unreal Engine C++ code:
//!
//! ### C++ Hook Points
//! ```cpp
//! // Call from UObject creation hook (any thread)
//! ObjectRegistry::on_object_created(UObjectPtr);
//!
//! // Call from UObject deletion hook (any thread)
//! ObjectRegistry::on_object_deleted(UObjectPtr);
//!
//! // Call from engine tick (main thread only)
//! ObjectRegistry::drain_events() -> Vec<ObjectEvent>;
//! ```
//!
//! ## Handle Lifecycle & Tick-Scoped Access
//!
//! ```rust
//! // Handles can be stored across ticks
//! let my_handle: ObjectId = /* from creation event */;
//!
//! // During engine tick, create a tick context for cheap reference access
//! let events = unsafe { ObjectRegistry::drain_events() };
//! ObjectRegistry::with_tick_context(|ctx| {
//!     // Process events
//!     for event in events {
//!         match event {
//!             ObjectEvent::Created { id } => {
//!                 // Get cheap reference access during tick
//!                 if let Some(obj_ref) = ctx.get_ref(&id) {
//!                     // Use obj_ref safely - it's a &ue::UObjectBase
//!                 }
//!             }
//!             _ => {}
//!         }
//!     }
//!     
//!     // Later, during the same tick:
//!     if my_handle.is_valid() {
//!         if let Some(obj_ref) = ctx.get_ref(&my_handle) {
//!             // Safe to use obj_ref here
//!         }
//!         if let Some(obj_mut) = ctx.get_mut(&my_handle) {
//!             // Safe to mutate object here
//!         }
//!     }
//! });
//! ```
//!
//! ## Performance Considerations
//!
//! - **Lock Contention**: The registry uses RwLocks to minimize contention between readers
//! - **Event Batching**: Object events are batched and processed once per tick
//! - **Memory Management**: Deleted objects are periodically cleaned up to prevent memory leaks
//! - **Handle Storage**: Handles are lightweight (8 bytes) and cheap to copy
//! - **Tick References**: Reference access during tick is very cheap (no Send/Sync overhead)
//!
//! ## Safety Notes
//!
//! This API makes several important safety assumptions:
//!
//! 1. **UObject Lifetime**: UObjects remain valid throughout the entire duration of an engine tick
//! 2. **Thread Model**: Only the main thread processes ticks and accesses raw pointers
//! 3. **Hook Reliability**: The C++ integration correctly calls creation/deletion hooks for all objects
//! 4. **No Reentrancy**: Engine tick callbacks should not trigger object creation/deletion recursively
//! 5. **Tick Context**: [`TickContext`] is only used during engine tick on the main thread
//!
//! Violating these assumptions may lead to undefined behavior.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use crate::ue;

/// Thread-safe wrapper for UObject pointers
///
/// Safety: Raw pointers are only dereferenced on the main thread during engine tick.
/// Cross-thread storage and passing is safe because we only store the pointer value,
/// never dereference it until we're back on the main thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UObjectPtr(*mut ue::UObjectBase);
unsafe impl Send for UObjectPtr {}
unsafe impl Sync for UObjectPtr {}

impl UObjectPtr {
    /// Convert to raw pointer (only safe during engine tick on main thread)
    pub unsafe fn as_raw(self) -> *mut ue::UObjectBase {
        self.0
    }
}

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

/// Provides cheap access to UObject references during engine tick.
///
/// This context is only valid during a single engine tick and provides
/// very cheap access to object references without Send/Sync overhead.
///
/// # Safety
/// - Only use during engine tick on the main thread
/// - References returned are only valid for the duration of the tick
/// - Do not store references beyond the tick context
pub struct TickContext {
    borrow_states: RefCell<HashMap<ObjectId, BorrowState>>,
}

/// Thread-local borrow state for each object
#[derive(Debug, Default)]
struct BorrowState {
    /// Number of immutable borrows currently active
    immutable_borrows: usize,
    /// Whether a mutable borrow is currently active
    mutable_borrow: bool,
}

/// An immutable reference to a UObject that tracks borrowing
pub struct ObjectRef<'a> {
    object_ref: &'a ue::UObjectBase,
    id: ObjectId,
    context: &'a TickContext, // Ensures proper lifetime
}

/// A mutable reference to a UObject that tracks borrowing
pub struct ObjectRefMut<'a> {
    object_ref: &'a mut ue::UObjectBase,
    id: ObjectId,
    context: &'a TickContext, // Ensures proper lifetime
}

impl<'a> std::ops::Deref for ObjectRef<'a> {
    type Target = ue::UObjectBase;

    fn deref(&self) -> &Self::Target {
        self.object_ref
    }
}

impl<'a> std::ops::Deref for ObjectRefMut<'a> {
    type Target = ue::UObjectBase;

    fn deref(&self) -> &Self::Target {
        self.object_ref
    }
}

impl<'a> std::ops::DerefMut for ObjectRefMut<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.object_ref
    }
}

impl<'a> Drop for ObjectRef<'a> {
    fn drop(&mut self) {
        let mut states = self.context.borrow_states.borrow_mut();
        let state = states.get_mut(&self.id).unwrap();
        debug_assert!(
            state.immutable_borrows > 0,
            "Immutable borrow count underflow"
        );
        state.immutable_borrows -= 1;
    }
}

impl<'a> Drop for ObjectRefMut<'a> {
    fn drop(&mut self) {
        let mut states = self.context.borrow_states.borrow_mut();
        let state = states.get_mut(&self.id).unwrap();
        debug_assert!(state.mutable_borrow, "Mutable borrow was not active");
        state.mutable_borrow = false;
    }
}

impl TickContext {
    /// Must only be created on main thread during engine tick and only one may exist at a time
    pub unsafe fn new() -> Self {
        Self {
            borrow_states: Default::default(),
        }
    }

    /// Get an immutable reference to a UObject
    ///
    /// Returns None if the object is invalid or deleted.
    /// The returned reference is only valid for the duration of this tick.
    pub fn get_ref<'a>(&'a self, id: ObjectId) -> Option<ObjectRef<'a>> {
        // Get the object pointer
        let ptr = ObjectRegistry::get_object_ptr(id)?;

        // Get or create borrow state
        let mut borrow_states = self.borrow_states.borrow_mut();
        let borrow_state = borrow_states.entry(id).or_default();

        // Check if we can create an immutable borrow
        if borrow_state.mutable_borrow {
            // Cannot borrow immutably while mutably borrowed
            return None;
        }

        // Increment immutable borrow count
        borrow_state.immutable_borrows += 1;

        // Create the reference
        // Safety: We're on the main thread during tick, and the object is guaranteed
        // to remain valid for the duration of the tick
        let object_ref = unsafe { &*ptr.as_raw() };

        Some(ObjectRef {
            object_ref,
            id,
            context: self,
        })
    }

    /// Get a mutable reference to a UObject
    ///
    /// Returns None if the object is invalid, deleted, or already borrowed mutably.
    /// The returned reference is only valid for the duration of this tick.
    ///
    /// # Panics
    /// Panics if the object is already borrowed mutably (similar to RefCell behavior)
    pub fn get_mut<'a>(&'a self, id: ObjectId) -> Option<ObjectRefMut<'a>> {
        // Get the object pointer
        let ptr = ObjectRegistry::get_object_ptr(id)?;

        // Get or create borrow state
        let mut borrow_states = self.borrow_states.borrow_mut();
        let borrow_state = borrow_states.entry(id).or_default();

        // Check if we can create a mutable borrow
        if borrow_state.mutable_borrow || borrow_state.immutable_borrows > 0 {
            // Cannot borrow mutably while already borrowed
            return None;
        }

        // Set mutable borrow flag
        borrow_state.mutable_borrow = true;

        // Create the reference
        // Safety: Same reasoning as above
        let object_ref = unsafe { &mut *ptr.as_raw() };

        Some(ObjectRefMut {
            object_ref,
            id,
            context: self,
        })
    }
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
    pub fn on_object_created(object_ptr: *mut ue::UObjectBase) {
        let reg = Self::instance();
        let ptr = UObjectPtr(object_ptr);

        let id = ObjectId(reg.next_id.fetch_add(1, Ordering::SeqCst));

        // Update internal state
        {
            let mut objects = reg.objects.write().unwrap();
            objects.insert(id, ObjectState { ptr, valid: true });
        }

        {
            let mut ptr_to_id = reg.ptr_to_id.write().unwrap();
            ptr_to_id.insert(ptr, id);
        }

        // Queue event for main thread processing
        {
            let mut events = reg.pending_events.lock().unwrap();
            events.push_back(ObjectEvent::Created { id });
        }
    }

    /// Called from ObjectDeleted hook (arbitrary thread)
    pub fn on_object_deleted(object_ptr: *mut ue::UObjectBase) {
        let reg = Self::instance();
        let ptr = UObjectPtr(object_ptr);

        let id = {
            let ptr_to_id = reg.ptr_to_id.read().unwrap();
            match ptr_to_id.get(&ptr) {
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
            ptr_to_id.remove(&ptr);
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
    pub fn get_object_ptr(id: ObjectId) -> Option<UObjectPtr> {
        let reg = Self::instance();
        let objects = reg.objects.read().unwrap();
        objects
            .get(&id)
            .filter(|state| state.valid)
            .map(|state| state.ptr)
    }

    /// Check if a handle is still valid
    pub fn is_handle_valid(id: ObjectId) -> bool {
        let reg = Self::instance();
        let objects = reg.objects.read().unwrap();
        objects.get(&id).map(|state| state.valid).unwrap_or(false)
    }
}

impl ObjectId {
    /// Check if this handle is still valid
    pub fn is_valid(self) -> bool {
        ObjectRegistry::is_handle_valid(self)
    }

    /// Get the raw UObject pointer (only safe during main thread tick)
    /// Returns None if object was deleted
    pub fn get_ptr(self) -> Option<UObjectPtr> {
        ObjectRegistry::get_object_ptr(self)
    }
}
