use std::collections::HashMap;

use crate::{
    kismet::ExprIndex,
    kismet_nodes::{GenericNode, GenericPin, KismetGraph, PinType},
    ue,
};
use anyhow::{bail, Result};
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
    dbg!(&exs);
    let to_add = exs.keys().cloned().collect::<Vec<_>>();

    let mut ctx = Ctx {
        exs,
        snarl: Default::default(),
        to_add,
        node_map: Default::default(),
    };

    build_node(&mut ctx, ExprIndex(0));

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

    let (inputs, mut outputs) = if node.top_level {
        (
            vec![GenericPin {
                name: "exec".into(),
                pin_type: PinType::Exec,
            }],
            vec![GenericPin {
                name: "then".into(),
                pin_type: PinType::Exec,
            }],
        )
    } else {
        (
            vec![],
            vec![GenericPin {
                name: "output".into(),
                pin_type: PinType::Data,
            }],
        )
    };

    let mut input_index = inputs.len();

    fn inc(i: &mut usize) -> usize {
        *i += 1;
        *i - 1
    }

    let id = ctx.snarl.insert_node(
        egui::Pos2::ZERO,
        GenericNode {
            node_type: name.as_str().into(),
            inputs,
            outputs,
        },
    );
    ctx.node_map.insert(index, id);

    let mut more_inputs = vec![];
    let mut more_outputs = vec![];

    if node.top_level {
        if let Some(next) = node.next {
            out_conns.push((0, build_node(ctx, next)));
        }
    }
    fn pin(name: impl Into<String>, pin_type: PinType) -> GenericPin {
        GenericPin {
            name: name.into(),
            pin_type,
        }
    }
    use crate::kismet::literal::Expr as Ex;
    match &node.expr {
        // Ex::ExSkipOffsetConst(ex) => {} TODO
        Ex::ExLocalVariable(_) => {}
        Ex::ExInstanceVariable(_) => {}
        Ex::ExDefaultVariable(_) => {}
        Ex::ExReturn(ex) => {
            in_conns.push((inc(&mut input_index), build_node(ctx, ex.return_expression)));
            more_inputs.push(pin("return", PinType::Data));
        }
        Ex::ExJump(ex) => {
            let next = ExprIndex(ex.code_offset as usize);
            out_conns.push((0, build_node(ctx, next)));
        }
        Ex::ExJumpIfNot(ex) => {
            let if_not = ExprIndex(ex.code_offset as usize);
            out_conns.push((1, build_node(ctx, if_not)));
            more_outputs.push(GenericPin {
                name: "else".into(),
                pin_type: PinType::Exec,
            });
            in_conns.push((
                inc(&mut input_index),
                build_node(ctx, ex.boolean_expression),
            ));
            more_inputs.push(pin("condition", PinType::Data));
        }
        //     Ex::ExAssert(ex_assert) => bail!("todo map ExAssert"),
        Ex::ExNothing(_) => {}
        Ex::ExNothingInt32(_) => {}
        Ex::ExLet(ex) => {
            in_conns.push((inc(&mut input_index), build_node(ctx, ex.variable)));
            more_inputs.push(pin("variable", PinType::Data));
            in_conns.push((inc(&mut input_index), build_node(ctx, ex.expression)));
            more_inputs.push(pin("expression", PinType::Data));
        }
        //     Ex::ExBitFieldConst(ex_bit_field_const) => bail!("todo map ExBitFieldConst"),
        Ex::ExClassContext(ex) => {
            in_conns.push((inc(&mut input_index), build_node(ctx, ex.object_expression)));
            more_inputs.push(pin("object", PinType::Data));
            in_conns.push((
                inc(&mut input_index),
                build_node(ctx, ex.context_expression),
            ));
            more_inputs.push(pin("context", PinType::Data));
        }
        //     Ex::ExMetaCast(ex_meta_cast) => bail!("todo map ExMetaCast"),
        Ex::ExLetBool(ex) => {
            in_conns.push((
                inc(&mut input_index),
                build_node(ctx, ex.variable_expression),
            ));
            more_inputs.push(pin("variable", PinType::Data));
            in_conns.push((
                inc(&mut input_index),
                build_node(ctx, ex.assignment_expression),
            ));
            more_inputs.push(pin("expression", PinType::Data));
        }
        Ex::ExEndParmValue(_) => {}
        Ex::ExEndFunctionParms(_) => {}
        Ex::ExSelf(_) => {}
        //     Ex::ExSkip(ex_skip) => bail!("todo map ExSkip"),
        Ex::ExContext(ex) => {
            in_conns.push((inc(&mut input_index), build_node(ctx, ex.object_expression)));
            more_inputs.push(pin("object", PinType::Data));
            in_conns.push((
                inc(&mut input_index),
                build_node(ctx, ex.context_expression),
            ));
            more_inputs.push(pin("context", PinType::Data));
        }
        Ex::ExContextFailSilent(ex) => {
            in_conns.push((inc(&mut input_index), build_node(ctx, ex.object_expression)));
            more_inputs.push(pin("object", PinType::Data));
            in_conns.push((
                inc(&mut input_index),
                build_node(ctx, ex.context_expression),
            ));
            more_inputs.push(pin("context", PinType::Data));
        }
        Ex::ExVirtualFunction(ex) => {
            for (i, p) in ex.parameters.iter().enumerate() {
                in_conns.push((inc(&mut input_index), build_node(ctx, *p)));
                more_inputs.push(pin(format!("param {i}"), PinType::Data));
            }
        }
        Ex::ExFinalFunction(ex) => {
            for (i, p) in ex.parameters.iter().enumerate() {
                in_conns.push((inc(&mut input_index), build_node(ctx, *p)));
                more_inputs.push(pin(format!("param {i}"), PinType::Data));
            }
        }
        Ex::ExIntConst(_) => {}
        Ex::ExFloatConst(_) => {}
        Ex::ExStringConst(_) => {}
        Ex::ExObjectConst(_) => {}
        Ex::ExNameConst(_) => {}
        Ex::ExRotationConst(_) => {}
        Ex::ExVectorConst(_) => {}
        Ex::ExByteConst(_) => {}
        Ex::ExIntZero(_) => {}
        Ex::ExIntOne(_) => {}
        Ex::ExTrue(_) => {}
        Ex::ExFalse(_) => {}
        Ex::ExTextConst(ex) => {
            in_conns.push((inc(&mut input_index), build_node(ctx, ex.value)));
            more_inputs.push(pin("value", PinType::Data));
        }
        Ex::ExNoObject(_) => {}
        Ex::ExTransformConst(_) => {}
        Ex::ExIntConstByte(_) => {}
        Ex::ExNoInterface(_) => {}
        Ex::ExDynamicCast(ex) => {
            in_conns.push((inc(&mut input_index), build_node(ctx, ex.target_expression)));
            more_inputs.push(pin("input", PinType::Data));
        }
        Ex::ExStructConst(ex) => {
            for (i, p) in ex.value.iter().enumerate() {
                in_conns.push((inc(&mut input_index), build_node(ctx, *p)));
                more_inputs.push(pin(format!("member {i}"), PinType::Data));
            }
        }
        Ex::ExEndStructConst(_) => {}
        Ex::ExSetArray(ex) => {
            in_conns.push((
                inc(&mut input_index),
                build_node(ctx, ex.assigning_property.expect("TODO old versions")),
            ));
            more_inputs.push(pin("property", PinType::Data));
            for (i, p) in ex.elements.iter().enumerate() {
                in_conns.push((inc(&mut input_index), build_node(ctx, *p)));
                more_inputs.push(pin(format!("element {i}"), PinType::Data));
            }
        }
        Ex::ExEndArray(_) => {}
        Ex::ExPropertyConst(_) => {}
        Ex::ExUnicodeStringConst(_) => {}
        Ex::ExInt64Const(_) => {}
        Ex::ExUInt64Const(_) => {}
        Ex::ExDoubleConst(_) => {}
        Ex::ExCast(ex) => {
            in_conns.push((inc(&mut input_index), build_node(ctx, ex.target)));
            more_inputs.push(pin("target", PinType::Data));
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
            in_conns.push((inc(&mut input_index), build_node(ctx, ex.struct_expression)));
            more_inputs.push(pin("expr", PinType::Data));
        }
        //     Ex::ExLetMulticastDelegate(ex_let_multicast_delegate) => {
        //         bail!("todo map ExLetMulticastDelegate")
        //     }
        //     Ex::ExLetDelegate(ex_let_delegate) => bail!("todo map ExLetDelegate"),
        Ex::ExLocalVirtualFunction(ex) => {
            for (i, p) in ex.parameters.iter().enumerate() {
                in_conns.push((inc(&mut input_index), build_node(ctx, *p)));
                more_inputs.push(pin(format!("param {i}"), PinType::Data));
            }
        }
        Ex::ExLocalFinalFunction(ex) => {
            for (i, p) in ex.parameters.iter().enumerate() {
                in_conns.push((inc(&mut input_index), build_node(ctx, *p)));
                more_inputs.push(pin(format!("param {i}"), PinType::Data));
            }
        }
        //     Ex::ExLocalOutVariable(ex_local_out_variable) => bail!("todo map ExLocalOutVariable"),
        //     Ex::ExDeprecatedOp4A(ex_deprecated_op4_a) => bail!("todo map ExDeprecatedOp4A"),
        //     Ex::ExInstanceDelegate(ex_instance_delegate) => bail!("todo map ExInstanceDelegate"),
        Ex::ExPushExecutionFlow(ex) => {
            let push = ExprIndex(ex.pushing_address as usize);
            out_conns.push((1, build_node(ctx, push)));
            more_outputs.push(GenericPin {
                name: "push".into(),
                pin_type: PinType::Exec,
            });
        }
        Ex::ExPopExecutionFlow(_) => {}
        Ex::ExComputedJump(ex) => {
            in_conns.push((
                inc(&mut input_index),
                build_node(ctx, ex.code_offset_expression),
            ));
            more_inputs.push(pin("offset", PinType::Data));
        }
        Ex::ExPopExecutionFlowIfNot(ex) => {
            in_conns.push((
                inc(&mut input_index),
                build_node(ctx, ex.boolean_expression),
            ));
            more_inputs.push(pin("condition", PinType::Data));
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
            in_conns.push((
                inc(&mut input_index),
                build_node(ctx, ex.variable_expression),
            ));
            more_inputs.push(pin("variable", PinType::Data));
            in_conns.push((
                inc(&mut input_index),
                build_node(ctx, ex.assignment_expression),
            ));
            more_inputs.push(pin("expression", PinType::Data));
        }
        //     Ex::ExLetWeakObjPtr(ex_let_weak_obj_ptr) => bail!("todo map ExLetWeakObjPtr"),
        //     Ex::ExBindDelegate(ex_bind_delegate) => bail!("todo map ExBindDelegate"),
        //     Ex::ExRemoveMulticastDelegate(ex_remove_multicast_delegate) => {
        //         bail!("todo map ExRemoveMulticastDelegate")
        //     }
        Ex::ExCallMulticastDelegate(ex) => {
            in_conns.push((inc(&mut input_index), build_node(ctx, ex.delegate)));
            more_inputs.push(pin(format!("delegate"), PinType::Data));
            for (i, p) in ex.parameters.iter().enumerate() {
                in_conns.push((inc(&mut input_index), build_node(ctx, *p)));
                more_inputs.push(pin(format!("param {i}"), PinType::Data));
            }
        }
        //     Ex::ExLetValueOnPersistentFrame(ex_let_value_on_persistent_frame) => {
        //         bail!("todo map ExLetValueOnPersistentFrame")
        //     }
        //     Ex::ExArrayConst(ex_array_const) => bail!("todo map ExArrayConst"),
        //     Ex::ExEndArrayConst(ex_end_array_const) => bail!("todo map ExEndArrayConst"),
        //     Ex::ExSoftObjectConst(ex_soft_object_const) => bail!("todo map ExSoftObjectConst"),
        Ex::ExCallMath(ex) => {
            for (i, p) in ex.parameters.iter().enumerate() {
                in_conns.push((inc(&mut input_index), build_node(ctx, *p)));
                more_inputs.push(pin(format!("param {i}"), PinType::Data));
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
    ctx.snarl[id].inputs.extend(more_inputs);
    ctx.snarl[id].outputs.extend(more_outputs);

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

mod layout {
    type Position = eframe::egui::Pos2;

    use super::*;

    pub fn layout(snarl: &mut Snarl<GenericNode>) {
        let mut layout = GraphLayout::new();

        for (id, _pos, node) in snarl.nodes_pos_ids() {
            layout.add_node(Node::new(id));
        }
        for (out_pin, in_pin) in snarl.wires() {
            let connection_type = match snarl[out_pin.node].outputs[out_pin.output].pin_type {
                PinType::Exec => ConnectionType::Exec,
                PinType::Data => ConnectionType::Data,
                _ => unreachable!(),
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
                x: cell.col as f32 * 300.0,
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
