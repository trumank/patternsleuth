use crate::ue::{self, FName};
use anyhow::{anyhow, bail, Result};
use byteorder::{ReadBytesExt, WriteBytesExt as _, LE};
use std::collections::HashMap;
use std::io::{Cursor, Read};

macro_rules! build_walk {
    ($ex:ident, $member_name:ident : Box<Expr>) => {
        walk_expression(&$ex.$member_name);
    };
    ($ex:ident, $member_name:ident : Vec<Expr>) => {
        for $ex in $ex.$member_name.iter() {
            walk_expression(&$ex);
        }
    };
    ($ex:ident, $member_name:ident : $tp:ty) => {};
}

macro_rules! build_from {
    ($expr:ident, $member_name:ident : u32) => {
        let $member_name = $expr.$member_name;
    };
    ($expr:ident, $member_name:ident : i32) => {
        let $member_name = $expr.$member_name;
    };
    ($expr:ident, $member_name:ident : f32) => {
        let $member_name = $expr.$member_name;
    };
    ($expr:ident, $member_name:ident : Vector<f64>) => {
        let $member_name = (&$expr.$member_name).into();
    };
    ($expr:ident, $member_name:ident : Transform<f64>) => {
        let $member_name = (&$expr.$member_name).into();
    };
    ($expr:ident, $member_name:ident : EScriptInstrumentationType) => {
        let $member_name = $expr.$member_name.into();
    };
    ($expr:ident, $member_name:ident : ECastToken) => {
        let $member_name = $expr.$member_name.into();
    };
    ($expr:ident, $member_name:ident : KismetPropertyPointer) => {
        let $member_name = (&$expr.$member_name).into();
    };
    ($expr:ident, $member_name:ident : PackageIndex) => {
        let $member_name = $expr.$member_name.into();
    };
    ($expr:ident, $member_name:ident : Option<PackageIndex>) => {
        let $member_name = $expr.$member_name.map(|pi| pi.into());
    };
    ($expr:ident, $member_name:ident : Box<Expr>) => {
        let $member_name = Box::new($expr.$member_name.as_ref().into());
    };
    ($expr:ident, $member_name:ident : Option<Box<Expr>>) => {
        let $member_name = $expr.$member_name.as_ref().map(|e| Box::new(e.as_ref().into()));
    };
    ($expr:ident, $member_name:ident : Vec<Expr>) => {
        let $member_name = $expr.$member_name.iter().map(|o| o.into()).collect();
    };
    ($expr:ident, $member_name:ident : FName) => {
        let $member_name = FName($expr.$member_name.get_owned_content());
    };
    ($expr:ident, $member_name:ident : String) => {
        let $member_name = $expr.$member_name.clone();
    };
    ($expr:ident, $member_name:ident : Option<FName>) => {
        let $member_name = $expr.$member_name.as_ref().map(|n| FName(n.get_owned_content()));
    };
    ($expr:ident, $member_name:ident : Box<FScriptText>) => {
        let $member_name = Box::new(FScriptText); // TODO
    };
    ($expr:ident, $member_name:ident : Vec<KismetSwitchCase>) => {
        let $member_name = $expr.$member_name.iter().map(|o| KismetSwitchCase).collect(); // TODO
    };
    ($expr:ident, $member_name:ident : $($tp:tt)*) => {
        //compile_error!(stringify!($($tp)*));
        let $member_name = todo!(stringify!($($tp)*));
    };
    ($expr:ident, $member_name:ident : $tp:ty) => {
        compile_error!(stringify!($ty));
        //let $member_name = todo!();
    };
}

macro_rules! expression {
    ($name:ident, $( $member_name:ident: [ $($member_type:tt)* ] ),* ) => {
        #[derive(Debug, Clone)]
        pub struct $name {
            $( pub $member_name: $($member_type)*, )*
        }

        impl From<$name> for Expr {
            fn from(value: $name) -> Expr {
                Expr::$name(value)
            }
        }
    };
}

