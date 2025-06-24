use crossbeam_channel::{Receiver, Sender};
use regex::Regex;
use std::{cell::RefCell, collections::HashMap};

use eframe::egui;

#[cfg(windows)]
use egui_winit::winit::platform::windows::EventLoopBuilderExtWindows;
#[cfg(unix)]
use egui_winit::winit::platform::x11::EventLoopBuilderExtX11;
use indexmap::IndexMap;

use crate::object_cache::{ObjectEvent, ObjectId, TickContext};

use super::*;

pub type GuiFn = Box<dyn FnOnce(&TickContext) -> GuiRet + Send + Sync>;
pub type GuiRet = ();

thread_local! {
    static STATE: RefCell<InnerState> = RefCell::new(InnerState::new());
}

pub fn init() {
    let (tx_main, rx_ui) = crossbeam_channel::bounded::<crate::gui::GuiRet>(0);
    let (tx_ui, rx_main) = crossbeam_channel::bounded::<crate::gui::GuiFn>(0);

    events::register(move |hooks::GameTick(events)| {
        let ctx = unsafe { TickContext::new() };

        STATE.with_borrow_mut(|s| {
            for e in events {
                match e {
                    ObjectEvent::Created { id } => {
                        let Some(object) = ctx.get_ref(*id) else {
                            continue;
                        };

                        let cache = ObjectCache::new(&object);
                        if s.filter.matches(&cache) {
                            s.filtered.insert(*id, cache.clone());
                        }
                        s.objects.insert(*id, cache);

                        let Some(obj) = object.cast::<ue::UFunction>() else {
                            continue;
                        };
                        if obj.script.is_empty() {
                            continue;
                        }
                        // tracing::info!("len={}: {}", obj.script.len(), obj.path());
                        let mut stream = std::io::Cursor::new(obj.script.as_slice());

                        let ex = crate::kismet::read_all(&mut stream);
                        // match ex {
                        //     Ok(ex) => tracing::info!("ex: {ex:#?}"),
                        //     // Err(err) => tracing::error!("ex: {err}"),
                        //     _ => {}
                        // }

                        for (i, field) in obj.props().enumerate() {
                            // if let Some(value) = field.get::<ue::FNameProperty>() {
                            //     let value = value.to_string();
                            //     if !value.is_empty() {
                            //         tracing::info!(
                            //             "  {} = {}",
                            //             field.field.name().to_string(),
                            //             value.to_string()
                            //         );
                            //     }
                            // }
                        }
                    }
                    ObjectEvent::Deleted { id } => {
                        s.objects.shift_remove(id);
                        s.filtered.shift_remove(id);
                    }
                }
            }
        });

        if let Ok(f) = rx_main.try_recv() {
            #[allow(clippy::unit_arg)]
            tx_main.send(f(&ctx)).unwrap();
        }
    });

    std::thread::spawn(move || run((tx_ui, rx_ui)).unwrap());
}

fn run(channels: (Sender<GuiFn>, Receiver<GuiRet>)) -> Result<(), eframe::Error> {
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
        Box::new(|_cc| Ok(Box::new(MyApp::new(channels)))),
    )
}

#[derive(Debug, Clone)]
struct ObjectCache {
    name: String,
    script_status: Option<(usize, Result<String, String>)>,
}
impl ObjectCache {
    fn new(object: &ue::UObjectBase) -> Self {
        let script_status = if let Some(func) = object.cast::<ue::UFunction>() {
            let mut stream = std::io::Cursor::new(func.script.as_slice());
            let ex = crate::kismet::read_all(&mut stream);
            Some((
                func.script.len(),
                ex.map(|ex| format!("{}", ex.len()))
                    .map_err(|e| e.to_string()),
            ))
        } else {
            None
        };

        Self {
            name: object.path(),
            script_status,
        }
    }
}

