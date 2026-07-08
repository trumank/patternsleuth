mod cdp;
mod crash_handler;
mod debugger;
pub mod live_resolver;
mod logging;
mod scripts;

use libue as ue;

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use patternsleuth::resolvers::impl_try_collector;
use patternsleuth::resolvers::unreal::{
    engine_version::EngineVersion,
    fname::{FNameCtorWchar, FNameToString},
    fuobject_hash_tables::FUObjectHashTablesGet,
    game_loop::FEngineLoopTick,
    gmalloc::GMalloc,
    guobject_array::GUObjectArray,
    kismet::GNatives,
    static_construct_object::StaticConstructObjectInternal,
    static_load_object::StaticLoadObject,
    uobject_conditional_post_load::UObjectConditionalPostLoad,
};
use windows::Win32::{
    Foundation::HMODULE,
    System::{
        SystemServices::*,
        Threading::{GetCurrentThread, QueueUserAPC},
    },
};

retour::static_detour! {
    static HookEngineLoopTick: unsafe extern "system" fn(*mut c_void, f32, u8);
    static HookConditionalPostLoad: unsafe extern "system" fn(*mut ue::UObjectBase);
}

static DEFERRED_INIT_DONE: AtomicBool = AtomicBool::new(false);

#[unsafe(no_mangle)]
#[allow(non_snake_case, unused_variables)]
extern "system" fn DllMain(dll_module: HMODULE, call_reason: u32, _: *mut ()) -> bool {
    unsafe {
        match call_reason {
            DLL_PROCESS_ATTACH => {
                QueueUserAPC(Some(init), GetCurrentThread(), 0);
            }
            DLL_PROCESS_DETACH => (),
            _ => (),
        }

        true
    }
}

unsafe extern "system" fn init(_: usize) {
    unsafe {
        if let Ok(bin_dir) = setup() {
            tracing::info!("cdpdbg loaded");

            if let Err(e) = patch(bin_dir) {
                tracing::error!("{e:#}");
            }
        }
    }
}

fn setup() -> Result<PathBuf> {
    let exe_path = std::env::current_exe()?;
    let bin_dir = exe_path.parent().context("could not find exe parent dir")?;

    let log_guard = logging::setup_logging(bin_dir)?;
    std::mem::forget(log_guard);

    unsafe {
        crash_handler::setup();
    }

    Ok(bin_dir.to_path_buf())
}

impl_try_collector! {
    #[derive(Debug, PartialEq, Clone)]
    struct CdpDbgResolution {
        engine_version: EngineVersion,
        gmalloc: GMalloc,
        guobject_array: GUObjectArray,
        fnametostring: FNameToString,
        fname_ctor_wchar: FNameCtorWchar,
        gnatives: GNatives,
        engine_loop_tick: FEngineLoopTick,
        conditional_post_load: UObjectConditionalPostLoad,
        fuobject_hash_tables_get: FUObjectHashTablesGet,
        static_load_object: StaticLoadObject,
        static_construct_object: StaticConstructObjectInternal,
    }
}

unsafe fn patch(_bin_dir: PathBuf) -> Result<()> {
    unsafe {
        let exe = patternsleuth::process::internal::read_image()?;

        tracing::info!("starting scan");
        let resolution = exe.resolve(CdpDbgResolution::resolver())?;
        tracing::info!("finished scan");

        tracing::info!("results: {:?}", resolution);

        let selected = ue::UeLayout::for_version(
            resolution.engine_version.major,
            resolution.engine_version.minor,
        )
        .with_context(|| format!("unsupported engine version {}", resolution.engine_version))?;
        tracing::info!(
            "engine version {} -> selected {} layout",
            resolution.engine_version,
            selected.name
        );

        let guobject_array: &'static ue::FUObjectArray =
            &*(resolution.guobject_array.0 as *const ue::FUObjectArray);

        ue::init(ue::UeGlobals {
            gmalloc: resolution.gmalloc.0 as *const *const ue::FMalloc,
            fname_to_string: std::mem::transmute(resolution.fnametostring.0),
            fname_ctor_wchar: std::mem::transmute(resolution.fname_ctor_wchar.0),
            guobject_array,
            fuobject_hash_tables_get: std::mem::transmute(resolution.fuobject_hash_tables_get.0),
            static_load_object: std::mem::transmute(resolution.static_load_object.0),
            static_construct_object: std::mem::transmute(resolution.static_construct_object.0),
            gnatives: resolution.gnatives.0 as *mut ue::GNatives,
            layout: selected.layout,
        });

        tracing::info!("libue initialized");

        // Hook engine tick — deferred init runs on first tick
        let tick_target: unsafe extern "system" fn(*mut c_void, f32, u8) =
            std::mem::transmute(resolution.engine_loop_tick.0);
        HookEngineLoopTick
            .initialize(tick_target, hooked_engine_loop_tick)?
            .enable()?;

        // Hook ConditionalPostLoad — catches UFunction loads after init
        let post_load_target: unsafe extern "system" fn(*mut ue::UObjectBase) =
            std::mem::transmute(resolution.conditional_post_load.0);
        HookConditionalPostLoad
            .initialize(post_load_target, hooked_conditional_post_load)?
            .enable()?;

        tracing::info!("Hooks installed, waiting for first tick...");

        Ok(())
    }
}

