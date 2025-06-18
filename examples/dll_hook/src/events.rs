use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

trait Event: 'static {}
impl<T> Event for T where T: 'static {}

trait CallbackTrait: Send + Sync {
    fn call(&self, event: &dyn Any);
}

struct Callback<T: Event> {
    func: Box<dyn Fn(&T) + Send + Sync>,
}

impl<T: Event> CallbackTrait for Callback<T> {
    fn call(&self, event: &dyn Any) {
        if let Some(event) = event.downcast_ref::<T>() {
            (self.func)(event);
        }
    }
}

pub struct EventDispatcher {
    callbacks: RwLock<HashMap<TypeId, Vec<Arc<dyn CallbackTrait>>>>,
}

impl EventDispatcher {
    pub fn new() -> Self {
        Self {
            callbacks: RwLock::new(HashMap::new()),
        }
    }

    pub fn register<T: Event, F>(&self, callback: F)
    where
        F: Fn(&T) + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        let callback = Arc::new(Callback {
            func: Box::new(callback),
        });

        let mut callbacks = self.callbacks.write().unwrap();
        callbacks.entry(type_id).or_default().push(callback);
    }

    pub fn fire<T: Event>(&self, event: T) {
        let type_id = TypeId::of::<T>();
        let callbacks = self.callbacks.read().unwrap();

        if let Some(event_callbacks) = callbacks.get(&type_id) {
            for callback in event_callbacks {
                callback.call(&event);
            }
        }
    }

    pub fn clear<T: Event>(&self) {
        let type_id = TypeId::of::<T>();
        let mut callbacks = self.callbacks.write().unwrap();
        callbacks.remove(&type_id);
    }

    pub fn clear_all(&self) {
        let mut callbacks = self.callbacks.write().unwrap();
        callbacks.clear();
    }
}

static GLOBAL_DISPATCHER: std::sync::LazyLock<EventDispatcher> =
    std::sync::LazyLock::new(EventDispatcher::new);

pub fn register<T: Event, F>(callback: F)
where
    F: Fn(&T) + Send + Sync + 'static,
{
    GLOBAL_DISPATCHER.register(callback);
}

pub fn fire<T: Event>(event: T) {
    GLOBAL_DISPATCHER.fire(event);
}

pub fn clear<T: Event>() {
    GLOBAL_DISPATCHER.clear::<T>();
}

pub fn clear_all() {
    GLOBAL_DISPATCHER.clear_all();
}
