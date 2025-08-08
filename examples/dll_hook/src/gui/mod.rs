mod search;

use super::*;
use crate::{
    kismet::normalize_and_serialize,
    object_cache::{ObjectEvent, ObjectId, TickContext},
};

use egui::{self, Color32};
use egui_wgpu_win32::{Window, WindowConfig};
use indexmap::IndexMap;
use search::{ObjectCache, ObjectFilter};
use std::{cell::RefCell, collections::HashMap};

fn render_text_property<T>(ui: &mut egui::Ui, value: &mut T) -> bool
where
    T: std::fmt::Display + std::str::FromStr,
{
    let mut text = value.to_string();
    if ui.text_edit_singleline(&mut text).changed() {
        if let Ok(parsed_value) = text.parse() {
            *value = parsed_value;
            return true;
        }
    }
    false
}

fn render_string_property(ui: &mut egui::Ui, value: &mut ue::FString) -> bool {
    let mut text = value.to_string();
    if ui.text_edit_singleline(&mut text).changed() {
        *value = text.as_str().into();
        return true;
    }
    false
}

fn render_bool_property(ui: &mut egui::Ui, value: &mut ue::FBoolPropertyDataMut) -> bool {
    let mut current_value = value.get();
    if ui.checkbox(&mut current_value, "").changed() {
        value.set(current_value);
        return true;
    }
    false
}

fn render_struct_property(ui: &mut egui::Ui, struct_data: ue::FStructPropertyDataMut) -> bool {
    let struct_name = struct_data.get_struct().path();

    let id = ui.id().with("struct_collapse");
    let mut open = ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
    let mut changed = false;

    ui.horizontal(|ui| {
        if ui
            .selectable_label(open, if open { "▼" } else { "►" })
            .clicked()
        {
            open = !open;
            ui.data_mut(|d| d.insert_temp(id, open));
        }
        ui.label(format!("Struct ({})", struct_name));
    });

    if open {
        let props: Vec<_> = struct_data.props().collect();
        ui.vertical(|ui| {
            ui.indent("struct_properties", |ui| {
                for mut prop in props {
                    if let Some(fprop) = prop.field.cast::<ue::FProperty>() {
                        let name = fprop.name().to_string();
                        let offset = fprop.offset();

                        ui.horizontal(|ui| {
                            ui.label(format!("0x{offset:02x} {name}"));
                            ui.push_id(format!("struct_prop_{offset}"), |ui| {
                                changed |= render_property_ui(ui, &mut prop);
                            });
                        });
                    }
                }
            });
        });
    }

    changed
}

fn render_property_ui<'o>(ui: &mut egui::Ui, accessor: &mut impl ue::PropertyAccess<'o>) -> bool {
    let mut changed = false;

    if let Some(mut val) = accessor.try_get_mut::<ue::FInt8Property>() {
        changed = render_text_property(ui, &mut *val);
    } else if let Some(mut val) = accessor.try_get_mut::<ue::FInt16Property>() {
        changed = render_text_property(ui, &mut *val);
    } else if let Some(mut val) = accessor.try_get_mut::<ue::FIntProperty>() {
        changed = render_text_property(ui, &mut *val);
    } else if let Some(mut val) = accessor.try_get_mut::<ue::FInt64Property>() {
        changed = render_text_property(ui, &mut *val);
    } else if let Some(mut val) = accessor.try_get_mut::<ue::FByteProperty>() {
        changed = render_text_property(ui, &mut *val);
    } else if let Some(mut val) = accessor.try_get_mut::<ue::FUInt16Property>() {
        changed = render_text_property(ui, &mut *val);
    } else if let Some(mut val) = accessor.try_get_mut::<ue::FUInt32Property>() {
        changed = render_text_property(ui, &mut *val);
    } else if let Some(mut val) = accessor.try_get_mut::<ue::FUInt64Property>() {
        changed = render_text_property(ui, &mut *val);
    } else if let Some(mut val) = accessor.try_get_mut::<ue::FFloatProperty>() {
        changed = render_text_property(ui, &mut *val);
    } else if let Some(mut val) = accessor.try_get_mut::<ue::FDoubleProperty>() {
        changed = render_text_property(ui, &mut *val);
    } else if let Some(mut val) = accessor.try_get_mut::<ue::FStrProperty>() {
        changed = render_string_property(ui, &mut val);
    } else if let Some(mut val) = accessor.try_get_mut::<ue::FBoolProperty>() {
        changed = render_bool_property(ui, &mut val);
    } else if let Some(val) = accessor.try_get::<ue::FNameProperty>() {
        // TODO: FName editing
        ui.colored_label(egui::Color32::GRAY, format!("{}", *val));
    } else if let Some(array_data) = accessor.try_get_mut::<ue::FArrayProperty>() {
        render_array_property_ui(ui, array_data);
    } else if let Some(struct_data) = accessor.try_get_mut::<ue::FStructProperty>() {
        changed = render_struct_property(ui, struct_data);
    } else if let Some(obj_data) = accessor.try_get::<ue::FObjectProperty>() {
        // Display object path, no editing
        if let Some(obj) = *obj_data {
            ui.colored_label(egui::Color32::LIGHT_BLUE, obj.path());
        } else {
            ui.colored_label(egui::Color32::GRAY, "nullptr");
        }
    } else {
        let mut cast_flags = accessor.field().class().cast_flags;
        cast_flags.remove(
            ue::EClassCastFlags::CASTCLASS_UField
                | ue::EClassCastFlags::CASTCLASS_FProperty
                | ue::EClassCastFlags::CASTCLASS_FObjectPropertyBase,
        );
        ui.colored_label(
            egui::Color32::DARK_RED,
            format!("{:?} {}", cast_flags, accessor.field().name()),
        );
    }

    changed
}

