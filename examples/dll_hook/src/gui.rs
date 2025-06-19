use crossbeam_channel::{Receiver, Sender};
use std::{cell::RefCell, collections::HashSet};

use eframe::egui;

#[cfg(windows)]
use egui_winit::winit::platform::windows::EventLoopBuilderExtWindows;
#[cfg(unix)]
use egui_winit::winit::platform::x11::EventLoopBuilderExtX11;
use indexmap::IndexMap;

use crate::{
    object_cache::{ObjectEvent, ObjectId, TickContext},
    ue::EClassCastFlags,
};

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
                        let name = object.path();
                        let cache = ObjectCache { name };
                        if s.filter.matches(&cache) {
                            s.filtered.insert(*id, cache.clone());
                        }
                        s.objects.insert(*id, cache);

                        // if let Some(obj) = ctx.get_ref(*id) {
                        //     for (i, field) in obj.props().enumerate() {
                        //         if let Some(value) = field.get::<ue::FNameProperty>() {
                        //             let value = value.to_string();
                        //             if !value.is_empty() {
                        //                 println!(
                        //                     "{} = {}",
                        //                     field.field.name().to_string(),
                        //                     value.to_string()
                        //                 );
                        //             }
                        //         }
                        //     }
                        // }
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
        Box::new(|_cc| Box::new(MyApp::new(channels))),
    )
}

#[derive(Debug, Clone)]
struct ObjectCache {
    name: String,
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

    open_objects: HashSet<ObjectId>,
}
impl InnerState {
    fn new() -> Self {
        Self {
            buh: 0,
            filter: ObjectFilter::default(),
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
                    ui.horizontal(|ui| {
                        if ui.button(&format!("{i:10?}")).clicked() {
                            state.open_objects.insert(*i);
                        }
                        ui.label(&obj.name);
                    });
                }
                ui.allocate_space(ui.available_size());
            },
        );

        state.open_objects.retain(|obj| {
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

                            let script = &func.script;
                            ui.label(&format!("script {script:?}"));
                        }

                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for (i, mut field) in object.props_mut().enumerate() {
                                if let Some(prop) = field.field.cast::<ue::FProperty>() {
                                    ui.horizontal(|ui| {
                                        ui.label(&format!(
                                            "{i}: 0x{:02x} {}",
                                            prop.offset(),
                                            prop.name().to_string()
                                        ));
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
