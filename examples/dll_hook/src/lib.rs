mod app;
mod gui;
mod hooks;
mod object_cache;
mod ue;

use std::mem::MaybeUninit;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use patternsleuth_resolvers::unreal::UObjectBaseUtilityGetPathName;
use patternsleuth_resolvers::unreal::blueprint_library::UFunctionBind;
use patternsleuth_resolvers::unreal::{
    KismetSystemLibrary,
    fname::FNameToString,
    game_loop::{FEngineLoopInit, UGameEngineTick},
    gmalloc::GMalloc,
    guobject_array::{
        FUObjectArrayAllocateUObjectIndex, FUObjectArrayFreeUObjectIndex, GUObjectArray,
    },
    kismet::{FFrameStep, FFrameStepExplicitProperty, FFrameStepViaExec},
};
use patternsleuth_resolvers::{impl_try_collector, resolve};
use simple_log::{LogConfigBuilder, error, info};
use windows::Win32::{
    Foundation::HMODULE,
    System::{
        SystemServices::*,
        Threading::{GetCurrentThread, QueueUserAPC},
    },
};

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
    if let Ok(bin_dir) = setup() {
        info!("dll_hook loaded",);

        if let Err(e) = patch(bin_dir) {
            error!("{e:#}");
        }
    }
}

fn setup() -> Result<PathBuf> {
    let exe_path = std::env::current_exe()?;
    let bin_dir = exe_path.parent().context("could not find exe parent dir")?;
    let config = LogConfigBuilder::builder()
        .path(bin_dir.join("dll_hook.txt").to_str().unwrap()) // TODO why does this not take a path??
        .time_format("%Y-%m-%d %H:%M:%S.%f")
        .level("debug")
        .output_file()
        .size(u64::MAX)
        .build();
    simple_log::new(config).map_err(|e| anyhow!("{e}"))?;
    Ok(bin_dir.to_path_buf())
}

#[derive(Debug, PartialEq)]
pub struct FFrameKismetExecutionMessage(usize);

mod resolvers {
    use super::*;

    use patternsleuth_image::scanner::Pattern;
    use patternsleuth_resolvers::{futures::future::join_all, *};

    impl_resolver_singleton!(collect, FFrameKismetExecutionMessage);
    impl_resolver_singleton!(PEImage, FFrameKismetExecutionMessage, |ctx| async {
        // void FFrame::KismetExecutionMessage(wchar16 const* Message, enum ELogVerbosity::Type Verbosity, class FName WarningId)
        let patterns = ["48 89 5C 24 ?? 57 48 83 EC 40 0F B6 DA 48 8B F9"];
        let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;
        Ok(Self(ensure_one(res.into_iter().flatten())?))
    });
}

impl_try_collector! {
    #[derive(Debug, PartialEq, Clone)]
    struct DllHookResolution {
        gmalloc: GMalloc,
        guobject_array: GUObjectArray,
        fnametostring: FNameToString,
        allocate_uobject: FUObjectArrayAllocateUObjectIndex,
        free_uobject: FUObjectArrayFreeUObjectIndex,
        game_tick: UGameEngineTick,
        engine_loop_init: FEngineLoopInit,
        kismet_system_library: KismetSystemLibrary,
        fframe_step_via_exec: FFrameStepViaExec,
        fframe_step: FFrameStep,
        fframe_step_explicit_property: FFrameStepExplicitProperty,
        fframe_kismet_execution_message: FFrameKismetExecutionMessage,
        ufunction_bind: UFunctionBind,
        uobject_base_utility_get_path_name: UObjectBaseUtilityGetPathName,
    }
}

static mut GLOBALS: MaybeUninit<Globals> = MaybeUninit::uninit();

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
    pub fn uobject_base_utility_get_path_name(&self) -> ue::FnUObjectBaseUtilityGetPathName {
        unsafe { std::mem::transmute(self.resolution.uobject_base_utility_get_path_name.0) }
    }
    pub fn guobject_array(&self) -> parking_lot::FairMutexGuard<'static, &ue::FUObjectArray> {
        self.guobject_array.lock()
    }
    pub unsafe fn guobject_array_unchecked(&self) -> &ue::FUObjectArray {
        *self.guobject_array.data_ptr()
    }
}

pub fn globals() -> &'static Globals {
    #[allow(static_mut_refs)]
    unsafe {
        GLOBALS.assume_init_ref()
    }
}

#[macro_export]
macro_rules! assert_main_thread {
    () => {
        assert_eq!(std::thread::current().id(), globals().main_thread_id);
    };
}

fn dump_backtrace() {
    info!(
        "Dumping backtrace on thread {:?}:",
        std::thread::current().id()
    );
    let backtrace = backtrace::Backtrace::new();
    for (index, frame) in backtrace.frames().iter().enumerate() {
        info!("  {index}: {:?} {:?}", frame.ip(), frame.symbols());
    }
}

unsafe fn patch(bin_dir: PathBuf) -> Result<()> {
    unsafe {
        let exe = patternsleuth_image::process::internal::read_image()?;

        info!("starting scan");
        let resolution = resolve(&exe, DllHookResolution::resolver())?;
        info!("finished scan");

        info!("results: {:?}", resolution);

        let guobject_array: &'static ue::FUObjectArray =
            &*(resolution.guobject_array.0 as *const ue::FUObjectArray);

        #[allow(static_mut_refs)]
        GLOBALS.write(Globals {
            guobject_array: guobject_array.into(),
            resolution,
            main_thread_id: std::thread::current().id(),
        });

        hooks::initialize()?;

        info!("initialized");

        app::run(bin_dir)
    }
}
