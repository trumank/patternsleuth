use std::sync::{
    mpsc::{Receiver, SyncSender},
    Arc, Mutex, OnceLock, RwLock,
};

use eframe::egui;

#[cfg(windows)]
use egui_winit::winit::platform::windows::EventLoopBuilderExtWindows;
#[cfg(unix)]
use egui_winit::winit::platform::x11::EventLoopBuilderExtX11;
use indexmap::IndexMap;

use super::*;

pub type GuiFn = Box<dyn FnOnce() -> GuiRet + Send + Sync>;
pub type GuiRet = ();

pub fn run(channels: (SyncSender<GuiFn>, Receiver<GuiRet>)) -> Result<(), eframe::Error> {
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
        Box::new(|_cc| Box::new(MyApp::new(channels))),
    )
}

type ObjectIndex = i32;
#[derive(Debug, Clone)]
struct ObjectCache {
    name: String,
    //weak_ptr: FWeakObjectPtr,
}

#[derive(Debug)]
enum Event {
    CreateUObject(ObjectIndex, ObjectCache),
    DeleteUObject(ObjectIndex),
    KismetMessage {
        message: String,
        verbosity: u8,
        warning_id: ue::FName,
    },
    KismetPrintMessage {
        message: String,
    },
}

#[allow(unused)]
struct Listeners {
    create_uobject: Arc<dyn Fn(&ue::UObjectBase)>,
    delete_uobject: Arc<dyn Fn(&ue::UObjectBase)>,
    kismet_message: Arc<dyn Fn(&widestring::U16CStr, u8, ue::FName)>,
    kismet_print_message: Arc<dyn Fn(&str)>,
}

struct ObjectFilter {
    name_search: String,
}
impl ObjectFilter {
    fn matches(&self, object: &ObjectCache) -> bool {
        if self.name_search.is_empty() {
            true
        } else {
            object.name.to_ascii_lowercase().contains(&self.name_search)
        }
    }
}

#[derive(Default, Clone)]
struct ObjectNameCache {
    names: IndexMap<ObjectIndex, ObjectCache>,
}
impl ObjectNameCache {
    fn get(&self, index: ObjectIndex) -> Option<&ObjectCache> {
        self.names.get(&index)
    }
    fn remove(&mut self, index: ObjectIndex) {
        self.names.remove(&index);
    }
    fn get_or_init<'a>(&'a mut self, object: &ue::UObjectBase) -> &'a ObjectCache {
        self.names
            .entry(object.internal_index)
            .or_insert_with(|| ObjectCache {
                name: object.name_private.to_string(),
            })
    }
}

struct MyApp {
    tx_ui: SyncSender<GuiFn>,
    rx_ui: Receiver<GuiRet>,
    inner_state: Arc<Mutex<InnerState>>,
}

#[derive(Default)]
struct InnerState {
    buh: i32,
}

macro_rules! move_clone {
    ( ( $($($arg:ident)+$(,)?)* ), $expr:expr) => {
        {
            $( $(
                    let $arg = $arg.clone();
            )*)*
            $expr
        }
    };
}

