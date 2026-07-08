//! py_repl — single self-contained REPL DLL.
//!
//! RustPython is a pure-Rust Python 3 interpreter statically linked into this
//! cdylib (stdlib frozen in), so there's no python3.dll / stdlib zip to ship
//! and no loader stub — drop this one DLL next to the game and inject it.
//!
//! See `src/server.rs` and `src/interp.rs` for the wire format and threading
//! model. Bindings (UObject/Struct/Array proxies with native attribute access)
//! live in `src/bindings.rs`.

mod bindings;
mod interp;
mod logging;
mod server;
mod trampoline;

use libue as ue;

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use patternsleuth::resolvers::impl_try_collector;
use patternsleuth::resolvers::unreal::{
    dispatch_begin_play::AActorDispatchBeginPlay,
    engine_version::EngineVersion,
    fname::{FNameCtorWchar, FNameToString},
    fuobject_hash_tables::FUObjectHashTablesGet,
    game_loop::{FEngineLoopTick, UWorldTick},
    gmalloc::GMalloc,
    guobject_array::GUObjectArray,
    kismet::GNatives,
    static_construct_object::StaticConstructObjectInternal,
    static_load_object::StaticLoadObject,
};
use windows::Win32::{
    Foundation::HMODULE,
    System::{
        SystemServices::*,
        Threading::{GetCurrentThread, QueueUserAPC},
    },
};

const REPL_PORT: u16 = 8765;

retour::static_detour! {
    static HookEngineLoopTick: unsafe extern "system" fn(*mut c_void, f32, u8);
    // void AActor::DispatchBeginPlay(bool bFromLevelStreaming) — UE 4.24+ signature
    // (the target is DRG/4.27). `this` is the AActor in RCX, the bool in DL.
    static HookDispatchBeginPlay: unsafe extern "system" fn(*mut c_void, bool);
    // void UWorld::Tick(ELevelTick TickType, float DeltaSeconds). `this` (UWorld*)
    // in RCX, TickType in EDX, and — being the 3rd positional arg and a float —
    // DeltaSeconds in XMM2. `DeltaSeconds` is already dilated/clamped.
    static HookUWorldTick: unsafe extern "system" fn(*mut c_void, u32, f32);
}

static DEFERRED_INIT_DONE: AtomicBool = AtomicBool::new(false);

#[unsafe(no_mangle)]
#[allow(non_snake_case, unused_variables)]
extern "system" fn DllMain(_dll_module: HMODULE, call_reason: u32, _: *mut ()) -> bool {
    if call_reason == DLL_PROCESS_ATTACH {
        unsafe { QueueUserAPC(Some(init), GetCurrentThread(), 0) };
    }
    true
}

unsafe extern "system" fn init(_: usize) {
    if let Ok(bin_dir) = setup() {
        tracing::info!("py_repl loaded");
        if let Err(e) = unsafe { patch(bin_dir) } {
            tracing::error!("patch: {e:#}");
        }
    }
}

fn setup() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let bin = exe.parent().context("exe parent")?.to_path_buf();
    let guard = logging::setup_logging(&bin)?;
    std::mem::forget(guard);
    Ok(bin)
}

impl_try_collector! {
    #[derive(Debug, PartialEq, Clone)]
    struct PyReplResolution {
        engine_version: EngineVersion,
        gmalloc: GMalloc,
        guobject_array: GUObjectArray,
        fname_to_string: FNameToString,
        fname_ctor_wchar: FNameCtorWchar,
        engine_loop_tick: FEngineLoopTick,
        fuobject_hash_tables_get: FUObjectHashTablesGet,
        static_load_object: StaticLoadObject,
        static_construct_object: StaticConstructObjectInternal,
        gnatives: GNatives,
        aactor_dispatch_begin_play: AActorDispatchBeginPlay,
    }
}

