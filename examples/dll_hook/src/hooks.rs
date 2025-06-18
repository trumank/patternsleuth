use std::{
    ffi::c_void,
    sync::{Arc, LazyLock, Mutex, Weak},
};

use anyhow::Result;

use crate::{
    assert_main_thread, globals,
    object_cache::{ObjectEvent, ObjectRegistry, UObjectPtr},
    ue,
};

retour::static_detour! {
    static HookUGameEngineTick: unsafe extern "system" fn(*mut c_void, f32, u8);
    static HookFEngineLoopInit: unsafe extern "system" fn(*mut c_void);
    static HookAllocateUObject: unsafe extern "system" fn(*mut c_void, *const ue::UObjectBase, bool);
    static HookFreeUObject: unsafe extern "system" fn(*mut ue::UObjectBase, *const c_void); // inlined into UObject dtor so args are messed up
    static HookKismetPrintString: unsafe extern "system" fn(*mut ue::UObjectBase, *mut ue::kismet::FFrame, *mut c_void);
    static HookKismetExecutionMessage: unsafe extern "system" fn(*const u16, u8, ue::FName);
    static HookUFunctionBind: unsafe extern "system" fn(*mut ue::UFunction);
}

pub type UObjectLock = parking_lot::FairMutexGuard<'static, &'static ue::FUObjectArray>;
static mut GUOBJECT_LOCK: Option<UObjectLock> = None;

pub struct GameTick(pub Vec<ObjectEvent>);
pub struct CreateUObject<'a>(pub &'a ue::UObjectBase);
pub struct DeleteUObject<'a>(pub &'a ue::UObjectBase);

pub unsafe fn initialize() -> Result<()> {
    assert_main_thread!();

    GUOBJECT_LOCK = Some(globals().guobject_array());

    HookFEngineLoopInit.initialize(
        std::mem::transmute(globals().resolution.engine_loop_init.0),
        move |engine_loop| {
            assert_main_thread!();

            HookFEngineLoopInit.call(engine_loop);
            simple_log::info!("ENGINE LOOP INIT");
        },
    )?;
    HookFEngineLoopInit.enable()?;

    HookUGameEngineTick.initialize(
        std::mem::transmute(globals().resolution.game_tick.0),
        move |game_engine, delta_seconds, idle_mode| {
            assert_main_thread!();

            // info!("tick time={:0.5}", delta_seconds);

            HookUGameEngineTick.call(game_engine, delta_seconds, idle_mode);
            let events = ObjectRegistry::drain_events();
            crate::events::fire(GameTick(events));
        },
    )?;
    HookUGameEngineTick.enable()?;

    HookAllocateUObject.initialize(
        std::mem::transmute(globals().resolution.allocate_uobject.0),
        |this, object, merging_threads| {
            HookAllocateUObject.call(this, object, merging_threads);
            ObjectRegistry::on_object_created(object as *mut _);
        },
    )?;
    HookAllocateUObject.enable()?;

    HookFreeUObject.initialize(
        std::mem::transmute(globals().resolution.free_uobject.0),
        |this, object| {
            ObjectRegistry::on_object_deleted(this as *mut _);
            HookFreeUObject.call(this, object);
        },
    )?;
    HookFreeUObject.enable()?;

    // HookKismetPrintString.initialize(
    //     std::mem::transmute(
    //         *globals()
    //             .resolution
    //             .kismet_system_library
    //             .0
    //             .get("PrintString")
    //             .unwrap(),
    //     ),
    //     |_context, stack, _result| {
    //         let stack = &mut *stack;

    //         let mut ctx: Option<&ue::UObject> = None;
    //         let mut string = ue::FString::default();
    //         let mut print_to_screen = false;
    //         let mut print_to_log = false;
    //         let mut color = ue::FLinearColor::default();
    //         let mut duration = 0f32;

    //         ue::kismet::arg(stack, &mut ctx);
    //         ue::kismet::arg(stack, &mut string);
    //         ue::kismet::arg(stack, &mut print_to_screen);
    //         ue::kismet::arg(stack, &mut print_to_log);
    //         ue::kismet::arg(stack, &mut color);
    //         ue::kismet::arg(stack, &mut duration);

    //         //let s = string.to_string();
    //         //info!("PrintString({s:?})");
    //         kismet_print_message::call(&string.to_string());

    //         if !stack.code.is_null() {
    //             stack.code = stack.code.add(1);
    //         }
    //     },
    // )?;
    // HookKismetPrintString.enable()?;

    // HookKismetExecutionMessage.initialize(
    //     std::mem::transmute(globals().resolution.fframe_kismet_execution_message.0),
    //     |message, verbosity, warning_id| {
    //         kismet_execution_message::call(
    //             widestring::U16CStr::from_ptr_str(message),
    //             verbosity,
    //             warning_id,
    //         );
    //         HookKismetExecutionMessage.call(message, verbosity, warning_id);
    //     },
    // )?;
    // HookKismetExecutionMessage.enable()?;

    type ExecFn = unsafe extern "system" fn(*mut ue::UObject, *mut ue::kismet::FFrame, *mut c_void);

    let hooks = [
        (
            "/Game/_AssemblyStorm/TestMod/BPL_NativeTest.BPL_NativeTest_C:Do Stuff",
            do_stuff as ExecFn,
        ),
        (
            "/Game/_AssemblyStorm/TestMod/BPL_NativeTest.BPL_NativeTest_C:Regex",
            exec_regex as ExecFn,
        ),
    ]
    .into_iter()
    .collect::<std::collections::HashMap<_, ExecFn>>();

    HookUFunctionBind.initialize(
        std::mem::transmute(globals().resolution.ufunction_bind.0),
        move |function| {
            HookUFunctionBind.call(function);
            if let Some(function) = function.as_mut() {
                // let path = function
                //     .ustruct
                //     .ufield
                //     .uobject
                //     .uobject_base_utility
                //     .uobject_base
                //     .get_path_name(None);
                // if let Some(hook) = hooks.get(path.as_str()) {
                //     simple_log::info!(
                //         "UFunction::Bind({path}) func = {:?} flags = {:?}",
                //         function.func,
                //         function.function_flags
                //     );
                //     function
                //         .function_flags
                //         .insert(ue::EFunctionFlags::FUNC_Native | ue::EFunctionFlags::FUNC_Final);
                //     function.func = *hook;
                // }
            }
        },
    )?;
    HookUFunctionBind.enable()?;

    Ok(())
}

