use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use crate::ue;

type InternalIndex = i32;
type Objects = HashMap<InternalIndex, ObjectProxy>;

fn objects() -> &'static Mutex<Objects> {
    static OBJECTS: OnceLock<Mutex<Objects>> = OnceLock::new();
    OBJECTS.get_or_init(Default::default)
}

// call from main thread
pub fn object_created(object: &ue::UObjectBase) {
    let proxy = ObjectProxy {
        name: object.name_private.to_string(),
    };
    objects()
        .lock()
        .unwrap()
        .insert(object.internal_index, proxy);
}
// call from main thread
pub fn object_deleted(object: &ue::UObjectBase) {
    objects().lock().unwrap().remove(&object.internal_index);
}

#[derive(Debug)]
struct ObjectProxy {
    name: String,
}
