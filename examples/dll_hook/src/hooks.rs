use std::{
    ffi::c_void,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use anyhow::Result;
use simple_log::info;

use crate::{object_cache, res, ue};

retour::static_detour! {
    static HookUGameEngineTick: unsafe extern "system" fn(*mut c_void, f32, u8);
    static HookAllocateUObject: unsafe extern "system" fn(*mut c_void, *const ue::UObjectBase, bool);
    static HookFreeUObject: unsafe extern "system" fn(*mut ue::UObjectBase, *const c_void); // inlined into UObject dtor so args are messed up
    static HookKismetPrintString: unsafe extern "system" fn(*mut ue::UObjectBase, *mut ue::kismet::FFrame, *mut c_void);
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

event!(create_uobject(object: &ue::UObjectBase));
event!(delete_uobject(object: &ue::UObjectBase));

pub unsafe fn initialize() -> Result<()> {
    HookUGameEngineTick.initialize(
        std::mem::transmute(res().game_tick.0),
        move |game_engine, delta_seconds, idle_mode| {
            //info!("tick time={:0.5}", delta_seconds);
            HookUGameEngineTick.call(game_engine, delta_seconds, idle_mode);
        },
    )?;
    HookUGameEngineTick.enable()?;

    HookAllocateUObject.initialize(
        std::mem::transmute(res().allocate_uobject.0),
        |this, object, merging_threads| {
            HookAllocateUObject.call(this, object, merging_threads);
            object_cache::object_created(&*object);
            create_uobject::call(&*object);
        },
    )?;
    HookAllocateUObject.enable()?;

    HookFreeUObject.initialize(std::mem::transmute(res().free_uobject.0), |this, object| {
        object_cache::object_deleted(&*this);
        delete_uobject::call(&*this);
        HookFreeUObject.call(this, object);
    })?;
    HookFreeUObject.enable()?;

    HookKismetPrintString.initialize(
        std::mem::transmute(*res().kismet_system_library.0.get("PrintString").unwrap()),
        |context, stack, result| {
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

            let s = string.to_string();
            info!("PrintString({s:?})");

            if !stack.code.is_null() {
                stack.code = stack.code.add(1);
            }
        },
    )?;
    HookKismetPrintString.enable()?;

    Ok(())
}