unsafe extern "system" fn do_stuff(
    _context: *mut ue::UObject,
    stack: *mut ue::kismet::FFrame,
    _result: *mut c_void,
) {
    let stack = stack.as_mut().unwrap();
    let mut ctx: Option<&ue::UObject> = None;
    ue::kismet::arg(stack, &mut ctx);

    simple_log::info!("doing stuff!!");

    stack.code = stack.code.add(1);
}

unsafe extern "system" fn exec_regex(
    _context: *mut ue::UObject,
    stack: *mut ue::kismet::FFrame,
    _result: *mut c_void,
) {
    let stack = stack.as_mut().unwrap();

    let mut ctx: Option<&ue::UObject> = None;
    let mut regex = ue::FString::default();
    let mut input = ue::FString::default();
    let mut matches: ue::TArray<ue::FString> = Default::default();

    ue::kismet::arg(stack, &mut regex);
    ue::kismet::arg(stack, &mut input);
    ue::kismet::arg(stack, &mut ctx);
    ue::kismet::arg(stack, &mut matches);
    let matches_address = (stack.most_recent_property_address as *mut ue::TArray<ue::FString>)
        .as_mut()
        .unwrap();

    matches_address.clear();
    if let Ok(re) = regex::Regex::new(&regex.to_string()) {
        for cap in re.captures(&input.to_string()).iter() {
            for cap in cap.iter() {
                let new_str = ue::FString::from(
                    widestring::U16CString::from_str(
                        cap.as_ref().map(|m| m.as_str()).unwrap_or_default(),
                    )
                    .unwrap()
                    .as_slice_with_nul(),
                );
                matches_address.push(new_str);
            }
        }
    }

    std::mem::forget(matches);

    stack.code = stack.code.add(1);
}