fn hooked_engine_loop_tick(this: *mut c_void, delta_seconds: f32, idle_mode: u8) {
    if !DEFERRED_INIT_DONE.swap(true, Ordering::SeqCst) {
        tracing::info!("First engine tick — running deferred init");

        if let Err(e) = deferred_init() {
            tracing::error!("Deferred init failed: {e:#}");
        }
    } else {
        // Drain any classes queued by off-thread post-load events. The
        // build calls into `StaticLoadObject`, which must run on the game
        // thread, so we can only do it from here.
        debugger::process_pending_class_builds();
    }

    unsafe { HookEngineLoopTick.call(this, delta_seconds, idle_mode) }
}

fn hooked_conditional_post_load(this: *mut ue::UObjectBase) {
    // ConditionalPostLoad is idempotent: the real PostLoad only runs on the
    // first call (when RF_NeedPostLoad is set), but every child object's
    // load forces a re-call on its Class/Outer as a dependency step. Snapshot
    // the flag *before* the call so we see the "first time" entries only.
    let is_real_post_load = unsafe {
        (*this)
            .object_flags()
            .contains(ue::EObjectFlags::RF_NeedPostLoad)
    };

    unsafe { HookConditionalPostLoad.call(this) };

    if !is_real_post_load {
        return;
    }

    // Only process after deferred init is done
    if !DEFERRED_INIT_DONE.load(Ordering::SeqCst) {
        return;
    }

    let obj = unsafe { &*this };

    // React on both UClass and UFunction post-loads. The owning class's
    // PostLoad isn't guaranteed to run last (children can finish after the
    // parent), so we defer script building to engine tick where we can
    // (a) call `StaticLoadObject` on the game thread and (b) re-check
    // class readiness once all sibling children have post-loaded.
    let class_obj: &ue::UObjectBase = if obj.cast::<ue::UClass>().is_some() {
        obj
    } else if obj.cast::<ue::UFunction>().is_some() {
        let Some(outer) = (unsafe { obj.outer_private().as_ref() }) else {
            return;
        };
        if outer.cast::<ue::UClass>().is_none() {
            return;
        }
        outer
    } else {
        return;
    };

    let weak = ue::TWeakObjectPtr::<ue::UClass>::new(class_obj);
    debugger::queue_class_build(weak);
}

fn deferred_init() -> Result<()> {
    // Test hash table lookups
    let ht = ue::hash_tables();
    let test_paths = [
        "/Script/Engine",
        "/Script/CoreUObject",
        "/Engine/Transient",
        "/Script/Engine.Default__Actor",
    ];
    for path in test_paths {
        match ht.find_object_by_path(path) {
            Some(obj) => tracing::info!("hash table lookup '{path}' => {}", obj.path()),
            None => tracing::warn!("hash table lookup '{path}' => NOT FOUND"),
        }
    }

    let guobject_array = ue::globals().guobject_array;
    let guobject_array = unsafe { &*guobject_array };

    // Build script table — engine is running, GUObjectArray is populated
    let scripts = scripts::build_scripts(guobject_array);
    tracing::info!("Built {} script entries", scripts.len());

    // Get GNatives as mutable reference — libue holds the resolved table pointer.
    let gnatives: &'static mut ue::GNatives = unsafe { &mut *ue::globals().gnatives };

    // Initialize debugger (installs 0xFE handler, creates channels)
    let channels = unsafe { debugger::init(scripts, gnatives) };

    // Spawn tokio runtime for CDP server on background thread
    std::thread::Builder::new()
        .name("cdpdbg-server".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .thread_name("cdpdbg-tokio")
                .build()
                .expect("failed to create tokio runtime");

            rt.block_on(cdp::run_server(channels));
        })
        .context("failed to spawn CDP server thread")?;

    tracing::info!("CDP debugger ready");

    Ok(())
}