unsafe fn patch(_bin: PathBuf) -> Result<()> {
    unsafe {
        let exe = patternsleuth::process::internal::read_image()?;
        let res = exe.resolve(PyReplResolution::resolver())?;
        tracing::info!("resolved: {:?}", res);

        let selected =
            ue::UeLayout::for_version(res.engine_version.major, res.engine_version.minor)
                .with_context(|| format!("unsupported engine version {}", res.engine_version))?;
        tracing::info!(
            "engine version {} -> selected {} layout",
            res.engine_version,
            selected.name
        );

        let guobject_array: &'static ue::FUObjectArray =
            &*(res.guobject_array.0 as *const ue::FUObjectArray);

        ue::init(ue::UeGlobals {
            gmalloc: res.gmalloc.0 as *const *const ue::FMalloc,
            fname_to_string: std::mem::transmute(res.fname_to_string.0),
            fname_ctor_wchar: std::mem::transmute(res.fname_ctor_wchar.0),
            guobject_array,
            fuobject_hash_tables_get: std::mem::transmute(res.fuobject_hash_tables_get.0),
            static_load_object: std::mem::transmute(res.static_load_object.0),
            static_construct_object: std::mem::transmute(res.static_construct_object.0),
            gnatives: res.gnatives.0 as *mut ue::GNatives,
            layout: selected.layout,
        });

        let tick: unsafe extern "system" fn(*mut c_void, f32, u8) =
            std::mem::transmute(res.engine_loop_tick.0);
        HookEngineLoopTick.initialize(tick, hooked_tick)?.enable()?;

        let dispatch_begin_play: unsafe extern "system" fn(*mut c_void, bool) =
            std::mem::transmute(res.aactor_dispatch_begin_play.0);
        HookDispatchBeginPlay
            .initialize(dispatch_begin_play, hooked_dispatch_begin_play)?
            .enable()?;

        // UWorld::Tick drives `ue.on_tick`. Resolved independently of the
        // mandatory collector above so a WIP/failed resolver only disables the
        // tick hook instead of taking down the whole REPL.
        match exe.resolve(UWorldTick::resolver()) {
            Ok(world_tick) => {
                let uworld_tick: unsafe extern "system" fn(*mut c_void, u32, f32) =
                    std::mem::transmute(world_tick.0);
                HookUWorldTick
                    .initialize(uworld_tick, hooked_uworld_tick)?
                    .enable()?;
                tracing::info!("UWorld::Tick hook installed ({:#x})", world_tick.0);
            }
            Err(e) => tracing::warn!("UWorld::Tick unresolved — ue.on_tick disabled: {e:#}"),
        }

        tracing::info!("hooks installed");
        Ok(())
    }
}

fn hooked_tick(this: *mut c_void, dt: f32, idle: u8) {
    if !DEFERRED_INIT_DONE.swap(true, Ordering::Relaxed) {
        // libue is fully initialized at this point — safe to start serving.
        if let Err(e) = server::spawn(REPL_PORT) {
            tracing::error!("server spawn: {e:#}");
        }
    }
    server::drain_and_execute(interp::run);
    unsafe { HookEngineLoopTick.call(this, dt, idle) }
}

/// Native detour on `AActor::DispatchBeginPlay` — the single chokepoint every
/// actor passes through exactly once (spawned, map-placed, or level-streamed),
/// sitting above the virtual `BeginPlay`. Registered `ue.on_begin_play`
/// callbacks fire here, *before* the engine runs the actor's BeginPlay, so they
/// can mutate the fully-constructed-but-not-yet-live actor.
fn hooked_dispatch_begin_play(this: *mut c_void, b_from_level_streaming: bool) {
    bindings::dispatch_begin_play(this as *const ue::UObjectBase);
    unsafe { HookDispatchBeginPlay.call(this, b_from_level_streaming) }
}

/// Native detour on `UWorld::Tick`. We dispatch registered `ue.on_tick`
/// callbacks *after* the engine's own tick so Python observes a fully-settled
/// world (actor ticks, physics, latent actions and timers for the frame have
/// all run). `DeltaSeconds` is the world's dilated/clamped frame delta.
fn hooked_uworld_tick(this: *mut c_void, tick_type: u32, dt: f32) {
    unsafe { HookUWorldTick.call(this, tick_type, dt) };
    bindings::dispatch_world_tick(this as *const ue::UObjectBase, dt);
}
