use crate::ue::{self, FName};
use anyhow::{anyhow, bail, Result};
use byteorder::{ReadBytesExt, LE};
use std::io::Read;

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
        #[derive(Debug)]
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

            #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, strum::FromRepr)]
            #[repr(u8)]
            pub enum ExprOp {
                $( $name = $op, )*
            }
            #[derive(Debug)]
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

            #[derive(Debug)]
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

//use unreal_asset::types::PackageIndex;
#[derive(Debug)]
struct KismetPropertyPointer(pub u64);
// {
// owner: PackageIndex,
// path: Vec<String>,
// }
#[derive(Debug, Clone, Copy)]
struct PackageIndex(pub u64);
#[derive(Debug)]
struct FScriptText;
#[derive(Debug)]
struct KismetSwitchCase;

#[derive(Debug)]
struct Vector<T: Clone> {
    x: T,
    y: T,
    z: T,
}

#[derive(Debug)]
struct Vector4<T: Clone> {
    x: T,
    y: T,
    z: T,
    w: T,
}

#[derive(Debug)]
struct Transform<T: Clone> {
    rotation: Vector4<T>,
    translation: Vector<T>,
    scale: Vector<T>,
}

#[derive(Debug)]
#[repr(u8)]
enum ECastToken {
    ObjectToInterface,
    ObjectToBool,
    InterfaceToBool,
    Max,
}

