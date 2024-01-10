use std::path::Path;

use anyhow::Result;
use simple_log::info;

use crate::{globals, gui, guobject_array_unchecked, ue};

pub fn run(_bin_dir: impl AsRef<Path>) -> Result<()> {
    std::thread::spawn(move || {
        //unsafe { testing(); }
        //gui::run().unwrap();
    });
    Ok(())
}

unsafe fn testing() {
    type FnFNameToString = unsafe extern "system" fn(&ue::FName, &mut ue::FString);

    let fnametostring: FnFNameToString = std::mem::transmute(globals().resolution.fnametostring.0);

    loop {
        info!("a");
        let objects = guobject_array_unchecked().objects();
        let refs = objects
            .iter()
            .filter(|obj| {
                if let Some(obj) = obj {
                    let mut name = ue::FString::default();
                    fnametostring(&obj.NamePrivate, &mut name);
                    name.to_string().to_ascii_lowercase().contains("get")
                } else {
                    false
                }
            })
            .collect::<Vec<_>>();
        for (i, obj) in refs.iter().enumerate() {
            if let Some(obj) = obj {
                let mut name = ue::FString::default();
                fnametostring(&obj.NamePrivate, &mut name);

                let mut class = ue::FString::default();
                fnametostring(
                    &(*obj.ClassPrivate)
                        .UStruct
                        .UField
                        .UObject
                        .UObjectBaseUtility
                        .UObjectBase
                        .NamePrivate,
                    &mut class,
                );
                let class = class.to_string();

                if class == "Function" {
                    // TODO safe casting
                    let s = &*((*obj as *const _) as *const ue::UStruct);
                    if s.Script.num > 0 {
                        info!("{:x?}", s.Script);
                        info!("{i:10} {} {}", class, name.to_string());
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}
