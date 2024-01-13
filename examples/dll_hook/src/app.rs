use std::path::Path;

use anyhow::Result;
use simple_log::info;

use crate::{globals, ue};

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
                    obj.name_private
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
                let name = obj.name_private.to_string();

                let class = &(*obj.class_private)
                    .ustruct
                    .ufield
                    .uobject
                    .uobject_base_utility
                    .uobject_base
                    .name_private
                    .to_string();

                if class == "Function" {
                    // TODO safe casting
                    let s = &*((*obj as *const _) as *const ue::UStruct);
                    if !s.script.is_empty() {
                        info!("{:x?}", s.script);
                        info!("{i:10} {} {}", class, name.to_string());
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}
