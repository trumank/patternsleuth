use std::collections::{HashMap, HashSet};

use crate::{
    kismet::ExprIndex,
    kismet_nodes::{GenericNode, GenericPin, KismetGraph, PinType},
    ue,
};
use anyhow::{bail, Result};
use eframe::egui;
use egui_snarl::{InPinId, NodeId, OutPinId, Snarl};

struct GraphNode {
    pos: egui::Pos2,
    node: GenericNode,
}

struct GraphConnection {}

struct Ctx {
    exs: crate::kismet::literal::ExprGraph,
    snarl: Snarl<GenericNode>,
    next_pos: egui::Pos2,

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
        next_pos: Default::default(),
        to_add,
        node_map: Default::default(),
    };

    build_node(ctx.next_pos, &mut ctx, ExprIndex(0));

    while let Some(i) = ctx.to_add.pop() {
        if ctx.node_map.contains_key(&i) {
            continue;
        }

        ctx.next_pos.y += 200.0;

        build_node(ctx.next_pos, &mut ctx, i);
    }

    layout::layout(&mut ctx.snarl);

    Ok(KismetGraph { snarl: ctx.snarl })
}

fn build_node(pos: egui::Pos2, ctx: &mut Ctx, index: ExprIndex) -> NodeId {
    ctx.next_pos.y = ctx.next_pos.y.max(pos.y);

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

    let mut input_index = 1;

    fn inc(i: &mut usize) -> usize {
        *i += 1;
        *i - 1
    }

    let id = ctx.snarl.insert_node(
        pos,
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
            out_conns.push((0, build_node(pos + egui::vec2(200., 0.), ctx, next)));
        }
    }
    use crate::kismet::literal::Expr as Ex;
    match &node.expr {
        Ex::ExJump(ex) => {
            let next = ExprIndex(ex.code_offset as usize);
            out_conns.push((0, build_node(pos + egui::vec2(200., 0.), ctx, next)));
        }
        Ex::ExJumpIfNot(ex) => {
            let if_not = ExprIndex(ex.code_offset as usize);
            out_conns.push((1, build_node(pos + egui::vec2(200., 200.), ctx, if_not)));
            more_outputs.push(GenericPin {
                name: "else".into(),
                pin_type: PinType::Exec,
            });
        }
        // Ex::ExSkipOffsetConst(ex) => {} TODO
        Ex::ExPushExecutionFlow(ex) => {
            let push = ExprIndex(ex.pushing_address as usize);
            out_conns.push((1, build_node(pos + egui::vec2(200., 200.), ctx, push)));
            more_outputs.push(GenericPin {
                name: "push".into(),
                pin_type: PinType::Exec,
            });
        }
        Ex::ExLet(ex) => {
            in_conns.push((
                inc(&mut input_index),
                build_node(pos + egui::vec2(-200., 100.), ctx, ex.variable),
            ));
            more_inputs.push(GenericPin {
                name: "variable".into(),
                pin_type: PinType::Data,
            });
            in_conns.push((
                inc(&mut input_index),
                build_node(pos + egui::vec2(-200., 200.), ctx, ex.expression),
            ));
            more_inputs.push(GenericPin {
                name: "expression".into(),
                pin_type: PinType::Data,
            });
        }
        _ => {}
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
            layout.add_node(Node::new(
                id,
                node.inputs.len(),
                node.outputs.len(),
                node.node_type.name(),
            ));
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

        let json = serde_json::to_string_pretty(&layout).unwrap();
        std::fs::write(
            "/home/truman/projects/ue/patternsleuth/examples/dll_hook/graph.json",
            json,
        )
        .unwrap();
        layout.compute_layout();

        for id in layout.nodes.keys().cloned() {
            snarl.get_node_info_mut(id).unwrap().pos = dbg!(layout.grid_to_position(id));
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
        pub input_count: usize,
        pub output_count: usize,
        pub name: String,
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
        pub fn new(
            id: NodeId,
            input_count: usize,
            output_count: usize,
            name: impl Into<String>,
        ) -> Self {
            Self {
                id,
                input_count,
                output_count,
                name: name.into(),
            }
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

            // Validate connection indices
            let from_node = &self.nodes[&connection.from_node];
            let to_node = &self.nodes[&connection.to_node];

            if connection.from_output >= from_node.output_count {
                panic!(
                    "Output index {} out of range for node {:?}",
                    connection.from_output, connection.from_node
                );
            }
            if connection.to_input >= to_node.input_count {
                panic!(
                    "Input index {} out of range for node {:?}",
                    connection.to_input, connection.to_node
                );
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
                        let node_name = &self.nodes[&node_id].name;
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
            // Pass 1: Exec connections with BFS
            let mut placed = HashSet::new();
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

            // BFS for exec connections
            let mut exec_outputs: HashMap<NodeId, Vec<(usize, NodeId)>> = HashMap::new();
            for conn in &self.connections {
                if conn.connection_type == ConnectionType::Exec {
                    exec_outputs
                        .entry(conn.from_node)
                        .or_default()
                        .push((conn.from_output, conn.to_node));
                }
            }
            for outputs in exec_outputs.values_mut() {
                outputs.sort();
            }

            // next completely empty row
            let mut next_row = 0;

            for root_id in roots {
                queue.push_back((root_id, cell(next_row, 0)));
                while let Some((current_id, current_cell)) = queue.pop_front() {
                    if self.grid_inv.contains_key(&current_id) {
                        // already placed
                        continue;
                    }
                    if self.grid.contains_key(&current_cell) {
                        // cell is occopied by another node, need to make space
                        self.print_grid("before shift");
                        self.shift_columns_right(current_cell.col);
                        // also shift and existing items in the queue
                        for (_id, c) in queue.iter_mut() {
                            if c.col >= current_cell.col {
                                c.col += 1;
                            }
                        }
                        self.print_grid("after shift");
                    }

                    let node = &self.nodes[&current_id].name;
                    self.print_grid(&format!("asdf {}", node));

                    self.place_node(current_id, current_cell);
                    placed.insert(current_id);
                    next_row = next_row.max(current_cell.row + 1);

                    if let Some(outputs) = exec_outputs.get(&current_id) {
                        for (i, &(_output_index, output_id)) in outputs.iter().enumerate() {
                            if !placed.contains(&output_id) {
                                queue.push_back((
                                    output_id,
                                    cell(current_cell.row + i as i32, current_cell.col + 1),
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

            // Pass 2: Data connections with DFS
            // let mut data_outputs: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
            // for conn in &self.connections {
            //     if conn.connection_type == ConnectionType::Data {
            //         data_outputs
            //             .entry(conn.from_node)
            //             .or_default()
            //             .push(conn.to_node);
            //     }
            // }

            // let mut data_placed = HashSet::new();
            // for &exec_node in &placed {
            //     if let Some(data_children) = data_outputs.get(&exec_node) {
            //         self.place_data_nodes_dfs(
            //             exec_node,
            //             data_children,
            //             &mut data_placed,
            //             &data_outputs,
            //         );
            //     }
            // }
        }

        // fn place_data_nodes_dfs(
        //     &mut self,
        //     parent_id: NodeId,
        //     children: &[NodeId],
        //     placed: &mut HashSet<NodeId>,
        //     outputs: &HashMap<NodeId, Vec<NodeId>>,
        // ) {
        //     let parent_cell = self.grid_inv.get(&parent_id).unwrap().clone();
        //     let mut col_offset = 1;

        //     for &child_id in children {
        //         if !placed.contains(&child_id) {
        //             // Make space by shifting columns right if needed
        //             let target_col = parent_cell.col + col_offset;
        //             let target_row = parent_cell.row + 1;

        //             self.shift_columns_right(target_col);

        //             self.place_node(child_id, cell(target_row, target_col));
        //             placed.insert(child_id);

        //             // Recursively place children
        //             if let Some(grandchildren) = outputs.get(&child_id) {
        //                 self.place_data_nodes_dfs(child_id, grandchildren, placed, outputs);
        //             }

        //             col_offset += 1;
        //         }
        //     }
        // }

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
    }
    #[cfg(test)]
    mod test {
        use super::*;

        fn n(id: NodeId, name: &str, inputs: usize, outputs: usize) -> Node {
            Node {
                id,
                input_count: inputs,
                output_count: outputs,
                name: name.to_string(),
            }
        }

        #[test]
        fn test_shift_nodes() {
            let mut layout = GraphLayout::new();

            let a = NodeId(1);
            let b = NodeId(2);
            let c = NodeId(3);

            // Create nodes in exec order: A -> B
            layout.nodes.insert(a, n(a, "A", 1, 1));
            layout.nodes.insert(b, n(b, "B", 1, 1));
            layout.nodes.insert(c, n(c, "C", 1, 1));

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
            let mut node = |name: &str, inputs: usize, outputs: usize| -> NodeId {
                counter += 1;
                let id = NodeId(counter);
                layout.nodes.insert(id, n(id, name.into(), inputs, outputs));
                id
            };

            // b -> a+-> c -> f
            //       +-> d -> e

            let a = node("A", 1, 2);
            let b = node("B", 1, 1);
            let c = node("C", 1, 2);
            let d = node("D", 1, 1);
            let e = node("E", 1, 1);
            let f = node("F", 1, 1);
            let g = node("G", 1, 1);
            let h = node("H", 1, 2);
            let i = node("I", 1, 1);
            let j = node("J", 1, 1);
            let k = node("K", 1, 1);

            use ConnectionType::*;
            layout.connections.push(conn(b, 0, a, 0, Exec));
            layout.connections.push(conn(a, 0, c, 0, Exec));
            layout.connections.push(conn(a, 1, d, 0, Exec));
            layout.connections.push(conn(d, 0, e, 0, Exec));
            layout.connections.push(conn(c, 0, f, 0, Exec));
            // layout.connections.push(conn(c, 0, g, 0, Exec));

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

// use crate::kismet::literal::Expr as Ex;
// match ex {
//     Ex::ExLocalVariable(ex_local_variable) => bail!("todo map ExLocalVariable"),
//     Ex::ExInstanceVariable(ex_instance_variable) => bail!("todo map ExInstanceVariable"),
//     Ex::ExDefaultVariable(ex_default_variable) => bail!("todo map ExDefaultVariable"),
//     Ex::ExReturn(ex_return) => {
//         name = "ExReturn";
//         // bail!("todo map ExReturn")
//     }
//     Ex::ExJump(ex_jump) => bail!("todo map ExJump"),
//     Ex::ExJumpIfNot(ex_jump_if_not) => bail!("todo map ExJumpIfNot"),
//     Ex::ExAssert(ex_assert) => bail!("todo map ExAssert"),
//     Ex::ExNothing(ex_nothing) => bail!("todo map ExNothing"),
//     Ex::ExNothingInt32(ex_nothing_int32) => bail!("todo map ExNothingInt32"),
//     Ex::ExLet(ex_let) => bail!("todo map ExLet"),
//     Ex::ExBitFieldConst(ex_bit_field_const) => bail!("todo map ExBitFieldConst"),
//     Ex::ExClassContext(ex_class_context) => bail!("todo map ExClassContext"),
//     Ex::ExMetaCast(ex_meta_cast) => bail!("todo map ExMetaCast"),
//     Ex::ExLetBool(ex_let_bool) => bail!("todo map ExLetBool"),
//     Ex::ExEndParmValue(ex_end_parm_value) => bail!("todo map ExEndParmValue"),
//     Ex::ExEndFunctionParms(ex_end_function_parms) => bail!("todo map ExEndFunctionParms"),
//     Ex::ExSelf(ex_self) => bail!("todo map ExSelf"),
//     Ex::ExSkip(ex_skip) => bail!("todo map ExSkip"),
//     Ex::ExContext(ex_context) => bail!("todo map ExContext"),
//     Ex::ExContextFailSilent(ex_context_fail_silent) => {
//         bail!("todo map ExContextFailSilent")
//     }
//     Ex::ExVirtualFunction(ex_virtual_function) => bail!("todo map ExVirtualFunction"),
//     Ex::ExFinalFunction(ex_final_function) => bail!("todo map ExFinalFunction"),
//     Ex::ExIntConst(ex_int_const) => bail!("todo map ExIntConst"),
//     Ex::ExFloatConst(ex_float_const) => bail!("todo map ExFloatConst"),
//     Ex::ExStringConst(ex_string_const) => bail!("todo map ExStringConst"),
//     Ex::ExObjectConst(ex_object_const) => bail!("todo map ExObjectConst"),
//     Ex::ExNameConst(ex_name_const) => bail!("todo map ExNameConst"),
//     Ex::ExRotationConst(ex_rotation_const) => bail!("todo map ExRotationConst"),
//     Ex::ExVectorConst(ex_vector_const) => bail!("todo map ExVectorConst"),
//     Ex::ExByteConst(ex_byte_const) => bail!("todo map ExByteConst"),
//     Ex::ExIntZero(ex_int_zero) => bail!("todo map ExIntZero"),
//     Ex::ExIntOne(ex_int_one) => bail!("todo map ExIntOne"),
//     Ex::ExTrue(ex_true) => bail!("todo map ExTrue"),
//     Ex::ExFalse(ex_false) => bail!("todo map ExFalse"),
//     Ex::ExTextConst(ex_text_const) => bail!("todo map ExTextConst"),
//     Ex::ExNoObject(ex_no_object) => bail!("todo map ExNoObject"),
//     Ex::ExTransformConst(ex_transform_const) => bail!("todo map ExTransformConst"),
//     Ex::ExIntConstByte(ex_int_const_byte) => bail!("todo map ExIntConstByte"),
//     Ex::ExNoInterface(ex_no_interface) => bail!("todo map ExNoInterface"),
//     Ex::ExDynamicCast(ex_dynamic_cast) => bail!("todo map ExDynamicCast"),
//     Ex::ExStructConst(ex_struct_const) => bail!("todo map ExStructConst"),
//     Ex::ExEndStructConst(ex_end_struct_const) => bail!("todo map ExEndStructConst"),
//     Ex::ExSetArray(ex_set_array) => bail!("todo map ExSetArray"),
//     Ex::ExEndArray(ex_end_array) => bail!("todo map ExEndArray"),
//     Ex::ExPropertyConst(ex_property_const) => bail!("todo map ExPropertyConst"),
//     Ex::ExUnicodeStringConst(ex_unicode_string_const) => {
//         bail!("todo map ExUnicodeStringConst")
//     }
//     Ex::ExInt64Const(ex_int64_const) => bail!("todo map ExInt64Const"),
//     Ex::ExUInt64Const(ex_uint64_const) => bail!("todo map ExUInt64Const"),
//     Ex::ExDoubleConst(ex_double_const) => bail!("todo map ExDoubleConst"),
//     Ex::ExCast(ex_cast) => bail!("todo map ExCast"),
//     Ex::ExSetSet(ex_set_set) => bail!("todo map ExSetSet"),
//     Ex::ExEndSet(ex_end_set) => bail!("todo map ExEndSet"),
//     Ex::ExSetMap(ex_set_map) => bail!("todo map ExSetMap"),
//     Ex::ExEndMap(ex_end_map) => bail!("todo map ExEndMap"),
//     Ex::ExSetConst(ex_set_const) => bail!("todo map ExSetConst"),
//     Ex::ExEndSetConst(ex_end_set_const) => bail!("todo map ExEndSetConst"),
//     Ex::ExMapConst(ex_map_const) => bail!("todo map ExMapConst"),
//     Ex::ExEndMapConst(ex_end_map_const) => bail!("todo map ExEndMapConst"),
//     Ex::ExVector3fConst(ex_vector3f_const) => bail!("todo map ExVector3fConst"),
//     Ex::ExStructMemberContext(ex_struct_member_context) => {
//         bail!("todo map ExStructMemberContext")
//     }
//     Ex::ExLetMulticastDelegate(ex_let_multicast_delegate) => {
//         bail!("todo map ExLetMulticastDelegate")
//     }
//     Ex::ExLetDelegate(ex_let_delegate) => bail!("todo map ExLetDelegate"),
//     Ex::ExLocalVirtualFunction(ex_local_virtual_function) => {
//         bail!("todo map ExLocalVirtualFunction")
//     }
//     Ex::ExLocalFinalFunction(ex_local_final_function) => {
//         bail!("todo map ExLocalFinalFunction")
//     }
//     Ex::ExLocalOutVariable(ex_local_out_variable) => bail!("todo map ExLocalOutVariable"),
//     Ex::ExDeprecatedOp4A(ex_deprecated_op4_a) => bail!("todo map ExDeprecatedOp4A"),
//     Ex::ExInstanceDelegate(ex_instance_delegate) => bail!("todo map ExInstanceDelegate"),
//     Ex::ExPushExecutionFlow(ex_push_execution_flow) => {
//         bail!("todo map ExPushExecutionFlow")
//     }
//     Ex::ExPopExecutionFlow(ex_pop_execution_flow) => bail!("todo map ExPopExecutionFlow"),
//     Ex::ExComputedJump(ex_computed_jump) => bail!("todo map ExComputedJump"),
//     Ex::ExPopExecutionFlowIfNot(ex_pop_execution_flow_if_not) => {
//         bail!("todo map ExPopExecutionFlowIfNot")
//     }
//     Ex::ExBreakpoint(ex_breakpoint) => bail!("todo map ExBreakpoint"),
//     Ex::ExInterfaceContext(ex_interface_context) => bail!("todo map ExInterfaceContext"),
//     Ex::ExObjToInterfaceCast(ex_obj_to_interface_cast) => {
//         bail!("todo map ExObjToInterfaceCast")
//     }
//     Ex::ExEndOfScript(ex_end_of_script) => {
//         name = "ExEndOfScript";
//     }
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
//     Ex::ExLetObj(ex_let_obj) => bail!("todo map ExLetObj"),
//     Ex::ExLetWeakObjPtr(ex_let_weak_obj_ptr) => bail!("todo map ExLetWeakObjPtr"),
//     Ex::ExBindDelegate(ex_bind_delegate) => bail!("todo map ExBindDelegate"),
//     Ex::ExRemoveMulticastDelegate(ex_remove_multicast_delegate) => {
//         bail!("todo map ExRemoveMulticastDelegate")
//     }
//     Ex::ExCallMulticastDelegate(ex_call_multicast_delegate) => {
//         bail!("todo map ExCallMulticastDelegate")
//     }
//     Ex::ExLetValueOnPersistentFrame(ex_let_value_on_persistent_frame) => {
//         bail!("todo map ExLetValueOnPersistentFrame")
//     }
//     Ex::ExArrayConst(ex_array_const) => bail!("todo map ExArrayConst"),
//     Ex::ExEndArrayConst(ex_end_array_const) => bail!("todo map ExEndArrayConst"),
//     Ex::ExSoftObjectConst(ex_soft_object_const) => bail!("todo map ExSoftObjectConst"),
//     Ex::ExCallMath(ex_call_math) => bail!("todo map ExCallMath"),
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
// };