struct ObjectFilter {
    name_search: String,
    re: Option<Regex>,
}
impl ObjectFilter {
    fn new(search: String) -> Self {
        let mut new = Self {
            name_search: String::new(),
            re: None,
        };
        new.set_search(search);
        new
    }
    fn get_search(&self) -> &str {
        &self.name_search
    }
    fn set_search(&mut self, value: String) {
        self.name_search = value;
        self.re = Regex::new(&self.name_search).ok()
    }
    fn matches(&self, object: &ObjectCache) -> bool {
        if let Some(re) = &self.re {
            re.is_match(&object.name)
        } else {
            true
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

    open_objects: HashMap<ObjectId, crate::kismet_nodes::KismetGraph>,
}
impl InnerState {
    fn new() -> Self {
        Self {
            buh: 0,
            filter: ObjectFilter::new("Function /Game/".to_string()),
            filtered: Default::default(),
            objects: Default::default(),
            kismet_log: "".to_string(),

            open_objects: Default::default(),
        }
    }
}

impl MyApp {
    fn new((tx_ui, rx_ui): (Sender<GuiFn>, Receiver<GuiRet>)) -> Self {
        Self { tx_ui, rx_ui }
    }
}
fn ui(state: &mut InnerState, ctx: &egui::Context, tick_ctx: &TickContext) {
    assert_main_thread!();

    ctx.set_visuals(egui::Visuals::dark());

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("My egui Application");

        ui.horizontal(|ui| {
            let name_label = ui.label("Search: ");
            let mut tmp = std::borrow::Cow::from(state.filter.get_search());
            let res = ui.text_edit_singleline(&mut tmp).labelled_by(name_label.id);
            if res.changed() {
                state.filter.set_search(tmp.to_string());

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
                    ui.horizontal(|ui| {
                        if ui.button(&format!("{i:10?}")).clicked() {
                            state
                                .open_objects
                                .insert(*i, crate::kismet_nodes::KismetGraph::new());
                        }
                        match &obj.script_status {
                            Some((len, res)) => {
                                ui.colored_label(egui::Color32::GREEN, format!("{len}"));
                                match res {
                                    Ok(t) => {
                                        ui.colored_label(egui::Color32::GREEN, t);
                                    }
                                    Err(t) => {
                                        ui.colored_label(egui::Color32::RED, t);
                                    }
                                }
                            }
                            None => {}
                        };
                        ui.label(&obj.name);
                    });
                }
                ui.allocate_space(ui.available_size());
            },
        );

        state.open_objects.retain(|obj, graph| {
            let mut object = tick_ctx.get_mut(*obj);
            let name = object
                .as_ref()
                .map(|o| o.path())
                .unwrap_or_else(|| format!("invalid {obj:?}"));

            let mut open = true;
            egui::Window::new(&name)
                .open(&mut open)
                .default_height(500.)
                .show(ctx, |ui| {
                    if let Some(object) = &mut object {
                        let class = object.class();
                        let cast_flags = class.class_cast_flags;
                        ui.label(format!("cast flags {cast_flags:?}"));
                        if let Some(func) = object.cast::<ue::UFunction>() {
                            ui.label("function");

                            // let script = &func.script;
                            // ui.label(&format!("script {script:?}"));
                            let mut stream = std::io::Cursor::new(func.script.as_slice());
                            let ex = crate::kismet::read_all(&mut stream);
                            // match ex {
                            //     Ok(ex) => tracing::info!("ex: {ex:#?}"),
                            //     // Err(err) => tracing::error!("ex: {err}"),
                            //     _ => {}
                            // }

                            // struct Immutable(String);

                            // egui::ScrollArea::vertical().show(ui, |ui| {
                            //     ui.text_edit_multiline(&mut format!("{ex:#?}").as_str())
                            // });
                            graph.ui(ui, egui::Id::new(name));
                            return;
                        }

                        let mut props = object
                            .props_mut()
                            .filter_map(|f| {
                                f.field
                                    .cast::<ue::FProperty>()
                                    .map(|p| (p.offset(), p.name(), f))
                            })
                            .collect::<Vec<_>>();

                        props.sort_by_key(|p| p.0);

                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for (offset, name, mut field) in props {
                                ui.horizontal(|ui| {
                                    ui.label(&format!("0x{:02x} {}", offset, name.to_string()));
                                    if let Some(p) = field.get::<ue::FNameProperty>() {
                                        let mut text = p.to_string();
                                        if ui.text_edit_singleline(&mut text).changed() {
                                            // TODO create FName
                                            // *p = text.as_str().into();
                                        }
                                    } else if let Some(p) = field.get::<ue::FStrProperty>() {
                                        let mut text = p.to_string();
                                        if ui.text_edit_singleline(&mut text).changed() {
                                            *p = text.as_str().into();
                                        }
                                    } else if let Some(p) = field.get::<ue::FIntProperty>() {
                                        let mut text = p.to_string();
                                        if ui.text_edit_singleline(&mut text).changed() {
                                            if let Ok(value) = text.parse() {
                                                *p = value;
                                            }
                                        }
                                    } else if let Some(p) = field.get::<ue::FFloatProperty>() {
                                        let mut text = p.to_string();
                                        if ui.text_edit_singleline(&mut text).changed() {
                                            if let Ok(value) = text.parse() {
                                                *p = value;
                                            }
                                        }
                                    } else if let Some(p) = field.get::<ue::FDoubleProperty>() {
                                        let mut text = p.to_string();
                                        if ui.text_edit_singleline(&mut text).changed() {
                                            if let Ok(value) = text.parse() {
                                                *p = value;
                                            }
                                        }
                                    }
                                });
                            }
                            ui.allocate_space(ui.available_size());
                        });
                    }
                });
            open
        });
    });
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(egui::Visuals::dark());

        let ctx = ctx.clone();
        self.tx_ui
            .send(Box::new(move |tick_ctx| {
                STATE.with_borrow_mut(|state| {
                    ui(state, &ctx, tick_ctx);
                })
            }))
            .unwrap();
        self.rx_ui.recv().unwrap();
    }
}
