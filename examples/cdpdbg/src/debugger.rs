use std::ffi::c_void;
use std::sync::Arc;

use libue as ue;
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::scripts::Script;

/// Custom debug break opcode — unused GNatives slot
pub const EX_DEBUG_BREAK: u8 = 0xFE;

// ── Channel message types ───────────────────────────────────────────────────

/// Game thread → CDP
pub enum DbgEvent {
    Paused {
        script_idx: usize,
        line: usize,
        frame: *const ue::kismet::FFrame,
    },
    ScriptParsed {
        script_idx: usize,
    },
}
unsafe impl Send for DbgEvent {}

/// CDP → Game thread
pub enum GameRequest {
    Resume(ResumeCmd),
    ReadProperties {
        object_id: String,
        resp: tokio::sync::oneshot::Sender<Value>,
    },
}

#[derive(Clone, Copy)]
pub enum ResumeCmd {
    Continue,
    StepOver,
    StepInto,
    StepOut,
}

// ── Breakpoint / step state ─────────────────────────────────────────────────

pub struct BcBreakpoint {
    pub id: String,
    pub script_idx: usize,
    pub func_idx: usize,
    pub line: usize,
    pub offset: usize,
    pub original_opcode: u8,
}

#[derive(Clone)]
pub struct StepState {
    pub mode: StepMode,
    pub origin_node: ue::TWeakObjectPtr<ue::UFunction>,
    pub origin_depth: usize,
    pub origin_line: Option<usize>,
    /// Saved original GNatives entries (before we overwrote them for stepping)
    pub saved_gnatives: Vec<ue::FNativesFuncPtr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepMode {
    Over,
    Into,
    Out,
}

// ── Shared state ────────────────────────────────────────────────────────────

pub struct SharedState {
    pub scripts: Mutex<Vec<Script>>,
    pub breakpoints: Mutex<Vec<BcBreakpoint>>,
    pub step_state: Mutex<Option<StepState>>,
    pub gnatives: *mut ue::GNatives,
    pub bp_counter: Mutex<u64>,

    /// UClasses whose scripts need to be (re)built on the next engine tick.
    /// Populated from the post-load hook (any thread), drained from tick
    /// (game thread) so we can call `StaticLoadObject` safely.
    pub pending_class_builds: Mutex<Vec<ue::TWeakObjectPtr<ue::UClass>>>,

    // Channels: game thread side
    pub event_tx: tokio::sync::mpsc::Sender<DbgEvent>,
    pub request_rx: crossbeam_channel::Receiver<GameRequest>,
}
unsafe impl Send for SharedState {}
unsafe impl Sync for SharedState {}

impl SharedState {
    fn gnatives(&self) -> &ue::GNatives {
        unsafe { &*self.gnatives }
    }
    #[allow(clippy::mut_from_ref)]
    fn gnatives_mut(&self) -> &mut ue::GNatives {
        unsafe { &mut *self.gnatives }
    }
}

static mut SHARED: Option<Arc<SharedState>> = None;

/// Queue a UClass to have its script (re)built on the next engine tick.
/// Callable from any thread — the actual build runs on the game thread
/// because `StaticLoadObject` requires it. Dedupes against entries already
/// in the queue.
pub fn queue_class_build(class_weak: ue::TWeakObjectPtr<ue::UClass>) {
    let state = shared();
    let mut queue = state.pending_class_builds.lock();
    if !queue.contains(&class_weak) {
        queue.push(class_weak);
    }
}

/// Drain the pending-class-builds queue and (re)build any class that has
/// finished post-loading. Must be called from the game thread.
pub fn process_pending_class_builds() {
    let state = shared();
    let pending: Vec<ue::TWeakObjectPtr<ue::UClass>> =
        std::mem::take(&mut *state.pending_class_builds.lock());
    for weak in pending {
        let Some(class) = weak.get() else { continue };
        let class_obj: &ue::UObjectBase = class;
        if let Some(script) = crate::scripts::try_build_class_script(class_obj) {
            tracing::debug!("Tick: class script {}", script.class_path);
            add_script(script);
        }
    }
}

/// Upsert a class-level script into the table. If a script with the same
/// `class_path` already exists, replace it in place and leave the index stable
/// (no new `ScriptParsed` event — DevTools won't refetch anyway). Otherwise
/// append and emit `ScriptParsed` for the new entry.
pub fn add_script(script: Script) {
    let state = shared();
    let mut scripts = state.scripts.lock();
    if let Some(existing) = scripts
        .iter_mut()
        .find(|s| s.class_path == script.class_path)
    {
        *existing = script;
        return;
    }
    let idx = scripts.len();
    scripts.push(script);
    drop(scripts);
    let _ = state
        .event_tx
        .try_send(DbgEvent::ScriptParsed { script_idx: idx });
}

pub fn shared() -> &'static Arc<SharedState> {
    #[allow(static_mut_refs)]
    unsafe {
        SHARED.as_ref().expect("debugger not initialized")
    }
}