macro_rules! for_each {
    ( $( $op:literal: $name:ident { $( $member_name:ident : [ $($member_type:tt)* ] )* }, )* ) => {
        pub mod literal {
            use super::*;

            pub type ExprGraph = HashMap<ExprIndex, ExprNode>;
            #[derive(Debug, Clone)]
            pub struct ExprNode {
                pub expr: Expr,
                pub top_level: bool,
                pub next: Option<ExprIndex>,
            }

            #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, strum::FromRepr, strum::AsRefStr)]
            #[repr(u8)]
            pub enum ExprOp {
                $( $name = $op, )*
            }
            #[derive(Debug, Clone)]
            pub enum Expr {
                $( $name($name), )*
            }
            impl Expr {
                pub fn op(&self) -> ExprOp {
                    match self {
                        $( Self::$name { .. } => ExprOp::$name, )*
                    }
                }
            }
            $( expression!($name, $($member_name : [$($member_type)*]),* );)*
            fn walk_expression(ex: &Expr) {
                match ex {
                    $( Expr::$name(ex) => {
                        $(build_walk!(ex, $member_name : $($member_type)*);)*
                    }, )*
                }
            }
        }
        pub mod pattern {
            use super::*;

            #[derive(Debug, Clone)]
            pub enum Expr {
                Any,
                $( $name($name), )*
            }
            $( expression!($name, $($member_name : [Option<$($member_type)*>]),* );)*
                /*
            fn walk_expression(ex: &Expr) {
                match ex {
                    Expr::Any => {},
                    $( Expr::$name(ex) => {
                        $(build_walk!(ex, $member_name : $($member_type)*);)*
                    }, )*
                }
            }
            */
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExprIndex(pub usize);

//use unreal_asset::types::PackageIndex;
#[derive(Debug, Clone)]
pub struct KismetPropertyPointer(pub u64);
// {
// owner: PackageIndex,
// path: Vec<String>,
// }
#[derive(Debug, Clone, Copy)]
pub struct PackageIndex(pub u64);
#[derive(Debug, Clone)]
pub struct FScriptText;
#[derive(Debug, Clone)]
pub struct KismetSwitchCase {
    case_index_value_term: ExprIndex,
    code_skip_size_type: u32,
    case_term: ExprIndex,
}

#[derive(Debug, Clone)]
pub struct Vector<T: Clone> {
    x: T,
    y: T,
    z: T,
}

#[derive(Debug, Clone)]
pub struct Vector4<T: Clone> {
    x: T,
    y: T,
    z: T,
    w: T,
}

#[derive(Debug, Clone)]
pub struct Transform<T: Clone> {
    rotation: Vector4<T>,
    translation: Vector<T>,
    scale: Vector<T>,
}

#[derive(Debug, Clone, strum::FromRepr)]
#[repr(u8)]
pub enum ECastToken {
    ObjectToInterface = 0x00,
    ObjectToBool = 0x01,
    InterfaceToBool = 0x02,
    DoubleToFloat = 0x03,
    FloatToDouble = 0x04,
}

#[derive(Debug, Clone)]
#[repr(u8)]
pub enum EScriptInstrumentationType {
    Class,
    ClassScope,
    Instance,
    Event,
    InlineEvent,
    ResumeEvent,
    PureNodeEntry,
    NodeDebugSite,
    NodeEntry,
    NodeExit,
    PushState,
    RestoreState,
    ResetState,
    SuspendState,
    PopState,
    TunnelEndOfThread,
    Stop,
}

for_each!(
    0x00: ExLocalVariable { variable: [ KismetPropertyPointer ] },
    0x01: ExInstanceVariable { variable: [ KismetPropertyPointer ] },
    0x02: ExDefaultVariable { variable: [ KismetPropertyPointer ] },
    // 0x03
    0x04: ExReturn { return_expression: [ ExprIndex ] },
    // 0x05
    0x06: ExJump { code_offset: [ ExprIndex ] },
    0x07: ExJumpIfNot { code_offset: [ ExprIndex ] boolean_expression: [ ExprIndex ] },
    // 0x08
    0x09: ExAssert { line_number: [ u16 ] debug_mode: [ bool ] assert_expression: [ ExprIndex ] },
    // 0x0A
    0x0B: ExNothing {  },
    0x0C: ExNothingInt32 {  },
    // 0x0D
    // 0x0E
    0x0F: ExLet { value: [ KismetPropertyPointer ] variable: [ ExprIndex ] expression: [ ExprIndex ] },
    // 0x10
    0x11: ExBitFieldConst { /* TODO */ },
    0x12: ExClassContext { object_expression: [ ExprIndex ] offset: [ u32 ] r_value_pointer: [ KismetPropertyPointer ] context_expression: [ ExprIndex ] },
    0x13: ExMetaCast { class_ptr: [ PackageIndex ] target_expression: [ ExprIndex ] },
    0x14: ExLetBool { variable_expression: [ ExprIndex ] assignment_expression: [ ExprIndex ] },
    0x15: ExEndParmValue {  },
    0x16: ExEndFunctionParms {  },
    0x17: ExSelf {  },
    0x18: ExSkip { code_offset: [ u32 ] skip_expression: [ ExprIndex ] },
    0x19: ExContext { object_expression: [ ExprIndex ] offset: [ u32 ] r_value_pointer: [ KismetPropertyPointer ] context_expression: [ ExprIndex ] },
    0x1A: ExContextFailSilent { object_expression: [ ExprIndex ] offset: [ u32 ] r_value_pointer: [ KismetPropertyPointer ] context_expression: [ ExprIndex ] },
    0x1B: ExVirtualFunction { virtual_function_name: [ FName ] parameters: [ Vec<ExprIndex> ] },
    0x1C: ExFinalFunction { stack_node: [ PackageIndex ] parameters: [ Vec<ExprIndex> ] },
    0x1D: ExIntConst { value: [ i32 ] },
    0x1E: ExFloatConst { value: [ f32 ] },
    0x1F: ExStringConst { value: [ String ] },
    0x20: ExObjectConst { value: [ PackageIndex ] },
    0x21: ExNameConst { value: [ FName ] },
    0x22: ExRotationConst { rotator: [ Vector<f64> ] },
    0x23: ExVectorConst { value: [ Vector<f64> ] },
    0x24: ExByteConst { value: [ u8 ] },
    0x25: ExIntZero {  },
    0x26: ExIntOne {  },
    0x27: ExTrue {  },
    0x28: ExFalse {  },
    0x29: ExTextConst { value: [ ExprIndex ] },
    0x2A: ExNoObject {  },
    0x2B: ExTransformConst { value: [ Transform<f64> ] },
    0x2C: ExIntConstByte {  },
    0x2D: ExNoInterface {  },
    0x2E: ExDynamicCast { class_ptr: [ PackageIndex ] target_expression: [ ExprIndex ] },
    0x2F: ExStructConst { struct_value: [ PackageIndex ] struct_size: [ i32 ] value: [ Vec<ExprIndex> ] },
    0x30: ExEndStructConst {  },
    0x31: ExSetArray { assigning_property: [ Option<ExprIndex> ] array_inner_prop: [ Option<PackageIndex> ] elements: [ Vec<ExprIndex> ] },
    0x32: ExEndArray {  },
    0x33: ExPropertyConst { property: [ KismetPropertyPointer ] },
    0x34: ExUnicodeStringConst { value: [ String ] },
    0x35: ExInt64Const { value: [ i64 ] },
    0x36: ExUInt64Const {  },
    // 0x37: ExPrimitiveCast { conversion_type: [ ECastToken ] target: [ ExprIndex ] },
    0x37: ExDoubleConst { value: [ f64 ] },
    0x38: ExCast { conversion_type: [ ECastToken ] target: [ ExprIndex ] },
    0x39: ExSetSet { set_property: [ ExprIndex ] elements: [ Vec<ExprIndex> ] },
    0x3A: ExEndSet {  },
    0x3B: ExSetMap { map_property: [ ExprIndex ] elements: [ Vec<ExprIndex> ] },
    0x3C: ExEndMap {  },
    0x3D: ExSetConst { inner_property: [ KismetPropertyPointer ] elements: [ Vec<ExprIndex> ] },
    0x3E: ExEndSetConst {  },
    0x3F: ExMapConst { key_property: [ KismetPropertyPointer ] value_property: [ KismetPropertyPointer ] elements: [ Vec<ExprIndex> ] },
    0x40: ExEndMapConst {  },
    0x41: ExVector3fConst { /* TODO */ },
    0x42: ExStructMemberContext { struct_member_expression: [ KismetPropertyPointer ] struct_expression: [ ExprIndex ] },
    0x43: ExLetMulticastDelegate { variable_expression: [ ExprIndex ] assignment_expression: [ ExprIndex ] },
    0x44: ExLetDelegate { variable_expression: [ ExprIndex ] assignment_expression: [ ExprIndex ] },
    0x45: ExLocalVirtualFunction { virtual_function_name: [ FName ] parameters: [ Vec<ExprIndex> ] },
    0x46: ExLocalFinalFunction { stack_node: [ PackageIndex ] parameters: [ Vec<ExprIndex> ] },
    // 0x47
    0x48: ExLocalOutVariable { variable: [ KismetPropertyPointer ] },
    // 0x49
    0x4A: ExDeprecatedOp4A {  },
    0x4B: ExInstanceDelegate { function_name: [ FName ] },
    0x4C: ExPushExecutionFlow { pushing_address: [ u32 ] },
    0x4D: ExPopExecutionFlow {  },
    0x4E: ExComputedJump { code_offset_expression: [ ExprIndex ] },
    0x4F: ExPopExecutionFlowIfNot { boolean_expression: [ ExprIndex ] },
    0x50: ExBreakpoint {  },
    0x51: ExInterfaceContext { interface_value: [ ExprIndex ] },
    0x52: ExObjToInterfaceCast { class_ptr: [ PackageIndex ] target: [ ExprIndex ] },
    0x53: ExEndOfScript {  },
    0x54: ExCrossInterfaceCast { class_ptr: [ PackageIndex ] target: [ ExprIndex ] },
    0x55: ExInterfaceToObjCast { class_ptr: [ PackageIndex ] target: [ ExprIndex ] },
    // 0x56
    // 0x57
    // 0x58
    // 0x59
    0x5A: ExWireTracepoint {  },
    0x5B: ExSkipOffsetConst { skip: [ u32 ] },
    0x5C: ExAddMulticastDelegate { delegate: [ ExprIndex ] delegate_to_add: [ ExprIndex ] },
    0x5D: ExClearMulticastDelegate { delegate_to_clear: [ ExprIndex ] },
    0x5E: ExTracepoint {  },
    0x5F: ExLetObj { variable_expression: [ ExprIndex ] assignment_expression: [ ExprIndex ] },
    0x60: ExLetWeakObjPtr { variable_expression: [ ExprIndex ] assignment_expression: [ ExprIndex ] },
    0x61: ExBindDelegate { function_name: [ FName ] delegate: [ ExprIndex ] object_term: [ ExprIndex ] },
    0x62: ExRemoveMulticastDelegate { delegate: [ ExprIndex ] delegate_to_add: [ ExprIndex ] },
    0x63: ExCallMulticastDelegate { stack_node: [ PackageIndex ] parameters: [ Vec<ExprIndex> ] delegate: [ ExprIndex ] },
    0x64: ExLetValueOnPersistentFrame { destination_property: [ KismetPropertyPointer ] assignment_expression: [ ExprIndex ] },
    0x65: ExArrayConst { inner_property: [ KismetPropertyPointer ] elements: [ Vec<Expr> ] },
    0x66: ExEndArrayConst {  },
    0x67: ExSoftObjectConst { value: [ ExprIndex ] },
    0x68: ExCallMath { stack_node: [ PackageIndex ] parameters: [ Vec<ExprIndex> ] },
    0x69: ExSwitchValue { end_goto_offset: [ u32 ] index_term: [ ExprIndex ] default_term: [ ExprIndex ] cases: [ Vec<KismetSwitchCase> ] },
    0x6A: ExInstrumentationEvent { event_type: [ EScriptInstrumentationType ] event_name: [ Option<FName> ] },
    0x6B: ExArrayGetByRef { array_variable: [ ExprIndex ] array_index: [ ExprIndex ] },
    0x6C: ExClassSparseDataVariable { variable: [ KismetPropertyPointer ] },
    0x6D: ExFieldPathConst { value: [ ExprIndex ] },
    // 0x6E
    // 0x6F
    0x70: ExAutoRtfmTransact { /* TODO */ },
    0x71: ExAutoRtfmStopTransact { /* TODO */ },
    0x72: ExAutoRtfmAbortIfNot { /* TODO */ },
);

pub fn read_until(
    s: &mut Cursor<&[u8]>,
    graph: &mut literal::ExprGraph,
    until: literal::ExprOp,
) -> Result<Vec<ExprIndex>> {
    let mut exs = vec![];
    loop {
        let next = read(s, graph)?;
        if graph[&next].expr.op() == until {
            break;
        } else {
            exs.push(next);
        }
    }
    Ok(exs)
}

pub fn read_all(s: &mut Cursor<&[u8]>) -> Result<literal::ExprGraph> {
    let mut graph = literal::ExprGraph::default();
    let mut last = None;
    loop {
        let index = ExprIndex(s.position() as usize);
        let op = match s.read_u8() {
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            r => r,
        }?;
        let ex = read_body(s, &mut graph, try_from_opcode(op)?)?;
        graph.insert(
            index,
            literal::ExprNode {
                expr: ex,
                top_level: true,
                next: None,
            },
        );

        if let Some(last) = last {
            let last_node = graph.get_mut(&last).unwrap();
            use literal::Expr as Ex;
            let has_next = match &last_node.expr {
                // Ex::ExLocalVariable(ex_local_variable) => todo!(),
                // Ex::ExInstanceVariable(ex_instance_variable) => todo!(),
                // Ex::ExDefaultVariable(ex_default_variable) => todo!(),
                Ex::ExReturn(_) => false,
                Ex::ExJump(_) => false,
                // Ex::ExJumpIfNot(_) => false,
                // Ex::ExAssert(ex_assert) => todo!(),
                // Ex::ExNothing(ex_nothing) => todo!(),
                // Ex::ExNothingInt32(ex_nothing_int32) => todo!(),
                // Ex::ExLet(ex_let) => todo!(),
                // Ex::ExBitFieldConst(ex_bit_field_const) => todo!(),
                // Ex::ExClassContext(ex_class_context) => todo!(),
                // Ex::ExMetaCast(ex_meta_cast) => todo!(),
                // Ex::ExLetBool(ex_let_bool) => todo!(),
                // Ex::ExEndParmValue(ex_end_parm_value) => todo!(),
                // Ex::ExEndFunctionParms(ex_end_function_parms) => todo!(),
                // Ex::ExSelf(ex_self) => todo!(),
                // Ex::ExSkip(ex_skip) => todo!(),
                // Ex::ExContext(ex_context) => todo!(),
                // Ex::ExContextFailSilent(ex_context_fail_silent) => todo!(),
                // Ex::ExVirtualFunction(ex_virtual_function) => todo!(),
                // Ex::ExFinalFunction(ex_final_function) => todo!(),
                // Ex::ExIntConst(ex_int_const) => todo!(),
                // Ex::ExFloatConst(ex_float_const) => todo!(),
                // Ex::ExStringConst(ex_string_const) => todo!(),
                // Ex::ExObjectConst(ex_object_const) => todo!(),
                // Ex::ExNameConst(ex_name_const) => todo!(),
                // Ex::ExRotationConst(ex_rotation_const) => todo!(),
                // Ex::ExVectorConst(ex_vector_const) => todo!(),
                // Ex::ExByteConst(ex_byte_const) => todo!(),
                // Ex::ExIntZero(ex_int_zero) => todo!(),
                // Ex::ExIntOne(ex_int_one) => todo!(),
                // Ex::ExTrue(ex_true) => todo!(),
                // Ex::ExFalse(ex_false) => todo!(),
                // Ex::ExTextConst(ex_text_const) => todo!(),
                // Ex::ExNoObject(ex_no_object) => todo!(),
                // Ex::ExTransformConst(ex_transform_const) => todo!(),
                // Ex::ExIntConstByte(ex_int_const_byte) => todo!(),
                // Ex::ExNoInterface(ex_no_interface) => todo!(),
                // Ex::ExDynamicCast(ex_dynamic_cast) => todo!(),
                // Ex::ExStructConst(ex_struct_const) => todo!(),
                // Ex::ExEndStructConst(ex_end_struct_const) => todo!(),
                // Ex::ExSetArray(ex_set_array) => todo!(),
                // Ex::ExEndArray(ex_end_array) => todo!(),
                // Ex::ExPropertyConst(ex_property_const) => todo!(),
                // Ex::ExUnicodeStringConst(ex_unicode_string_const) => todo!(),
                // Ex::ExInt64Const(ex_int64_const) => todo!(),
                // Ex::ExUInt64Const(ex_uint64_const) => todo!(),
                // Ex::ExDoubleConst(ex_double_const) => todo!(),
                // Ex::ExCast(ex_cast) => todo!(),
                // Ex::ExSetSet(ex_set_set) => todo!(),
                // Ex::ExEndSet(ex_end_set) => todo!(),
                // Ex::ExSetMap(ex_set_map) => todo!(),
                // Ex::ExEndMap(ex_end_map) => todo!(),
                // Ex::ExSetConst(ex_set_const) => todo!(),
                // Ex::ExEndSetConst(ex_end_set_const) => todo!(),
                // Ex::ExMapConst(ex_map_const) => todo!(),
                // Ex::ExEndMapConst(ex_end_map_const) => todo!(),
                // Ex::ExVector3fConst(ex_vector3f_const) => todo!(),
                // Ex::ExStructMemberContext(ex_struct_member_context) => todo!(),
                // Ex::ExLetMulticastDelegate(ex_let_multicast_delegate) => todo!(),
                // Ex::ExLetDelegate(ex_let_delegate) => todo!(),
                // Ex::ExLocalVirtualFunction(ex_local_virtual_function) => todo!(),
                // Ex::ExLocalFinalFunction(ex_local_final_function) => todo!(),
                // Ex::ExLocalOutVariable(ex_local_out_variable) => todo!(),
                // Ex::ExDeprecatedOp4A(ex_deprecated_op4_a) => todo!(),
                // Ex::ExInstanceDelegate(ex_instance_delegate) => todo!(),
                // Ex::ExPushExecutionFlow(ex_push_execution_flow) => todo!(),
                Ex::ExPopExecutionFlow(_) => false,
                Ex::ExComputedJump(_) => false,
                // Ex::ExPopExecutionFlowIfNot(ex_pop_execution_flow_if_not) => todo!(),
                // Ex::ExBreakpoint(ex_breakpoint) => todo!(),
                // Ex::ExInterfaceContext(ex_interface_context) => todo!(),
                // Ex::ExObjToInterfaceCast(ex_obj_to_interface_cast) => todo!(),
                Ex::ExEndOfScript(_) => false,
                // Ex::ExCrossInterfaceCast(ex_cross_interface_cast) => todo!(),
                // Ex::ExInterfaceToObjCast(ex_interface_to_obj_cast) => todo!(),
                // Ex::ExWireTracepoint(ex_wire_tracepoint) => todo!(),
                // Ex::ExSkipOffsetConst(ex_skip_offset_const) => todo!(),
                // Ex::ExAddMulticastDelegate(ex_add_multicast_delegate) => todo!(),
                // Ex::ExClearMulticastDelegate(ex_clear_multicast_delegate) => todo!(),
                // Ex::ExTracepoint(ex_tracepoint) => todo!(),
                // Ex::ExLetObj(ex_let_obj) => todo!(),
                // Ex::ExLetWeakObjPtr(ex_let_weak_obj_ptr) => todo!(),
                // Ex::ExBindDelegate(ex_bind_delegate) => todo!(),
                // Ex::ExRemoveMulticastDelegate(ex_remove_multicast_delegate) => todo!(),
                // Ex::ExCallMulticastDelegate(ex_call_multicast_delegate) => todo!(),
                // Ex::ExLetValueOnPersistentFrame(ex_let_value_on_persistent_frame) => todo!(),
                // Ex::ExArrayConst(ex_array_const) => todo!(),
                // Ex::ExEndArrayConst(ex_end_array_const) => todo!(),
                // Ex::ExSoftObjectConst(ex_soft_object_const) => todo!(),
                // Ex::ExCallMath(ex_call_math) => todo!(),
                // Ex::ExSwitchValue(ex_switch_value) => todo!(),
                // Ex::ExInstrumentationEvent(ex_instrumentation_event) => todo!(),
                // Ex::ExArrayGetByRef(ex_array_get_by_ref) => todo!(),
                // Ex::ExClassSparseDataVariable(ex_class_sparse_data_variable) => todo!(),
                // Ex::ExFieldPathConst(ex_field_path_const) => todo!(),
                // Ex::ExAutoRtfmTransact(ex_auto_rtfm_transact) => todo!(),
                // Ex::ExAutoRtfmStopTransact(ex_auto_rtfm_stop_transact) => todo!(),
                // Ex::ExAutoRtfmAbortIfNot(ex_auto_rtfm_abort_if_not) => todo!(),
                _ => true,
            };
            if has_next {
                last_node.next = Some(index);
            }
        }

        last = Some(index);
    }
    Ok(graph)
}

pub fn normalize_and_serialize(exs: &mut Vec<literal::Expr>) -> Result<Vec<u8>> {
    use literal::{Expr as Ex, ExprOp as Op, *};
    impl Ctx<'_> {
        fn get_next(&mut self) -> Option<literal::Expr> {
            let next = self.exs.get(self.index).cloned();
            self.index += 1;
            next
        }
        fn advance(&mut self, expr_index: ExprIndex) -> literal::Expr {
            assert_eq!(expr_index.0, self.index);
            let next = self.exs[self.index].clone();
            self.index += 1;
            next
        }

        fn write_ex(&mut self, expr: Ex) -> Result<()> {
            self.ex_map.insert(ExprIndex(self.index), self.s.position());
            self.s.write_u8(expr.op() as u8)?;
            match expr {
                Ex::ExLocalVariable(ex) => {
                    self.s.write_u64::<LE>(ex.variable.0)?;
                }
                Ex::ExInstanceVariable(ex) => {
                    self.s.write_u64::<LE>(ex.variable.0)?;
                }
                Ex::ExDefaultVariable(ex) => {
                    self.s.write_u64::<LE>(ex.variable.0)?;
                }
                Ex::ExReturn(ex) => {
                    let e = self.advance(ex.return_expression);
                    self.write_ex(e)?;
                }
                Ex::ExJump(ex) => {
                    self.fixups.push((self.s.position(), ex.code_offset));
                    self.s.write_u32::<LE>(0)?;
                }
                Ex::ExJumpIfNot(ex) => {
                    self.fixups.push((self.s.position(), ex.code_offset));
                    self.s.write_u32::<LE>(0)?;
                    let e = self.advance(ex.boolean_expression);
                    self.write_ex(e)?;
                }
                Ex::ExAssert(ex) => bail!("todo write ExAssert"),
                Ex::ExNothing(_) => {}
                Ex::ExNothingInt32(_) => {}
                Ex::ExLet(ex) => {
                    self.s.write_u64::<LE>(ex.value.0)?;
                    let e = self.advance(ex.variable);
                    self.write_ex(e)?;
                    let e = self.advance(ex.expression);
                    self.write_ex(e)?;
                }
                Ex::ExBitFieldConst(ex) => bail!("todo write ExBitFieldConst"),
                Ex::ExClassContext(ex) => bail!("todo write ExClassContext"),
                Ex::ExMetaCast(ex) => bail!("todo write ExMetaCast"),
                Ex::ExLetBool(ex) => {
                    let e = self.advance(ex.variable_expression);
                    self.write_ex(e)?;
                    let e = self.advance(ex.assignment_expression);
                    self.write_ex(e)?;
                }
                Ex::ExEndParmValue(_) => {}
                Ex::ExEndFunctionParms(_) => {}
                Ex::ExSelf(_) => {}
                Ex::ExSkip(ex) => bail!("todo write ExSkip"),
                Ex::ExContext(ex) => {
                    let e = self.advance(ex.object_expression);
                    self.write_ex(e)?;
                    self.s.write_u32::<LE>(ex.offset)?;
                    self.s.write_u64::<LE>(ex.r_value_pointer.0)?;
                    let e = self.advance(ex.context_expression);
                    self.write_ex(e)?;
                }
                Ex::ExContextFailSilent(ex) => bail!("todo write ExContextFailSilent"),
                Ex::ExVirtualFunction(ex) => bail!("todo write ExVirtualFunction"),
                Ex::ExFinalFunction(ex) => {
                    self.s.write_u64::<LE>(ex.stack_node.0)?;
                    for parm in ex.parameters {
                        let e = self.advance(parm);
                        self.write_ex(e)?;
                    }
                    self.write_ex(ExEndFunctionParms {}.into())?;
                }
                Ex::ExIntConst(ex) => {
                    self.s.write_i32::<LE>(ex.value)?;
                }
                Ex::ExFloatConst(ex) => bail!("todo write ExFloatConst"),
                Ex::ExStringConst(ex) => bail!("todo write ExStringConst"),
                Ex::ExObjectConst(ex) => {
                    self.s.write_u64::<LE>(ex.value.0)?;
                }
                Ex::ExNameConst(ex) => bail!("todo write ExNameConst"),
                Ex::ExRotationConst(ex) => bail!("todo write ExRotationConst"),
                Ex::ExVectorConst(ex) => bail!("todo write ExVectorConst"),
                Ex::ExByteConst(ex) => bail!("todo write ExByteConst"),
                Ex::ExIntZero(_) => {}
                Ex::ExIntOne(_) => {}
                Ex::ExTrue(_) => {}
                Ex::ExFalse(_) => {}
                Ex::ExTextConst(ex) => bail!("todo write ExTextConst"),
                Ex::ExNoObject(_) => {}
                Ex::ExTransformConst(ex) => bail!("todo write ExTransformConst"),
                Ex::ExIntConstByte(ex) => bail!("todo write ExIntConstByte"),
                Ex::ExNoInterface(_) => {}
                Ex::ExDynamicCast(ex) => bail!("todo write ExDynamicCast"),
                Ex::ExStructConst(ex) => bail!("todo write ExStructConst"),
                Ex::ExEndStructConst(_) => {}
                Ex::ExSetArray(ex) => bail!("todo write ExSetArray"),
                Ex::ExEndArray(_) => {}
                Ex::ExPropertyConst(ex) => bail!("todo write ExPropertyConst"),
                Ex::ExUnicodeStringConst(ex) => bail!("todo write ExUnicodeStringConst"),
                Ex::ExInt64Const(ex) => bail!("todo write ExInt64Const"),
                Ex::ExUInt64Const(ex) => bail!("todo write ExUInt64Const"),
                Ex::ExDoubleConst(ex) => bail!("todo write ExDoubleConst"),
                Ex::ExCast(ex) => bail!("todo write ExCast"),
                Ex::ExSetSet(ex) => bail!("todo write ExSetSet"),
                Ex::ExEndSet(_) => {}
                Ex::ExSetMap(ex) => bail!("todo write ExSetMap"),
                Ex::ExEndMap(_) => {}
                Ex::ExSetConst(ex) => bail!("todo write ExSetConst"),
                Ex::ExEndSetConst(ex) => {}
                Ex::ExMapConst(ex) => bail!("todo write ExMapConst"),
                Ex::ExEndMapConst(ex) => {}
                Ex::ExVector3fConst(ex) => bail!("todo write ExVector3fConst"),
                Ex::ExStructMemberContext(ex) => bail!("todo write ExStructMemberContext"),
                Ex::ExLetMulticastDelegate(ex) => bail!("todo write ExLetMulticastDelegate"),
                Ex::ExLetDelegate(ex) => bail!("todo write ExLetDelegate"),
                Ex::ExLocalVirtualFunction(ex) => bail!("todo write ExLocalVirtualFunction"),
                Ex::ExLocalFinalFunction(ex) => {
                    self.s.write_u64::<LE>(ex.stack_node.0)?;
                    for parm in ex.parameters {
                        let e = self.advance(parm);
                        self.write_ex(e)?;
                    }
                    self.write_ex(ExEndFunctionParms {}.into())?;
                }
                Ex::ExLocalOutVariable(ex) => {
                    self.s.write_u64::<LE>(ex.variable.0)?;
                }
                Ex::ExDeprecatedOp4A(ex) => bail!("todo write ExDeprecatedOp4A"),
                Ex::ExInstanceDelegate(ex) => bail!("todo write ExInstanceDelegate"),
                Ex::ExPushExecutionFlow(ex) => bail!("todo write ExPushExecutionFlow"),
                Ex::ExPopExecutionFlow(ex) => bail!("todo write ExPopExecutionFlow"),
                Ex::ExComputedJump(ex) => bail!("todo write ExComputedJump"),
                Ex::ExPopExecutionFlowIfNot(ex) => bail!("todo write ExPopExecutionFlowIfNot"),
                Ex::ExBreakpoint(ex) => bail!("todo write ExBreakpoint"),
                Ex::ExInterfaceContext(ex) => bail!("todo write ExInterfaceContext"),
                Ex::ExObjToInterfaceCast(ex) => bail!("todo write ExObjToInterfaceCast"),
                Ex::ExEndOfScript(_) => {}
                Ex::ExCrossInterfaceCast(ex) => bail!("todo write ExCrossInterfaceCast"),
                Ex::ExInterfaceToObjCast(ex) => bail!("todo write ExInterfaceToObjCast"),
                Ex::ExWireTracepoint(ex) => bail!("todo write ExWireTracepoint"),
                Ex::ExSkipOffsetConst(ex) => bail!("todo write ExSkipOffsetConst"),
                Ex::ExAddMulticastDelegate(ex) => bail!("todo write ExAddMulticastDelegate"),
                Ex::ExClearMulticastDelegate(ex) => bail!("todo write ExClearMulticastDelegate"),
                Ex::ExTracepoint(ex) => bail!("todo write ExTracepoint"),
                Ex::ExLetObj(ex) => bail!("todo write ExLetObj"),
                Ex::ExLetWeakObjPtr(ex) => bail!("todo write ExLetWeakObjPtr"),
                Ex::ExBindDelegate(ex) => bail!("todo write ExBindDelegate"),
                Ex::ExRemoveMulticastDelegate(ex) => bail!("todo write ExRemoveMulticastDelegate"),
                Ex::ExCallMulticastDelegate(ex) => bail!("todo write ExCallMulticastDelegate"),
                Ex::ExLetValueOnPersistentFrame(ex) => {
                    bail!("todo write ExLetValueOnPersistentFrame")
                }
                Ex::ExArrayConst(ex) => bail!("todo write ExArrayConst"),
                Ex::ExEndArrayConst(_) => {}
                Ex::ExSoftObjectConst(ex) => bail!("todo write ExSoftObjectConst"),
                Ex::ExCallMath(ex) => bail!("todo write ExCallMath"),
                Ex::ExSwitchValue(ex) => bail!("todo write ExSwitchValue"),
                Ex::ExInstrumentationEvent(ex) => bail!("todo write ExInstrumentationEvent"),
                Ex::ExArrayGetByRef(ex) => bail!("todo write ExArrayGetByRef"),
                Ex::ExClassSparseDataVariable(ex) => bail!("todo write ExClassSparseDataVariable"),
                Ex::ExFieldPathConst(ex) => bail!("todo write ExFieldPathConst"),
                Ex::ExAutoRtfmTransact(ex) => bail!("todo write ExAutoRtfmTransact"),
                Ex::ExAutoRtfmStopTransact(ex) => bail!("todo write ExAutoRtfmStopTransact"),
                Ex::ExAutoRtfmAbortIfNot(ex) => bail!("todo write ExAutoRtfmAbortIfNot"),
            }

            Ok(())
        }
    }

    struct Ctx<'a> {
        fixups: Vec<(u64, ExprIndex)>,
        exs: &'a Vec<literal::Expr>,
        index: usize,
        s: Cursor<Vec<u8>>,
        ex_map: HashMap<ExprIndex, u64>,
    }
    let mut c = Ctx {
        fixups: vec![],
        exs,
        index: 0,
        s: Cursor::new(vec![]),
        ex_map: Default::default(),
    };

    while let Some(next) = c.get_next() {
        c.write_ex(next)?;
    }
    c.write_ex(ExEndOfScript {}.into())?;

    Ok(c.s.into_inner())
}

fn try_from_opcode(op: u8) -> Result<literal::ExprOp> {
    literal::ExprOp::from_repr(op).ok_or_else(|| anyhow!("invalid opcode {op}"))
}
fn read_fname(s: &mut Cursor<&[u8]>) -> Result<ue::FName> {
    let comparison_index = s.read_u32::<LE>()?;
    let _display_index = s.read_u32::<LE>()?;
    let number = s.read_u32::<LE>()?;
    Ok(ue::FName {
        comparison_index: ue::FNameEntryId {
            value: comparison_index,
        },
        number,
    })
}
fn read_string<S: Read>(s: &mut S) -> Result<String> {
    let mut bytes = vec![];
    loop {
        match s.read_u8()? {
            0 => break,
            b => bytes.push(b),
        }
    }
    Ok(String::from_utf8(bytes)?)
}
fn read_vector(s: &mut Cursor<&[u8]>) -> Result<Vector<f64>> {
    Ok(Vector {
        x: s.read_f64::<LE>()?,
        y: s.read_f64::<LE>()?,
        z: s.read_f64::<LE>()?,
    })
}
fn read_vector4(s: &mut Cursor<&[u8]>) -> Result<Vector4<f64>> {
    Ok(Vector4 {
        x: s.read_f64::<LE>()?,
        y: s.read_f64::<LE>()?,
        z: s.read_f64::<LE>()?,
        w: s.read_f64::<LE>()?,
    })
}
fn read_transform(s: &mut Cursor<&[u8]>) -> Result<Transform<f64>> {
    Ok(Transform {
        rotation: read_vector4(s)?,
        translation: read_vector(s)?,
        scale: read_vector(s)?,
    })
}

pub fn read(s: &mut Cursor<&[u8]>, graph: &mut literal::ExprGraph) -> Result<ExprIndex> {
    let index = ExprIndex(s.position() as usize);
    let op = s.read_u8()?;

    let op = try_from_opcode(op)?;

    // let span = tracing::error_span!("erm", op = format!("{op:?}")).entered();
    let ex = read_body(s, graph, op)?;
    // tracing::error!("ex {ex:#?}");
    // drop(span);
    graph.insert(
        index,
        literal::ExprNode {
            expr: ex,
            top_level: false,
            next: None,
        },
    );
    Ok(index)
}

pub fn read_body(
    s: &mut Cursor<&[u8]>,
    graph: &mut literal::ExprGraph,
    op: literal::ExprOp,
) -> Result<literal::Expr> {
    use literal::{ExprOp as Op, *};

    let ex = match op {
        Op::ExLocalVariable => ExLocalVariable {
            variable: KismetPropertyPointer(s.read_u64::<LE>()?),
        }
        .into(),
        Op::ExInstanceVariable => ExInstanceVariable {
            variable: KismetPropertyPointer(s.read_u64::<LE>()?),
        }
        .into(),
        Op::ExDefaultVariable => ExDefaultVariable {
            variable: KismetPropertyPointer(s.read_u64::<LE>()?),
        }
        .into(),
        Op::ExReturn => ExReturn {
            return_expression: read(s, graph)?,
        }
        .into(),
        Op::ExJump => ExJump {
            code_offset: ExprIndex(s.read_u32::<LE>()? as usize),
        }
        .into(),
        Op::ExJumpIfNot => ExJumpIfNot {
            code_offset: ExprIndex(s.read_u32::<LE>()? as usize),
            boolean_expression: read(s, graph)?,
        }
        .into(),
        Op::ExAssert => bail!("todo ExAssert"),
        Op::ExNothing => ExNothing {}.into(),
        Op::ExNothingInt32 => bail!("todo ExNothingInt32"),
        Op::ExLet => ExLet {
            value: KismetPropertyPointer(s.read_u64::<LE>()?),
            variable: read(s, graph)?,
            expression: read(s, graph)?,
        }
        .into(),
        Op::ExBitFieldConst => bail!("todo ExBitFieldConst"),
        Op::ExClassContext => bail!("todo ExClassContext"),
        Op::ExMetaCast => bail!("todo ExMetaCast"),
        Op::ExLetBool => ExLetBool {
            variable_expression: read(s, graph)?,
            assignment_expression: read(s, graph)?,
        }
        .into(),
        Op::ExEndParmValue => ExEndParmValue {}.into(),
        Op::ExEndFunctionParms => ExEndFunctionParms {}.into(),
        Op::ExSelf => ExSelf {}.into(),
        Op::ExSkip => ExSkip {
            code_offset: s.read_u32::<LE>()?,
            skip_expression: read(s, graph)?,
        }
        .into(),
        Op::ExContext => ExContext {
            object_expression: read(s, graph)?,
            offset: s.read_u32::<LE>()?,
            r_value_pointer: KismetPropertyPointer(s.read_u64::<LE>()?),
            context_expression: read(s, graph)?,
        }
        .into(),
        Op::ExContextFailSilent => ExContextFailSilent {
            object_expression: read(s, graph)?,
            offset: s.read_u32::<LE>()?,
            r_value_pointer: KismetPropertyPointer(s.read_u64::<LE>()?),
            context_expression: read(s, graph)?,
        }
        .into(),
        Op::ExVirtualFunction => ExVirtualFunction {
            virtual_function_name: read_fname(s)?,
            parameters: read_until(s, graph, ExprOp::ExEndFunctionParms)?,
        }
        .into(),
        Op::ExFinalFunction => ExFinalFunction {
            stack_node: PackageIndex(s.read_u64::<LE>()?),
            parameters: read_until(s, graph, ExprOp::ExEndFunctionParms)?,
        }
        .into(),
        Op::ExIntConst => ExIntConst {
            value: s.read_i32::<LE>()?,
        }
        .into(),
        Op::ExFloatConst => ExFloatConst {
            value: s.read_f32::<LE>()?,
        }
        .into(),
        Op::ExStringConst => ExStringConst {
            value: read_string(s)?,
        }
        .into(),
        Op::ExObjectConst => ExObjectConst {
            value: PackageIndex(s.read_u64::<LE>()?),
        }
        .into(),
        Op::ExNameConst => ExNameConst {
            value: read_fname(s)?,
        }
        .into(),
        Op::ExRotationConst => ExRotationConst {
            rotator: read_vector(s)?,
        }
        .into(),
        Op::ExVectorConst => ExVectorConst {
            value: read_vector(s)?,
        }
        .into(),
        Op::ExByteConst => ExByteConst {
            value: s.read_u8()?,
        }
        .into(),
        Op::ExIntZero => ExIntZero {}.into(),
        Op::ExIntOne => ExIntOne {}.into(),
        Op::ExTrue => ExTrue {}.into(),
        Op::ExFalse => ExFalse {}.into(),
        Op::ExTextConst => bail!("todo ExTextConst"),
        Op::ExNoObject => ExNoObject {}.into(),
        Op::ExTransformConst => ExTransformConst {
            value: read_transform(s)?,
        }
        .into(),
        Op::ExIntConstByte => bail!("todo ExIntConstByte"),
        Op::ExNoInterface => bail!("todo ExNoInterface"),
        Op::ExDynamicCast => ExDynamicCast {
            class_ptr: PackageIndex(s.read_u64::<LE>()?),
            target_expression: read(s, graph)?,
        }
        .into(),
        Op::ExStructConst => ExStructConst {
            struct_value: PackageIndex(s.read_u64::<LE>()?),
            struct_size: s.read_i32::<LE>()?,
            value: read_until(s, graph, ExprOp::ExEndStructConst)?,
        }
        .into(),
        Op::ExEndStructConst => ExEndStructConst {}.into(),
        Op::ExSetArray => ExSetArray {
            assigning_property: Some(read(s, graph)?),
            array_inner_prop: None, // TODO UE4 change KismetPropertyPointer(s.read_u64::<LE>()?),
            elements: read_until(s, graph, ExprOp::ExEndArray)?,
        }
        .into(),
        Op::ExEndArray => ExEndArray {}.into(),
        Op::ExPropertyConst => bail!("todo ExPropertyConst"),
        Op::ExUnicodeStringConst => bail!("todo ExUnicodeStringConst"),
        Op::ExInt64Const => bail!("todo ExInt64Const"),
        Op::ExUInt64Const => bail!("todo ExUInt64Const"),
        // Op::ExPrimitiveCast => ExPrimitiveCast {
        //     conversion_type: ECastToken::from_repr(s.read_u8()?)
        //         .ok_or_else(|| anyhow!("invalid ECastToken"))?,
        //     target: read(s, graph)?,
        // }
        // .into(),
        Op::ExDoubleConst => ExDoubleConst {
            value: s.read_f64::<LE>()?,
        }
        .into(),
        Op::ExCast => ExCast {
            conversion_type: ECastToken::from_repr(s.read_u8()?)
                .ok_or_else(|| anyhow!("invalid ECastToken"))?,
            target: read(s, graph)?,
        }
        .into(),
        Op::ExSetSet => bail!("todo ExSetSet"),
        Op::ExEndSet => ExEndSet {}.into(),
        Op::ExSetMap => bail!("todo ExSetMap"),
        Op::ExEndMap => ExEndMap {}.into(),
        Op::ExSetConst => bail!("todo ExSetConst"),
        Op::ExEndSetConst => ExEndSetConst {}.into(),
        Op::ExMapConst => bail!("todo ExMapConst"),
        Op::ExEndMapConst => ExEndMapConst {}.into(),
        Op::ExVector3fConst => bail!("todo ExVector3fConst"),
        Op::ExStructMemberContext => ExStructMemberContext {
            struct_member_expression: KismetPropertyPointer(s.read_u64::<LE>()?),
            struct_expression: read(s, graph)?,
        }
        .into(),
        Op::ExLetMulticastDelegate => bail!("todo ExLetMulticastDelegate"),
        Op::ExLetDelegate => bail!("todo ExLetDelegate"),
        Op::ExLocalVirtualFunction => ExLocalVirtualFunction {
            virtual_function_name: read_fname(s)?,
            parameters: read_until(s, graph, ExprOp::ExEndFunctionParms)?,
        }
        .into(),
        Op::ExLocalFinalFunction => ExLocalFinalFunction {
            stack_node: PackageIndex(s.read_u64::<LE>()?),
            parameters: read_until(s, graph, ExprOp::ExEndFunctionParms)?,
        }
        .into(),
        Op::ExLocalOutVariable => ExLocalOutVariable {
            variable: KismetPropertyPointer(s.read_u64::<LE>()?),
        }
        .into(),
        Op::ExDeprecatedOp4A => bail!("todo ExDeprecatedOp4A"),
        Op::ExInstanceDelegate => bail!("todo ExInstanceDelegate"),
        Op::ExPushExecutionFlow => ExPushExecutionFlow {
            pushing_address: s.read_u32::<LE>()?,
        }
        .into(),
        Op::ExPopExecutionFlow => ExPopExecutionFlow {}.into(),
        Op::ExComputedJump => ExComputedJump {
            code_offset_expression: read(s, graph)?,
        }
        .into(),
        Op::ExPopExecutionFlowIfNot => ExPopExecutionFlowIfNot {
            boolean_expression: read(s, graph)?,
        }
        .into(),
        Op::ExBreakpoint => bail!("todo ExBreakpoint"),
        Op::ExInterfaceContext => ExInterfaceContext {
            interface_value: read(s, graph)?,
        }
        .into(),
        Op::ExObjToInterfaceCast => ExObjToInterfaceCast {
            class_ptr: PackageIndex(s.read_u64::<LE>()?),
            target: read(s, graph)?,
        }
        .into(),
        Op::ExEndOfScript => ExEndOfScript {}.into(),
        Op::ExCrossInterfaceCast => bail!("todo ExCrossInterfaceCast"),
        Op::ExInterfaceToObjCast => bail!("todo ExInterfaceToObjCast"),
        Op::ExWireTracepoint => bail!("todo ExWireTracepoint"),
        Op::ExSkipOffsetConst => ExSkipOffsetConst {
            skip: s.read_u32::<LE>()?,
        }
        .into(),
        Op::ExAddMulticastDelegate => ExAddMulticastDelegate {
            delegate: read(s, graph)?,
            delegate_to_add: read(s, graph)?,
        }
        .into(),
        Op::ExClearMulticastDelegate => ExClearMulticastDelegate {
            delegate_to_clear: read(s, graph)?,
        }
        .into(),
        Op::ExTracepoint => bail!("todo ExTracepoint"),
        Op::ExLetObj => ExLetObj {
            variable_expression: read(s, graph)?,
            assignment_expression: read(s, graph)?,
        }
        .into(),
        Op::ExLetWeakObjPtr => bail!("todo ExLetWeakObjPtr"),
        Op::ExBindDelegate => ExBindDelegate {
            function_name: read_fname(s)?,
            delegate: read(s, graph)?,
            object_term: read(s, graph)?,
        }
        .into(),
        Op::ExRemoveMulticastDelegate => ExRemoveMulticastDelegate {
            delegate: read(s, graph)?,
            delegate_to_add: read(s, graph)?,
        }
        .into(),
        Op::ExCallMulticastDelegate => ExCallMulticastDelegate {
            stack_node: PackageIndex(s.read_u64::<LE>()?),
            parameters: read_until(s, graph, ExprOp::ExEndFunctionParms)?,
            delegate: ExprIndex(0), // TODO fake news?
        }
        .into(),
        Op::ExLetValueOnPersistentFrame => ExLetValueOnPersistentFrame {
            destination_property: KismetPropertyPointer(s.read_u64::<LE>()?),
            assignment_expression: read(s, graph)?,
        }
        .into(),
        Op::ExArrayConst => bail!("todo ExArrayConst"),
        Op::ExEndArrayConst => bail!("todo ExEndArrayConst"),
        Op::ExSoftObjectConst => bail!("todo ExSoftObjectConst"),
        Op::ExCallMath => ExCallMath {
            stack_node: PackageIndex(s.read_u64::<LE>()?),
            parameters: read_until(s, graph, ExprOp::ExEndFunctionParms)?,
        }
        .into(),
        Op::ExSwitchValue => {
            let case_count = s.read_u16::<LE>()?;
            let end_goto_offset = s.read_u32::<LE>()?;
            let index_term = read(s, graph)?;
            let mut cases = vec![];
            for _ in 0..case_count {
                cases.push(KismetSwitchCase {
                    case_index_value_term: read(s, graph)?,
                    code_skip_size_type: s.read_u32::<LE>()?,
                    case_term: read(s, graph)?,
                });
            }
            let default_term = read(s, graph)?;
            ExSwitchValue {
                end_goto_offset,
                index_term,
                default_term,
                cases,
            }
            .into()
        }
        Op::ExInstrumentationEvent => bail!("todo ExInstrumentationEvent"),
        Op::ExArrayGetByRef => bail!("todo ExArrayGetByRef"),
        Op::ExClassSparseDataVariable => bail!("todo ExClassSparseDataVariable"),
        Op::ExFieldPathConst => bail!("todo ExFieldPathConst"),
        Op::ExAutoRtfmTransact => bail!("todo ExAutoRtfmTransact"),
        Op::ExAutoRtfmStopTransact => bail!("todo ExAutoRtfmStopTransact"),
        Op::ExAutoRtfmAbortIfNot => bail!("todo ExAutoRtfmAbortIfNot"),
    };
    Ok(ex)
}

pub fn byte_size(ex: literal::Expr) -> Result<usize> {
    use literal::{Expr as Ex, *};
    match ex {
        Ex::ExLocalVariable(ex) => todo!(),
        Ex::ExInstanceVariable(ex) => todo!(),
        Ex::ExDefaultVariable(ex) => todo!(),
        Ex::ExReturn(ex) => todo!(),
        Ex::ExJump(ex) => todo!(),
        Ex::ExJumpIfNot(ex) => todo!(),
        Ex::ExAssert(ex) => todo!(),
        Ex::ExNothing(ex) => todo!(),
        Ex::ExNothingInt32(ex) => todo!(),
        Ex::ExLet(ex) => todo!(),
        Ex::ExBitFieldConst(ex) => todo!(),
        Ex::ExClassContext(ex) => todo!(),
        Ex::ExMetaCast(ex) => todo!(),
        Ex::ExLetBool(ex) => todo!(),
        Ex::ExEndParmValue(ex) => todo!(),
        Ex::ExEndFunctionParms(ex) => todo!(),
        Ex::ExSelf(ex) => todo!(),
        Ex::ExSkip(ex) => todo!(),
        Ex::ExContext(ex) => todo!(),
        Ex::ExContextFailSilent(ex) => todo!(),
        Ex::ExVirtualFunction(ex) => todo!(),
        Ex::ExFinalFunction(ex) => todo!(),
        Ex::ExIntConst(ex) => todo!(),
        Ex::ExFloatConst(ex) => todo!(),
        Ex::ExStringConst(ex) => todo!(),
        Ex::ExObjectConst(ex) => todo!(),
        Ex::ExNameConst(ex) => todo!(),
        Ex::ExRotationConst(ex) => todo!(),
        Ex::ExVectorConst(ex) => todo!(),
        Ex::ExByteConst(ex) => todo!(),
        Ex::ExIntZero(ex) => todo!(),
        Ex::ExIntOne(ex) => todo!(),
        Ex::ExTrue(ex) => todo!(),
        Ex::ExFalse(ex) => todo!(),
        Ex::ExTextConst(ex) => todo!(),
        Ex::ExNoObject(ex) => todo!(),
        Ex::ExTransformConst(ex) => todo!(),
        Ex::ExIntConstByte(ex) => todo!(),
        Ex::ExNoInterface(ex) => todo!(),
        Ex::ExDynamicCast(ex) => todo!(),
        Ex::ExStructConst(ex) => todo!(),
        Ex::ExEndStructConst(ex) => todo!(),
        Ex::ExSetArray(ex) => todo!(),
        Ex::ExEndArray(ex) => todo!(),
        Ex::ExPropertyConst(ex) => todo!(),
        Ex::ExUnicodeStringConst(ex) => todo!(),
        Ex::ExInt64Const(ex) => todo!(),
        Ex::ExUInt64Const(ex) => todo!(),
        Ex::ExDoubleConst(ex) => todo!(),
        Ex::ExCast(ex) => todo!(),
        Ex::ExSetSet(ex) => todo!(),
        Ex::ExEndSet(ex) => todo!(),
        Ex::ExSetMap(ex) => todo!(),
        Ex::ExEndMap(ex) => todo!(),
        Ex::ExSetConst(ex) => todo!(),
        Ex::ExEndSetConst(ex) => todo!(),
        Ex::ExMapConst(ex) => todo!(),
        Ex::ExEndMapConst(ex) => todo!(),
        Ex::ExVector3fConst(ex) => todo!(),
        Ex::ExStructMemberContext(ex) => todo!(),
        Ex::ExLetMulticastDelegate(ex) => todo!(),
        Ex::ExLetDelegate(ex) => todo!(),
        Ex::ExLocalVirtualFunction(ex) => todo!(),
        Ex::ExLocalFinalFunction(ex) => todo!(),
        Ex::ExLocalOutVariable(ex) => todo!(),
        Ex::ExDeprecatedOp4A(ex) => todo!(),
        Ex::ExInstanceDelegate(ex) => todo!(),
        Ex::ExPushExecutionFlow(ex) => todo!(),
        Ex::ExPopExecutionFlow(ex) => todo!(),
        Ex::ExComputedJump(ex) => todo!(),
        Ex::ExPopExecutionFlowIfNot(ex) => todo!(),
        Ex::ExBreakpoint(ex) => todo!(),
        Ex::ExInterfaceContext(ex) => todo!(),
        Ex::ExObjToInterfaceCast(ex) => todo!(),
        Ex::ExEndOfScript(ex) => todo!(),
        Ex::ExCrossInterfaceCast(ex) => todo!(),
        Ex::ExInterfaceToObjCast(ex) => todo!(),
        Ex::ExWireTracepoint(ex) => todo!(),
        Ex::ExSkipOffsetConst(ex) => todo!(),
        Ex::ExAddMulticastDelegate(ex) => todo!(),
        Ex::ExClearMulticastDelegate(ex) => todo!(),
        Ex::ExTracepoint(ex) => todo!(),
        Ex::ExLetObj(ex) => todo!(),
        Ex::ExLetWeakObjPtr(ex) => todo!(),
        Ex::ExBindDelegate(ex) => todo!(),
        Ex::ExRemoveMulticastDelegate(ex) => todo!(),
        Ex::ExCallMulticastDelegate(ex) => todo!(),
        Ex::ExLetValueOnPersistentFrame(ex) => todo!(),
        Ex::ExArrayConst(ex) => todo!(),
        Ex::ExEndArrayConst(ex) => todo!(),
        Ex::ExSoftObjectConst(ex) => todo!(),
        Ex::ExCallMath(ex) => todo!(),
        Ex::ExSwitchValue(ex) => todo!(),
        Ex::ExInstrumentationEvent(ex) => todo!(),
        Ex::ExArrayGetByRef(ex) => todo!(),
        Ex::ExClassSparseDataVariable(ex) => todo!(),
        Ex::ExFieldPathConst(ex) => todo!(),
        Ex::ExAutoRtfmTransact(ex) => todo!(),
        Ex::ExAutoRtfmStopTransact(ex) => todo!(),
        Ex::ExAutoRtfmAbortIfNot(ex) => todo!(),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse() {
        let script = include_bytes!(
            "../scripts/Function _RoomGenerator_RoomBuilderSquare.RoomBuilderSquare_C.Create Room.bin"
        );
        let graph = read_all(&mut std::io::Cursor::new(script)).unwrap();
        dbg!(graph);
    }
}

// #ifndef XFER
// #define XFER(T) \
// 			{ \
// 				T Temp; \
// 				if (!Ar.IsLoading()) \
// 				{ \
// 					Temp =  FPlatformMemory::ReadUnaligned<T>(&Script[iCode]); \
// 				} \
// 				Ar << Temp; \
// 				if (!Ar.IsSaving()) \
// 				{ \
// 					FPlatformMemory::WriteUnaligned<T>(&Script[iCode], Temp); \
// 				} \
// 				iCode += sizeof(T); \
// 			}
// #endif

// //FScriptName
// #ifndef XFERNAME
// 	#define XFERNAME() \
// 	{ \
//    	    FName Name; \
// 		FScriptName ScriptName; \
//         if (!Ar.IsLoading()) \
// 		{ \
// 			FMemory::Memcpy( &ScriptName, &Script[iCode], sizeof(FScriptName) ); \
// 			Name = ScriptNameToName(ScriptName); \
// 		} \
// 		Ar << Name; \
// 		if (!Ar.IsSaving()) \
// 		{ \
// 			ScriptName = NameToScriptName(Name); \
// 			FMemory::Memcpy( &Script[iCode], &ScriptName, sizeof(FScriptName) ); \
// 		} \
// 		iCode += sizeof(FScriptName); \
// 	}
// #endif	//XFERNAME

// // ASCII string
// #ifndef XFERSTRING
// 	#define XFERSTRING() \
// 	{ \
// 		do XFER(uint8) while( Script[iCode-1] ); \
// 	}
// #endif	//XFERSTRING

// // UTF-16 string
// #ifndef XFERUNICODESTRING
// 	#define XFERUNICODESTRING() \
// 	{ \
// 		do XFER(uint16) while( Script[iCode-1] || Script[iCode-2] ); \
// 	}
// #endif	//XFERUNICODESTRING

// //FText
// #ifndef XFERTEXT
// 	#define XFERTEXT() \
// 	{ \
// 		XFER(uint8); \
// 		const EBlueprintTextLiteralType TextLiteralType = (EBlueprintTextLiteralType)Script[iCode - 1]; \
// 		switch (TextLiteralType) \
// 		{ \
// 		case EBlueprintTextLiteralType::Empty: \
// 			break; \
// 		case EBlueprintTextLiteralType::LocalizedText: \
// 			SerializeExpr( iCode, Ar );	\
// 			SerializeExpr( iCode, Ar ); \
// 			SerializeExpr( iCode, Ar ); \
// 			break; \
// 		case EBlueprintTextLiteralType::InvariantText: \
// 			SerializeExpr( iCode, Ar );	\
// 			break; \
// 		case EBlueprintTextLiteralType::LiteralString: \
// 			SerializeExpr( iCode, Ar );	\
// 			break; \
// 		case EBlueprintTextLiteralType::StringTableEntry: \
// 			XFER_OBJECT_POINTER( UObject* ); \
// 			FIXUP_EXPR_OBJECT_POINTER( UObject* ); \
// 			SerializeExpr( iCode, Ar );	\
// 			SerializeExpr( iCode, Ar ); \
// 			break; \
// 		default: \
// 			checkf(false, TEXT("Unknown EBlueprintTextLiteralType! Please update XFERTEXT to handle this type of text.")); \
// 			break; \
// 		} \
// 	}
// #endif	//XFERTEXT

// #ifndef XFERPTR
// 	#define XFERPTR(T) \
// 	{ \
//    	    T AlignedPtr = NULL; \
// 		ScriptPointerType TempCode; \
//         if (!Ar.IsLoading()) \
// 		{ \
// 			FMemory::Memcpy( &TempCode, &Script[iCode], sizeof(ScriptPointerType) ); \
// 			AlignedPtr = (T)(TempCode); \
// 		} \
// 		Ar << AlignedPtr; \
// 		if (!Ar.IsSaving()) \
// 		{ \
// 			TempCode = (ScriptPointerType)(AlignedPtr); \
// 			FMemory::Memcpy( &Script[iCode], &TempCode, sizeof(ScriptPointerType) ); \
// 		} \
// 		iCode += sizeof(ScriptPointerType); \
// 	}
// #endif	//	XFERPTR

// #ifndef XFERTOBJPTR
// 	#define XFERTOBJPTR() \
// 	{ \
// 		TObjectPtr<UObject> AlignedPtr; \
// 		if (!Ar.IsLoading()) \
// 		{ \
// 			FMemory::Memcpy(&AlignedPtr, &Script[iCode], sizeof(ScriptPointerType)); \
// 		} \
// 			Ar << AlignedPtr; \
// 		if (!Ar.IsSaving()) \
// 		{ \
// 			FMemory::Memcpy(&Script[iCode], &AlignedPtr, sizeof(ScriptPointerType)); \
// 		} \
// 		iCode += sizeof(ScriptPointerType); \
// 	}
// #endif	//	XFERTOBJPTR

// #ifndef XFER_FUNC_POINTER
// 	#define XFER_FUNC_POINTER	XFERPTR(UStruct*)
// #endif	// XFER_FUNC_POINTER

// #ifndef XFER_FUNC_NAME
// 	#define XFER_FUNC_NAME		XFERNAME()
// #endif	// XFER_FUNC_NAME

// #ifndef XFER_PROP_POINTER
// 	#define XFER_PROP_POINTER	XFERPTR(FProperty*)
// #endif

// #ifndef XFER_OBJECT_POINTER
// 	#define XFER_OBJECT_POINTER(Type)	XFERPTR(Type)
// #endif

// #ifndef XFER_TOBJECT_PTR
// 	#define XFER_TOBJECT_PTR	XFERTOBJPTR
// #endif

// #ifndef FIXUP_EXPR_OBJECT_POINTER
// 	// sometimes after a UOBject* expression is loaded it may require some post-
// 	// processing (see: the overridden FIXUP_EXPR_OBJECT_POINTER(), defined in Class.cpp)
// 	#define FIXUP_EXPR_OBJECT_POINTER(Type)
// #endif

// /** UStruct::SerializeExpr() */
// #ifdef SERIALIZEEXPR_INC
// 	EExprToken Expr=(EExprToken)0;

// 	// Get expr token.
// 	XFER(uint8);
// 	Expr = (EExprToken)Script[iCode-1];

// 	switch( Expr )
// 	{
// 		case EX_Cast:
// 		{
// 			// A type conversion.
// 			XFER(uint8); //which kind of conversion
// 			SerializeExpr( iCode, Ar );
// 			break;
// 		}
// 		case EX_ObjToInterfaceCast:
// 		case EX_CrossInterfaceCast:
// 		case EX_InterfaceToObjCast:
// 		{
// 			// A conversion from an object or interface variable to a native interface variable.
// 			// We use a different bytecode to avoid the branching each time we process a cast token.

// 			XFER_OBJECT_POINTER(UClass*); // the interface class to convert to
// 			FIXUP_EXPR_OBJECT_POINTER(UClass*);

// 			SerializeExpr( iCode, Ar );
// 			break;
// 		}
// 		case EX_Let:
// 		{
// 			XFER_PROP_POINTER;
// 		}
// 		case EX_LetObj:
// 		case EX_LetWeakObjPtr:
// 		case EX_LetBool:
// 		case EX_LetDelegate:
// 		case EX_LetMulticastDelegate:
// 		{
// 			SerializeExpr( iCode, Ar ); // Variable expr.
// 			SerializeExpr( iCode, Ar ); // Assignment expr.
// 			break;
// 		}
// 		case EX_LetValueOnPersistentFrame:
// 		{
// 			XFER_PROP_POINTER;			// Destination property.
// 			SerializeExpr(iCode, Ar);	// Assignment expr.
// 			break;
// 		}
// 		case EX_StructMemberContext:
// 		{
// 			XFERPTR(FProperty*);        // struct member expr.
// 			SerializeExpr( iCode, Ar ); // struct expr.
// 			break;
// 		}
// 		case EX_Jump:
// 		{
// 			XFER(CodeSkipSizeType); // Code offset.
// 			break;
// 		}
// 		case EX_ComputedJump:
// 		{
// 			SerializeExpr( iCode, Ar ); // Integer expression, specifying code offset.
// 			break;
// 		}
// 		case EX_LocalVariable:
// 		case EX_InstanceVariable:
// 		case EX_DefaultVariable:
// 		case EX_LocalOutVariable:
// 		case EX_ClassSparseDataVariable:
// 		case EX_PropertyConst:
// 		{
// 			XFER_PROP_POINTER;
// 			break;
// 		}
// 		case EX_InterfaceContext:
// 		{
// 			SerializeExpr(iCode,Ar);
// 			break;
// 		}
// 		case EX_PushExecutionFlow:
// 		{
// 			XFER(CodeSkipSizeType);		// location to push
// 			break;
// 		}
// 		case EX_NothingInt32:
// 		{
// 			XFER(int32);
// 			break;
// 		}
// 		case EX_Nothing:
// 		case EX_EndOfScript:
// 		case EX_EndFunctionParms:
// 		case EX_EndStructConst:
// 		case EX_EndArray:
// 		case EX_EndArrayConst:
// 		case EX_EndSet:
// 		case EX_EndMap:
// 		case EX_EndSetConst:
// 		case EX_EndMapConst:
// 		case EX_IntZero:
// 		case EX_IntOne:
// 		case EX_True:
// 		case EX_False:
// 		case EX_NoObject:
// 		case EX_NoInterface:
// 		case EX_Self:
// 		case EX_EndParmValue:
// 		case EX_PopExecutionFlow:
// 		case EX_DeprecatedOp4A:
// 		{
// 			break;
// 		}
// 		case EX_WireTracepoint:
// 		case EX_Tracepoint:
// 		{
// 			break;
// 		}
// 		case EX_Breakpoint:
// 		{
// 			if (Ar.IsLoading())
// 			{
// 				// Turn breakpoints into tracepoints on load
// 				Script[iCode-1] = EX_Tracepoint;
// 			}
// 			break;
// 		}
// 		case EX_InstrumentationEvent:
// 		{
// 			if (Script[iCode] == EScriptInstrumentation::InlineEvent)
// 			{
// 				iCode += sizeof(FScriptName);
// 			}
// 			iCode += sizeof(uint8);
// 			break;
// 		}
// 		case EX_Return:
// 		{
// 			SerializeExpr( iCode, Ar ); // Return expression.
// 			break;
// 		}
// 		case EX_CallMath:
// 		case EX_LocalFinalFunction:
// 		case EX_FinalFunction:
// 		{
// 			XFER_FUNC_POINTER;											// Stack node.
// 			FIXUP_EXPR_OBJECT_POINTER(UStruct*);
// 			while( SerializeExpr( iCode, Ar ) != EX_EndFunctionParms ); // Parms.
// 			break;
// 		}
// 		case EX_LocalVirtualFunction:
// 		case EX_VirtualFunction:
// 		{
// 			XFER_FUNC_NAME;												// Virtual function name.
// 			while( SerializeExpr( iCode, Ar ) != EX_EndFunctionParms );	// Parms.
// 			break;
// 		}
// 		case EX_CallMulticastDelegate:
// 		{
// 			XFER_FUNC_POINTER;											// Stack node.
// 			FIXUP_EXPR_OBJECT_POINTER(UStruct*);
// 			while( SerializeExpr( iCode, Ar ) != EX_EndFunctionParms ); // Parms.
// 			break;
// 		}
// 		case EX_ClassContext:
// 		case EX_Context:
// 		case EX_Context_FailSilent:
// 		{
// 			SerializeExpr( iCode, Ar ); // Object expression.
// 			XFER(CodeSkipSizeType);		// Code offset for NULL expressions.
// 			XFERPTR(FField*);			// Property corresponding to the r-value data, in case the l-value needs to be mem-zero'd
// 			SerializeExpr( iCode, Ar ); // Context expression.
// 			break;
// 		}
// 		case EX_AddMulticastDelegate:
// 		case EX_RemoveMulticastDelegate:
// 		{
// 			SerializeExpr( iCode, Ar );	// Delegate property to assign to
// 			SerializeExpr( iCode, Ar ); // Delegate to add to the MC delegate for broadcast
// 			break;
// 		}
// 		case EX_ClearMulticastDelegate:
// 		{
// 			SerializeExpr( iCode, Ar );	// Delegate property to clear
// 			break;
// 		}
// 		case EX_IntConst:
// 		{
// 			XFER(int32);
// 			break;
// 		}
// 		case EX_Int64Const:
// 		{
// 			XFER(int64);
// 			break;
// 		}
// 		case EX_UInt64Const:
// 		{
// 			XFER(uint64);
// 			break;
// 		}
// 		case EX_SkipOffsetConst:
// 		{
// 			XFER(CodeSkipSizeType);
// 			break;
// 		}
// 		case EX_FloatConst:
// 		{
// 			XFER(float);
// 			break;
// 		}
// 		case EX_DoubleConst:
// 		{
// 			XFER(double);
// 			break;
// 		}
// 		case EX_StringConst:
// 		{
// 			XFERSTRING();
// 			break;
// 		}
// 		case EX_UnicodeStringConst:
// 		{
// 			XFERUNICODESTRING();
// 			break;
// 		}
// 		case EX_TextConst:
// 		{
// 			XFERTEXT();
// 			break;
// 		}
// 		case EX_ObjectConst:
// 		{
// 			XFER_TOBJECT_PTR();
// 			FIXUP_EXPR_OBJECT_POINTER(TObjectPtr<UObject>);

// 			break;
// 		}
// 		case EX_SoftObjectConst:
// 		{
// 			// if collecting references inform the archive of the reference:
// 			if (Ar.IsSaving() && Ar.IsObjectReferenceCollector())
// 			{
// 				XFER(uint8);
// 				Expr = (EExprToken)Script[iCode - 1];
// 				check(Expr == EX_StringConst || Expr == EX_UnicodeStringConst);
// 				FString LongPath;
// 				if (Expr == EX_StringConst)
// 				{
// 					LongPath = (ANSICHAR*)&Script[iCode];
// 					XFERSTRING();
// 				}
// 				else
// 				{
// 					LongPath = FString((UCS2CHAR*)&Script[iCode]);

// 					// Inline combine any surrogate pairs in the data when loading into a UTF-32 string
// 					StringConv::InlineCombineSurrogates(LongPath);
// 					XFERUNICODESTRING();
// 				}
// 				FSoftObjectPath Path(LongPath);
// 				Ar << Path;
// 				// we can't patch the path, but we could log an attempt to do so
// 				// or change the implementation to support patching (allocating
// 				// these strings in a special region or distinct object)
// 			}
// 			else
// 			{
// 				// else just write the string literal instructions:
// 				SerializeExpr(iCode, Ar);
// 			}
// 			break;
// 		}
// 		case EX_FieldPathConst:
// 		{
// 			SerializeExpr(iCode, Ar);
// 			break;
// 		}
// 		case EX_NameConst:
// 		{
// 			XFERNAME();
// 			break;
// 		}
// 		case EX_RotationConst:
// 		{
// 			if(Ar.UEVer() >= EUnrealEngineObjectUE5Version::LARGE_WORLD_COORDINATES)
// 			{
// 				XFER(int64); XFER(int64); XFER(int64);
// 			}
// 			else
// 			{
// 				XFER(int32); XFER(int32); XFER(int32);
// 			}
// 			break;
// 		}
// 		case EX_VectorConst:
// 		{
// 			if(Ar.UEVer() >= EUnrealEngineObjectUE5Version::LARGE_WORLD_COORDINATES)
// 			{
// 				XFER(double); XFER(double); XFER(double);
// 			}
// 			else
// 			{
// 				XFER(float); XFER(float); XFER(float);
// 			}
// 			break;
// 		}
// 		case EX_Vector3fConst:
// 		{
// 			XFER(float); XFER(float); XFER(float);
// 			break;
// 		}
// 		case EX_TransformConst:
// 		{
// 			if(Ar.UEVer() >= EUnrealEngineObjectUE5Version::LARGE_WORLD_COORDINATES)
// 			{
// 				// Rotation
// 				XFER(double); XFER(double); XFER(double); XFER(double);
// 				// Translation
// 				XFER(double); XFER(double); XFER(double);
// 				// Scale
// 				XFER(double); XFER(double); XFER(double);
// 			}
// 			else
// 			{
// 				// Rotation
// 				XFER(float); XFER(float); XFER(float); XFER(float);
// 				// Translation
// 				XFER(float); XFER(float); XFER(float);
// 				// Scale
// 				XFER(float); XFER(float); XFER(float);
// 			}
// 			break;
// 		}
// 		case EX_StructConst:
// 		{
// 			XFERPTR(UScriptStruct*);	// Struct.
// 			XFER(int32);					// Serialized struct size
// 			while( SerializeExpr( iCode, Ar ) != EX_EndStructConst );
// 			break;
// 		}
// 		case EX_SetArray:
// 		{
// 			// If not loading, or its a newer version
// 			if((!GetLinker()) || !Ar.IsLoading() || (Ar.UEVer() >= VER_UE4_CHANGE_SETARRAY_BYTECODE))
// 			{
// 				// Array property to assign to
// 				EExprToken TargetToken = SerializeExpr( iCode, Ar );
// 			}
// 			else
// 			{
// 				// Array Inner Prop
// 				XFERPTR(FProperty*);
// 			}

// 			while( SerializeExpr( iCode, Ar) != EX_EndArray );
// 			break;
// 		}
// 		case EX_SetSet:
// 			SerializeExpr( iCode, Ar ); // set property
// 			XFER(int32);			// Number of elements
// 			while( SerializeExpr( iCode, Ar) != EX_EndSet );
// 			break;
// 		case EX_SetMap:
// 			SerializeExpr( iCode, Ar ); // map property
// 			XFER(int32);			// Number of elements
// 			while( SerializeExpr( iCode, Ar) != EX_EndMap );
// 			break;
// 		case EX_ArrayConst:
// 		{
// 			XFERPTR(FProperty*);	// Inner property
// 			XFER(int32);			// Number of elements
// 			while (SerializeExpr(iCode, Ar) != EX_EndArrayConst);
// 			break;
// 		}
// 		case EX_SetConst:
// 		{
// 			XFERPTR(FProperty*);	// Inner property
// 			XFER(int32);			// Number of elements
// 			while (SerializeExpr(iCode, Ar) != EX_EndSetConst);
// 			break;
// 		}
// 		case EX_MapConst:
// 		{
// 			XFERPTR(FProperty*);	// Key property
// 			XFERPTR(FProperty*);	// Val property
// 			XFER(int32);			// Number of elements
// 			while (SerializeExpr(iCode, Ar) != EX_EndMapConst);
// 			break;
// 		}
// 		case EX_BitFieldConst:
// 		{
// 			XFERPTR(FProperty*);	// Bit property
// 			XFER(uint8);			// bit value
// 			break;
// 		}
// 		case EX_ByteConst:
// 		case EX_IntConstByte:
// 		{
// 			XFER(uint8);
// 			break;
// 		}
// 		case EX_MetaCast:
// 		{
// 			XFER_OBJECT_POINTER(UClass*);
// 			FIXUP_EXPR_OBJECT_POINTER(UClass*);

// 			SerializeExpr( iCode, Ar );
// 			break;
// 		}
// 		case EX_DynamicCast:
// 		{
// 			XFER_OBJECT_POINTER(UClass*);
// 			FIXUP_EXPR_OBJECT_POINTER(UClass*);

// 			SerializeExpr( iCode, Ar );
// 			break;
// 		}
// 		case EX_JumpIfNot:
// 		{
// 			XFER(CodeSkipSizeType);		// Code offset.
// 			SerializeExpr( iCode, Ar ); // Boolean expr.
// 			break;
// 		}
// 		case EX_PopExecutionFlowIfNot:
// 		{
// 			SerializeExpr( iCode, Ar ); // Boolean expr.
// 			break;
// 		}
// 		case EX_Assert:
// 		{
// 			XFER(uint16); // Line number.
// 			XFER(uint8); // debug mode or not
// 			SerializeExpr( iCode, Ar ); // Assert expr.
// 			break;
// 		}
// 		case EX_Skip:
// 		{
// 			XFER(CodeSkipSizeType);		// Skip size.
// 			SerializeExpr( iCode, Ar ); // Expression to possibly skip.
// 			break;
// 		}
// 		case EX_InstanceDelegate:
// 		{
// 			XFER_FUNC_NAME;				// the name of the function assigned to the delegate.
// 			break;
// 		}
// 		case EX_BindDelegate:
// 		{
// 			XFER_FUNC_NAME;
// 			SerializeExpr( iCode, Ar );	// Delegate property to assign to
// 			SerializeExpr( iCode, Ar );
// 			break;
// 		}
// 		case EX_SwitchValue:
// 		{
// 			XFER(uint16); // number of cases, without default one
// 			const uint16 NumCases = FPlatformMemory::ReadUnaligned<uint16>(&Script[iCode - sizeof(uint16)]);
// 			XFER(CodeSkipSizeType); // Code offset, go to it, when done.
// 			SerializeExpr(iCode, Ar);	//index term

// 			for (uint16 CaseIndex = 0; CaseIndex < NumCases; ++CaseIndex)
// 			{
// 				SerializeExpr(iCode, Ar);	// case index value term
// 				XFER(CodeSkipSizeType);		// offset to the next case
// 				SerializeExpr(iCode, Ar);	// case term
// 			}

// 			SerializeExpr(iCode, Ar);	//default term
// 			break;
// 		}
// 		case EX_ArrayGetByRef:
// 		{
// 			SerializeExpr( iCode, Ar );
// 			SerializeExpr( iCode, Ar );
// 			break;
// 		}
// 		case EX_AutoRtfmTransact:
// 		{
// 			XFER(int32); // Transaction id
// 			XFER(CodeSkipSizeType); // Code offset.
// 			while( SerializeExpr( iCode, Ar ) != EX_AutoRtfmStopTransact ); // Parms.
// 			break;
// 		}
// 		case EX_AutoRtfmStopTransact:
// 		{
// 			XFER(int32); // transaction id
// 			XFER(int8); // stop mode
// 			break;
// 		}
// 		case EX_AutoRtfmAbortIfNot:
// 		{
// 			SerializeExpr(iCode,Ar);
// 			break;
// 		}
// 		default:
// 		{
// 			// This should never occur.
// 			UE_LOG(LogScriptSerialization, Warning, TEXT("Error: Unknown bytecode 0x%02X; ignoring it"), (uint8)Expr );
// 			break;
// 		}
// 	}
