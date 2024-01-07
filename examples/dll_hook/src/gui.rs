use std::{
    ops::Deref,
    sync::{mpsc::Receiver, Arc},
};

use eframe::egui;

#[cfg(windows)]
use egui_winit::winit::platform::windows::EventLoopBuilderExtWindows;
#[cfg(unix)]
use egui_winit::winit::platform::x11::EventLoopBuilderExtX11;
use indexmap::IndexMap;

use crate::{hooks::UObjectLock, ue::FWeakObjectPtr};

use super::*;

pub fn run() -> Result<(), eframe::Error> {
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
        Box::new(|cc| Box::new(MyApp::new())),
    )
}

type ObjectIndex = i32;
#[derive(Debug, Clone)]
struct ObjectProxy {
    name: String,
    flags: i32,
    weak_ptr: FWeakObjectPtr,
}

#[derive(Debug)]
enum Event {
    CreateUObject(ObjectIndex, ObjectProxy),
    DeleteUObject(ObjectIndex),
}

struct Listeners {
    create_uobject: Arc<dyn Fn(&ue::UObjectBase)>,
    delete_uobject: Arc<dyn Fn(&ue::UObjectBase)>,
}

struct ObjectFilter {
    name_search: String,
}
impl ObjectFilter {
    fn matches(&self, object: &ObjectProxy) -> bool {
        if self.name_search.is_empty() {
            true
        } else {
            object.name.to_ascii_lowercase().contains(&self.name_search)
        }
    }
}

struct MyApp {
    filter: ObjectFilter,
    events: Receiver<Event>,
    listeners: Listeners,
    objects: IndexMap<ObjectIndex, ObjectProxy>,
    filtered: IndexMap<ObjectIndex, ObjectProxy>,
}

impl MyApp {
    fn new() -> Self {
        let (tx, events) = std::sync::mpsc::channel();
        let txc = tx.clone();
        let create_uobject = Arc::new(move |object: &ue::UObjectBase| {
            /*
            txc.send(Event::CreateUObject(
                object.InternalIndex,
                ObjectProxy {
                    name: ue::FName_ToString(&object.NamePrivate),
                    flags: 0,
                    weak_ptr: ue::FWeakObjectPtr::new(object),
                },
            ))
            .unwrap();
            */
        });
        let txc = tx.clone();
        let delete_uobject = Arc::new(move |object: &ue::UObjectBase| {
            txc.send(Event::DeleteUObject(object.InternalIndex))
                .unwrap();
        });
        Self {
            filter: ObjectFilter {
                name_search: "".into(),
            },
            events,
            listeners: Listeners {
                create_uobject: hooks::create_uobject::register(create_uobject),
                delete_uobject: hooks::delete_uobject::register(delete_uobject),
            },
            objects: Default::default(),
            filtered: Default::default(),
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

        let object_lock = guobject_array();

        for event in self.events.try_iter() {
            match event {
                Event::CreateUObject(index, object) => {
                    if self.filter.matches(&object) {
                        self.filtered.insert(index, object.clone());
                    }
                    self.objects.insert(index, object);
                }
                Event::DeleteUObject(index) => {
                    //self.objects.remove(&index);
                    //self.filtered.remove(&index);
                }
            };
        }

        ctx.set_visuals(egui::Visuals::dark());

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("My egui Application");

            ui.horizontal(|ui| {
                let name_label = ui.label("Search: ");
                let res = ui
                    .text_edit_singleline(&mut self.filter.name_search)
                    .labelled_by(name_label.id);
                if res.changed() {
                    self.filtered = self
                        .objects
                        .iter()
                        .filter_map(|(index, obj)| {
                            if self.filter.matches(obj) {
                                Some((*index, obj.clone()))
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
                self.filtered.len(),
                |ui, row_range| {
                    for (i, obj) in self
                        .filtered
                        .iter()
                        .skip(row_range.start)
                        .take(row_range.len())
                    {
                        ui.label(format!("{i:10} {:?} {}", obj.flags, obj.name));
                    }
                    ui.allocate_space(ui.available_size());
                },
            );
        });
    }
}
