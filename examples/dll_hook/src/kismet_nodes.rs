use eframe::egui::{self, Color32, Ui};
use egui_snarl::{
    ui::{
        AnyPins, NodeLayout, PinInfo, PinPlacement, SnarlStyle, SnarlViewer, SnarlWidget, WireStyle,
    },
    InPin, NodeId, OutPin, Snarl,
};

use crate::ue;

const fn float_color(r: f32, g: f32, b: f32) -> Color32 {
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

const EXEC_COLOR: Color32 = float_color(1.0, 1.0, 1.0);
const DATA_COLOR: Color32 = float_color(0.7, 1.0, 1.0);
const BOOL_COLOR: Color32 = float_color(0.300000, 0.0, 0.0);
const BYTE_COLOR: Color32 = float_color(0.0, 0.160000, 0.131270);
const FLOAT_COLOR: Color32 = float_color(0.357667, 1.0, 0.060000);
const NAME_COLOR: Color32 = float_color(0.607717, 0.224984, 1.0);
const STRING_COLOR: Color32 = float_color(1.0, 0.0, 0.660537);
const NUMBER_COLOR: Color32 = float_color(0.013575, 0.770000, 0.429609);

#[derive(Debug)]
pub struct GenericPin {
    pub name: String,
    pub pin_type: PinType,
}

#[derive(Debug)]
pub enum PinType {
    Exec,
    Data,
    Property(u64),
    Function(u64),
    Object(u64),

    Bool(bool),
    String(String),
    FName(ue::FName),
    Byte(u8),
    Int(i32),
    Float(f32),
}

impl PinType {
    fn pin_info(&self) -> PinInfo {
        match self {
            Self::Exec => {
                PinInfo::triangle()
                    .with_fill(EXEC_COLOR)
                    .with_wire_style(WireStyle::AxisAligned {
                        corner_radius: 10.0,
                    })
            }
            Self::Data => PinInfo::circle()
                .with_fill(DATA_COLOR)
                .with_wire_style(WireStyle::Bezier5),
            Self::Property(_) => PinInfo::circle(),
            Self::Function(_) => PinInfo::circle(),
            Self::Object(_) => PinInfo::circle(),
            Self::Bool(_) => PinInfo::triangle()
                .with_fill(BOOL_COLOR)
                .with_wire_style(WireStyle::Bezier5),
            Self::String(_) => PinInfo::circle()
                .with_fill(STRING_COLOR)
                .with_wire_style(WireStyle::Bezier5),
            Self::FName(_) => PinInfo::circle()
                .with_fill(NAME_COLOR)
                .with_wire_style(WireStyle::Bezier5),
            Self::Byte(_) => PinInfo::circle()
                .with_fill(BYTE_COLOR)
                .with_wire_style(WireStyle::Bezier5),
            Self::Int(_) => PinInfo::circle()
                .with_fill(NUMBER_COLOR)
                .with_wire_style(WireStyle::Bezier5),
            Self::Float(_) => PinInfo::circle()
                .with_fill(NUMBER_COLOR)
                .with_wire_style(WireStyle::Bezier5),
        }
    }
}

#[derive(Debug)]
pub struct GenericNode {
    pub node_type: NodeType,
    pub inputs: Vec<GenericPin>,
    pub outputs: Vec<GenericPin>,
}

#[derive(Debug)]
pub enum NodeType {
    Generic(String),
    Expr(crate::kismet::literal::ExprOp),
    FunctionDef(String),
}
impl NodeType {
    pub fn name(&self) -> String {
        match self {
            NodeType::Generic(name) => name.clone(),
            NodeType::Expr(expr_op) => format!("{expr_op:?}"),
            NodeType::FunctionDef(name) => name.clone(),
        }
    }
}
impl From<&str> for NodeType {
    fn from(value: &str) -> Self {
        Self::Generic(value.to_string())
    }
}

struct KismetViewer;

impl SnarlViewer<GenericNode> for KismetViewer {
    #[inline]
    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<GenericNode>) {
        let from_node = &snarl[from.id.node];
        let from_pin = &from_node.outputs[from.id.output];

        let to_node = &snarl[to.id.node];
        let to_pin = &to_node.inputs[to.id.input];

        if std::mem::discriminant(&from_pin.pin_type) != std::mem::discriminant(&to_pin.pin_type) {
            return;
        }

        match from_pin.pin_type {
            PinType::Exec => {
                for &remote in &from.remotes {
                    snarl.disconnect(from.id, remote);
                }
            }
            _ => {
                for &remote in &to.remotes {
                    snarl.disconnect(remote, to.id);
                }
            }
        }

        snarl.connect(from.id, to.id);
    }

    fn title(&mut self, node: &GenericNode) -> String {
        node.node_type.name()
    }

    fn inputs(&mut self, node: &GenericNode) -> usize {
        node.inputs.len()
    }

    fn outputs(&mut self, node: &GenericNode) -> usize {
        node.outputs.len()
    }

    #[allow(refining_impl_trait)]
    fn show_input(&mut self, pin: &InPin, ui: &mut Ui, snarl: &mut Snarl<GenericNode>) -> PinInfo {
        let node = &mut snarl[pin.id.node];
        let remotes = &pin.remotes;
        let pin = &mut node.inputs[pin.id.input];

        ui.label(&pin.name);

        match &mut pin.pin_type {
            PinType::Exec => {}
            PinType::Data => {}
            PinType::Property(ptr) => {
                let ptr = *ptr as *const ue::FProperty;
                let prop = unsafe { ptr.as_ref() };
                ui.label(format!("{:?}", prop.map(|p| p.name().to_string())));
            }
            PinType::Function(ptr) => {
                let ptr = *ptr as *const ue::UFunction;
                let prop = unsafe { ptr.as_ref().unwrap() };
                ui.label(format!("{}", prop.path()));
            }
            PinType::Object(ptr) => {
                let ptr = *ptr as *const ue::UObject;
                let prop = unsafe { ptr.as_ref().unwrap() };
                ui.label(format!("{}", prop.path()));
            }

            PinType::Bool(value) => {
                if remotes.is_empty() {
                    ui.checkbox(value, "");
                }
            }
            PinType::String(value) => {
                if remotes.is_empty() {
                    egui::TextEdit::singleline(value)
                        .clip_text(false)
                        .desired_width(0.0)
                        .margin(ui.spacing().item_spacing)
                        .show(ui);
                }
            }
            PinType::FName(value) => {
                if remotes.is_empty() {
                    ui.label(value.to_string());
                }
            }
            PinType::Byte(value) => {
                if remotes.is_empty() {
                    ui.add(egui::DragValue::new(value));
                }
            }
            PinType::Int(value) => {
                if remotes.is_empty() {
                    ui.add(egui::DragValue::new(value));
                }
            }
            PinType::Float(value) => {
                if remotes.is_empty() {
                    ui.add(egui::DragValue::new(value));
                }
            }
        }
        pin.pin_type.pin_info()
    }

    #[allow(refining_impl_trait)]
    fn show_output(
        &mut self,
        pin: &OutPin,
        ui: &mut Ui,
        snarl: &mut Snarl<GenericNode>,
    ) -> PinInfo {
        let node = &mut snarl[pin.id.node];
        let remotes = &pin.remotes;
        let pin = &mut node.outputs[pin.id.output];

        ui.label(&pin.name);

        pin.pin_type.pin_info()
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<GenericNode>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: egui::Pos2, ui: &mut Ui, snarl: &mut Snarl<GenericNode>) {
        ui.label("Add node");
        if ui.button("Number").clicked() {
            snarl.insert_node(
                pos,
                GenericNode {
                    node_type: "Number node".into(),
                    inputs: vec![
                        GenericPin {
                            name: "exec".into(),
                            pin_type: PinType::Exec,
                        },
                        GenericPin {
                            name: "input".into(),
                            pin_type: PinType::Int(1337),
                        },
                    ],
                    outputs: vec![
                        GenericPin {
                            name: "then".into(),
                            pin_type: PinType::Exec,
                        },
                        GenericPin {
                            name: "output".into(),
                            pin_type: PinType::Int(0),
                        },
                    ],
                },
            );
            ui.close_menu();
        }
        if ui.button("String").clicked() {
            snarl.insert_node(
                pos,
                GenericNode {
                    node_type: "String node".into(),
                    inputs: vec![
                        GenericPin {
                            name: "exec".into(),
                            pin_type: PinType::Exec,
                        },
                        GenericPin {
                            name: "input".into(),
                            pin_type: PinType::String("asdf".into()),
                        },
                    ],
                    outputs: vec![
                        GenericPin {
                            name: "then".into(),
                            pin_type: PinType::Exec,
                        },
                        GenericPin {
                            name: "output".into(),
                            pin_type: PinType::String("asdf".into()),
                        },
                    ],
                },
            );
            ui.close_menu();
        }
        if ui.button("If").clicked() {
            snarl.insert_node(
                pos,
                GenericNode {
                    node_type: "If".into(),
                    inputs: vec![
                        GenericPin {
                            name: "exec".into(),
                            pin_type: PinType::Exec,
                        },
                        GenericPin {
                            name: "condition".into(),
                            pin_type: PinType::Bool(false),
                        },
                    ],
                    outputs: vec![
                        GenericPin {
                            name: "then".into(),
                            pin_type: PinType::Exec,
                        },
                        GenericPin {
                            name: "else".into(),
                            pin_type: PinType::Exec,
                        },
                    ],
                },
            );
            ui.close_menu();
        }
        if ui.button("For").clicked() {
            snarl.insert_node(
                pos,
                GenericNode {
                    node_type: "For".into(),
                    inputs: vec![
                        GenericPin {
                            name: "exec".into(),
                            pin_type: PinType::Exec,
                        },
                        GenericPin {
                            name: "start".into(),
                            pin_type: PinType::Int(0),
                        },
                        GenericPin {
                            name: "end".into(),
                            pin_type: PinType::Int(10),
                        },
                    ],
                    outputs: vec![
                        GenericPin {
                            name: "then".into(),
                            pin_type: PinType::Exec,
                        },
                        GenericPin {
                            name: "value".into(),
                            pin_type: PinType::Int(0),
                        },
                        GenericPin {
                            name: "finish".into(),
                            pin_type: PinType::Exec,
                        },
                    ],
                },
            );
            ui.close_menu();
        }
    }

    fn has_dropped_wire_menu(
        &mut self,
        _src_pins: AnyPins,
        _snarl: &mut Snarl<GenericNode>,
    ) -> bool {
        true
    }

    fn show_dropped_wire_menu(
        &mut self,
        pos: egui::Pos2,
        ui: &mut Ui,
        src_pins: AnyPins,
        snarl: &mut Snarl<GenericNode>,
    ) {
    }

    fn has_node_menu(&mut self, _node: &GenericNode) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<GenericNode>,
    ) {
        ui.label("Node menu");
        if ui.button("Remove").clicked() {
            snarl.remove_node(node);
            ui.close_menu();
        }
    }

    fn has_on_hover_popup(&mut self, _: &GenericNode) -> bool {
        true
    }

    fn show_on_hover_popup(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut Ui,
        snarl: &mut Snarl<GenericNode>,
    ) {
    }

    fn header_frame(
        &mut self,
        frame: egui::Frame,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<GenericNode>,
    ) -> egui::Frame {
        let color = match snarl[node].node_type {
            NodeType::Generic(_) => Color32::from_rgb(70, 66, 40),
            NodeType::Expr(_) => Color32::from_rgb(70, 66, 40),
            NodeType::FunctionDef(_) => Color32::DARK_RED,
        };
        frame.fill(color)
    }
}

pub struct KismetGraph {
    pub snarl: Snarl<GenericNode>,
}

impl KismetGraph {
    pub fn new() -> Self {
        KismetGraph {
            snarl: Default::default(),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, id: egui::Id) {
        SnarlWidget::new()
            .id(id)
            .show(&mut self.snarl, &mut KismetViewer, ui);
    }
}