#[derive(Debug)]
#[repr(u8)]
enum EScriptInstrumentationType {
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
    0x04: ExReturn { return_expression: [ Box<Expr> ] },
    // 0x05
    0x06: ExJump { code_offset: [ u32 ] },
    0x07: ExJumpIfNot { code_offset: [ u32 ] boolean_expression: [ Box<Expr> ] },
    // 0x08
    0x09: ExAssert { line_number: [ u16 ] debug_mode: [ bool ] assert_expression: [ Box<Expr> ] },
    // 0x0A
    0x0B: ExNothing {  },
    0x0C: ExNothingInt32 {  },
    // 0x0D
    // 0x0E
    0x0F: ExLet { value: [ KismetPropertyPointer ] variable: [ Box<Expr> ] expression: [ Box<Expr> ] },
    // 0x10
    0x11: ExBitFieldConst { /* TODO */ },
    0x12: ExClassContext { object_expression: [ Box<Expr> ] offset: [ u32 ] r_value_pointer: [ KismetPropertyPointer ] context_expression: [ Box<Expr> ] },
    0x13: ExMetaCast { class_ptr: [ PackageIndex ] target_expression: [ Box<Expr> ] },
    0x14: ExLetBool { variable_expression: [ Box<Expr> ] assignment_expression: [ Box<Expr> ] },
    0x15: ExEndParmValue {  },
    0x16: ExEndFunctionParms {  },
    0x17: ExSelf {  },
    0x18: ExSkip { code_offset: [ u32 ] skip_expression: [ Box<Expr> ] },
    0x19: ExContext { object_expression: [ Box<Expr> ] offset: [ u32 ] r_value_pointer: [ KismetPropertyPointer ] context_expression: [ Box<Expr> ] },
    0x1A: ExContextFailSilent { object_expression: [ Box<Expr> ] offset: [ u32 ] r_value_pointer: [ KismetPropertyPointer ] context_expression: [ Box<Expr> ] },
    0x1B: ExVirtualFunction { virtual_function_name: [ FName ] parameters: [ Vec<Expr> ] },
    0x1C: ExFinalFunction { stack_node: [ PackageIndex ] parameters: [ Vec<Expr> ] },
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
    0x29: ExTextConst { value: [ Box<FScriptText> ] },
    0x2A: ExNoObject {  },
    0x2B: ExTransformConst { value: [ Transform<f64> ] },
    0x2C: ExIntConstByte {  },
    0x2D: ExNoInterface {  },
    0x2E: ExDynamicCast { class_ptr: [ PackageIndex ] target_expression: [ Box<Expr> ] },
    0x2F: ExStructConst { struct_value: [ PackageIndex ] struct_size: [ i32 ] value: [ Vec<Expr> ] },
    0x30: ExEndStructConst {  },
    0x31: ExSetArray { assigning_property: [ Option<Box<Expr>> ] array_inner_prop: [ Option<PackageIndex> ] elements: [ Vec<Expr> ] },
    0x32: ExEndArray {  },
    0x33: ExPropertyConst { property: [ KismetPropertyPointer ] },
    0x34: ExUnicodeStringConst { value: [ String ] },
    0x35: ExInt64Const { value: [ i64 ] },
    0x36: ExUInt64Const {  },
    0x37: ExPrimitiveCast { conversion_type: [ ECastToken ] target: [ Box<Expr> ] },
    0x38: ExCast { /* TODO */ },
    0x39: ExSetSet { set_property: [ Box<Expr> ] elements: [ Vec<Expr> ] },
    0x3A: ExEndSet {  },
    0x3B: ExSetMap { map_property: [ Box<Expr> ] elements: [ Vec<Expr> ] },
    0x3C: ExEndMap {  },
    0x3D: ExSetConst { inner_property: [ KismetPropertyPointer ] elements: [ Vec<Expr> ] },
    0x3E: ExEndSetConst {  },
    0x3F: ExMapConst { key_property: [ KismetPropertyPointer ] value_property: [ KismetPropertyPointer ] elements: [ Vec<Expr> ] },
    0x40: ExEndMapConst {  },
    0x41: ExVector3fConst { /* TODO */ },
    0x42: ExStructMemberContext { struct_member_expression: [ KismetPropertyPointer ] struct_expression: [ Box<Expr> ] },
    0x43: ExLetMulticastDelegate { variable_expression: [ Box<Expr> ] assignment_expression: [ Box<Expr> ] },
    0x44: ExLetDelegate { variable_expression: [ Box<Expr> ] assignment_expression: [ Box<Expr> ] },
    0x45: ExLocalVirtualFunction { virtual_function_name: [ FName ] parameters: [ Vec<Expr> ] },
    0x46: ExLocalFinalFunction { stack_node: [ PackageIndex ] parameters: [ Vec<Expr> ] },
    // 0x47
    0x48: ExLocalOutVariable { variable: [ KismetPropertyPointer ] },
    // 0x49
    0x4A: ExDeprecatedOp4A {  },
    0x4B: ExInstanceDelegate { function_name: [ FName ] },
    0x4C: ExPushExecutionFlow { pushing_address: [ u32 ] },
    0x4D: ExPopExecutionFlow {  },
    0x4E: ExComputedJump { code_offset_expression: [ Box<Expr> ] },
    0x4F: ExPopExecutionFlowIfNot { boolean_expression: [ Box<Expr> ] },
    0x50: ExBreakpoint {  },
    0x51: ExInterfaceContext { interface_value: [ Box<Expr> ] },
    0x52: ExObjToInterfaceCast { class_ptr: [ PackageIndex ] target: [ Box<Expr> ] },
    0x53: ExEndOfScript {  },
    0x54: ExCrossInterfaceCast { class_ptr: [ PackageIndex ] target: [ Box<Expr> ] },
    0x55: ExInterfaceToObjCast { class_ptr: [ PackageIndex ] target: [ Box<Expr> ] },
    // 0x56
    // 0x57
    // 0x58
    // 0x59
    0x5A: ExWireTracepoint {  },
    0x5B: ExSkipOffsetConst {  },
    0x5C: ExAddMulticastDelegate { delegate: [ Box<Expr> ] delegate_to_add: [ Box<Expr> ] },
    0x5D: ExClearMulticastDelegate { delegate_to_clear: [ Box<Expr> ] },
    0x5E: ExTracepoint {  },
    0x5F: ExLetObj { variable_expression: [ Box<Expr> ] assignment_expression: [ Box<Expr> ] },
    0x60: ExLetWeakObjPtr { variable_expression: [ Box<Expr> ] assignment_expression: [ Box<Expr> ] },
    0x61: ExBindDelegate { function_name: [ FName ] delegate: [ Box<Expr> ] object_term: [ Box<Expr> ] },
    0x62: ExRemoveMulticastDelegate { delegate: [ Box<Expr> ] delegate_to_add: [ Box<Expr> ] },
    0x63: ExCallMulticastDelegate { stack_node: [ PackageIndex ] parameters: [ Vec<Expr> ] delegate: [ Box<Expr> ] },
    0x64: ExLetValueOnPersistentFrame { destination_property: [ KismetPropertyPointer ] assignment_expression: [ Box<Expr> ] },
    0x65: ExArrayConst { inner_property: [ KismetPropertyPointer ] elements: [ Vec<Expr> ] },
    0x66: ExEndArrayConst {  },
    0x67: ExSoftObjectConst { value: [ Box<Expr> ] },
    0x68: ExCallMath { stack_node: [ PackageIndex ] parameters: [ Vec<Expr> ] },
    0x69: ExSwitchValue { end_goto_offset: [ u32 ] index_term: [ Box<Expr> ] default_term: [ Box<Expr> ] cases: [ Vec<KismetSwitchCase> ] },
    0x6A: ExInstrumentationEvent { event_type: [ EScriptInstrumentationType ] event_name: [ Option<FName> ] },
    0x6B: ExArrayGetByRef { array_variable: [ Box<Expr> ] array_index: [ Box<Expr> ] },
    0x6C: ExClassSparseDataVariable { variable: [ KismetPropertyPointer ] },
    0x6D: ExFieldPathConst { value: [ Box<Expr> ] },
    // 0x6E
    // 0x6F
    0x70: ExAutoRtfmTransact { /* TODO */ },
    0x71: ExAutoRtfmStopTransact { /* TODO */ },
    0x72: ExAutoRtfmAbortIfNot { /* TODO */ },
);

