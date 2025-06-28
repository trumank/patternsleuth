use std::collections::{HashMap, VecDeque};

use crate::{
    kismet::{ExprIndex, KismetPropertyPointer, PackageIndex},
    kismet_nodes::{GenericNode, GenericPin, KismetGraph, NodeType, PinType},
    ue,
};
use anyhow::{anyhow, bail, Context, Result};
use eframe::egui;
use egui_snarl::{InPinId, NodeId, OutPinId, Snarl};

struct Ctx {
    exs: crate::kismet::literal::ExprGraph,
    snarl: Snarl<GenericNode>,

    to_add: Vec<ExprIndex>,
    node_map: HashMap<ExprIndex, NodeId>,
}

pub fn transform(function: &ue::UFunction) -> Result<KismetGraph> {
    let mut stream = std::io::Cursor::new(function.script.as_slice());

    let exs = crate::kismet::read_all(&mut stream)?;
    let to_add = exs.keys().cloned().collect::<Vec<_>>();

    let mut ctx = Ctx {
        exs,
        snarl: Default::default(),
        to_add,
        node_map: Default::default(),
    };

    if !ctx.exs.is_empty() {
        let root = build_node(&mut ctx, ExprIndex(0));

        let def = ctx.snarl.insert_node(
            egui::Pos2::ZERO,
            GenericNode {
                node_type: NodeType::FunctionDef(function.name_private.to_string()),
                inputs: vec![],
                outputs: vec![GenericPin {
                    name: "then".into(),
                    pin_type: PinType::Exec,
                }],
            },
        );
        ctx.snarl.connect(
            OutPinId {
                node: def,
                output: 0,
            },
            InPinId {
                node: root,
                input: 0,
            },
        );
    }

    while let Some(i) = ctx.to_add.pop() {
        if ctx.node_map.contains_key(&i) {
            continue;
        }

        build_node(&mut ctx, i);
    }

    layout::layout(&mut ctx.snarl);

    Ok(KismetGraph { snarl: ctx.snarl })
}

