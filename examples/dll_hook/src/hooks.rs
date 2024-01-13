use std::{
    ffi::c_void,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use anyhow::Result;

use crate::{assert_main_thread, globals, object_cache, ue};

retour::static_detour! {
    static HookUGameEngineTick: unsafe extern "system" fn(*mut c_void, f32, u8);
    static HookFEngineLoopInit: unsafe extern "system" fn(*mut c_void);
    static HookAllocateUObject: unsafe extern "system" fn(*mut c_void, *const ue::UObjectBase, bool);
    static HookFreeUObject: unsafe extern "system" fn(*mut ue::UObjectBase, *const c_void); // inlined into UObject dtor so args are messed up
    static HookKismetPrintString: unsafe extern "system" fn(*mut ue::UObjectBase, *mut ue::kismet::FFrame, *mut c_void);
    static HookKismetExecutionMessage: unsafe extern "system" fn(*const u16, u8, ue::FName);
    static HookUFunctionBind: unsafe extern "system" fn(*mut ue::UFunction);
}

macro_rules! event {
    ($name:ident ( $($($arg_name:ident: $arg_ty:ty)+$(,)?)* ) ) => {
        pub mod $name {
            use super::*;

            pub type Listener = dyn Fn( $($($arg_ty,)*)* ) + Send + Sync;
            fn get() -> &'static Mutex<Vec<Weak<Listener>>> {
                static OBJECTS: OnceLock<Mutex<Vec<Weak<Listener>>>> = OnceLock::new();
                OBJECTS.get_or_init(|| Default::default())
            }
            pub fn register(listener: Arc<Listener>) -> Arc<Listener> {
                get().lock().unwrap().push(Arc::downgrade(&listener));
                listener
            }
            pub fn call( $($($arg_name: $arg_ty,)*)* ) {
                get().lock().unwrap().retain(|f| {
                    if let Some(f) = f.upgrade() {
                        f( $($($arg_name,)*)* );
                        true
                    } else {
                        false
                    }
                });
            }
        }
    };
}

event!(create_uobject(/*uobject_array: &UObjectLock,*/ object: &ue::UObjectBase));
event!(delete_uobject(/*uobject_array: &UObjectLock,*/ object: &ue::UObjectBase));
event!(kismet_execution_message(message: &widestring::U16CStr, verbosity: u8, warning_id: ue::FName));
event!(kismet_print_message(message: &str));

pub type UObjectLock = parking_lot::FairMutexGuard<'static, &'static ue::FUObjectArray>;
static mut GUOBJECT_LOCK: Option<UObjectLock> = None;

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

            //info!("tick time={:0.5}", delta_seconds);

            GUOBJECT_LOCK.take();
            HookUGameEngineTick.call(game_engine, delta_seconds, idle_mode);
            GUOBJECT_LOCK = Some(globals().guobject_array());
        },
    )?;
    HookUGameEngineTick.enable()?;

    HookAllocateUObject.initialize(
        std::mem::transmute(globals().resolution.allocate_uobject.0),
        |this, object, merging_threads| {
            //assert_main_thread!();

            //info!("allocate uobject {:?}", object);

            HookAllocateUObject.call(this, object, merging_threads);

            object_cache::object_created(&*object);
            create_uobject::call(/*GUOBJECT_LOCK.as_ref().unwrap(),*/ &*object);
        },
    )?;
    HookAllocateUObject.enable()?;

    HookFreeUObject.initialize(
        std::mem::transmute(globals().resolution.free_uobject.0),
        |this, object| {
            //assert_main_thread!();

            //info!("delete uobject {:?}", object);

            object_cache::object_deleted(&*this);
            delete_uobject::call(/*GUOBJECT_LOCK.as_ref().unwrap(),*/ &*this);

            HookFreeUObject.call(this, object);
        },
    )?;
    HookFreeUObject.enable()?;

    HookKismetPrintString.initialize(
        std::mem::transmute(
            *globals()
                .resolution
                .kismet_system_library
                .0
                .get("PrintString")
                .unwrap(),
        ),
        |_context, stack, _result| {
            let stack = &mut *stack;

            let mut ctx: Option<&ue::UObject> = None;
            let mut string = ue::FString::default();
            let mut print_to_screen = false;
            let mut print_to_log = false;
            let mut color = ue::FLinearColor::default();
            let mut duration = 0f32;

            ue::kismet::arg(stack, &mut ctx);
            ue::kismet::arg(stack, &mut string);
            ue::kismet::arg(stack, &mut print_to_screen);
            ue::kismet::arg(stack, &mut print_to_log);
            ue::kismet::arg(stack, &mut color);
            ue::kismet::arg(stack, &mut duration);

            //let s = string.to_string();
            //info!("PrintString({s:?})");
            kismet_print_message::call(&string.to_string());

            if !stack.code.is_null() {
                stack.code = stack.code.add(1);
            }
        },
    )?;
    HookKismetPrintString.enable()?;

    HookKismetExecutionMessage.initialize(
        std::mem::transmute(globals().resolution.fframe_kismet_execution_message.0),
        |message, verbosity, warning_id| {
            kismet_execution_message::call(
                widestring::U16CStr::from_ptr_str(message),
                verbosity,
                warning_id,
            );
            HookKismetExecutionMessage.call(message, verbosity, warning_id);
        },
    )?;
    HookKismetExecutionMessage.enable()?;

    type execFn = unsafe extern "system" fn(*mut ue::UObject, *mut ue::kismet::FFrame, *mut c_void);

    let hooks = [
        (
            "/Game/_AssemblyStorm/TestMod/BPL_NativeTest.BPL_NativeTest_C:Do Stuff",
            do_stuff as execFn,
        ),
        (
            "/Game/_AssemblyStorm/TestMod/BPL_NativeTest.BPL_NativeTest_C:Regex",
            exec_regex as execFn,
        ),
    ]
    .into_iter()
    .collect::<std::collections::HashMap<_, execFn>>();

    HookUFunctionBind.initialize(
        std::mem::transmute(globals().resolution.ufunction_bind.0),
        move |function| {
            HookUFunctionBind.call(function);
            if let Some(function) = function.as_mut() {
                let path = ue::UObjectBase_GetPathName(
                    &function
                        .UStruct
                        .UField
                        .UObject
                        .UObjectBaseUtility
                        .UObjectBase,
                    None,
                );
                if let Some(hook) = hooks.get(path.as_str()) {
                    simple_log::info!(
                        "UFunction::Bind({path}) func = {:?} flags = {:?}",
                        function.Func,
                        function.FunctionFlags
                    );
                    function
                        .FunctionFlags
                        .insert(ue::EFunctionFlags::FUNC_Native | ue::EFunctionFlags::FUNC_Final);
                    function.Func = *hook;
                }
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