pub fn read_until<S: Read>(s: &mut S, until: literal::ExprOp) -> Result<Vec<literal::Expr>> {
    let mut exs = vec![];
    loop {
        let next = read(s)?;
        if next.op() == until {
            break;
        } else {
            exs.push(next);
        }
    }
    Ok(exs)
}

pub fn read_all<S: Read>(s: &mut S) -> Result<Vec<literal::Expr>> {
    let mut exs = vec![];
    loop {
        let op = match s.read_u8() {
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            r => r,
        }?;
        exs.push(read_body(s, try_from_opcode(op)?)?);
    }
    Ok(exs)
}

fn try_from_opcode(op: u8) -> Result<literal::ExprOp> {
    literal::ExprOp::from_repr(op).ok_or_else(|| anyhow!("invalid opcode {op}"))
}
fn read_fname<S: Read>(s: &mut S) -> Result<ue::FName> {
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

pub fn read<S: Read>(s: &mut S) -> Result<literal::Expr> {
    let op = s.read_u8()?;

    let op = try_from_opcode(op)?;

    // let span = tracing::error_span!("erm", op = format!("{op:?}")).entered();
    let ex = read_body(s, op);
    // tracing::error!("ex {ex:#?}");
    // drop(span);
    ex
}

pub fn read_body<S: Read>(s: &mut S, op: literal::ExprOp) -> Result<literal::Expr> {
    use literal::{Expr as Ex, ExprOp as Op, *};

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
            return_expression: read(s)?.into(),
        }
        .into(),
        Op::ExJump => ExJump {
            code_offset: s.read_u32::<LE>()?,
        }
        .into(),
        Op::ExJumpIfNot => ExJumpIfNot {
            code_offset: s.read_u32::<LE>()?,
            boolean_expression: read(s)?.into(),
        }
        .into(),
        Op::ExAssert => bail!("todo ExAssert"),
        Op::ExNothing => ExNothing {}.into(),
        Op::ExNothingInt32 => bail!("todo ExNothingInt32"),
        Op::ExLet => ExLet {
            value: KismetPropertyPointer(s.read_u64::<LE>()?),
            variable: read(s)?.into(),
            expression: read(s)?.into(),
        }
        .into(),
        Op::ExBitFieldConst => bail!("todo ExBitFieldConst"),
        Op::ExClassContext => bail!("todo ExClassContext"),
        Op::ExMetaCast => bail!("todo ExMetaCast"),
        Op::ExLetBool => ExLetBool {
            variable_expression: read(s)?.into(),
            assignment_expression: read(s)?.into(),
        }
        .into(),
        Op::ExEndParmValue => ExEndParmValue {}.into(),
        Op::ExEndFunctionParms => ExEndFunctionParms {}.into(),
        Op::ExSelf => ExSelf {}.into(),
        Op::ExSkip => ExSkip {
            code_offset: s.read_u32::<LE>()?,
            skip_expression: read(s)?.into(),
        }
        .into(),
        Op::ExContext => ExContext {
            object_expression: read(s)?.into(),
            offset: s.read_u32::<LE>()?,
            r_value_pointer: KismetPropertyPointer(s.read_u64::<LE>()?),
            context_expression: read(s)?.into(),
        }
        .into(),
        Op::ExContextFailSilent => ExContextFailSilent {
            object_expression: read(s)?.into(),
            offset: s.read_u32::<LE>()?,
            r_value_pointer: KismetPropertyPointer(s.read_u64::<LE>()?),
            context_expression: read(s)?.into(),
        }
        .into(),
        Op::ExVirtualFunction => ExVirtualFunction {
            virtual_function_name: read_fname(s)?,
            parameters: read_until(s, ExprOp::ExEndFunctionParms)?,
        }
        .into(),
        Op::ExFinalFunction => ExFinalFunction {
            stack_node: PackageIndex(s.read_u64::<LE>()?),
            parameters: read_until(s, ExprOp::ExEndFunctionParms)?,
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
        Op::ExRotationConst => bail!("todo ExRotationConst"),
        Op::ExVectorConst => bail!("todo ExVectorConst"),
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
        Op::ExTransformConst => bail!("todo ExTransformConst"),
        Op::ExIntConstByte => bail!("todo ExIntConstByte"),
        Op::ExNoInterface => bail!("todo ExNoInterface"),
        Op::ExDynamicCast => ExDynamicCast {
            class_ptr: PackageIndex(s.read_u64::<LE>()?),
            target_expression: read(s)?.into(),
        }
        .into(),
        Op::ExStructConst => ExStructConst {
            struct_value: PackageIndex(s.read_u64::<LE>()?),
            struct_size: s.read_i32::<LE>()?,
            value: read_until(s, ExprOp::ExEndStructConst)?,
        }
        .into(),
        Op::ExEndStructConst => ExEndStructConst {}.into(),
        Op::ExSetArray => ExSetArray {
            assigning_property: Some(read(s)?.into()),
            array_inner_prop: None, // TODO UE4 change KismetPropertyPointer(s.read_u64::<LE>()?),
            elements: read_until(s, ExprOp::ExEndArray)?,
        }
        .into(),
        Op::ExEndArray => ExEndArray {}.into(),
        Op::ExPropertyConst => bail!("todo ExPropertyConst"),
        Op::ExUnicodeStringConst => bail!("todo ExUnicodeStringConst"),
        Op::ExInt64Const => bail!("todo ExInt64Const"),
        Op::ExUInt64Const => bail!("todo ExUInt64Const"),
        Op::ExPrimitiveCast => bail!("todo ExPrimitiveCast"),
        Op::ExCast => bail!("todo ExCast"),
        Op::ExSetSet => bail!("todo ExSetSet"),
        Op::ExEndSet => ExEndSet {}.into(),
        Op::ExSetMap => bail!("todo ExSetMap"),
        Op::ExEndMap => ExEndMap {}.into(),
        Op::ExSetConst => bail!("todo ExSetConst"),
        Op::ExEndSetConst => ExEndSetConst {}.into(),
        Op::ExMapConst => bail!("todo ExMapConst"),
        Op::ExEndMapConst => ExEndMapConst {}.into(),
        Op::ExVector3fConst => bail!("todo ExVector3fConst"),
        Op::ExStructMemberContext => bail!("todo ExStructMemberContext"),
        Op::ExLetMulticastDelegate => bail!("todo ExLetMulticastDelegate"),
        Op::ExLetDelegate => bail!("todo ExLetDelegate"),
        Op::ExLocalVirtualFunction => ExLocalVirtualFunction {
            virtual_function_name: read_fname(s)?,
            parameters: read_until(s, ExprOp::ExEndFunctionParms)?,
        }
        .into(),
        Op::ExLocalFinalFunction => ExLocalFinalFunction {
            stack_node: PackageIndex(s.read_u64::<LE>()?),
            parameters: read_until(s, ExprOp::ExEndFunctionParms)?,
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
            code_offset_expression: read(s)?.into(),
        }
        .into(),
        Op::ExPopExecutionFlowIfNot => ExPopExecutionFlowIfNot {
            boolean_expression: read(s)?.into(),
        }
        .into(),
        Op::ExBreakpoint => bail!("todo ExBreakpoint"),
        Op::ExInterfaceContext => bail!("todo ExInterfaceContext"),
        Op::ExObjToInterfaceCast => bail!("todo ExObjToInterfaceCast"),
        Op::ExEndOfScript => ExEndOfScript {}.into(),
        Op::ExCrossInterfaceCast => bail!("todo ExCrossInterfaceCast"),
        Op::ExInterfaceToObjCast => bail!("todo ExInterfaceToObjCast"),
        Op::ExWireTracepoint => bail!("todo ExWireTracepoint"),
        Op::ExSkipOffsetConst => bail!("todo ExSkipOffsetConst"),
        Op::ExAddMulticastDelegate => bail!("todo ExAddMulticastDelegate"),
        Op::ExClearMulticastDelegate => bail!("todo ExClearMulticastDelegate"),
        Op::ExTracepoint => bail!("todo ExTracepoint"),
        Op::ExLetObj => ExLetObj {
            variable_expression: read(s)?.into(),
            assignment_expression: read(s)?.into(),
        }
        .into(),
        Op::ExLetWeakObjPtr => bail!("todo ExLetWeakObjPtr"),
        Op::ExBindDelegate => bail!("todo ExBindDelegate"),
        Op::ExRemoveMulticastDelegate => bail!("todo ExRemoveMulticastDelegate"),
        Op::ExCallMulticastDelegate => bail!("todo ExCallMulticastDelegate"),
        Op::ExLetValueOnPersistentFrame => ExLetValueOnPersistentFrame {
            destination_property: KismetPropertyPointer(s.read_u64::<LE>()?),
            assignment_expression: read(s)?.into(),
        }
        .into(),
        Op::ExArrayConst => bail!("todo ExArrayConst"),
        Op::ExEndArrayConst => bail!("todo ExEndArrayConst"),
        Op::ExSoftObjectConst => bail!("todo ExSoftObjectConst"),
        Op::ExCallMath => ExCallMath {
            stack_node: PackageIndex(s.read_u64::<LE>()?),
            parameters: read_until(s, ExprOp::ExEndFunctionParms)?,
        }
        .into(),
        Op::ExSwitchValue => bail!("todo ExSwitchValue"),
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