fn build_node(ctx: &mut Ctx, index: ExprIndex) -> NodeId {
    if let Some(id) = ctx.node_map.get(&index) {
        return *id;
    }
    let node = ctx.exs[&index].clone();

    let op = node.expr.op();
    let name = format!("{}: {}", index.0, op.as_ref());

    let mut in_conns = vec![];
    let mut out_conns = vec![];

    let mut inputs = vec![];
    let mut outputs = vec![];

    let id = ctx.snarl.insert_node(
        egui::Pos2::ZERO,
        GenericNode {
            node_type: NodeType::Expr(op),
            inputs: vec![],
            outputs: vec![],
        },
    );
    ctx.node_map.insert(index, id);

    fn pin(name: impl Into<String>, pin_type: PinType) -> GenericPin {
        GenericPin {
            name: name.into(),
            pin_type,
        }
    }

    if node.top_level {
        if let Some(next) = node.next {
            out_conns.push((outputs.len(), build_node(ctx, next)));
            outputs.push(pin("then", PinType::Exec));
        }
        inputs.push(pin("exec", PinType::Exec));
    } else {
        outputs.push(pin("output", PinType::Data));
    }
    use crate::kismet::literal::Expr as Ex;
    match &node.expr {
        // Ex::ExSkipOffsetConst(ex) => {} TODO
        Ex::ExLocalVariable(ex) => {
            inputs.push(pin("property", PinType::Property(ex.variable.0)));
        }
        Ex::ExInstanceVariable(ex) => {
            inputs.push(pin("property", PinType::Property(ex.variable.0)));
        }
        Ex::ExDefaultVariable(_) => {}
        Ex::ExReturn(ex) => {
            in_conns.push((inputs.len(), build_node(ctx, ex.return_expression)));
            inputs.push(pin("return", PinType::Data));
        }
        Ex::ExJump(ex) => {
            out_conns.push((outputs.len(), build_node(ctx, ex.code_offset)));
            outputs.push(pin("then", PinType::Exec));
        }
        Ex::ExJumpIfNot(ex) => {
            out_conns.push((outputs.len(), build_node(ctx, ex.code_offset)));
            outputs.push(pin("else", PinType::Exec));
            in_conns.push((inputs.len(), build_node(ctx, ex.boolean_expression)));
            inputs.push(pin("condition", PinType::Data));
        }
        //     Ex::ExAssert(ex_assert) => bail!("todo map ExAssert"),
        Ex::ExNothing(_) => {}
        Ex::ExNothingInt32(_) => {}
        Ex::ExLet(ex) => {
            inputs.push(pin("value", PinType::Property(ex.value.0)));
            in_conns.push((inputs.len(), build_node(ctx, ex.variable)));
            inputs.push(pin("variable", PinType::Data));
            in_conns.push((inputs.len(), build_node(ctx, ex.expression)));
            inputs.push(pin("expression", PinType::Data));
        }
        //     Ex::ExBitFieldConst(ex_bit_field_const) => bail!("todo map ExBitFieldConst"),
        Ex::ExClassContext(ex) => {
            in_conns.push((inputs.len(), build_node(ctx, ex.object_expression)));
            inputs.push(pin("object", PinType::Data));
            inputs.push(pin("offset", PinType::Int(ex.offset as i32)));
            inputs.push(pin("property", PinType::Property(ex.r_value_pointer.0)));
            in_conns.push((inputs.len(), build_node(ctx, ex.context_expression)));
            inputs.push(pin("context", PinType::Data));
        }
        //     Ex::ExMetaCast(ex_meta_cast) => bail!("todo map ExMetaCast"),
        Ex::ExLetBool(ex) => {
            in_conns.push((inputs.len(), build_node(ctx, ex.variable_expression)));
            inputs.push(pin("variable", PinType::Data));
            in_conns.push((inputs.len(), build_node(ctx, ex.assignment_expression)));
            inputs.push(pin("expression", PinType::Data));
        }
        Ex::ExEndParmValue(_) => {}
        Ex::ExEndFunctionParms(_) => {}
        Ex::ExSelf(_) => {}
        //     Ex::ExSkip(ex_skip) => bail!("todo map ExSkip"),
        Ex::ExContext(ex) => {
            in_conns.push((inputs.len(), build_node(ctx, ex.object_expression)));
            inputs.push(pin("object", PinType::Data));
            inputs.push(pin("offset", PinType::Int(ex.offset as i32)));
            inputs.push(pin("property", PinType::Property(ex.r_value_pointer.0)));
            in_conns.push((inputs.len(), build_node(ctx, ex.context_expression)));
            inputs.push(pin("context", PinType::Data));
        }
        Ex::ExContextFailSilent(ex) => {
            in_conns.push((inputs.len(), build_node(ctx, ex.object_expression)));
            inputs.push(pin("object", PinType::Data));
            inputs.push(pin("offset", PinType::Int(ex.offset as i32)));
            inputs.push(pin("property", PinType::Property(ex.r_value_pointer.0)));
            in_conns.push((inputs.len(), build_node(ctx, ex.context_expression)));
            inputs.push(pin("context", PinType::Data));
        }
        Ex::ExVirtualFunction(ex) => {
            for (i, p) in ex.parameters.iter().enumerate() {
                in_conns.push((inputs.len(), build_node(ctx, *p)));
                inputs.push(pin(format!("param {i}"), PinType::Data));
            }
        }
        Ex::ExFinalFunction(ex) => {
            inputs.push(pin("func", PinType::Function(ex.stack_node.0)));
            for (i, p) in ex.parameters.iter().enumerate() {
                in_conns.push((inputs.len(), build_node(ctx, *p)));
                inputs.push(pin(format!("param {i}"), PinType::Data));
            }
        }
        Ex::ExIntConst(ex) => {
            inputs.push(pin("value", PinType::Int(ex.value)));
        }
        Ex::ExFloatConst(ex) => {
            inputs.push(pin("value", PinType::Float(ex.value)));
        }
        Ex::ExStringConst(ex) => {
            inputs.push(pin("value", PinType::String(ex.value.to_string())));
        }
        Ex::ExObjectConst(ex) => {
            inputs.push(pin("value", PinType::Object(ex.value.0)));
        }
        Ex::ExNameConst(_) => {}
        Ex::ExRotationConst(_) => {}
        Ex::ExVectorConst(_) => {}
        Ex::ExByteConst(_) => {}
        Ex::ExIntZero(_) => {}
        Ex::ExIntOne(_) => {}
        Ex::ExTrue(_) => {}
        Ex::ExFalse(_) => {}
        Ex::ExTextConst(ex) => {
            in_conns.push((inputs.len(), build_node(ctx, ex.value)));
            inputs.push(pin("value", PinType::Data));
        }
        Ex::ExNoObject(_) => {}
        Ex::ExTransformConst(_) => {}
        Ex::ExIntConstByte(_) => {}
        Ex::ExNoInterface(_) => {}
        Ex::ExDynamicCast(ex) => {
            in_conns.push((inputs.len(), build_node(ctx, ex.target_expression)));
            inputs.push(pin("input", PinType::Data));
        }
        Ex::ExStructConst(ex) => {
            for (i, p) in ex.value.iter().enumerate() {
                in_conns.push((inputs.len(), build_node(ctx, *p)));
                inputs.push(pin(format!("member {i}"), PinType::Data));
            }
        }
        Ex::ExEndStructConst(_) => {}
        Ex::ExSetArray(ex) => {
            in_conns.push((
                inputs.len(),
                build_node(ctx, ex.assigning_property.expect("TODO old versions")),
            ));
            inputs.push(pin("property", PinType::Data));
            for (i, p) in ex.elements.iter().enumerate() {
                in_conns.push((inputs.len(), build_node(ctx, *p)));
                inputs.push(pin(format!("element {i}"), PinType::Data));
            }
        }
        Ex::ExEndArray(_) => {}
        Ex::ExPropertyConst(_) => {}
        Ex::ExUnicodeStringConst(_) => {}
        Ex::ExInt64Const(_) => {}
        Ex::ExUInt64Const(_) => {}
        Ex::ExDoubleConst(_) => {}
        Ex::ExCast(ex) => {
            in_conns.push((inputs.len(), build_node(ctx, ex.target)));
            inputs.push(pin("target", PinType::Data));
        }
        //     Ex::ExSetSet(ex_set_set) => bail!("todo map ExSetSet"),
        Ex::ExEndSet(_) => {}
        //     Ex::ExSetMap(ex_set_map) => bail!("todo map ExSetMap"),
        Ex::ExEndMap(_) => {}
        //     Ex::ExSetConst(ex_set_const) => bail!("todo map ExSetConst"),
        Ex::ExEndSetConst(_) => {}
        //     Ex::ExMapConst(ex_map_const) => bail!("todo map ExMapConst"),
        Ex::ExEndMapConst(_) => {}
        //     Ex::ExVector3fConst(ex_vector3f_const) => bail!("todo map ExVector3fConst"),
        Ex::ExStructMemberContext(ex) => {
            in_conns.push((inputs.len(), build_node(ctx, ex.struct_expression)));
            inputs.push(pin("expr", PinType::Data));
        }
        //     Ex::ExLetMulticastDelegate(ex_let_multicast_delegate) => {
        //         bail!("todo map ExLetMulticastDelegate")
        //     }
        //     Ex::ExLetDelegate(ex_let_delegate) => bail!("todo map ExLetDelegate"),
        Ex::ExLocalVirtualFunction(ex) => {
            for (i, p) in ex.parameters.iter().enumerate() {
                in_conns.push((inputs.len(), build_node(ctx, *p)));
                inputs.push(pin(format!("param {i}"), PinType::Data));
            }
        }
        Ex::ExLocalFinalFunction(ex) => {
            inputs.push(pin("func", PinType::Function(ex.stack_node.0)));
            for (i, p) in ex.parameters.iter().enumerate() {
                in_conns.push((inputs.len(), build_node(ctx, *p)));
                inputs.push(pin(format!("param {i}"), PinType::Data));
            }
        }
        Ex::ExLocalOutVariable(ex) => {
            inputs.push(pin("variable", PinType::Property(ex.variable.0)));
        }
        //     Ex::ExDeprecatedOp4A(ex_deprecated_op4_a) => bail!("todo map ExDeprecatedOp4A"),
        //     Ex::ExInstanceDelegate(ex_instance_delegate) => bail!("todo map ExInstanceDelegate"),
        Ex::ExPushExecutionFlow(ex) => {
            let push = ExprIndex(ex.pushing_address as usize);
            out_conns.push((outputs.len(), build_node(ctx, push)));
            outputs.push(pin("push", PinType::Exec));
        }
        Ex::ExPopExecutionFlow(_) => {}
        Ex::ExComputedJump(ex) => {
            in_conns.push((inputs.len(), build_node(ctx, ex.code_offset_expression)));
            inputs.push(pin("offset", PinType::Data));
        }
        Ex::ExPopExecutionFlowIfNot(ex) => {
            in_conns.push((inputs.len(), build_node(ctx, ex.boolean_expression)));
            inputs.push(pin("condition", PinType::Data));
        }
        //     Ex::ExBreakpoint(ex_breakpoint) => bail!("todo map ExBreakpoint"),
        //     Ex::ExInterfaceContext(ex_interface_context) => bail!("todo map ExInterfaceContext"),
        //     Ex::ExObjToInterfaceCast(ex_obj_to_interface_cast) => {
        //         bail!("todo map ExObjToInterfaceCast")
        //     }
        Ex::ExEndOfScript(_) => {}
        //     Ex::ExCrossInterfaceCast(ex_cross_interface_cast) => {
        //         bail!("todo map ExCrossInterfaceCast")
        //     }
        //     Ex::ExInterfaceToObjCast(ex_interface_to_obj_cast) => {
        //         bail!("todo map ExInterfaceToObjCast")
        //     }
        //     Ex::ExWireTracepoint(ex_wire_tracepoint) => bail!("todo map ExWireTracepoint"),
        //     Ex::ExSkipOffsetConst(ex_skip_offset_const) => bail!("todo map ExSkipOffsetConst"),
        //     Ex::ExAddMulticastDelegate(ex_add_multicast_delegate) => {
        //         bail!("todo map ExAddMulticastDelegate")
        //     }
        //     Ex::ExClearMulticastDelegate(ex_clear_multicast_delegate) => {
        //         bail!("todo map ExClearMulticastDelegate")
        //     }
        //     Ex::ExTracepoint(ex_tracepoint) => bail!("todo map ExTracepoint"),
        Ex::ExLetObj(ex) => {
            in_conns.push((inputs.len(), build_node(ctx, ex.variable_expression)));
            inputs.push(pin("variable", PinType::Data));
            in_conns.push((inputs.len(), build_node(ctx, ex.assignment_expression)));
            inputs.push(pin("expression", PinType::Data));
        }
        //     Ex::ExLetWeakObjPtr(ex_let_weak_obj_ptr) => bail!("todo map ExLetWeakObjPtr"),
        //     Ex::ExBindDelegate(ex_bind_delegate) => bail!("todo map ExBindDelegate"),
        //     Ex::ExRemoveMulticastDelegate(ex_remove_multicast_delegate) => {
        //         bail!("todo map ExRemoveMulticastDelegate")
        //     }
        Ex::ExCallMulticastDelegate(ex) => {
            inputs.push(pin("func", PinType::Function(ex.stack_node.0)));
            for (i, p) in ex.parameters.iter().enumerate() {
                in_conns.push((inputs.len(), build_node(ctx, *p)));
                inputs.push(pin(format!("param {i}"), PinType::Data));
            }
        }
        Ex::ExLetValueOnPersistentFrame(ex) => {
            inputs.push(pin(
                "property",
                PinType::Property(ex.destination_property.0),
            ));
            in_conns.push((inputs.len(), build_node(ctx, ex.assignment_expression)));
            inputs.push(pin("value", PinType::Data));
        }
        //     Ex::ExArrayConst(ex_array_const) => bail!("todo map ExArrayConst"),
        //     Ex::ExEndArrayConst(ex_end_array_const) => bail!("todo map ExEndArrayConst"),
        //     Ex::ExSoftObjectConst(ex_soft_object_const) => bail!("todo map ExSoftObjectConst"),
        Ex::ExCallMath(ex) => {
            inputs.push(pin("func", PinType::Function(ex.stack_node.0)));
            for (i, p) in ex.parameters.iter().enumerate() {
                in_conns.push((inputs.len(), build_node(ctx, *p)));
                inputs.push(pin(format!("param {i}"), PinType::Data));
            }
        }
        //     Ex::ExSwitchValue(ex_switch_value) => bail!("todo map ExSwitchValue"),
        //     Ex::ExInstrumentationEvent(ex_instrumentation_event) => {
        //         bail!("todo map ExInstrumentationEvent")
        //     }
        //     Ex::ExArrayGetByRef(ex_array_get_by_ref) => bail!("todo map ExArrayGetByRef"),
        //     Ex::ExClassSparseDataVariable(ex_class_sparse_data_variable) => {
        //         bail!("todo map ExClassSparseDataVariable")
        //     }
        //     Ex::ExFieldPathConst(ex_field_path_const) => bail!("todo map ExFieldPathConst"),
        //     Ex::ExAutoRtfmTransact(ex_auto_rtfm_transact) => bail!("todo map ExAutoRtfmTransact"),
        //     Ex::ExAutoRtfmStopTransact(ex_auto_rtfm_stop_transact) => {
        //         bail!("todo map ExAutoRtfmStopTransact")
        //     }
        //     Ex::ExAutoRtfmAbortIfNot(ex_auto_rtfm_abort_if_not) => {
        //         bail!("todo map ExAutoRtfmAbortIfNot")
        //     }
        _ => {
            dbg!(name);
        }
    }
    ctx.snarl[id].inputs.extend(inputs);
    ctx.snarl[id].outputs.extend(outputs);

    for (input, prev_id) in in_conns {
        ctx.snarl.connect(
            OutPinId {
                node: prev_id,
                output: 0,
            },
            InPinId { node: id, input },
        );
    }
    for (output, next_id) in out_conns {
        ctx.snarl.connect(
            OutPinId { node: id, output },
            InPinId {
                node: next_id,
                input: 0,
            },
        );
    }

    id
}

