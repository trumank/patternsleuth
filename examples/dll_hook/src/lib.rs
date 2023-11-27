use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use patternsleuth::resolvers::unreal::{ConsoleManagerSingleton, FNameToString, GUObjectArray};
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
        .build();
    simple_log::new(config).map_err(|e| anyhow!("{e}"))?;
    Ok(bin_dir.to_path_buf())
}

#[derive(Debug)]
pub struct DllHookResolution {
    guobject_array: Arc<GUObjectArray>,
    fnametostring: Arc<FNameToString>,
    console_manager_singleton: Arc<ConsoleManagerSingleton>,
}

mod resolvers {
    use super::DllHookResolution;

    use patternsleuth::resolvers::{
        futures::try_join,
        unreal::{ConsoleManagerSingleton, FNameToString, GUObjectArray},
        *,
    };

    impl_resolver!(DllHookResolution, |ctx| async {
        let (guobject_array, fnametostring, console_manager_singleton) = try_join!(
            ctx.resolve(GUObjectArray::resolver()),
            ctx.resolve(FNameToString::resolver()),
            ctx.resolve(ConsoleManagerSingleton::resolver()),
        )?;
        Ok(DllHookResolution {
            guobject_array,
            fnametostring,
            console_manager_singleton,
        })
    });
}

unsafe fn patch(bin_dir: PathBuf) -> Result<()> {
    let exe = patternsleuth::process::internal::read_image()?;

    info!("starting scan");
    let resolution = exe.resolve(DllHookResolution::resolver())?;
    info!("finished scan");

    info!("results: {:?}", resolution);

    info!("done executing");

    std::thread::spawn(move || {
        gui::main(resolution).unwrap();
    });

    Ok(())
}

mod gui {

    use eframe::egui;

    use egui_winit::winit::platform::windows::EventLoopBuilderExtWindows;

    use super::*;

    pub fn main(resolution: DllHookResolution) -> Result<(), eframe::Error> {
        let event_loop_builder: Option<eframe::EventLoopBuilderHook> =
            Some(Box::new(|event_loop_builder| {
                event_loop_builder.with_any_thread(true);
            }));
        let options = eframe::NativeOptions {
            event_loop_builder,
            viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
            ..Default::default()
        };
        eframe::run_native(
            "My egui App",
            options,
            Box::new(|cc| Box::new(MyApp::new(resolution))),
        )
    }

    struct MyApp {
        resolution: DllHookResolution,
        search: String,
    }

    impl MyApp {
        fn new(resolution: DllHookResolution) -> Self {
            Self {
                resolution,
                search: "".to_owned(),
            }
        }
    }

    impl eframe::App for MyApp {
        fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            ctx.set_visuals(egui::Visuals::dark());

            unsafe {
                let guobjectarray =
                    &*(self.resolution.guobject_array.0 as *const ue::FUObjectArray);
                type FnFNameToString = unsafe extern "system" fn(&ue::FName, &mut ue::FString);

                let fnametostring: FnFNameToString =
                    std::mem::transmute(self.resolution.fnametostring.0);

                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.heading("My egui Application");
                    ui.horizontal(|ui| {
                        let name_label = ui.label("Search: ");
                        ui.text_edit_singleline(&mut self.search)
                            .labelled_by(name_label.id);
                    });

                    let text_style = egui::TextStyle::Body;
                    let row_height = ui.text_style_height(&text_style);

                    let (total_rows, refs) = if self.search.is_empty() {
                        (guobjectarray.ObjObjects.NumElements as usize, None)
                    } else {
                        let search = self.search.to_ascii_lowercase();
                        let refs = guobjectarray
                            .iter()
                            .filter(|obj| {
                                if let Some(obj) = obj {
                                    let mut name = ue::FString::default();
                                    fnametostring(&obj.NamePrivate, &mut name);
                                    name.to_os_string()
                                        .to_string_lossy()
                                        .to_ascii_lowercase()
                                        .contains(&search)
                                } else {
                                    false
                                }
                            })
                            .collect::<Vec<_>>();
                        (refs.len(), Some(refs))
                    };

                    egui::ScrollArea::vertical().show_rows(
                        ui,
                        row_height,
                        total_rows,
                        |ui, row_range| {
                            let iter = if let Some(refs) = &refs {
                                itertools::Either::Left(refs.iter().map(|o| *o))
                            } else {
                                itertools::Either::Right(guobjectarray.iter())
                            };
                            for (i, obj) in
                                iter.enumerate().skip(row_range.start).take(row_range.len())
                            {
                                if let Some(obj) = obj {
                                    let mut name = ue::FString::default();
                                    fnametostring(&obj.NamePrivate, &mut name);
                                    ui.label(format!(
                                        "{i:10} {}",
                                        name.to_os_string().to_string_lossy()
                                    ));
                                } else {
                                    ui.label(format!("{i:10} null"));
                                }
                            }
                            ui.allocate_space(ui.available_size());
                        },
                    );
                });
            }
        }
    }
}

mod ue {
    use std::ffi::OsString;