fn render_array_property_ui(ui: &mut egui::Ui, mut array_data: ue::FArrayPropertyDataMut) {
    let num_elements = array_data.len();
    let inner_prop = array_data.inner_property();

    let id = ui.id().with("array_collapse");
    let mut open = ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));

    ui.horizontal(|ui| {
        if ui
            .selectable_label(open, if open { "▼" } else { "►" })
            .clicked()
        {
            open = !open;
            ui.data_mut(|d| d.insert_temp(id, open));
        }
        ui.label(format!("Array[{}] ({})", num_elements, inner_prop.name()));

        if ui.small_button("+").clicked() {
            array_data.add_zeroed_element(1);
        }
        if ui.small_button("-").clicked() && num_elements > 0 {
            array_data.remove_element(num_elements - 1, 1);
        }
        if ui.small_button("Clear").clicked() {
            array_data.empty(0);
        }
    });

    if open {
        let current_num = array_data.len();
        let mut to_remove = None;
        for i in 0..current_num {
            ui.horizontal(|ui| {
                ui.label(format!("[{i}]"));

                let mut elem = array_data.get_element_mut(i);
                ui.push_id(format!("array_elem_{i}"), |ui| {
                    render_property_ui(ui, &mut elem);
                });

                if ui.small_button("×").clicked() {
                    to_remove = Some(i);
                }
            });
        }

        if let Some(i) = to_remove {
            array_data.remove_element(i, 1);
        }

        if ui.small_button("+ Add Element").clicked() {
            array_data.add_zeroed_element(1);
        }
    }
}

thread_local! {
    static STATE: RefCell<(InnerState, Option<Window>)> = Default::default();
}

pub fn init() {
    events::register(move |hooks::GameTick(events)| {
        let ctx = unsafe { TickContext::new() };

        STATE.with_borrow_mut(|(s, window)| {
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
                        match ex {
                            // Ok(ex) => tracing::info!("ex: {ex:#?}"),
                            Ok(ex) => {
                                tracing::info!("ex: {}", ex.len());
                            }
                            Err(err) => tracing::error!("ex: {err}"),
                        }
                    }
                    ObjectEvent::Deleted { id } => {
                        s.objects.shift_remove(id);
                        s.filtered.shift_remove(id);
                    }
                }
            }

            if window.is_none() {
                *window = Some(
                    Window::new(WindowConfig {
                        title: "dll_hook".into(),
                        width: 600,
                        height: 400,
                        resizable: true,
                    })
                    .unwrap(),
                );
            }
            window.as_mut().unwrap().tick(|c| {
                ui(s, c, &ctx);
            });
        });
    });
}

struct InnerState {
    buh: i32,
    filter: ObjectFilter,
    kismet_log: String,
    objects: IndexMap<ObjectId, ObjectCache>,
    filtered: IndexMap<ObjectId, ObjectCache>,

    open_objects: HashMap<ObjectId, Result<crate::kismet_nodes::KismetGraph>>,
}
impl Default for InnerState {
    fn default() -> Self {
        Self::new()
    }
}
impl InnerState {
    fn new() -> Self {
        Self {
            buh: 0,
            // filter: ObjectFilter::new("Function /Game/".to_string()),
            filter: ObjectFilter::new("^GameEngine".to_string()),
            filtered: Default::default(),
            objects: Default::default(),
            kismet_log: "".to_string(),

            open_objects: Default::default(),
        }
    }
}

