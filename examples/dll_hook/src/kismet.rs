use crate::ue::{self, FName};
use anyhow::{anyhow, bail, Result};
use byteorder::{ReadBytesExt, WriteBytesExt as _, LE};
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

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
pub enum FScriptText {
    Empty,
    LocalizedText {
        localized_source: ExprIndex,
        localized_key: ExprIndex,
        localized_namespace: ExprIndex,
    },
    InvariantText {
        invariant_literal_string: ExprIndex,
    },
    LiteralString {
        literal_string: ExprIndex,
    },
    StringTableEntry {
        string_table_asset: PackageIndex,
        string_table_id: ExprIndex,
        string_table_key: ExprIndex,
    },
}
#[derive(Debug, Clone, strum::FromRepr)]
#[repr(u8)]
pub enum EBlueprintTextLiteralType {
    Empty,
    LocalizedText,
    InvariantText,
    LiteralString,
    StringTableEntry,
}
impl EBlueprintTextLiteralType {
    fn try_from_repr(repr: u8) -> Result<Self> {
        Self::from_repr(repr).ok_or_else(|| anyhow!("invalid EBlueprintTextLiteralType: {repr}"))
    }
}
#[derive(Debug, Clone)]
pub struct KismetSwitchCase {
    pub case_index_value_term: ExprIndex,
    pub code_skip_size_type: u32,
    pub case_term: ExprIndex,
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
impl ECastToken {
    fn try_from_repr(repr: u8) -> Result<Self> {
        Self::from_repr(repr).ok_or_else(|| anyhow!("invalid ECastToken: {repr}"))
    }
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
    0x29: ExTextConst { value: [ FScriptText ] },
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
    0x36: ExUInt64Const { value: [ u64 ] },
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
    0x4C: ExPushExecutionFlow { pushing_address: [ ExprIndex ] },
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
            match &last_node.expr {
                Ex::ExReturn(_)
                | Ex::ExJump(_)
                | Ex::ExPopExecutionFlow(_)
                | Ex::ExComputedJump(_)
                | Ex::ExEndOfScript(_) => {}
                _ => {
                    last_node.next = Some(index);
                }
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
                Ex::ExVirtualFunction(ex) => {
                    write_fname(&mut self.s, ex.virtual_function_name)?;
                    for parm in ex.parameters {
                        let e = self.advance(parm);
                        self.write_ex(e)?;
                    }
                    self.write_ex(ExEndFunctionParms {}.into())?;
                }
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
                Ex::ExFloatConst(ex) => {
                    self.s.write_f32::<LE>(ex.value)?;
                }
                Ex::ExStringConst(ex) => bail!("todo write ExStringConst"),
                Ex::ExObjectConst(ex) => {
                    self.s.write_u64::<LE>(ex.value.0)?;
                }
                Ex::ExNameConst(ex) => bail!("todo write ExNameConst"),
                Ex::ExRotationConst(ex) => bail!("todo write ExRotationConst"),
                Ex::ExVectorConst(ex) => bail!("todo write ExVectorConst"),
                Ex::ExByteConst(ex) => {
                    self.s.write_u8(ex.value)?;
                }
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
                Ex::ExStructConst(ex) => {
                    self.s.write_u64::<LE>(ex.struct_value.0)?;
                    self.s.write_i32::<LE>(ex.struct_size)?;
                    for member in ex.value {
                        let e = self.advance(member);
                        self.write_ex(e)?;
                    }
                    self.write_ex(ExEndStructConst {}.into())?;
                }
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
                Ex::ExStructMemberContext(ex) => {
                    self.s.write_u64::<LE>(ex.struct_member_expression.0)?;
                    let e = self.advance(ex.struct_expression);
                    self.write_ex(e)?;
                }
                Ex::ExLetMulticastDelegate(ex) => bail!("todo write ExLetMulticastDelegate"),
                Ex::ExLetDelegate(ex) => bail!("todo write ExLetDelegate"),
                Ex::ExLocalVirtualFunction(ex) => {
                    write_fname(&mut self.s, ex.virtual_function_name)?;
                    for parm in ex.parameters {
                        let e = self.advance(parm);
                        self.write_ex(e)?;
                    }
                    self.write_ex(ExEndFunctionParms {}.into())?;
                }
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
                Ex::ExPushExecutionFlow(ex) => {
                    self.fixups.push((self.s.position(), ex.pushing_address));
                    self.s.write_u32::<LE>(0)?;
                }
                Ex::ExPopExecutionFlow(_) => {}
                Ex::ExComputedJump(ex) => bail!("todo write ExComputedJump"),
                Ex::ExPopExecutionFlowIfNot(ex) => {
                    let e = self.advance(ex.boolean_expression);
                    self.write_ex(e)?;
                }
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
                Ex::ExCallMath(ex) => {
                    self.s.write_u64::<LE>(ex.stack_node.0)?;
                    for parm in ex.parameters {
                        let e = self.advance(parm);
                        self.write_ex(e)?;
                    }
                    self.write_ex(ExEndFunctionParms {}.into())?;
                }
                Ex::ExSwitchValue(ex) => {
                    self.s.write_u16::<LE>(ex.cases.len().try_into().unwrap())?;
                    let switch_end = self.s.position();
                    self.s.write_u32::<LE>(0)?;

                    let e = self.advance(ex.index_term);
                    self.write_ex(e)?;

                    for case in ex.cases {
                        let e = self.advance(case.case_index_value_term);
                        self.write_ex(e)?;

                        let case_end = self.s.position();
                        self.s.write_u32::<LE>(0)?;

                        let e = self.advance(case.case_term);
                        self.write_ex(e)?;

                        // fixup case end
                        {
                            let pos = self.s.position();
                            self.s.set_position(case_end);
                            self.s.write_u32::<LE>(pos as u32)?;
                            self.s.set_position(pos);
                        }
                    }

                    let e = self.advance(ex.default_term);
                    self.write_ex(e)?;

                    // fixup switch end
                    {
                        let pos = self.s.position();
                        self.s.set_position(switch_end);
                        self.s.write_u32::<LE>(pos as u32)?;
                        self.s.set_position(pos);
                    }
                }
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

    for (index, expr) in c.fixups {
        c.s.set_position(index);
        c.s.write_u32::<LE>(c.ex_map[&expr] as u32)?;
    }

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
fn write_fname<S: Write>(s: &mut S, fname: ue::FName) -> Result<()> {
    s.write_u32::<LE>(fname.comparison_index.value)?;
    s.write_u32::<LE>(fname.comparison_index.value)?; // display index
    s.write_u32::<LE>(fname.number)?;
    Ok(())
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
fn read_unicode_string<S: Read>(s: &mut S) -> Result<String> {
    let mut chars = vec![];
    loop {
        match s.read_u16::<LE>()? {
            0 => break,
            c => chars.push(c),
        }
    }
    Ok(String::from_utf16(&chars)?)
}
fn read_fscript_text(s: &mut Cursor<&[u8]>, graph: &mut literal::ExprGraph) -> Result<FScriptText> {
    Ok(
        match EBlueprintTextLiteralType::try_from_repr(s.read_u8()?)? {
            EBlueprintTextLiteralType::Empty => FScriptText::Empty,
            EBlueprintTextLiteralType::LocalizedText => FScriptText::LocalizedText {
                localized_source: read(s, graph)?,
                localized_key: read(s, graph)?,
                localized_namespace: read(s, graph)?,
            },
            EBlueprintTextLiteralType::InvariantText => FScriptText::InvariantText {
                invariant_literal_string: read(s, graph)?,
            },
            EBlueprintTextLiteralType::LiteralString => FScriptText::LiteralString {
                literal_string: read(s, graph)?,
            },
            EBlueprintTextLiteralType::StringTableEntry => FScriptText::StringTableEntry {
                string_table_asset: PackageIndex(s.read_u64::<LE>()?),
                string_table_id: read(s, graph)?,
                string_table_key: read(s, graph)?,
            },
        },
    )
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
        Op::ExTextConst => ExTextConst {
            value: read_fscript_text(s, graph)?,
        }
        .into(),
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
        Op::ExUnicodeStringConst => ExUnicodeStringConst {
            value: read_unicode_string(s)?,
        }
        .into(),
        Op::ExInt64Const => ExInt64Const {
            value: s.read_i64::<LE>()?,
        }
        .into(),
        Op::ExUInt64Const => ExUInt64Const {
            value: s.read_u64::<LE>()?,
        }
        .into(),
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
            conversion_type: ECastToken::try_from_repr(s.read_u8()?)?,
            target: read(s, graph)?,
        }
        .into(),
        Op::ExSetSet => ExSetSet {
            set_property: read(s, graph)?,
            elements: read_until(s, graph, ExprOp::ExEndSet)?,
        }
        .into(),
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
            pushing_address: ExprIndex(s.read_u32::<LE>()? as usize),
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