impl MyApp {
    fn new((tx_ui, rx_ui): (SyncSender<GuiFn>, Receiver<GuiRet>)) -> Self {
        // let (tx, events) = std::sync::mpsc::channel();
        // let ctx: Arc<OnceLock<egui::Context>> = Default::default();
        // let cache: Arc<RwLock<ObjectNameCache>> = Default::default();

        // let create_uobject = move_clone!(
        //     (tx, ctx, cache),
        //     Arc::new(move |object: &ue::UObjectBase| {
        //         //info!("before create_uobject");
        //         cache.write().unwrap().get_or_init(object);
        //         tx.send(Event::CreateUObject(
        //             object.internal_index,
        //             ObjectCache {
        //                 name: object.name_private.to_string(),
        //             },
        //         ))
        //         .unwrap();
        //         if let Some(ctx) = ctx.get() {
        //             ctx.request_repaint();
        //         }
        //     })
        // );

        // let delete_uobject = move_clone!(
        //     (tx, ctx, cache),
        //     Arc::new(move |object: &ue::UObjectBase| {
        //         //info!("before delete_uobject");
        //         cache.write().unwrap().remove(object.internal_index);
        //         tx.send(Event::DeleteUObject(object.internal_index))
        //             .unwrap();
        //         if let Some(ctx) = ctx.get() {
        //             ctx.request_repaint();
        //         }
        //     })
        // );
        // let kismet_message = move_clone!(
        //     (tx, ctx),
        //     Arc::new(
        //         move |message: &widestring::U16CStr, verbosity: u8, warning_id: ue::FName| {
        //             tx.send(Event::KismetMessage {
        //                 message: message.to_string().unwrap(),
        //                 verbosity,
        //                 warning_id,
        //             })
        //             .unwrap();
        //             if let Some(ctx) = ctx.get() {
        //                 ctx.request_repaint();
        //             }
        //         },
        //     )
        // );
        // let txc = tx.clone();
        // let kismet_print = Arc::new(move |message: &str| {
        //     txc.send(Event::KismetPrintMessage {
        //         message: message.into(),
        //     })
        //     .unwrap();
        // });
        Self {
            tx_ui,
            rx_ui,
            inner_state: Arc::new(Mutex::new(InnerState::default())),
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(egui::Visuals::dark());

        let ctx = ctx.clone();
        let state = self.inner_state.clone();
        self.tx_ui
            .send(Box::new(move || {
                let mut state = state.lock().unwrap();

                egui::CentralPanel::default().show(&ctx, |ui| {
                    ui.heading("running in main thread");
                    assert_main_thread!();

                    if ui.button("buhton").clicked() {
                        state.buh += 1;
                    }
                    ui.label(format!("counter: {}", state.buh));
                });
            }))
            .unwrap();
        self.rx_ui.recv().unwrap();

        //let object_lock = guobject_array();

        // self.ctx.get_or_init(|| ctx.clone());

        // for event in self.events.try_iter() {
        //     match event {
        //         Event::CreateUObject(index, object) => {
        //             if self.filter.matches(&object) {
        //                 self.filtered.insert(index, object.clone());
        //             }
        //         }
        //         Event::DeleteUObject(index) => {
        //             self.filtered.remove(&index);
        //         }
        //         Event::KismetMessage {
        //             message,
        //             verbosity: _,
        //             warning_id: _,
        //         } => {
        //             self.kismet_log.push_str(&format!("Kismet VM: {message}\n"));
        //         }
        //         Event::KismetPrintMessage { message } => {
        //             self.kismet_log
        //                 .push_str(&format!("PrintString: {message}\n"));
        //         }
        //     };
        // }

        // ctx.set_visuals(egui::Visuals::dark());

        // egui::CentralPanel::default().show(ctx, |ui| {
        //     ui.heading("My egui Application");

        //     ui.horizontal(|ui| {
        //         let name_label = ui.label("Search: ");
        //         let res = ui
        //             .text_edit_singleline(&mut self.filter.name_search)
        //             .labelled_by(name_label.id);
        //         if res.changed() {
        //             self.filtered = self
        //                 .objects
        //                 .read()
        //                 .unwrap()
        //                 .names
        //                 .iter()
        //                 .filter_map(|(index, obj)| {
        //                     if self.filter.matches(obj) {
        //                         Some((*index, obj.clone()))
        //                     } else {
        //                         None
        //                     }
        //                 })
        //                 .collect::<IndexMap<_, _>>();
        //         }
        //     });

        //     let text_style = egui::TextStyle::Body;
        //     let row_height = ui.text_style_height(&text_style);

        //     egui::ScrollArea::vertical().show_rows(
        //         ui,
        //         row_height,
        //         self.filtered.len(),
        //         |ui, row_range| {
        //             for (i, obj) in self
        //                 .filtered
        //                 .iter()
        //                 .skip(row_range.start)
        //                 .take(row_range.len())
        //             {
        //                 ui.label(format!("{i:10} {}", obj.name));
        //             }
        //             ui.allocate_space(ui.available_size());
        //         },
        //     );

        // egui::Window::new("object search 2")
        //     .default_height(500.)
        //     .show(ctx, |ui| {
        //         let name_label = ui.label("Search: ");
        //         let _res = ui
        //             .text_edit_singleline(&mut self.filter2)
        //             .labelled_by(name_label.id);

        //         let text_style = egui::TextStyle::Body;
        //         let row_height = ui.text_style_height(&text_style);

        //         //info!("before names lock");
        //         let objects = unsafe { globals().guobject_array_unchecked() }.objects();
        //         let mut names = self.objects.write().unwrap();

        //         //info!("before filter");
        //         let filtered = objects
        //             .iter()
        //             //.take(100)
        //             .flatten()
        //             .filter(|obj| {
        //                 let cached = &names.get_or_init(obj);
        //                 cached.name.contains(&self.filter2)
        //             })
        //             .collect::<Vec<_>>();
        //         //let filtered = vec!["h"];

        //         //info!("before print");
        //         egui::ScrollArea::vertical().show_rows(
        //             ui,
        //             row_height,
        //             filtered.len(),
        //             |ui, row_range| {
        //                 for (i, obj) in filtered
        //                     .iter()
        //                     .enumerate()
        //                     .skip(row_range.start)
        //                     .take(row_range.len())
        //                 {
        //                     ui.label(format!(
        //                         "{i:10} {}",
        //                         names.get(obj.internal_index).unwrap().name
        //                     ));
        //                 }
        //                 ui.allocate_space(ui.available_size());
        //             },
        //         );
        //     });

        // let _log_window = |name: &str, mut log: &str| {
        //     egui::Window::new(name)
        //         .default_height(500.)
        //         .show(ctx, |ui| {
        //             egui::ScrollArea::vertical()
        //                 .stick_to_bottom(true)
        //                 .show(ui, |ui| {
        //                     ui.add(
        //                         egui::TextEdit::multiline(&mut log)
        //                             .desired_width(f32::INFINITY)
        //                             .desired_rows(10)
        //                             .font(egui::TextStyle::Monospace),
        //                     );
        //                 });
        //         });
        // };

        //log_window("Kismet Messages", &self.kismet_log);
        // });
    }
}