fn update_filtered(state: &mut InnerState) {
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
                update_filtered(state);
            }
        });

        ui.horizontal(|ui| {
            ui.label("Filter options:");
            let mut flags_changed = false;

            flags_changed |= ui
                .checkbox(
                    &mut state.filter.flags.include_class_default_objects,
                    "CDOs",
                )
                .changed();
            flags_changed |= ui
                .checkbox(&mut state.filter.flags.include_instances, "Instances")
                .changed();
            flags_changed |= ui
                .checkbox(
                    &mut state.filter.flags.search_parent_classes,
                    "Search parent classes",
                )
                .changed();

            if flags_changed {
                update_filtered(state);
            }
        });

        ui.horizontal(|ui| {
            ui.label("Class filter:");
            let mut class_filter = state
                .filter
                .flags
                .class_name_filter
                .clone()
                .unwrap_or_default();
            let res = ui.text_edit_singleline(&mut class_filter);
            if res.changed() {
                state.filter.flags.class_name_filter = if class_filter.is_empty() {
                    None
                } else {
                    Some(class_filter)
                };
                update_filtered(state);
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
                        if ui.button(format!("{i:10?}")).clicked() {
                            let graph = tick_ctx
                                .get_ref(*i)
                                .unwrap()
                                .cast::<ue::UFunction>()
                                .ok_or_else(|| anyhow::anyhow!("not a function"))
                                .and_then(kismet_transform::transform);
                            state.open_objects.insert(*i, graph);
                        }
                        if let Some((len, res)) = &obj.script_status {
                            ui.colored_label(egui::Color32::GREEN, format!("{len}"));
                            match res {
                                Ok(t) => {
                                    ui.colored_label(egui::Color32::GREEN, t);
                                }
                                Err(t) => {
                                    ui.colored_label(egui::Color32::RED, t);
                                }
                            }
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
                        if let Some(func) = object.cast_mut::<ue::UFunction>() {
                            ui.label("function");

                            match graph {
                                Ok(graph) => {
                                    let mut out = kismet_transform::compile(func, graph);
                                    match &mut out {
                                        Ok(r) => {
                                            // ui.colored_label(Color32::GREEN, &format!("{r:?}"));
                                            let ser = normalize_and_serialize(r);

                                            match ser {
                                                Ok(r) => {
                                                    // let hex = r
                                                    //     .iter()
                                                    //     .map(|b| format!("{b:02x}"))
                                                    //     .collect::<Vec<_>>()
                                                    //     .join(" ");
                                                    // ui.colored_label(
                                                    //     Color32::GREEN,
                                                    //     &format!("{}", hex),
                                                    // );
                                                    func.script.clear();
                                                    func.script.extend(&r);
                                                }
                                                Err(r) => {
                                                    ui.colored_label(Color32::RED, format!("{r}"));
                                                }
                                            }
                                        }
                                        Err(r) => {
                                            ui.colored_label(Color32::RED, format!("{r}"));
                                        }
                                    }
                                    graph.ui(ui, egui::Id::new(name));
                                }
                                Err(msg) => {
                                    ui.colored_label(Color32::RED, msg.to_string());
                                }
                            }
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
                            use egui_extras::{Column, TableBuilder};

                            TableBuilder::new(ui)
                                .column(Column::exact(60.0)) // Offset column
                                .column(Column::exact(200.0)) // Name column
                                .column(Column::remainder().at_least(200.0)) // Value column
                                .header(20.0, |mut header| {
                                    header.col(|ui| {
                                        ui.strong("Offset");
                                    });
                                    header.col(|ui| {
                                        ui.strong("Name");
                                    });
                                    header.col(|ui| {
                                        ui.strong("Value");
                                    });
                                })
                                .body(|mut body| {
                                    for (offset, name, mut field) in props {
                                        body.row(18.0, |mut row| {
                                            row.col(|ui| {
                                                ui.label(format!("0x{offset:02x}"));
                                            });
                                            row.col(|ui| {
                                                ui.label(name.to_string());
                                            });
                                            row.col(|ui| {
                                                ui.push_id(format!("offset_{offset}"), |ui| {
                                                    render_property_ui(ui, &mut field);
                                                });
                                            });
                                        });
                                    }
                                });
                        });
                    }
                });
            open
        });
    });
}