// ── Init ────────────────────────────────────────────────────────────────────

pub struct DebuggerChannels {
    pub event_rx: tokio::sync::mpsc::Receiver<DbgEvent>,
    pub request_tx: crossbeam_channel::Sender<GameRequest>,
}

pub unsafe fn init(scripts: Vec<Script>, gnatives: &'static mut ue::GNatives) -> DebuggerChannels {
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(256);
    let (request_tx, request_rx) = crossbeam_channel::bounded(1);

    let gnatives_ptr = gnatives as *mut ue::GNatives;

    // Install our debug break handler at GNatives[0xFE]
    unsafe {
        (*gnatives_ptr)[EX_DEBUG_BREAK as usize] = Some(debug_break_handler);
    }
    tracing::info!(
        "Installed debug break handler at GNatives[0x{:02X}]",
        EX_DEBUG_BREAK
    );

    unsafe {
        SHARED = Some(Arc::new(SharedState {
            scripts: Mutex::new(scripts),
            breakpoints: Mutex::new(Vec::new()),
            step_state: Mutex::new(None),
            gnatives: gnatives_ptr,
            bp_counter: Mutex::new(0),
            pending_class_builds: Mutex::new(Vec::new()),
            event_tx,
            request_rx,
        }));
    }

    DebuggerChannels {
        event_rx,
        request_tx,
    }
}

// ── Breakpoint management (called from CDP thread) ──────────────────────────

pub fn set_breakpoint(state: &SharedState, script_idx: usize, line: usize, col: usize) -> String {
    let scripts = state.scripts.lock();
    let script = &scripts[script_idx];
    let Some((func_idx, offset)) = script.location_to_offset(line, col) else {
        tracing::warn!(
            "setBreakpoint: no offset for {} line {line}",
            script.class_path
        );
        return String::new();
    };
    let offset = offset as usize;
    let bytecode = script.function_bytes(func_idx);
    if bytecode.is_empty() {
        tracing::warn!("setBreakpoint: function bytes unavailable (stale UFunction)");
        return String::new();
    }
    let original_opcode = bytecode[offset];

    // Atomic single-byte write on x86
    unsafe {
        (bytecode.as_ptr().add(offset) as *mut u8).write(EX_DEBUG_BREAK);
    }

    let mut counter = state.bp_counter.lock();
    *counter += 1;
    let id = format!("bp-{}", *counter);

    tracing::info!(
        "Set breakpoint {id}: {}::{} +0x{offset:x} (line {line})",
        script.class_path,
        script.functions[func_idx].short_name,
    );

    state.breakpoints.lock().push(BcBreakpoint {
        id: id.clone(),
        script_idx,
        func_idx,
        line,
        offset,
        original_opcode,
    });

    id
}

pub fn remove_breakpoint(state: &SharedState, bp_id: &str) {
    let mut breakpoints = state.breakpoints.lock();
    if let Some(pos) = breakpoints.iter().position(|b| b.id == bp_id) {
        let bp = breakpoints.remove(pos);
        let scripts = state.scripts.lock();
        let bytecode = scripts[bp.script_idx].function_bytes(bp.func_idx);
        if !bytecode.is_empty() {
            unsafe {
                (bytecode.as_ptr().add(bp.offset) as *mut u8).write(bp.original_opcode);
            }
        }
        tracing::info!("Removed breakpoint {}", bp.id);
    }
}

