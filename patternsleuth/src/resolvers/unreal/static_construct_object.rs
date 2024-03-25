use std::{collections::HashSet, fmt::Debug};

use futures::{future::join_all, join};
use iced_x86::{Code, FlowControl, OpKind, Register};
use itertools::Itertools;
use patternsleuth_scanner::Pattern;

use crate::{
    disassemble::{disassemble, disassemble_single, Control},
    resolvers::{
        bail_out, ensure_one, impl_resolver, impl_resolver_singleton, try_ensure_one, unreal::util,
        Context, Result,
    },
    Image, MemoryAccessorTrait, MemoryTrait,
};

/// class UObject * __cdecl StaticConstructObject_Internal(struct FStaticConstructObjectParameters const &)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct StaticConstructObjectInternal(pub usize);
impl_resolver_singleton!(@all StaticConstructObjectInternal, |ctx| async {
    let any = join!(
        ctx.resolve(StaticConstructObjectInternalPatterns::resolver()),
        ctx.resolve(StaticConstructObjectInternalString::resolver()),
    );

    Ok(Self(*ensure_one(
        [any.0.map(|r| r.0), any.1.map(|r| r.0)]
            .iter()
            .filter_map(|r| r.as_ref().ok()),
    )?))
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct StaticConstructObjectInternalPatterns(pub usize);
impl_resolver_singleton!(@all StaticConstructObjectInternalPatterns, |ctx| async {
    let patterns = [
        "48 89 44 24 28 C7 44 24 20 00 00 00 00 E8 | ?? ?? ?? ?? 48 8B 5C 24 ?? 48 8B ?? 24",
        "E8 | ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? C0 E9 ?? 32 88 ?? ?? ?? ?? 80 E1 01 30 88 ?? ?? ?? ?? 48",
        "E8 | ?? ?? ?? ?? 48 8B D8 48 39 75 30 74 15",
        /*
                03f4df3f c6  44  24       MOV        byte ptr [RSP  + local_88 ],0x0
                         30  00
                03f4df44 0f  57  c0       XORPS      XMM0 ,XMM0
                03f4df47 0f  11  44       MOVUPS     xmmword ptr [RSP  + local_80[0] ],XMM0
                         24  38
                03f4df4c 4c  89  ff       MOV        RDI ,R15
                03f4df4f e8  2c  b6       CALL       StaticConstructObject_Internal                   undefined StaticConstructObject_
                         02  03
                03f4df54 48  89  c3       MOV        RBX ,RAX

         */
        "c6 44 24 30  00 0f 57 c0 0f 11 44 24 38 4c 89 ff e8 | ?? ?? ?? ?? 48 89"
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(Self(try_ensure_one(res.iter().flatten().map(
        |a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) },
    ))?))
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct StaticConstructObjectInternalString(pub usize);

impl_resolver!(@collect StaticConstructObjectInternalString);

impl_resolver!(@ElfImage StaticConstructObjectInternalString, |ctx| async {
    let strings = ctx.scan(util::utf16_pattern("NewObject with empty name can\'t be used to create default")).await;
    let refs = util::scan_xrefs(ctx, &strings).await;
    let target_addr = refs.iter().take(6).map(|&addr| -> Option<Vec<(usize, usize)>> {
        // find e8 call
        let mut callsites = Vec::default();
        // ...06f83ff0 is the real one?
        disassemble(ctx.image(), addr, |inst| {
            let cur = inst.ip() as usize;
            if !(addr..addr + 130).contains(&cur) {
                return Ok(Control::Break);
            }
            if  !inst.is_call_near_indirect()
                && inst.is_call_near() {
                // eprintln!("Found call to @ {:08x}", inst.ip_rel_memory_address());
                callsites.push(inst.ip_rel_memory_address() as usize);
            }
            Ok(Control::Continue)
        }).ok()?;
        // eprintln!("");
        // the seq is always
        // call FStaticConstructObjectParameters::FStaticConstructObjectParameters .0
        // call StaticConstructObjectInternal .1

        let callsites = callsites.iter().zip(callsites.iter().skip(1)).map(|(&x, &y)| (x,y)).collect::<Vec<_>>();
        Some(callsites)
    }).flatten().reduce(|x, y| {
        let x:HashSet<(usize, usize)> = HashSet::from_iter(x);
        let y:HashSet<(usize, usize)> = HashSet::from_iter(y);
        let z = x.intersection(&y);
        z.cloned().collect()
    }).unwrap_or_default();
    Ok(Self(ensure_one(target_addr)?.1))
});

impl_resolver!(@PEImage StaticConstructObjectInternalString, |ctx| async {
    let strings = join_all(
        [
            "UBehaviorTreeManager\0",
            "ULeaderboardFlushCallbackProxy\0",
            "UPlayMontageCallbackProxy\0",
        ]
        .iter()
        .map(|s| {
            ctx.scan(
                Pattern::from_bytes(s.encode_utf16().flat_map(u16::to_le_bytes).collect()).unwrap(),
            )
        }),
    )
    .await;

    let refs_indirect = join_all(
        strings
            .iter()
            .flatten()
            .map(|s| ctx.scan(Pattern::from_bytes(usize::to_le_bytes(*s).into()).unwrap())),
    )
    .await;

    let refs = join_all(
        strings
            .iter()
            .flatten()
            .chain(refs_indirect.iter().flatten())
            .flat_map(|s| {
                [
                    ctx.scan(Pattern::new(format!("48 8d ?? X0x{s:X}")).unwrap()),
                    ctx.scan(Pattern::new(format!("4c 8d ?? X0x{s:X}")).unwrap()),
                    ctx.scan(Pattern::new(format!("48 8d ?? X0x{:X}", s + 2)).unwrap()),
                    ctx.scan(Pattern::new(format!("4c 8d ?? X0x{:X}", s + 2)).unwrap()),
                ]
            }),
    )
    .await;

    let fns = refs
        .into_iter()
        .flatten()
        .map(|r| -> Result<_> { Ok(ctx.image().get_root_function(r)?.map(|f| f.range.start)) })
        .collect::<Result<Vec<_>>>()? // TODO avoid this collect?
        .into_iter()
        .flatten();

    fn check_is_new_object(img: &Image<'_>, f: usize) -> Result<bool> {
        let cmp = "NewObject with empty name can't"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect_vec();

        let check = |f| -> Result<bool> {
            let mut is = false;
            disassemble(img, f, |inst| {
                let cur = inst.ip() as usize;
                if Some(f) != img.get_root_function(cur)?.map(|f| f.range.start) {
                    return Ok(Control::Break);
                }

                if inst.code() == Code::Lea_r64_m
                    && inst.memory_base() == Register::RIP
                    && inst.op0_kind() == OpKind::Register
                    && inst.op1_kind() == OpKind::Memory
                {
                    let ptr = inst.ip_rel_memory_address() as usize;
                    if img
                        .memory
                        .range(ptr..ptr + cmp.len())
                        .map(|data| data == cmp)
                        .unwrap_or(false)
                    {
                        is = true;
                        return Ok(Control::Exit);
                    }
                }

                Ok(Control::Continue)
            })?;
            Ok(is)
        };

        if check(f)? {
            return Ok(true);
        } else {
            // sometimes can be a call deep so check all outgoing calls as well
            for call in util::find_calls(img, f)? {
                let mut f = call.callee;
                // sometimes there's a jmp stub between
                if let Some(inst) = disassemble_single(img, f)? {
                    if inst.flow_control() == FlowControl::UnconditionalBranch {
                        f = inst.near_branch_target() as usize;
                    }
                }

                if check(f)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn check_is_static_construct(img: &Image<'_>, f: usize) -> Result<bool> {
        let mut is = false;
        disassemble(img, f, |inst| {
            let cur = inst.ip() as usize;
            if Some(f) != img.get_root_function(cur)?.map(|f| f.range.start) {
                return Ok(Control::Break);
            }

            if inst.immediate32() == 0x10000080 {
                is = true;
                return Ok(Control::Exit);
            }

            Ok(Control::Continue)
        })?;
        Ok(is)
    }

    let new_object = {
        let mut fns = fns.collect_vec();

        let mut new_object = None;
        for f in &fns {
            if check_is_new_object(ctx.image(), *f)? {
                new_object = Some(*f);
                break;
            }
        }

        'root: for _ in 0..2 {
            #[derive(Clone, Copy)]
            enum CallType {
                Call,
                Jump,
            }

            if new_object.is_none() {
                let calls = join_all(fns.into_iter().flat_map(|f| {
                    [
                        ctx.scan_tagged2(
                            CallType::Call,
                            Pattern::new(format!("e8 X0x{f:x}")).unwrap(),
                        ),
                        ctx.scan_tagged2(
                            CallType::Jump,
                            Pattern::new(format!("e9 X0x{f:x}")).unwrap(),
                        ),
                    ]
                }))
                .await;

                fns = calls
                    .into_iter()
                    .flatten()
                    .map(|(t, r)| -> Result<_> {
                        Ok(match t {
                            CallType::Call => {
                                ctx.image().get_root_function(r)?.map(|f| f.range.start)
                            }
                            CallType::Jump => Some(r),
                        })
                    })
                    .collect::<Result<Vec<_>>>()? // TODO avoid this collect?
                    .into_iter()
                    .flatten()
                    .collect_vec();
            }

            for f in &fns {
                if check_is_new_object(ctx.image(), *f)? {
                    new_object = Some(*f);
                    break 'root;
                }
            }
        }
        new_object
    }
    .context("could not find NewObject<>")?;

    let mut checked = HashSet::new();
    for call in util::find_calls(ctx.image(), new_object)? {
        if !checked.contains(&call.callee) {
            checked.insert(call.callee);

            let mut f = call.callee;
            if let Some(inst) = disassemble_single(ctx.image(), f)? {
                if inst.flow_control() == FlowControl::UnconditionalBranch {
                    f = inst.near_branch_target() as usize;
                }
            }

            if check_is_static_construct(ctx.image(), f)? {
                return Ok(Self(call.callee));
            }
        }
    }
    // try one call deeper
    for f in checked.clone().into_iter() {
        for call in util::find_calls(ctx.image(), f)? {
            if !checked.contains(&call.callee) {
                checked.insert(call.callee);
                if check_is_static_construct(ctx.image(), call.callee)? {
                    return Ok(Self(call.callee));
                }
            }
        }
    }

    bail_out!("could not find StaticConstructObject_Internal call");
});
