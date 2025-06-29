mod crash_handler;
mod events;
mod gui;
mod hooks;
mod kismet;
mod kismet_nodes;
mod kismet_transform;
mod logging;
mod object_cache;
mod ue;

use std::path::PathBuf;

use anyhow::{Context, Result};
use patternsleuth::resolvers::impl_try_collector;
use patternsleuth::resolvers::unreal::blueprint_library::UFunctionBind;
use patternsleuth::resolvers::unreal::{
    fname::FNameToString,
    game_loop::{FEngineLoopInit, FEngineLoopTick},
    gmalloc::GMalloc,
    guobject_array::{
        FUObjectArrayAllocateUObjectIndex, FUObjectArrayFreeUObjectIndex, GUObjectArray,
    },
    kismet::{FFrameStep, FFrameStepExplicitProperty, FFrameStepViaExec},
};
use windows::Win32::{
    Foundation::HMODULE,
    System::{
        SystemServices::*,
        Threading::{GetCurrentThread, QueueUserAPC},
    },
};

#[no_mangle]
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
    if let Ok(bin_dir) = setup() {
        tracing::info!("dll_hook loaded",);

        if let Err(e) = patch(bin_dir) {
            tracing::error!("{e:#}");
        }
    }
}

fn setup() -> Result<PathBuf> {
    let exe_path = std::env::current_exe()?;
    let bin_dir = exe_path.parent().context("could not find exe parent dir")?;

    let log_guard = logging::setup_logging(bin_dir)?;

    std::mem::forget(log_guard); // TODO hold onto this and drop on exit?

    unsafe {
        crash_handler::setup_windows_exception_handler();
    }

    Ok(bin_dir.to_path_buf())
}

impl_try_collector! {
    #[derive(Debug, PartialEq, Clone)]
    struct DllHookResolution {
        gmalloc: GMalloc,
        guobject_array: GUObjectArray,
        fnametostring: FNameToString,
        allocate_uobject: FUObjectArrayAllocateUObjectIndex,
        free_uobject: FUObjectArrayFreeUObjectIndex,
        game_tick: FEngineLoopTick,
        engine_loop_init: FEngineLoopInit,
        fframe_step_via_exec: FFrameStepViaExec,
        fframe_step: FFrameStep,
        fframe_step_explicit_property: FFrameStepExplicitProperty,
        ufunction_bind: UFunctionBind,
    }
}

static mut GLOBALS: Option<Globals> = None;

pub struct Globals {
    resolution: DllHookResolution,
    guobject_array: parking_lot::FairMutex<&'static ue::FUObjectArray>,
    main_thread_id: std::thread::ThreadId,
}

impl Globals {
    pub fn gmalloc(&self) -> &ue::FMalloc {
        unsafe { &**(self.resolution.gmalloc.0 as *const *const ue::FMalloc) }
    }
    pub fn fframe_step(&self) -> ue::FnFFrameStep {
        unsafe { std::mem::transmute(self.resolution.fframe_step.0) }
    }
    pub fn fframe_step_explicit_property(&self) -> ue::FnFFrameStepExplicitProperty {
        unsafe { std::mem::transmute(self.resolution.fframe_step_explicit_property.0) }
    }
    pub fn fname_to_string(&self) -> ue::FnFNameToString {
        unsafe { std::mem::transmute(self.resolution.fnametostring.0) }
    }
    pub fn guobject_array(&self) -> parking_lot::FairMutexGuard<'static, &ue::FUObjectArray> {
        self.guobject_array.lock()
    }
    pub unsafe fn guobject_array_unchecked(&self) -> &ue::FUObjectArray {
        *self.guobject_array.data_ptr()
    }
}

pub fn globals() -> &'static Globals {
    unsafe { GLOBALS.as_ref().unwrap() }
}

#[macro_export]
macro_rules! assert_main_thread {
    () => {
        assert_eq!(std::thread::current().id(), globals().main_thread_id);
    };
}

fn dump_backtrace() {
    tracing::info!(
        "Dumping backtrace on thread {:?}:",
        std::thread::current().id()
    );
    let backtrace = backtrace::Backtrace::new();
    for (index, frame) in backtrace.frames().iter().enumerate() {
        tracing::info!("  {index}: {:?} {:?}", frame.ip(), frame.symbols());
    }
}

unsafe fn patch(bin_dir: PathBuf) -> Result<()> {
    let exe = patternsleuth::process::internal::read_image()?;

    tracing::info!("starting scan");
    let resolution = exe.resolve(DllHookResolution::resolver())?;
    tracing::info!("finished scan");

    tracing::info!("results: {:?}", resolution);

    let guobject_array: &'static ue::FUObjectArray =
        &*(resolution.guobject_array.0 as *const ue::FUObjectArray);

    GLOBALS = Some(Globals {
        guobject_array: guobject_array.into(),
        resolution,
        main_thread_id: std::thread::current().id(),
    });

    hooks::initialize()?;

    gui::init();

    Ok(())
}