pub fn compile(
    function: &mut ue::UFunction,
    graph: &KismetGraph,
) -> Result<Vec<crate::kismet::literal::Expr>> {
    use crate::kismet::literal::{Expr, ExprOp as Op, *};

    let snarl = &graph.snarl;
    let entry = snarl
        .node_ids()
        .find_map(|(id, n)| matches!(n.node_type, NodeType::FunctionDef(_)).then_some(id))
        .context("no function entry found")?;

    let mut prev_map: HashMap<InPinId, OutPinId> = Default::default();
    let mut next_map: HashMap<OutPinId, InPinId> = Default::default();

    for (out_pin, in_pin) in snarl.wires() {
        prev_map.insert(in_pin, out_pin);
        next_map.insert(out_pin, in_pin);
    }

    struct Ctx<'a> {
        snarl: &'a Snarl<GenericNode>,
        exs: Vec<Expr>,
        queue: VecDeque<NodeId>,
        prev_map: HashMap<InPinId, OutPinId>,
        next_map: HashMap<OutPinId, InPinId>,
    }

    let mut c = Ctx {
        snarl,
        exs: Default::default(),
        queue: Default::default(),
        prev_map,
        next_map,
    };

    impl Ctx<'_> {
        fn get_out_pin(&self, node: NodeId, pin_name: &str) -> Result<(usize, &GenericPin)> {
            self.snarl[node]
                .outputs
                .iter()
                .enumerate()
                .find(|(_, p)| p.name == pin_name)
                .with_context(|| format!("missing output pin \"{pin_name}\""))
        }
        fn get_in_pin(&self, node: NodeId, pin_name: &str) -> Result<(usize, &GenericPin)> {
            self.snarl[node]
                .inputs
                .iter()
                .enumerate()
                .find(|(_, p)| p.name == pin_name)
                .with_context(|| format!("missing input pin \"{pin_name}\""))
        }
        fn get_next(&self, node: NodeId, pin_name: &str) -> Result<NodeId> {
            let output = self.get_out_pin(node, pin_name)?.0;
            self.next_map
                .get(&OutPinId { node, output })
                .map(|info| info.node)
                .with_context(|| format!("pin \"{pin_name}\" not connected"))
        }
        fn get_prev(&self, node: NodeId, pin_name: &str) -> Result<NodeId> {
            let input = self.get_in_pin(node, pin_name)?.0;
            self.prev_map
                .get(&InPinId { node, input })
                .map(|info| info.node)
                .with_context(|| format!("pin \"{pin_name}\" not connected"))
        }
        fn pin_prop(&self, node: NodeId, pin_name: &str) -> Result<KismetPropertyPointer> {
            match self.get_in_pin(node, pin_name)?.1.pin_type {
                PinType::Property(v) => Ok(KismetPropertyPointer(v)),
                _ => bail!("expected Property pin type"),
            }
        }
        fn pin_object(&self, node: NodeId, pin_name: &str) -> Result<PackageIndex> {
            match self.get_in_pin(node, pin_name)?.1.pin_type {
                PinType::Object(v) => Ok(PackageIndex(v)),
                _ => bail!("expected Object pin type"),
            }
        }
        fn pin_function(&self, node: NodeId, pin_name: &str) -> Result<PackageIndex> {
            match self.get_in_pin(node, pin_name)?.1.pin_type {
                PinType::Function(v) => Ok(PackageIndex(v)),
                _ => bail!("expected Function pin type"),
            }
        }
        fn pin_int(&self, node: NodeId, pin_name: &str) -> Result<i32> {
            match self.get_in_pin(node, pin_name)?.1.pin_type {
                PinType::Int(v) => Ok(v),
                _ => bail!("expected Int pin type"),
            }
        }
    }

    fn build_ex(c: &mut Ctx, id: NodeId) -> Result<ExprIndex> {
        let node = &c.snarl[id];
        let op = match node.node_type {
            NodeType::Expr(expr_op) => expr_op,
            _ => unreachable!(),
        };
        let res = ExprIndex(c.exs.len());
        c.exs.push(ExNothing {}.into()); // tmp value
        let ex: crate::kismet::literal::Expr = match op {
            Op::ExLocalVariable => ExLocalVariable {
                variable: c.pin_prop(id, "property")?,
            }
            .into(),
            Op::ExInstanceVariable => ExInstanceVariable {
                variable: c.pin_prop(id, "property")?,
            }
            .into(),
            Op::ExDefaultVariable => bail!("gen ExDefaultVariable"),
            Op::ExReturn => ExReturn {
                return_expression: build_ex(c, c.get_prev(id, "return")?)?,
            }
            .into(),
            Op::ExJump => bail!("gen ExJump"),
            Op::ExJumpIfNot => bail!("gen ExJumpIfNot"),
            Op::ExAssert => bail!("gen ExAssert"),
            Op::ExNothing => ExNothing {}.into(),
            Op::ExNothingInt32 => bail!("gen ExNothingInt32"),
            Op::ExLet => ExLet {
                value: c.pin_prop(id, "value")?,
                variable: build_ex(c, c.get_prev(id, "variable")?)?,
                expression: build_ex(c, c.get_prev(id, "expression")?)?,
            }
            .into(),
            Op::ExBitFieldConst => bail!("gen ExBitFieldConst"),
            Op::ExClassContext => bail!("gen ExClassContext"),
            Op::ExMetaCast => bail!("gen ExMetaCast"),
            Op::ExLetBool => ExLetBool {
                variable_expression: build_ex(c, c.get_prev(id, "variable")?)?,
                assignment_expression: build_ex(c, c.get_prev(id, "expression")?)?,
            }
            .into(),
            Op::ExEndParmValue => bail!("gen ExEndParmValue"),
            Op::ExEndFunctionParms => bail!("gen ExEndFunctionParms"),
            Op::ExSelf => bail!("gen ExSelf"),
            Op::ExSkip => bail!("gen ExSkip"),
            Op::ExContext => ExContext {
                object_expression: build_ex(c, c.get_prev(id, "object")?)?,
                offset: c.pin_int(id, "offset")? as u32,
                r_value_pointer: c.pin_prop(id, "property")?,
                context_expression: build_ex(c, c.get_prev(id, "context")?)?,
            }
            .into(),
            Op::ExContextFailSilent => bail!("gen ExContextFailSilent"),
            Op::ExVirtualFunction => bail!("gen ExVirtualFunction"),
            Op::ExFinalFunction => ExFinalFunction {
                stack_node: c.pin_function(id, "func")?,
                parameters: node
                    .inputs
                    .iter()
                    .filter_map(|p| p.name.starts_with("param ").then_some(&p.name))
                    .map(|n| c.get_prev(id, n).and_then(|n| build_ex(c, n)))
                    .collect::<Result<Vec<_>>>()?,
            }
            .into(),
            Op::ExIntConst => ExIntConst {
                value: c.pin_int(id, "value")?,
            }
            .into(),
            Op::ExFloatConst => bail!("gen ExFloatConst"),
            Op::ExStringConst => bail!("gen ExStringConst"),
            Op::ExObjectConst => ExObjectConst {
                value: c.pin_object(id, "value")?,
            }
            .into(),
            Op::ExNameConst => bail!("gen ExNameConst"),
            Op::ExRotationConst => bail!("gen ExRotationConst"),
            Op::ExVectorConst => bail!("gen ExVectorConst"),
            Op::ExByteConst => bail!("gen ExByteConst"),
            Op::ExIntZero => bail!("gen ExIntZero"),
            Op::ExIntOne => bail!("gen ExIntOne"),
            Op::ExTrue => bail!("gen ExTrue"),
            Op::ExFalse => bail!("gen ExFalse"),
            Op::ExTextConst => bail!("gen ExTextConst"),
            Op::ExNoObject => bail!("gen ExNoObject"),
            Op::ExTransformConst => bail!("gen ExTransformConst"),
            Op::ExIntConstByte => bail!("gen ExIntConstByte"),
            Op::ExNoInterface => bail!("gen ExNoInterface"),
            Op::ExDynamicCast => bail!("gen ExDynamicCast"),
            Op::ExStructConst => bail!("gen ExStructConst"),
            Op::ExEndStructConst => bail!("gen ExEndStructConst"),
            Op::ExSetArray => bail!("gen ExSetArray"),
            Op::ExEndArray => bail!("gen ExEndArray"),
            Op::ExPropertyConst => bail!("gen ExPropertyConst"),
            Op::ExUnicodeStringConst => bail!("gen ExUnicodeStringConst"),
            Op::ExInt64Const => bail!("gen ExInt64Const"),
            Op::ExUInt64Const => bail!("gen ExUInt64Const"),
            Op::ExDoubleConst => bail!("gen ExDoubleConst"),
            Op::ExCast => bail!("gen ExCast"),
            Op::ExSetSet => bail!("gen ExSetSet"),
            Op::ExEndSet => bail!("gen ExEndSet"),
            Op::ExSetMap => bail!("gen ExSetMap"),
            Op::ExEndMap => bail!("gen ExEndMap"),
            Op::ExSetConst => bail!("gen ExSetConst"),
            Op::ExEndSetConst => bail!("gen ExEndSetConst"),
            Op::ExMapConst => bail!("gen ExMapConst"),
            Op::ExEndMapConst => bail!("gen ExEndMapConst"),
            Op::ExVector3fConst => bail!("gen ExVector3fConst"),
            Op::ExStructMemberContext => bail!("gen ExStructMemberContext"),
            Op::ExLetMulticastDelegate => bail!("gen ExLetMulticastDelegate"),
            Op::ExLetDelegate => bail!("gen ExLetDelegate"),
            Op::ExLocalVirtualFunction => bail!("gen ExLocalVirtualFunction"),
            Op::ExLocalFinalFunction => bail!("gen ExLocalFinalFunction"),
            Op::ExLocalOutVariable => ExLocalOutVariable {
                variable: c.pin_prop(id, "variable")?,
            }
            .into(),
            Op::ExDeprecatedOp4A => bail!("gen ExDeprecatedOp4A"),
            Op::ExInstanceDelegate => bail!("gen ExInstanceDelegate"),
            Op::ExPushExecutionFlow => bail!("gen ExPushExecutionFlow"),
            Op::ExPopExecutionFlow => bail!("gen ExPopExecutionFlow"),
            Op::ExComputedJump => bail!("gen ExComputedJump"),
            Op::ExPopExecutionFlowIfNot => bail!("gen ExPopExecutionFlowIfNot"),
            Op::ExBreakpoint => bail!("gen ExBreakpoint"),
            Op::ExInterfaceContext => bail!("gen ExInterfaceContext"),
            Op::ExObjToInterfaceCast => bail!("gen ExObjToInterfaceCast"),
            Op::ExEndOfScript => bail!("gen ExEndOfScript"),
            Op::ExCrossInterfaceCast => bail!("gen ExCrossInterfaceCast"),
            Op::ExInterfaceToObjCast => bail!("gen ExInterfaceToObjCast"),
            Op::ExWireTracepoint => bail!("gen ExWireTracepoint"),
            Op::ExSkipOffsetConst => bail!("gen ExSkipOffsetConst"),
            Op::ExAddMulticastDelegate => bail!("gen ExAddMulticastDelegate"),
            Op::ExClearMulticastDelegate => bail!("gen ExClearMulticastDelegate"),
            Op::ExTracepoint => bail!("gen ExTracepoint"),
            Op::ExLetObj => bail!("gen ExLetObj"),
            Op::ExLetWeakObjPtr => bail!("gen ExLetWeakObjPtr"),
            Op::ExBindDelegate => bail!("gen ExBindDelegate"),
            Op::ExRemoveMulticastDelegate => bail!("gen ExRemoveMulticastDelegate"),
            Op::ExCallMulticastDelegate => bail!("gen ExCallMulticastDelegate"),
            Op::ExLetValueOnPersistentFrame => bail!("gen ExLetValueOnPersistentFrame"),
            Op::ExArrayConst => bail!("gen ExArrayConst"),
            Op::ExEndArrayConst => bail!("gen ExEndArrayConst"),
            Op::ExSoftObjectConst => bail!("gen ExSoftObjectConst"),
            Op::ExCallMath => bail!("gen ExCallMath"),
            Op::ExSwitchValue => bail!("gen ExSwitchValue"),
            Op::ExInstrumentationEvent => bail!("gen ExInstrumentationEvent"),
            Op::ExArrayGetByRef => bail!("gen ExArrayGetByRef"),
            Op::ExClassSparseDataVariable => bail!("gen ExClassSparseDataVariable"),
            Op::ExFieldPathConst => bail!("gen ExFieldPathConst"),
            Op::ExAutoRtfmTransact => bail!("gen ExAutoRtfmTransact"),
            Op::ExAutoRtfmStopTransact => bail!("gen ExAutoRtfmStopTransact"),
            Op::ExAutoRtfmAbortIfNot => bail!("gen ExAutoRtfmAbortIfNot"),
        };
        c.exs[res.0] = ex;

        if c.get_out_pin(id, "then").is_ok() {
            c.queue.push_front(c.get_next(id, "then")?);
        }

        Ok(res)
    }

    c.queue.push_back(c.get_next(entry, "then")?);

    while let Some(next) = c.queue.pop_front() {
        build_ex(&mut c, next)?;
    }

    // let exec_map: HashMap<NodeId> = self
    //     .connections
    //     .iter()
    //     .filter(|c| c.connection_type == ConnectionType::Exec)
    //     .map(|c| c.from_node)
    //     .collect();

    Ok(c.exs)
}

