use crossbeam_channel::{Receiver, Sender};
use std::cell::RefCell;

use eframe::egui;

#[cfg(windows)]
use egui_winit::winit::platform::windows::EventLoopBuilderExtWindows;
#[cfg(unix)]
use egui_winit::winit::platform::x11::EventLoopBuilderExtX11;
use indexmap::IndexMap;

use crate::object_cache::{ObjectEvent, ObjectId};

use super::*;

pub type GuiFn = Box<dyn FnOnce() -> GuiRet + Send + Sync>;
pub type GuiRet = ();

thread_local! {
    static STATE: RefCell<InnerState> = RefCell::new(InnerState::new());
}

pub fn init() {
    let (tx_main, rx_ui) = crossbeam_channel::bounded::<crate::gui::GuiRet>(0);
    let (tx_ui, rx_main) = crossbeam_channel::bounded::<crate::gui::GuiFn>(0);

    events::register(move |hooks::GameTick(events)| {
        STATE.with_borrow_mut(|s| {
            for e in events {
                match e {
                    ObjectEvent::Created { id } => {
                        let Some(ptr) = id.get_ptr() else {
                            continue;
                        };
                        let object = unsafe { &*ptr.0 };
                        let name = object.path();
                        let cache = ObjectCache { name };
                        if s.filter.matches(&cache) {
                            s.filtered.insert(*id, cache.clone());
                        }
                        s.objects.insert(*id, cache);
                    }
                    ObjectEvent::Deleted { id } => {
                        s.objects.remove(id);
                        s.filtered.remove(id);
                    }
                }
            }
        });

        if let Ok(f) = rx_main.try_recv() {
            #[allow(clippy::unit_arg)]
            tx_main.send(f()).unwrap();
        }
    });

    std::thread::spawn(move || run((tx_ui, rx_ui)).unwrap());
}

pub fn run(channels: (Sender<GuiFn>, Receiver<GuiRet>)) -> Result<(), eframe::Error> {
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

#[derive(Default)]
struct ObjectFilter {
    name_search: String,
}
impl ObjectFilter {
    fn matches(&self, object: &ObjectCache) -> bool {
        if self.name_search.is_empty() {
            true
        } else {
            object
                .name
                .to_ascii_lowercase()
                .contains(&self.name_search.to_ascii_lowercase())
        }
    }
}

struct MyApp {
    tx_ui: Sender<GuiFn>,
    rx_ui: Receiver<GuiRet>,
}

struct InnerState {
    buh: i32,
    filter: ObjectFilter,
    kismet_log: String,
    objects: IndexMap<ObjectId, ObjectCache>,
    filtered: IndexMap<ObjectId, ObjectCache>,
}
impl InnerState {
    fn new() -> Self {
        Self {
            buh: 0,
            filter: ObjectFilter::default(),
            filtered: Default::default(),
            objects: Default::default(),
            kismet_log: "".to_string(),
        }
    }
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
    fn new((tx_ui, rx_ui): (Sender<GuiFn>, Receiver<GuiRet>)) -> Self {
        Self { tx_ui, rx_ui }
    }
}
fn ui(state: &mut InnerState, ctx: &egui::Context) {
    assert_main_thread!();

    // for event in state.events.try_iter() {
    //     match event {
    //         Event::CreateUObject(index, object) => {
    //             if state.filter.matches(&object) {
    //                 state.filtered.insert(index, object.clone());
    //             }
    //         }
    //         Event::DeleteUObject(index) => {
    //             state.filtered.remove(&index);
    //         }
    //         Event::KismetMessage {
    //             message,
    //             verbosity: _,
    //             warning_id: _,
    //         } => {
    //             state
    //                 .kismet_log
    //                 .push_str(&format!("Kismet VM: {message}\n"));
    //         }
    //         Event::KismetPrintMessage { message } => {
    //             state
    //                 .kismet_log
    //                 .push_str(&format!("PrintString: {message}\n"));
    //         }
    //     };
    // }

    ctx.set_visuals(egui::Visuals::dark());

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("My egui Application");

        ui.horizontal(|ui| {
            let name_label = ui.label("Search: ");
            let res = ui
                .text_edit_singleline(&mut state.filter.name_search)
                .labelled_by(name_label.id);
            if res.changed() {
                state.filtered = state
                    .objects
                    .iter()
                    .filter_map(|(id, obj)| {
                        if state.filter.matches(obj) {
                            Some((*id, obj.clone()))
                        } else {
                            None
                        }
                    })
                    .collect::<IndexMap<_, _>>();
            }
        });

        let text_style = egui::TextStyle::Body;
        let row_height = ui.text_style_height(&text_style);

        egui::ScrollArea::vertical().show_rows(
            ui,
            row_height,
            state.filtered.len(),
            |ui, row_range| {
                for (i, obj) in state
                    .filtered
                    .iter()
                    .skip(row_range.start)
                    .take(row_range.len())
                {
                    ui.label(format!("{i:10?} {}", obj.name));
                }
                ui.allocate_space(ui.available_size());
            },
        );

        // egui::Window::new("object search 2")
        //     .default_height(500.)
        //     .show(ctx, |ui| {
        //         let name_label = ui.label("Search: ");
        //         let _res = ui
        //             .text_edit_singleline(&mut state.filter2)
        //             .labelled_by(name_label.id);

        //         let text_style = egui::TextStyle::Body;
        //         let row_height = ui.text_style_height(&text_style);

        //         //info!("before names lock");
        //         let objects = unsafe { globals().guobject_array_unchecked() }.objects();
        //         let mut names = state.objects.write().unwrap();

        //         //info!("before filter");
        //         let filtered = objects
        //             .iter()
        //             //.take(100)
        //             .flatten()
        //             .filter(|obj| {
        //                 let cached = &names.get_or_init(obj);
        //                 cached.name.contains(&state.filter2)
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

        let log_window = |name: &str, mut log: &str| {
            egui::Window::new(name)
                .default_height(500.)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut log)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(10)
                                    .font(egui::TextStyle::Monospace),
                            );
                        });
                });
        };

        log_window("Kismet Messages", &state.kismet_log);
    });

    // egui::CentralPanel::default().show(&ctx, |ui| {
    //     ui.heading("running in main thread");

    //     if ui.button("buhton").clicked() {
    //         state.buh += 1;
    //     }
    //     ui.label(format!("counter: {}", state.buh));
    // });
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(egui::Visuals::dark());

        let ctx = ctx.clone();
        self.tx_ui
            .send(Box::new(move || {
                STATE.with_borrow_mut(|state| {
                    ui(state, &ctx);
                })
            }))
            .unwrap();
        self.rx_ui.recv().unwrap();
    }
}
