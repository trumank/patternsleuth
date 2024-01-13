use std::path::Path;

use anyhow::Result;
use simple_log::info;

use crate::{globals, gui, ue};

pub fn run(_bin_dir: impl AsRef<Path>) -> Result<()> {
    std::thread::spawn(move || {
        //unsafe { testing(); }
        //gui::run().unwrap();
    });
    Ok(())
}

unsafe fn testing() {
    loop {
        info!("a");
        let objects = globals().guobject_array_unchecked().objects();
        let refs = objects
            .iter()
            .filter(|obj| {
                if let Some(obj) = obj {
                    obj.NamePrivate
                        .to_string()
                        .to_ascii_lowercase()
                        .contains("get")
                } else {
                    false
                }
            })
            .collect::<Vec<_>>();
        for (i, obj) in refs.iter().enumerate() {
            if let Some(obj) = obj {
                let name = obj.NamePrivate.to_string();

                let class = &(*obj.ClassPrivate)
                    .UStruct
                    .UField
                    .UObject
                    .UObjectBaseUtility
                    .UObjectBase
                    .NamePrivate
                    .to_string();

                if class == "Function" {
                    // TODO safe casting
                    let s = &*((*obj as *const _) as *const ue::UStruct);
                    if !s.Script.is_empty() {
                        info!("{:x?}", s.Script);
                        info!("{i:10} {} {}", class, name.to_string());
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}