mod layout {
    type Position = eframe::egui::Pos2;

    use super::*;

    pub fn layout(snarl: &mut Snarl<GenericNode>) {
        let mut layout = GraphLayout::new();

        for (id, _pos, _node) in snarl.nodes_pos_ids() {
            layout.add_node(Node::new(id));
        }
        for (out_pin, in_pin) in snarl.wires() {
            let connection_type = match snarl[out_pin.node].outputs[out_pin.output].pin_type {
                PinType::Exec => ConnectionType::Exec,
                PinType::Data => ConnectionType::Data,
                _ => ConnectionType::Other,
            };
            layout.add_connection(Connection {
                connection_type,
                from_node: out_pin.node,
                from_output: out_pin.output,
                to_node: in_pin.node,
                to_input: in_pin.input,
            });
        }

        layout.compute_layout();

        for id in layout.nodes.keys().cloned() {
            snarl.get_node_info_mut(id).unwrap().pos = layout.grid_to_position(id);
        }
    }

    use std::collections::{HashMap, HashSet, VecDeque};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct GridCell {
        pub row: i32,
        pub col: i32,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub enum ConnectionType {
        Exec,
        Data,
        Other,
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct Connection {
        pub connection_type: ConnectionType,
        pub from_node: NodeId,
        pub from_output: usize,
        pub to_node: NodeId,
        pub to_input: usize,
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    pub struct Node {
        pub id: NodeId,
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    pub struct GraphLayout {
        pub nodes: HashMap<NodeId, Node>,
        pub connections: Vec<Connection>,
        pub grid: HashMap<GridCell, NodeId>,
        pub grid_inv: HashMap<NodeId, GridCell>,
    }

    fn cell(row: i32, col: i32) -> GridCell {
        GridCell { row, col }
    }

    impl Node {
        pub fn new(id: NodeId) -> Self {
            Self { id }
        }
    }

    impl GraphLayout {
        pub fn new() -> Self {
            Self {
                nodes: HashMap::new(),
                connections: Vec::new(),
                grid: HashMap::new(),
                grid_inv: HashMap::new(),
            }
        }

        pub fn add_node(&mut self, node: Node) {
            self.nodes.insert(node.id, node);
        }

        pub fn add_connection(&mut self, connection: Connection) {
            // Validate that nodes exist
            if !self.nodes.contains_key(&connection.from_node) {
                panic!("Source node {:?} does not exist", connection.from_node);
            }
            if !self.nodes.contains_key(&connection.to_node) {
                panic!("Target node {:?} does not exist", connection.to_node);
            }

            self.connections.push(connection);
        }
        pub fn print_grid(&self, name: &str) {
            println!("Grid {name}:");
            if self.grid.is_empty() {
                println!("Empty grid");
                return;
            }

            // Find grid bounds
            let min_row = self.grid.keys().map(|n| n.row).min().unwrap();
            let max_row = self.grid.keys().map(|n| n.row).max().unwrap();
            let min_col = self.grid.keys().map(|n| n.col).min().unwrap();
            let max_col = self.grid.keys().map(|n| n.col).max().unwrap();

            // Print column headers
            for col in min_col..=max_col {
                print!("{:8}", col);
            }
            println!();

            // Print each row
            for row in min_row..=max_row {
                print!("{:5}: ", row);
                for col in min_col..=max_col {
                    let cell = cell(row, col);
                    if let Some(&node_id) = self.grid.get(&cell) {
                        // let node_name = &self.nodes[&node_id].name;
                        let node_name = format!("{}", node_id.0);
                        print!("{:8}", &node_name[..node_name.len().min(7)]);
                    } else {
                        print!("{:8}", ".");
                    }
                }
                println!();
            }
        }
        fn grid_to_position(&self, node: NodeId) -> Position {
            let cell = self.grid_inv.get(&node).unwrap();
            Position {
                x: cell.col as f32 * 400.0,
                y: cell.row as f32 * 200.0,
            }
        }
    }

    impl GraphLayout {
        fn compute_layout(&mut self) {
            let mut queue = VecDeque::new();

            // Find root nodes (no exec inputs and at least one exec output)
            let exec_inputs: HashSet<NodeId> = self
                .connections
                .iter()
                .filter(|c| c.connection_type == ConnectionType::Exec)
                .map(|c| c.to_node)
                .collect();

            let exec_outputs: HashSet<NodeId> = self
                .connections
                .iter()
                .filter(|c| c.connection_type == ConnectionType::Exec)
                .map(|c| c.from_node)
                .collect();

            let roots: Vec<NodeId> = self
                .nodes
                .keys()
                .copied()
                .filter(|id| !exec_inputs.contains(&id) && exec_outputs.contains(&id))
                .collect();

            let mut inputs: HashMap<NodeId, Vec<(usize, NodeId)>> = HashMap::new();
            let mut outputs: HashMap<NodeId, Vec<(usize, NodeId)>> = HashMap::new();
            for conn in &self.connections {
                inputs
                    .entry(conn.to_node)
                    .or_default()
                    .push((conn.to_input, conn.from_node));
                outputs
                    .entry(conn.from_node)
                    .or_default()
                    .push((conn.from_output, conn.to_node));
            }
            for input in inputs.values_mut() {
                input.sort();
            }
            for outputs in outputs.values_mut() {
                outputs.sort();
            }

            // next completely empty row
            let mut next_row = 0;

            #[derive(PartialEq)]
            enum Dir {
                Right,
                Left,
            }

            for root_id in roots {
                queue.push_back((root_id, cell(next_row, 0), Dir::Right));
                while let Some((current_id, current_cell, dir)) = queue.pop_front() {
                    if self.grid_inv.contains_key(&current_id) {
                        // already placed
                        continue;
                    }
                    if self.grid.contains_key(&current_cell) {
                        // cell is occopied by another node, need to make space
                        let shift_fn: Box<dyn Fn(&mut GridCell)> = match dir {
                            Dir::Right => Box::new(|c: &mut GridCell| {
                                if c.row >= current_cell.row && c.col >= current_cell.col {
                                    c.col += 1;
                                }
                            }),
                            Dir::Left => {
                                next_row += 1;
                                Box::new(|c: &mut GridCell| {
                                    if c.row >= current_cell.row && c.col >= current_cell.col {
                                        c.row += 1;
                                    }
                                })
                            }
                        };
                        self.shift(&shift_fn);
                        for (_id, c, _dir) in queue.iter_mut() {
                            shift_fn(c);
                        }
                    }

                    self.place_node(current_id, current_cell);
                    next_row = next_row.max(current_cell.row + 1);

                    if let Some(outputs) = outputs.get(&current_id) {
                        for &(i, output_id) in outputs {
                            if !self.grid_inv.contains_key(&output_id) {
                                queue.push_back((
                                    output_id,
                                    cell(current_cell.row + i as i32, current_cell.col + 1),
                                    Dir::Right,
                                ));
                            }
                        }
                    }
                    if let Some(inputs) = inputs.get(&current_id) {
                        let mut offset = 0;
                        for &(i, input_id) in inputs {
                            // special case working backwards to remain horizontal if space is avaialble
                            if i == 0
                                && !self
                                    .grid
                                    .contains_key(&cell(current_cell.row, current_cell.col - 1))
                            {
                            } else {
                                offset += 1;
                            }
                            if !self.grid_inv.contains_key(&input_id) {
                                queue.push_back((
                                    input_id,
                                    cell(current_cell.row + offset as i32, current_cell.col - 1),
                                    Dir::Left,
                                ));
                            }
                        }
                    }
                }
            }

            for node_id in self.nodes.keys().copied().collect::<Vec<_>>() {
                if !self.grid_inv.contains_key(&node_id) {
                    self.place_node(node_id, cell(next_row, 0));
                    next_row += 1;
                }
            }
        }

        fn check(&self) {
            for (key, value) in &self.grid {
                assert_eq!(Some(key), self.grid_inv.get(value));
            }
            for (key, value) in &self.grid_inv {
                assert_eq!(Some(key), self.grid.get(value));
            }
        }
        fn place_node(&mut self, id: NodeId, cell: GridCell) {
            assert_eq!(self.grid.insert(cell, id), None);
            assert_eq!(self.grid_inv.insert(id, cell), None);
        }

        fn shift_columns_right(&mut self, from_col: i32) {
            let mut to_move: Vec<(GridCell, NodeId)> = Vec::new();
            for (&cell, &node_id) in &self.grid {
                if cell.col >= from_col {
                    to_move.push((cell, node_id));
                }
            }
            for (old_cell, old_id) in &to_move {
                self.grid.remove(old_cell);
                self.grid_inv.remove(old_id);
            }
            for (old_cell, node_id) in to_move {
                self.place_node(node_id, cell(old_cell.row, old_cell.col + 1));
            }
        }
        fn shift_rows_down(&mut self, from_row: i32) {
            let mut to_move: Vec<(GridCell, NodeId)> = Vec::new();
            for (&cell, &node_id) in &self.grid {
                if cell.row >= from_row {
                    to_move.push((cell, node_id));
                }
            }
            for (old_cell, old_id) in &to_move {
                self.grid.remove(old_cell);
                self.grid_inv.remove(old_id);
            }
            for (old_cell, node_id) in to_move {
                self.place_node(node_id, cell(old_cell.row + 1, old_cell.col));
            }
        }
        fn shift<F>(&mut self, shift_fn: F)
        where
            F: Fn(&mut GridCell),
        {
            let mut to_move: Vec<(GridCell, GridCell, NodeId)> = Vec::new();
            for (&cell, &node_id) in &self.grid {
                let mut shifted = cell;
                shift_fn(&mut shifted);
                if cell != shifted {
                    to_move.push((cell, shifted, node_id));
                }
            }
            for (old_cell, _new_cell, old_id) in &to_move {
                self.grid.remove(old_cell);
                self.grid_inv.remove(old_id);
            }
            for (_old_cell, new_cell, node_id) in to_move {
                self.place_node(node_id, new_cell);
            }
        }
    }
    #[cfg(test)]
    mod test {
        use super::*;

        fn n(id: NodeId, name: &str) -> Node {
            Node { id }
        }

        #[test]
        fn test_shift_nodes() {
            let mut layout = GraphLayout::new();

            let a = NodeId(1);
            let b = NodeId(2);
            let c = NodeId(3);

            // Create nodes in exec order: A -> B
            layout.nodes.insert(a, n(a, "A"));
            layout.nodes.insert(b, n(b, "B"));
            layout.nodes.insert(c, n(c, "C"));

            // Set initial positions as if exec layout was done
            layout.place_node(a, cell(0, 0));
            layout.place_node(b, cell(0, 1));
            layout.place_node(c, cell(0, 2));

            layout.print_grid("initial");

            layout.shift_columns_right(1);

            layout.print_grid("shifted");

            assert_eq!(*layout.grid_inv.get(&a).unwrap(), cell(0, 0));
            assert_eq!(*layout.grid_inv.get(&b).unwrap(), cell(0, 2));
            assert_eq!(*layout.grid_inv.get(&c).unwrap(), cell(0, 3));

            // layout.connections.push(Connection {
            //     connection_type: ConnectionType::Data,
            //     from_node: NodeId(2),
            //     from_output: 0,
            //     to_node: NodeId(1),
            //     to_input: 0,
            // });

            // let final_distance =
            //     (layout.nodes[&2].grid_cell.col - layout.nodes[&1].grid_cell.col).abs();
            // assert!(
            //     final_distance > 1,
            //     "Nodes should be further apart after data layout"
            // );
        }
        #[test]
        fn test_basic_layout() {
            let mut layout = GraphLayout::new();

            fn conn(
                f_id: NodeId,
                f_pin: usize,
                t_id: NodeId,
                t_pin: usize,
                t: ConnectionType,
            ) -> Connection {
                Connection {
                    connection_type: t,
                    from_node: f_id,
                    from_output: f_pin,
                    to_node: t_id,
                    to_input: t_pin,
                }
            }

            let mut counter = 0;
            let mut node = |name: &str| -> NodeId {
                counter += 1;
                let id = NodeId(counter);
                layout.nodes.insert(id, n(id, name.into()));
                id
            };

            // b -> a+-> c -> f
            //       +-> d -> e

            let a = node("A");
            let b = node("B");
            let c = node("C");
            let d = node("D");
            let e = node("E");
            let f = node("F");
            let g = node("G");
            let h = node("H");
            let i = node("I");
            let j = node("J");
            let k = node("K");

            use ConnectionType::*;
            layout.connections.push(conn(b, 0, a, 0, Exec));
            layout.connections.push(conn(a, 0, c, 0, Exec));
            layout.connections.push(conn(a, 1, d, 0, Exec));
            layout.connections.push(conn(d, 0, e, 0, Exec));
            layout.connections.push(conn(c, 0, f, 0, Exec));
            // layout.connections.push(conn(g, 0, c, 1, Data));
            // layout.connections.push(conn(g, 0, f, 1, Data));

            layout.connections.push(conn(h, 0, i, 0, Exec));
            layout.connections.push(conn(h, 0, k, 0, Exec));
            layout.connections.push(conn(i, 0, j, 0, Exec));

            layout.compute_layout();

            layout.print_grid("shifted");

            // assert_eq!(*layout.grid_inv.get(&a).unwrap(), cell(0, 0));
            // assert_eq!(*layout.grid_inv.get(&b).unwrap(), cell(0, 2));
            // assert_eq!(*layout.grid_inv.get(&c).unwrap(), cell(0, 3));

            // layout.connections.push(Connection {
            //     connection_type: ConnectionType::Data,
            //     from_node: NodeId(2),
            //     from_output: 0,
            //     to_node: NodeId(1),
            //     to_input: 0,
            // });

            // let final_distance =
            //     (layout.nodes[&2].grid_cell.col - layout.nodes[&1].grid_cell.col).abs();
            // assert!(
            //     final_distance > 1,
            //     "Nodes should be further apart after data layout"
            // );
        }

        #[test]
        fn test_graph_json() {
            let mut layout: GraphLayout =
                serde_json::from_slice(include_bytes!("../graph.json")).unwrap();

            layout.compute_layout();

            layout.print_grid("json");

            // assert_eq!(*layout.grid_inv.get(&a).unwrap(), cell(0, 0));
            // assert_eq!(*layout.grid_inv.get(&b).unwrap(), cell(0, 2));
            // assert_eq!(*layout.grid_inv.get(&c).unwrap(), cell(0, 3));

            // layout.connections.push(Connection {
            //     connection_type: ConnectionType::Data,
            //     from_node: NodeId(2),
            //     from_output: 0,
            //     to_node: NodeId(1),
            //     to_input: 0,
            // });

            // let final_distance =
            //     (layout.nodes[&2].grid_cell.col - layout.nodes[&1].grid_cell.col).abs();
            // assert!(
            //     final_distance > 1,
            //     "Nodes should be further apart after data layout"
            // );
        }
    }
}