/// Install step gnatives: redirect all GNatives to our handler, save originals
pub fn install_step_gnatives(state: &SharedState) -> Vec<ue::FNativesFuncPtr> {
    let saved: Vec<ue::FNativesFuncPtr> = state.gnatives_mut().iter().copied().flatten().collect();
    // Actually we need to save the full Option array
    let saved_full: Vec<ue::FNativesFuncPtr> = (0..255)
        .map(|i| state.gnatives()[i].unwrap_or(debug_break_handler))
        .collect();

    for i in 0..255u8 {
        if i != EX_DEBUG_BREAK {
            state.gnatives_mut()[i as usize] = Some(debug_break_handler);
        }
    }
    saved_full
}

/// Restore original GNatives from saved snapshot
pub fn restore_gnatives(state: &SharedState, saved: &[ue::FNativesFuncPtr]) {
    for (i, handler) in saved.iter().enumerate() {
        if i != EX_DEBUG_BREAK as usize {
            state.gnatives_mut()[i] = Some(*handler);
        }
    }
}

// ── Debug break handler (runs on game thread) ───────────────────────────────

unsafe extern "system" fn debug_break_handler(
    object: *mut ue::UObject,
    stack: &mut ue::kismet::FFrame,
    result: *mut c_void,
) {
    let state = shared();
    let func = unsafe { &*(stack.node() as *const ue::UFunction) };
    let bytecode = func.script().as_slice();
    let offset = unsafe { (stack.code() as *const u8).offset_from(bytecode.as_ptr()) as usize - 1 };
    tracing::debug!(
        "debug_break_handler: node={:?} offset=0x{:x} opcode=0x{:02x}",
        stack.node(),
        offset,
        bytecode[offset]
    );

    // Check if this is a breakpoint hit
    let node_weak = ue::TWeakObjectPtr::<ue::UFunction>::new(func);
    let current_location = {
        let scripts = state.scripts.lock();
        scripts
            .iter()
            .enumerate()
            .find_map(|(si, s)| s.function_for_weak(&node_weak).map(|fi| (si, fi)))
    };

    let breakpoints = state.breakpoints.lock();
    let bp_hit = current_location.and_then(|(si, fi)| {
        breakpoints
            .iter()
            .find(|bp| bp.script_idx == si && bp.func_idx == fi && bp.offset == offset)
    });

    let step = state.step_state.lock().clone();

    // Determine original opcode and handler
    let original_opcode = if let Some(bp) = bp_hit {
        bp.original_opcode
    } else {
        // Step hit: bytecode is unchanged, read the real opcode
        bytecode[offset]
    };

    let original_handler = if let Some(ref ss) = step {
        ss.saved_gnatives[original_opcode as usize]
    } else {
        state.gnatives()[original_opcode as usize].unwrap_or_else(|| {
            tracing::error!("No handler for opcode 0x{:02X}", original_opcode);
            debug_break_handler // fallback, shouldn't happen
        })
    };

    let bp_info = bp_hit.map(|bp| (bp.script_idx, bp.line));
    drop(breakpoints); // release lock before potentially blocking

    // Determine if we should pause
    let should_pause = if bp_info.is_some() {
        true
    } else if let Some(ref ss) = step {
        check_step_condition(ss, state, stack, offset)
    } else {
        false
    };

    if !should_pause {
        // Call original handler and return
        unsafe {
            original_handler(object, stack, result);
        }
        return;
    }

    // === PAUSING ===

    // If stepping, tear down step gnatives
    if step.is_some() {
        let ss = state.step_state.lock().take().unwrap();
        restore_gnatives(state, &ss.saved_gnatives);
        tracing::info!("Step {:?} complete", ss.mode);
    }

    // Resolve script + line
    let (script_idx, line) = if let Some(bp) = bp_info {
        bp
    } else {
        current_location
            .and_then(|(si, fi)| {
                state.scripts.lock()[si]
                    .offset_to_line(fi, offset as u64)
                    .map(|l| (si, l))
            })
            .unwrap_or((0, 0))
    };

    tracing::info!(
        "Paused: {} (line {line})",
        state
            .scripts
            .lock()
            .get(script_idx)
            .map(|s| s.class_path.clone())
            .unwrap_or_else(|| "???".into())
    );

    // Send paused event — game thread stays blocked, so the FFrame chain is stable
    let _ = state.event_tx.blocking_send(DbgEvent::Paused {
        script_idx,
        line,
        frame: stack as *const ue::kismet::FFrame,
    });

    let frame_depth = frame_depth(stack);

    // Service requests while paused
    while let Ok(req) = state.request_rx.recv() {
        match req {
            GameRequest::Resume(ResumeCmd::Continue) => break,
            GameRequest::Resume(cmd) => {
                let mode = match cmd {
                    ResumeCmd::StepOver => StepMode::Over,
                    ResumeCmd::StepInto => StepMode::Into,
                    ResumeCmd::StepOut => StepMode::Out,
                    ResumeCmd::Continue => unreachable!(),
                };
                if mode == StepMode::Out && frame_depth <= 1 {
                    tracing::debug!("StepOut at top frame, resuming");
                    break;
                }
                let saved = install_step_gnatives(state);
                *state.step_state.lock() = Some(StepState {
                    mode,
                    origin_node: node_weak,
                    origin_depth: frame_depth,
                    origin_line: Some(line),
                    saved_gnatives: saved,
                });
                break;
            }
            GameRequest::ReadProperties { object_id, resp } => {
                let result = read_properties(&object_id, stack);
                let _ = resp.send(result);
            }
        }
    }

    // Call original handler and return to UE
    unsafe {
        original_handler(object, stack, result);
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

pub fn frame_depth(stack: &ue::kismet::FFrame) -> usize {
    let mut depth = 0usize;
    let mut fp: *const ue::kismet::FFrame = stack;
    while !fp.is_null() {
        depth += 1;
        fp = unsafe { (*fp).previous_frame() };
        if depth > 64 {
            break;
        }
    }
    depth
}

pub fn nth_frame(stack: &ue::kismet::FFrame, n: usize) -> Option<&ue::kismet::FFrame> {
    let mut fp: *const ue::kismet::FFrame = stack;
    for _ in 0..n {
        if fp.is_null() {
            return None;
        }
        fp = unsafe { (*fp).previous_frame() };
    }
    if fp.is_null() {
        None
    } else {
        Some(unsafe { &*fp })
    }
}

pub fn iter_frames(stack: &ue::kismet::FFrame) -> impl Iterator<Item = &ue::kismet::FFrame> {
    let mut fp: *const ue::kismet::FFrame = stack;
    let mut count = 0usize;
    std::iter::from_fn(move || {
        if fp.is_null() || count > 64 {
            return None;
        }
        let frame = unsafe { &*fp };
        fp = frame.previous_frame();
        count += 1;
        Some(frame)
    })
}

fn check_step_condition(
    ss: &StepState,
    state: &SharedState,
    stack: &ue::kismet::FFrame,
    offset: usize,
) -> bool {
    let current_node = unsafe { &*(stack.node() as *const ue::UFunction) };
    let current_node_weak = ue::TWeakObjectPtr::<ue::UFunction>::new(current_node);
    let current_depth = frame_depth(stack);

    // Resolve current line
    let current_line = {
        let scripts = state.scripts.lock();
        scripts
            .iter()
            .find_map(|s| s.function_for_weak(&current_node_weak).map(|fi| (s, fi)))
            .and_then(|(s, fi)| s.offset_to_line(fi, offset as u64))
    };

    let same_line = match (ss.origin_line, current_line) {
        (Some(orig), Some(cur)) => orig == cur,
        _ => false,
    };

    match ss.mode {
        StepMode::Into => !same_line,
        StepMode::Over => {
            if current_depth < ss.origin_depth {
                true
            } else if current_depth > ss.origin_depth {
                false
            } else {
                current_node_weak != ss.origin_node || !same_line
            }
        }
        StepMode::Out => current_depth < ss.origin_depth,
    }
}

fn read_properties(object_id: &str, stack: &ue::kismet::FFrame) -> Value {
    // locals-N: read UFunction properties from the Locals buffer
    if let Some(idx_str) = object_id.strip_prefix("locals-") {
        let Ok(idx) = idx_str.parse::<usize>() else {
            return json!([]);
        };
        let Some(frame) = nth_frame(stack, idx) else {
            return json!([]);
        };
        if frame.locals().is_null() || frame.node().is_null() {
            return json!([]);
        }
        let ustruct = unsafe { &*(frame.node() as *const ue::UStruct) };
        return read_struct_properties(ustruct, frame.locals() as *const ());
    }

    // this-N: read UClass properties from the UObject instance
    if let Some(idx_str) = object_id.strip_prefix("this-") {
        let Ok(idx) = idx_str.parse::<usize>() else {
            return json!([]);
        };
        let Some(frame) = nth_frame(stack, idx) else {
            return json!([]);
        };
        if frame.object().is_null() {
            return json!([]);
        }
        return read_uobject_properties(frame.object() as *const ue::UObjectBase);
    }

    // struct-STRUCT_ADDR-DATA_ADDR: expand nested struct
    if let Some(rest) = object_id.strip_prefix("struct-") {
        let parts: Vec<&str> = rest.splitn(2, '-').collect();
        if parts.len() == 2 {
            if let (Ok(struct_addr), Ok(data_addr)) = (
                usize::from_str_radix(parts[0], 16),
                usize::from_str_radix(parts[1], 16),
            ) {
                let ustruct = unsafe { &*(struct_addr as *const ue::UStruct) };
                return read_struct_properties(ustruct, data_addr as *const ());
            }
        }
    }

    // obj-ADDR: expand a UObject
    if let Some(addr_str) = object_id.strip_prefix("obj-") {
        if let Ok(addr) = usize::from_str_radix(addr_str, 16) {
            return read_uobject_properties(addr as *const ue::UObjectBase);
        }
    }

    // array-PROP_ADDR-DATA_ADDR: expand array elements
    if let Some(rest) = object_id.strip_prefix("array-") {
        let parts: Vec<&str> = rest.splitn(2, '-').collect();
        if parts.len() == 2 {
            if let (Ok(prop_addr), Ok(data_addr)) = (
                usize::from_str_radix(parts[0], 16),
                usize::from_str_radix(parts[1], 16),
            ) {
                return read_array_elements(
                    unsafe { &*(prop_addr as *const ue::FArrayProperty) },
                    data_addr as *const (),
                );
            }
        }
    }

    // set-PROP_ADDR-DATA_ADDR: expand set elements
    if let Some(rest) = object_id.strip_prefix("set-") {
        let parts: Vec<&str> = rest.splitn(2, '-').collect();
        if parts.len() == 2 {
            if let (Ok(prop_addr), Ok(data_addr)) = (
                usize::from_str_radix(parts[0], 16),
                usize::from_str_radix(parts[1], 16),
            ) {
                return read_set_elements(
                    unsafe { &*(prop_addr as *const ue::FSetProperty) },
                    data_addr as *const (),
                );
            }
        }
    }

    // mappair-PROP_ADDR-DATA_ADDR-INDEX: expand a single map entry as {key, value}
    if let Some(rest) = object_id.strip_prefix("mappair-") {
        let parts: Vec<&str> = rest.splitn(3, '-').collect();
        if parts.len() == 3 {
            if let (Ok(prop_addr), Ok(data_addr), Ok(index)) = (
                usize::from_str_radix(parts[0], 16),
                usize::from_str_radix(parts[1], 16),
                parts[2].parse::<i32>(),
            ) {
                let map_prop = unsafe { &*(prop_addr as *const ue::FMapProperty) };
                let map_data = unsafe { ue::PropTrait::value(map_prop, data_addr as *const ()) };
                if map_data.is_valid_index(index) {
                    let key_bound = ue::BoundArrayElement {
                        data_ptr: map_data.key_data(index),
                        property: map_data.key_property(),
                    };
                    let val_bound = ue::BoundArrayElement {
                        data_ptr: map_data.value_data(index),
                        property: map_data.value_property(),
                    };
                    return json!([
                        {
                            "name": "key",
                            "value": prop_to_remote(&key_bound),
                            "writable": false, "configurable": false, "enumerable": true, "isOwn": true,
                        },
                        {
                            "name": "value",
                            "value": prop_to_remote(&val_bound),
                            "writable": false, "configurable": false, "enumerable": true, "isOwn": true,
                        },
                    ]);
                }
            }
        }
    }

    // map-PROP_ADDR-DATA_ADDR: expand map entries
    if let Some(rest) = object_id.strip_prefix("map-") {
        let parts: Vec<&str> = rest.splitn(2, '-').collect();
        if parts.len() == 2 {
            if let (Ok(prop_addr), Ok(data_addr)) = (
                usize::from_str_radix(parts[0], 16),
                usize::from_str_radix(parts[1], 16),
            ) {
                return read_map_elements(
                    unsafe { &*(prop_addr as *const ue::FMapProperty) },
                    data_addr as *const (),
                );
            }
        }
    }

    json!([])
}

fn read_uobject_properties(obj_ptr: *const ue::UObjectBase) -> Value {
    let obj = unsafe { &*obj_ptr };
    let class = obj.class();
    let mut result = read_struct_properties(class, obj_ptr as *const ());

    if let Value::Array(arr) = &mut result {
        let path = obj.object_path();
        let class_path =
            unsafe { (*(obj.class_private() as *const ue::UObjectBase)).object_path() };
        let addr = obj_ptr as u64;

        let mut meta = vec![
            json!({
                "name": "__path__",
                "value": {"type": "string", "value": path},
                "writable": false, "configurable": false, "enumerable": true, "isOwn": true,
            }),
            json!({
                "name": "__class__",
                "value": {"type": "string", "value": class_path},
                "writable": false, "configurable": false, "enumerable": true, "isOwn": true,
            }),
            json!({
                "name": "__address__",
                "value": {"type": "string", "value": format!("0x{addr:x}")},
                "writable": false, "configurable": false, "enumerable": true, "isOwn": true,
            }),
        ];

        if let Some(outer) = unsafe { obj.outer_private().as_ref() } {
            let outer_addr = outer as *const _ as u64;
            let outer_path = outer.object_path();
            meta.push(json!({
                "name": "__outer__",
                "value": {
                    "type": "object",
                    "className": "UObject",
                    "description": outer_path,
                    "objectId": format!("obj-{outer_addr:x}"),
                },
                "writable": false, "configurable": false, "enumerable": true, "isOwn": true,
            }));
        }

        meta.append(arr);
        *arr = meta;
    }
    result
}

fn read_struct_properties(ustruct: &ue::UStruct, base: *const ()) -> Value {
    let mut descriptors = Vec::new();

    // Use UObjectBase::props() style iteration — bind fields to object base
    for field in ustruct.iter_props() {
        let name = field.name().to_string();
        let bound = ue::BoundField {
            object: base,
            field,
        };
        let remote = prop_to_remote(&bound);

        descriptors.push(json!({
            "name": name,
            "value": remote,
            "writable": false,
            "configurable": false,
            "enumerable": true,
            "isOwn": true,
        }));
    }

    json!(descriptors)
}

/// Convert a bound property to a CDP RemoteObject JSON value.
/// Works with any PropertyAccess implementor (BoundField, BoundArrayElement, BoundSetElement).
fn prop_to_remote<'o>(prop: &impl ue::PropertyAccess<'o>) -> Value {
    use ue::*;

    macro_rules! try_prop {
        ($T:ty, $fmt:expr) => {
            if let Some(v) = prop.try_get::<$T>() {
                return $fmt(v);
            }
        };
    }

    // Numeric types
    try_prop!(
        FIntProperty,
        |v: FPropertyData<'_, i32>| json!({"type": "number", "value": *v})
    );
    try_prop!(
        FInt8Property,
        |v: FPropertyData<'_, i8>| json!({"type": "number", "value": *v})
    );
    try_prop!(
        FInt16Property,
        |v: FPropertyData<'_, i16>| json!({"type": "number", "value": *v})
    );
    try_prop!(
        FInt64Property,
        |v: FPropertyData<'_, i64>| json!({"type": "number", "value": *v})
    );
    try_prop!(
        FByteProperty,
        |v: FPropertyData<'_, u8>| json!({"type": "number", "value": *v})
    );
    try_prop!(
        FUInt16Property,
        |v: FPropertyData<'_, u16>| json!({"type": "number", "value": *v})
    );
    try_prop!(
        FUInt32Property,
        |v: FPropertyData<'_, u32>| json!({"type": "number", "value": *v})
    );
    try_prop!(
        FUInt64Property,
        |v: FPropertyData<'_, u64>| json!({"type": "number", "value": *v})
    );
    try_prop!(
        FFloatProperty,
        |v: FPropertyData<'_, f32>| json!({"type": "number", "value": *v, "description": format!("{:.4}", *v)})
    );
    try_prop!(
        FDoubleProperty,
        |v: FPropertyData<'_, f64>| json!({"type": "number", "value": *v, "description": format!("{:.4}", *v)})
    );

    // Bool
    if let Some(v) = prop.try_get::<FBoolProperty>() {
        return json!({"type": "boolean", "value": v.get()});
    }

    // String
    try_prop!(
        FStrProperty,
        |v: FPropertyData<'_, FString>| json!({"type": "string", "value": v.to_string()})
    );

    // Name
    try_prop!(
        FNameProperty,
        |v: FPropertyData<'_, FName>| json!({"type": "string", "value": v.to_string()})
    );

    // Object reference
    if let Some(v) = prop.try_get::<FObjectProperty>() {
        match *v {
            Some(obj) => {
                let obj_addr = obj as *const _ as u64;
                let path = obj.object_path();
                return json!({
                    "type": "object",
                    "className": "UObject",
                    "description": path,
                    "objectId": format!("obj-{obj_addr:x}"),
                });
            }
            None => return json!({"type": "object", "subtype": "null", "value": null}),
        }
    }

    // Struct — expandable
    if let Some(v) = prop.try_get::<FStructProperty>() {
        let data_addr = v.data_ptr() as u64;
        let struct_addr = v.get_struct() as *const UStruct as u64;
        return json!({
            "type": "object",
            "className": "Struct",
            "description": "Struct",
            "objectId": format!("struct-{struct_addr:x}-{data_addr:x}"),
        });
    }

    // Array — expandable
    if let Some(v) = prop.try_get::<FArrayProperty>() {
        let count = v.len();
        let prop_addr = v.array_property() as *const FArrayProperty as u64;
        let data_addr = v.script_array_ptr() as u64;
        return json!({
            "type": "object",
            "className": "TArray",
            "description": format!("Array[{count}]"),
            "objectId": format!("array-{prop_addr:x}-{data_addr:x}"),
        });
    }

    // Set — expandable
    if let Some(v) = prop.try_get::<FSetProperty>() {
        let count = v.len();
        let prop_addr = v.set_property() as *const FSetProperty as u64;
        let data_addr = v.script_set_ptr() as u64;
        return json!({
            "type": "object",
            "className": "TSet",
            "description": format!("Set[{count}]"),
            "objectId": format!("set-{prop_addr:x}-{data_addr:x}"),
        });
    }

    // Map — expandable
    if let Some(v) = prop.try_get::<ue::FMapProperty>() {
        let count = v.len();
        let prop_addr = v.map_property() as *const ue::FMapProperty as u64;
        let data_addr = v.script_map_ptr() as u64;
        return json!({
            "type": "object",
            "className": "TMap",
            "description": format!("Map[{count}]"),
            "objectId": format!("map-{prop_addr:x}-{data_addr:x}"),
        });
    }

    // Fallback: show the property type name
    let type_name = prop.field().class().name.to_string();
    json!({"type": "string", "value": format!("<{type_name}>")})
}

fn read_array_elements(array_prop: &ue::FArrayProperty, data: *const ()) -> Value {
    let arr_data = unsafe { ue::PropTrait::value(array_prop, data) };
    let count = arr_data.len();

    let mut descriptors = Vec::new();
    descriptors.push(json!({
        "name": "length",
        "value": {"type": "number", "value": count},
        "writable": false, "configurable": false, "enumerable": true, "isOwn": true,
    }));

    for i in 0..count {
        if let Some(elem) = arr_data.get_element(i) {
            let remote = prop_to_remote(&elem);
            descriptors.push(json!({
                "name": format!("{i}"),
                "value": remote,
                "writable": false, "configurable": false, "enumerable": true, "isOwn": true,
            }));
        }
    }

    json!(descriptors)
}

fn read_set_elements(set_prop: &ue::FSetProperty, data: *const ()) -> Value {
    let set_data = unsafe { ue::PropTrait::value(set_prop, data) };
    let count = set_data.len();

    let mut descriptors = Vec::new();
    descriptors.push(json!({
        "name": "size",
        "value": {"type": "number", "value": count},
        "writable": false, "configurable": false, "enumerable": true, "isOwn": true,
    }));

    for (i, elem) in set_data.iter().enumerate() {
        let remote = prop_to_remote(&elem);
        descriptors.push(json!({
            "name": format!("{i}"),
            "value": remote,
            "writable": false, "configurable": false, "enumerable": true, "isOwn": true,
        }));
    }

    json!(descriptors)
}

fn read_map_elements(map_prop: &ue::FMapProperty, data: *const ()) -> Value {
    let map_data = unsafe { ue::PropTrait::value(map_prop, data) };
    let count = map_data.len();
    let max_index = map_data.max_index();

    let mut descriptors = Vec::new();
    descriptors.push(json!({
        "name": "size",
        "value": {"type": "number", "value": count},
        "writable": false, "configurable": false, "enumerable": true, "isOwn": true,
    }));

    let mut entry_idx = 0;
    for i in 0..max_index {
        if !map_data.is_valid_index(i) {
            continue;
        }
        let key_ptr = map_data.key_data(i);
        let val_ptr = map_data.value_data(i);

        let key_bound = ue::BoundArrayElement {
            data_ptr: key_ptr,
            property: map_data.key_property(),
        };
        let val_bound = ue::BoundArrayElement {
            data_ptr: val_ptr,
            property: map_data.value_property(),
        };

        let key_remote = prop_to_remote(&key_bound);
        let val_remote = prop_to_remote(&val_bound);

        // Each entry is an expandable {key, value} object
        let pair_id = format!(
            "mappair-{:x}-{:x}-{i}",
            map_prop as *const _ as u64, data as u64
        );
        descriptors.push(json!({
            "name": format!("{entry_idx}"),
            "value": {
                "type": "object",
                "className": "MapEntry",
                "description": "{ key, value }",
                "objectId": pair_id,
                "preview": {
                    "type": "object",
                    "overflow": false,
                    "properties": [
                        {"name": "key", "type": key_remote.get("type").and_then(|v| v.as_str()).unwrap_or("object"), "value": format!("{}", key_remote.get("value").or_else(|| key_remote.get("description")).unwrap_or(&json!("...")))},
                        {"name": "value", "type": val_remote.get("type").and_then(|v| v.as_str()).unwrap_or("object"), "value": format!("{}", val_remote.get("value").or_else(|| val_remote.get("description")).unwrap_or(&json!("...")))},
                    ]
                }
            },
            "writable": false, "configurable": false, "enumerable": true, "isOwn": true,
        }));
        entry_idx += 1;
    }

    json!(descriptors)
}
