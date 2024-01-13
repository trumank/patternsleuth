#![feature(backtrace_frames)]

mod app;
mod gui;
mod hooks;
mod object_cache;
mod ue;

use std::{ffi::c_void, path::PathBuf};

use anyhow::{anyhow, Context, Result};
use patternsleuth::resolvers::impl_try_collector;
use patternsleuth::resolvers::unreal::blueprint_library::UFunctionBind;
use patternsleuth::resolvers::unreal::UObjectBaseUtilityGetPathName;
use patternsleuth::resolvers::unreal::{
    fname::FNameToString,
    game_loop::{FEngineLoopInit, UGameEngineTick},
    gmalloc::GMalloc,
    guobject_array::{
        FUObjectArrayAllocateUObjectIndex, FUObjectArrayFreeUObjectIndex, GUObjectArray,
    },
    kismet::{FFrameStep, FFrameStepExplicitProperty, FFrameStepViaExec},
    KismetSystemLibrary,
};
use simple_log::{error, info, LogConfigBuilder};
use windows::Win32::{
    Foundation::HMODULE,
    System::{
        SystemServices::*,
        Threading::{GetCurrentThread, QueueUserAPC},
    },
};

// x3daudio1_7.dll
#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn X3DAudioCalculate() {}
#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn X3DAudioInitialize() {}

// d3d9.dll
#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn D3DPERF_EndEvent() {}
#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn D3DPERF_BeginEvent() {}

// d3d11.dll
#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn D3D11CreateDevice() {}

// dxgi.dll
#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn CreateDXGIFactory() {}
#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn CreateDXGIFactory1() {}

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
pub struct StartRecordingReplay(usize);
type FnStartRecordingReplay = unsafe extern "system" fn(
    this: *const ue::UObject, // game instance
    name: &ue::FString,
    friendly_name: &ue::FString,
    additional_options: &ue::TArray<ue::FString>,
    analytics_provider: ue::TSharedPtr<c_void>,
);
impl StartRecordingReplay {
    fn get(&self) -> FnStartRecordingReplay {
        unsafe { std::mem::transmute(self.0) }
    }
}

#[derive(Debug, PartialEq)]
pub struct StopRecordingReplay(usize);
type FnStopRecordingReplay = unsafe extern "system" fn(
    this: *const ue::UObject, // game instance
);
impl StopRecordingReplay {
    fn get(&self) -> FnStopRecordingReplay {
        unsafe { std::mem::transmute(self.0) }
    }
}

#[derive(Debug, PartialEq)]
pub struct FFrameKismetExecutionMessage(usize);

mod resolvers {
    use super::*;

    use patternsleuth::{
        resolvers::{futures::future::join_all, *},
        scanner::Pattern,
    };

    impl_resolver!(StartRecordingReplay, |ctx| async {
        // public: virtual void __cdecl UGameInstance::StartRecordingReplay(class FString const &, class FString const &, class TArray<class FString, class TSizedDefaultAllocator<32> > const &, class TSharedPtr<class IAnalyticsProvider, 0>)
        let patterns = [
            "48 89 5C 24 08 48 89 6C 24 10 48 89 74 24 18 48 89 7C 24 20 41 56 48 83 EC 40 49 8B F1 49 8B E8 4C 8B F2 48 8B F9 E8 ?? ?? ?? 00 48 8B D8 48 85 C0 74 24 E8 ?? ?? ?? 00 48 85 C0 74 1A 4C 8D 48 ?? 48 63 40 ?? 3B 43 ?? 7F 0D 48 8B C8 48 8B 43 ?? 4C 39 0C C8 74 02 33 DB 48 8D 8F ?? 00 00 00 48 8B D3 E8"
        ];

        let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

        Ok(Self(ensure_one(res.into_iter().flatten())?))
    });

    impl_resolver!(StopRecordingReplay, |ctx| async {
        // public: virtual void __cdecl UGameInstance::StopRecordingReplay(void)
        let patterns = [
            "48 89 5C 24 08 57 48 83 EC 20 48 8B F9 E8 ?? ?? ?? 00 48 8B D8 48 85 C0 74 24 E8 ?? ?? ?? 00 48 85 C0 74 1A 48 8D 50 ?? 48 63 40 ?? 3B 43 ?? 7F 0D 48 8B C8 48 8B 43 ?? 48 39 14 C8 74 02 33 DB 48 8D 8F ?? 00 00 00 48 8B D3 E8 ?? ?? ?? 00 48 85 C0 74 08 48 8B C8 E8 ?? ?? ?? 00 48 8B 5C 24 30 48 83 C4"
        ];

        let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

        Ok(Self(ensure_one(res.into_iter().flatten())?))
    });

    impl_resolver_singleton!(FFrameKismetExecutionMessage, |ctx| async {
        // void FFrame::KismetExecutionMessage(wchar16 const* Message, enum ELogVerbosity::Type Verbosity, class FName WarningId)
        let patterns = ["48 89 5C 24 ?? 57 48 83 EC 40 0F B6 DA 48 8B F9"];
        let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;
        Ok(Self(ensure_one(res.into_iter().flatten())?))
    });
}

impl_try_collector! {
    #[derive(Debug, PartialEq, Clone)]
    struct DllHookResolution {
        start_recording_replay: StartRecordingReplay,
        stop_recording_replay: StopRecordingReplay,
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

static mut GLOBALS: Option<Globals> = None;

struct Globals {
    resolution: DllHookResolution,
    guobject_array: parking_lot::FairMutex<&'static ue::FUObjectArray>,
    main_thread_id: std::thread::ThreadId,
}

impl Globals {
    fn gmalloc(&self) -> &ue::FMalloc {
        unsafe { &**(self.resolution.gmalloc.0 as *const *const ue::FMalloc) }
    }
    pub fn fframe_step(&self) -> ue::FnFFrame_Step {
        unsafe { std::mem::transmute(self.resolution.fframe_step.0) }
    }
    pub fn fframe_step_explicit_property(&self) -> ue::FnFFrame_StepExplicitProperty {
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
    unsafe { GLOBALS.as_ref().unwrap() }
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
    let exe = patternsleuth::process::internal::read_image()?;

    info!("starting scan");
    let resolution = exe.resolve(DllHookResolution::resolver())?;
    info!("finished scan");

    info!("results: {:?}", resolution);

    let guobject_array: &'static ue::FUObjectArray =
        &*(resolution.guobject_array.0 as *const ue::FUObjectArray);

    GLOBALS = Some(Globals {
        guobject_array: guobject_array.into(),
        resolution,
        main_thread_id: std::thread::current().id(),
    });

    hooks::initialize()?;

    info!("initialized");

    app::run(bin_dir)
}