    #[derive(Debug)]
    #[repr(C)]
    pub struct FUObjectArray {
        /* offset 0x0000 */ pub ObjFirstGCIndex: i32,
        /* offset 0x0004 */ pub ObjLastNonGCIndex: i32,
        /* offset 0x0008 */ pub MaxObjectsNotConsideredByGC: i32,
        /* offset 0x000c */ pub OpenForDisregardForGC: bool,
        /* offset 0x0010 */
        pub ObjObjects: FChunkedFixedUObjectArray,
        /* offset 0x0030 */ //FWindowsCriticalSection ObjObjectsCritical;
        /* offset 0x0058 */ //TArray<int,TSizedDefaultAllocator<32> > ObjAvailableList;
        /* offset 0x0068 */ //TArray<FUObjectArray::FUObjectCreateListener *,TSizedDefaultAllocator<32> > UObjectCreateListeners;
        /* offset 0x0078 */ //TArray<FUObjectArray::FUObjectDeleteListener *,TSizedDefaultAllocator<32> > UObjectDeleteListeners;
        /* offset 0x0088 */ //FWindowsCriticalSection UObjectDeleteListenersCritical;
        /* offset 0x00b0 */ //FThreadSafeCounter MasterSerialNumber;
    }
    impl FUObjectArray {
        pub fn iter(&self) -> ObjectIterator<'_> {
            ObjectIterator {
                array: self,
                index: 0,
            }
        }
    }

    pub struct ObjectIterator<'a> {
        array: &'a FUObjectArray,
        index: i32,
    }
    impl<'a> Iterator for ObjectIterator<'a> {
        type Item = Option<&'a UObjectBase>;
        fn size_hint(&self) -> (usize, Option<usize>) {
            let size = self.array.ObjObjects.NumElements as usize;
            (size, Some(size))
        }
        fn nth(&mut self, n: usize) -> Option<Self::Item> {
            let n = n as i32;
            if self.index < n {
                self.index = n;
            }
            self.next()
        }
        fn next(&mut self) -> Option<Option<&'a UObjectBase>> {
            if self.index >= self.array.ObjObjects.NumElements {
                None
            } else {
                let per_chunk = self.array.ObjObjects.MaxElements / self.array.ObjObjects.MaxChunks;

                let obj = unsafe {
                    let chunk = *self
                        .array
                        .ObjObjects
                        .Objects
                        .add((self.index / per_chunk) as usize);
                    let item = &*chunk.add((self.index % per_chunk) as usize);
                    item.Object.as_ref()
                };

                self.index += 1;
                Some(obj)
            }
        }
    }

    #[derive(Debug)]
    #[repr(C)]
    pub struct FChunkedFixedUObjectArray {
        /* offset 0x0000 */ pub Objects: *const *const FUObjectItem,
        /* offset 0x0008 */ pub PreAllocatedObjects: *const FUObjectItem,
        /* offset 0x0010 */ pub MaxElements: i32,
        /* offset 0x0014 */ pub NumElements: i32,
        /* offset 0x0018 */ pub MaxChunks: i32,
        /* offset 0x001c */ pub NumChunks: i32,
    }

    #[derive(Debug)]
    #[repr(C)]
    pub struct FUObjectItem {
        /* offset 0x0000 */ pub Object: *const UObjectBase,
        /* offset 0x0008 */ pub Flags: i32,
        /* offset 0x000c */ pub ClusterRootIndex: i32,
        /* offset 0x0010 */ pub SerialNumber: i32,
    }

    pub type EObjectFlags = u32; // TODO
    pub type UClass = (); // TODO
    pub type UObject = (); // TODO

    #[derive(Debug)]
    #[repr(C)]
    pub struct UObjectBase {
        pub vftable: *const std::ffi::c_void,
        /* offset 0x0008 */ pub ObjectFlags: EObjectFlags,
        /* offset 0x000c */ pub InternalIndex: i32,
        /* offset 0x0010 */ pub ClassPrivate: *const UClass,
        /* offset 0x0018 */ pub NamePrivate: FName,
        /* offset 0x0020 */ pub OuterPrivate: *const UObject,
    }

    #[derive(Debug)]
    #[repr(C)]
    pub struct FName {
        /* offset 0x0000 */ pub ComparisonIndex: FNameEntryId,
        /* offset 0x0004 */ pub Number: u32,
    }

    #[derive(Debug)]
    #[repr(C)]
    pub struct FNameEntryId {
        /* offset 0x0000 */ pub Value: u32,
    }

    pub type FString = TArray<u16>;

    #[derive(Debug)]
    #[repr(C)]
    pub struct TArray<T> {
        data: *const T,
        num: i32,
        max: i32,
    }
    impl<T> Default for TArray<T> {
        fn default() -> Self {
            Self {
                data: std::ptr::null(),
                num: 0,
                max: 0,
            }
        }
    }
    impl<T> TArray<T> {
        pub fn as_slice(&self) -> &[T] {
            unsafe { std::slice::from_raw_parts(self.data, self.num as usize) }
        }
        pub fn as_slice_mut(&mut self) -> &mut [T] {
            unsafe { std::slice::from_raw_parts_mut(self.data as *mut _, self.num as usize) }
        }
        pub fn from_slice(slice: &[T]) -> TArray<T> {
            TArray {
                data: slice.as_ptr(),
                num: slice.len() as i32,
                max: slice.len() as i32,
            }
        }
    }

    impl FString {
        pub fn to_os_string(&self) -> OsString {
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::ffi::OsStringExt;
                let slice = self.as_slice();
                let len = slice
                    .iter()
                    .enumerate()
                    .find_map(|(i, &b)| (b == 0).then_some(i))
                    .unwrap_or(slice.len());
                std::ffi::OsString::from_wide(&slice[0..len])
            }
            #[cfg(not(target_os = "windows"))]
            unimplemented!()
        }
    }
}
