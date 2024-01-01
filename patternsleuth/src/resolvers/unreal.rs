use std::{
    collections::{HashMap, HashSet},
    fmt::{Debug, Display},
};

use futures::{future::join_all, join, try_join};
use iced_x86::{Code, Decoder, DecoderOptions, FlowControl, Instruction, OpKind, Register};
use itertools::Itertools;
use patternsleuth_scanner::Pattern;

use crate::{
    disassemble::{disassemble, Control},
    resolvers::{
        bail_out, ensure_one, impl_resolver, impl_resolver_singleton, try_ensure_one, Context,
        Result,
    },
    Addressable, Image, Matchable, MemoryAccessorTrait, MemoryTrait,
};

/// currently seems to be 4.22+
#[derive(PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct EngineVersion {
    pub major: u16,
    pub minor: u16,
}
impl Display for EngineVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}
impl Debug for EngineVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EngineVersion({}.{})", self.major, self.minor)
    }
}
impl_resolver!(EngineVersion, |ctx| async {
    let patterns = [
        "C7 03 | 04 00 ?? 00 66 89 4B 04 48 3B F8 74 ?? 48",
        "C7 05 ?? ?? ?? ?? | 04 00 ?? 00 66 89 ?? ?? ?? ?? ?? C7 05",
        "C7 05 ?? ?? ?? ?? | 04 00 ?? 00 66 89 ?? ?? ?? ?? ?? 89",
        "41 C7 ?? | 04 00 ?? 00 ?? ?? 00 00 00 66 41 89",
        "41 C7 ?? | 04 00 18 00 66 41 89 ?? 04",
        "41 C7 04 24 | 04 00 ?? 00 66 ?? 89 ?? 24",
        "41 C7 04 24 | 04 00 ?? 00 B9 ?? 00 00 00",
        "C7 05 ?? ?? ?? ?? | 04 00 ?? 00 89 05 ?? ?? ?? ?? E8",
        "C7 05 ?? ?? ?? ?? | 04 00 ?? 00 66 89 ?? ?? ?? ?? ?? 89 05",
        "C7 46 20 | 04 00 ?? 00 66 44 89 76 24 44 89 76 28 48 39 C7",
        "C7 03 | 04 00 ?? 00 66 44 89 63 04 C7 43 08 C1 5C 08 80 E8",
        "C7 47 20 | 04 00 ?? 00 66 89 6F 24 C7 47 28 ?? ?? ?? ?? 49",
        "C7 03 | 04 00 ?? 00 66 89 6B 04 89 7B 08 48 83 C3 10",
        "41 C7 06 | 05 00 ?? ?? 48 8B 5C 24 ?? 49 8D 76 ?? 33 ED 41 89 46",
        "C7 06 | 05 00 ?? ?? 48 8B 5C 24 20 4C 8D 76 10 33 ED",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    try_ensure_one(
        res.iter()
            .flatten()
            .map(|a| {
                Ok(EngineVersion {
                    major: ctx.image().memory.u16_le(*a)?,
                    minor: ctx.image().memory.u16_le(a + 2)?,
                })
            })
            .filter_ok(|ver| match ver.major {
                // TODO 4.0 can false positive so ignore it. need to harden if this is to work on 4.0 games
                4 if (1..=27).contains(&ver.minor) => true,
                5 if (0..).contains(&ver.minor) => true,
                _ => false,
            }),
    )
});

/// currently seems to be 4.22+
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct EngineVersionStrings {
    pub branch_name: String,
    pub build_date: String,
    pub build_version: String,
}
impl_resolver!(EngineVersionStrings, |ctx| async {
    let patterns = [
        "48 8D 05 [ ?? ?? ?? ?? ] C3 CC CC CC CC CC CC CC CC 48 8D 05 [ ?? ?? ?? ?? ] C3 CC CC CC CC CC CC CC CC 48 8D 05 [ ?? ?? ?? ?? ] C3 CC CC CC CC CC CC CC CC",
    ];

    let res = join_all(
        patterns
            .iter()
            .map(|p| ctx.scan_tagged((), Pattern::new(p).unwrap())),
    )
    .await;

    let mem = &ctx.image().memory;

    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .into_iter()
    .map(|month| month.encode_utf16().flat_map(u16::to_le_bytes).collect())
    .collect::<HashSet<Vec<u8>>>();

    for (_, pattern, addresses) in res {
        for a in addresses {
            let caps = mem.captures(&pattern, a)?.unwrap();
            let date = caps[1].rip();
            if mem
                .range(date..date + 6)
                .ok()
                .filter(|r| months.contains(&r.to_vec()))
                .is_some()
            {
                return Ok(EngineVersionStrings {
                    branch_name: mem.read_wstring(caps[0].rip())?,
                    build_date: mem.read_wstring(caps[1].rip())?,
                    build_version: mem.read_wstring(caps[2].rip())?,
                });
            }
        }
    }

    bail_out!("not found");
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct GUObjectArray(pub usize);
impl_resolver_singleton!(GUObjectArray, |ctx| async {
    let patterns = [
        "74 ?? 48 8D 0D | ?? ?? ?? ?? C6 05 ?? ?? ?? ?? 01 E8 ?? ?? ?? ?? C6 05 ?? ?? ?? ?? 01",
        "75 ?? 48 ?? ?? 48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 45 33 C9 4C 89 74 24",
        "45 84 c0 48 c7 41 10 00 00 00 00 b8 ff ff ff ff 4c 8d 1d | ?? ?? ?? ?? 89 41 08 4c 8b d1 4c 89 19 0f 45 05 ?? ?? ?? ?? ff c0 89 41 08 3b 05",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(GUObjectArray(try_ensure_one(res.iter().flatten().map(
        |a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) },
    ))?))
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct GMalloc(pub usize);
impl_resolver_singleton!(GMalloc, |ctx| async {
    let any = join!(
        ctx.resolve(GMallocPatterns::resolver()),
        ctx.resolve(GMallocString::resolver()),
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
pub struct GMallocPatterns(pub usize);
impl_resolver_singleton!(GMallocPatterns, |ctx| async {
    let patterns = [
        "48 ?? ?? f0 ?? 0f b1 ?? | ?? ?? ?? ?? 74 ?? ?? 85 ?? 74 ?? ?? 8b", // Purgatory
        "eb 03 ?? 8b ?? 48 8b ?? f0 ?? 0f b1 ?? | ?? ?? ?? ?? 74 ?? ?? 85 ?? 74 ?? ?? 8b", // Purg_notX
        "e8 ?? ?? ?? ?? 48 8b ?? f0 ?? 0f b1 ?? | ?? ?? ?? ?? 74 ?? ?? 85 ?? 74 ?? ?? 8b", // Purg_withX
        "48 85 C9 74 2E 53 48 83 EC 20 48 8B D9 48 8B ?? | ?? ?? ?? ?? 48 85 C9", // A 
        "75 ?? E8 ?? ?? ?? ?? 48 8b 0d | ?? ?? ?? ?? 48 8b ?? 48 ?? ?? ff 50 ?? 48 83 c4 ?? ?? c3", // bnew1
        "48 85 C9 74 ?? 4C 8B 05 | ?? ?? ?? ?? 4D 85 C0 0F 84", // altshort
        "48 ?? ?? ?? ?? ?? ?? e8 ?? ?? ?? ?? 48 8b 0d | ?? ?? ?? ?? 48 8b 01 ff 50 ?? 84 c0 75 ?? b9 38 00 00 00", // gcreatemallocshort
        "84 C0 75 ?? B9 38 00 00 00 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 85 c0 74 ?? 48 8b 0d | ?? ?? ?? ?? 48 8d 05 ?? ?? ?? ?? 48 89", // gcreatemallocmiddle
        "ff 15 ?? ?? ?? ?? 48 8b 5c 24 ?? 48 89 3d | ?? ?? ?? ?? 48 8b 7c 24 20 48 83 c4 28 c3", // gcreatemallocend
        "48 89 ?? f0 ?? 0f b1 ?? | ?? ?? ?? ?? 48 39 ?? 74 ?? 48 8b 1d", // clang1
        "48 89 ?? f0 ?? 0f b1 ?? | ?? ?? ?? ?? 48 39 ?? 75 ?? 48 83 c4", // clang2
        "48 89 5C 24 08 57 48 83 EC 20 48 8B F9 ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? FF 48 8B 0D | ?? ?? ?? ?? ?? 8B ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 ?? ?? ?? ?? 48",
        "48 89 5C 24 08 57 48 83 EC ?? 48 83 3D ?? ?? ?? ?? 00 8B DA 48 8B F9 75 07 E8 ?? ?? ?? FF EB 07 33 C9 E8 ?? ?? ?? FF 48 8B 0D | ?? ?? ?? ?? 44 8B C3 48 8B D7 48 8B 01 FF 50 10 80 3D ?? ?? ?? ?? 00 48 8B D8 75 ?? 48 8B 05 ?? ?? ?? ?? 48 85 C0 75 05 E8 ?? ?? ?? FF ?? 44 24 ?? 01",
        "48 89 5C 24 08 57 48 83 EC 20 48 8B F9 8B DA 48 8B 0D | ?? ?? ?? ?? 48 85 C9 75 2E 65 48 8B 04 25 58 00 00 00 44 8B 05 ?? ?? ?? ?? BA 18 00 00 00 4E 8B 04 C0 42 8B 04 02 39 05 ?? ?? ?? ?? 7E 09 EB 1E 48 8B 0D ?? ?? ?? ?? 48 8B 01 44 8B C3 48 8B D7 48 8B 5C 24 30 48 83 C4 20 5F 48 FF 60 10 48 8D 0D",
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
pub struct GMallocString(pub usize);
impl_resolver_singleton!(GMallocString, |ctx| async {
    let strings = ctx
        .scan(
            Pattern::from_bytes(
                "DeleteFile %s\0"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            )
            .unwrap(),
        )
        .await;

    let refs_indirect = join_all(
        strings
            .iter()
            .map(|s| ctx.scan(Pattern::from_bytes(usize::to_le_bytes(*s).into()).unwrap())),
    )
    .await;

    let refs = join_all(
        strings
            .iter()
            .chain(refs_indirect.iter().flatten())
            .flat_map(|s| {
                [
                    ctx.scan(Pattern::new(format!("48 8d ?? X0x{s:X}")).unwrap()),
                    ctx.scan(Pattern::new(format!("4c 8d ?? X0x{s:X}")).unwrap()),
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

    fn find_global(
        img: &Image<'_>,
        f: usize,
        depth: usize,
        searched: &mut HashSet<usize>,
    ) -> Result<Option<usize>> {
        searched.insert(f);

        //println!("searching {f:x?}");

        let mut mov_rcx = None;
        let mut gmalloc = None;
        let mut calls = vec![];

        disassemble(img, f, |inst| {
            let cur = inst.ip() as usize;
            if !(f..f + 1000).contains(&cur)
                && Some(f) != img.get_root_function(cur)?.map(|f| f.range.start)
            {
                //println!("bailing at {:x}", inst.ip());
                return Ok(Control::Break);
            }

            if inst.code() == Code::Cmp_rm64_imm8
                && inst.memory_base() == Register::RIP
                && inst.op0_kind() == OpKind::Memory
                && inst.op1_kind() == OpKind::Immediate8to64
                && inst.immediate8() == 0
            {
                gmalloc = Some(inst.ip_rel_memory_address() as usize);
                return Ok(Control::Exit);
            }

            if inst.code() == Code::Test_rm64_r64
                && inst.op0_register() == Register::RCX
                && inst.op1_register() == Register::RCX
            {
                if let Some(mov_rcx) = mov_rcx {
                    gmalloc = Some(mov_rcx);
                    return Ok(Control::Exit);
                }
            }

            if inst.code() == Code::Mov_r64_rm64
                && inst.memory_base() == Register::RIP
                && inst.op0_register() == Register::RCX
            {
                /*
                println!(
                    "{depth} {:x} {:x} {:x?}",
                    inst.ip(),
                    inst.ip_rel_memory_address(),
                    inst
                );
                */
                mov_rcx = Some(inst.ip_rel_memory_address() as usize);
            } else {
                mov_rcx = None;
            }

            match inst.flow_control() {
                FlowControl::Call
                | FlowControl::ConditionalBranch
                | FlowControl::UnconditionalBranch => {
                    let call = inst.near_branch_target() as usize;
                    //println!("{:x} {:x}", inst.ip(), call);
                    if Some(f) != img.get_root_function(call)?.map(|f| f.range.start) {
                        calls.push(call);
                    }
                }
                _ => {}
            }

            Ok(Control::Continue)
        })?;

        if gmalloc.is_some() {
            Ok(gmalloc)
        } else {
            if depth > 0 {
                for call in calls.iter().rev() {
                    if !searched.contains(call) {
                        let res = find_global(img, *call, depth - 1, searched)?;
                        if res.is_some() {
                            return Ok(res);
                        }
                    }
                }
            }
            Ok(None)
        }
    }

    let fns = fns
        .map(|f| find_global(ctx.image(), f, 3, &mut Default::default()))
        .flatten_ok();

    Ok(Self(try_ensure_one(fns)?))
});

/// public: static class FUObjectHashTables & __cdecl FUObjectHashTables::Get(void)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FUObjectHashTablesGet(pub usize);
impl_resolver_singleton!(FUObjectHashTablesGet, |ctx| async {
    let patterns = [
        "48 89 5C 24 08 48 89 6C 24 10 48 89 74 24 18 57 48 83 EC 40 41 0F B6 F9 49 8B D8 48 8B F2 48 8B E9 E8 | ?? ?? ?? ?? 44 8B 84 24 80 00 00 00 4C 8B CB 44 ?? ?? 24 ?? 48 8B D5 44 ?? 44 24 ?? ?? ?? ?? ?? ?? 44 ?? ?? 44 ?? ?? ?? ?? ?? 44 ?? ?? 24 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48",
        "48 89 5C 24 08 48 89 74 24 10 4C 89 44 24 18 57 48 83 EC 40 41 0F B6 D9 48 8B FA 48 8B F1 E8 | ?? ?? ?? ?? 44 8B 84 24 80 00 00 00 48 8B D6 ?? 8B ?? 24 ?? 48 8B C8 ?? ?? ?? 24 ?? ?? ?? ?? ?? ?? 44 89 44 24 ?? 44 0F B6 44 24 70 44 ?? ?? 24 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 8B",
        "48 89 5C 24 08 48 89 6C 24 10 48 89 74 24 18 57 48 83 EC 40 41 0F B6 F9 49 8B D8 48 8B F2 48 8B E9 E8 | ?? ?? ?? ?? 44 8B 44 24 78 4C 8B CB 44 89 44 24 38 48 8B D5 44 8B 44 24 70 48 8B C8 44 89 44 24 30 4C 8B C6 C6 44 24 28 00 40 88 7C 24 20 E8 ?? ?? ?? ?? 48 8B 5C 24 50 48 8B 6C 24 58 48 8B 74 24 60",
        "e8 | ?? ?? ?? ?? 45 33 ff 48 8b f0 33 c0 f0 44 0f b1 3d",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(Self(try_ensure_one(res.iter().flatten().map(
        |a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) },
    ))?))
});

/// public: void __cdecl FUObjectArray::AllocateUObjectIndex(class UObjectBase *, bool)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FUObjectArrayAllocateUObjectIndex(pub usize);
impl_resolver_singleton!(FUObjectArrayAllocateUObjectIndex, |ctx| async {
    let strings = ctx
        .scan(
            Pattern::from_bytes(
                "Unable to add more objects to disregard for GC pool (Max: %d)\x00"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            )
            .unwrap(),
        )
        .await;

    let refs_indirect = join_all(
        strings
            .iter()
            .map(|s| ctx.scan(Pattern::from_bytes(usize::to_le_bytes(*s).into()).unwrap())),
    )
    .await;

    let refs = join_all(
        strings
            .iter()
            .chain(refs_indirect.iter().flatten())
            .flat_map(|s| {
                [
                    ctx.scan(Pattern::new(format!("48 8d ?? X0x{s:X}")).unwrap()),
                    ctx.scan(Pattern::new(format!("4c 8d ?? X0x{s:X}")).unwrap()),
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

    Ok(FUObjectArrayAllocateUObjectIndex(ensure_one(fns)?))
});

/// public: void __cdecl FUObjectArray::FreeUObjectIndex(class UObjectBase *)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FUObjectArrayFreeUObjectIndex(pub usize);
impl_resolver_singleton!(FUObjectArrayFreeUObjectIndex, |ctx| async {
    let refs_future = async {
        let strings = join_all([
            ctx.scan(
                Pattern::from_bytes("Removing object (0x%016llx) at index %d but the index points to a different object (0x%016llx)!".encode_utf16().flat_map(u16::to_le_bytes).collect()).unwrap(),
            ),
            ctx.scan(
                Pattern::from_bytes("Unexpected concurency while adding new object".encode_utf16().flat_map(u16::to_le_bytes).collect()).unwrap()
            ),
        ])
        .await;

        let refs_indirect = join_all(
            strings
                .iter()
                .flatten()
                .map(|s| ctx.scan(Pattern::from_bytes(usize::to_le_bytes(*s).into()).unwrap())),
        )
        .await;

        Ok(join_all(
            strings
                .iter()
                .flatten()
                .chain(refs_indirect.iter().flatten())
                .flat_map(|s| {
                    [
                        ctx.scan(Pattern::new(format!("48 8d ?? X0x{s:X}")).unwrap()),
                        ctx.scan(Pattern::new(format!("4c 8d ?? X0x{s:X}")).unwrap()),
                    ]
                }),
        )
        .await)
    };

    // same string is present in both functions so resolve the other so we can filter it out
    let (allocate_uobject, refs) = try_join!(
        ctx.resolve(FUObjectArrayAllocateUObjectIndex::resolver()),
        refs_future,
    )?;

    let fns = refs
        .into_iter()
        .flatten()
        .map(|r| -> Result<_> { Ok(ctx.image().get_root_function(r)?.map(|f| f.range.start)) })
        .collect::<Result<Vec<_>>>()? // TODO avoid this collect?
        .into_iter()
        .flatten()
        .filter(|f| *f != allocate_uobject.0);

    Ok(FUObjectArrayFreeUObjectIndex(ensure_one(fns)?))
});

/// void __cdecl UObjectBaseShutdown(void)
/// could be used to determine object listener offsets, but only for recent UE versions
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UObjectBaseShutdown(pub usize);
impl_resolver_singleton!(UObjectBaseShutdown, |ctx| async {
    let strings = ctx
        .scan(
            Pattern::from_bytes(
                "All UObject delete listeners should be unregistered when shutting down the UObject array\x00"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            )
            .unwrap(),
        )
        .await;

    let refs = join_all(strings.iter().flat_map(|s| {
        [
            ctx.scan(Pattern::new(format!("48 8d ?? X0x{s:X}")).unwrap()),
            ctx.scan(Pattern::new(format!("4c 8d ?? X0x{s:X}")).unwrap()),
        ]
    }))
    .await;

    let fns = refs
        .into_iter()
        .flatten()
        .map(|r| -> Result<_> { Ok(ctx.image().get_root_function(r)?.map(|f| f.range.start)) })
        .collect::<Result<Vec<_>>>()? // TODO avoid this collect?
        .into_iter()
        .flatten();

    Ok(UObjectBaseShutdown(ensure_one(fns)?))
});

/// public: __cdecl FName::FName(wchar_t const *, enum EFindName)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNameCtorWchar(pub usize);
impl_resolver_singleton!(FNameCtorWchar, |ctx| async {
    let strings = ["TGPUSkinVertexFactoryUnlimited\0", "MovementComponent0\0"];
    let strings = join_all(strings.iter().map(|s| {
        ctx.scan(
            Pattern::from_bytes(s.encode_utf16().flat_map(u16::to_le_bytes).collect()).unwrap(),
        )
    }))
    .await;

    let refs = join_all(strings.iter().flatten().flat_map(|s| {
        [
            format!("48 8d 15 X0x{s:x} 48 8d 0d ?? ?? ?? ?? e8 | ?? ?? ?? ??"),
            format!("41 b8 01 00 00 00 48 8d 15 X0x{s:x} 48 8d 0d ?? ?? ?? ?? e9 | ?? ?? ?? ??"),
        ]
        .into_iter()
        .map(|p| ctx.scan(Pattern::new(p).unwrap()))
    }))
    .await;

    Ok(FNameCtorWchar(try_ensure_one(
        refs.iter()
            .flatten()
            .map(|a| Ok(ctx.image().memory.rip4(*a)?)),
    )?))
});

/// Can be either of the following:
/// `public: class FString __cdecl FName::ToString(void) const`
/// `public: void __cdecl FName::ToString(class FString &) const`
///
/// They take the same arguments and either can be used as long as the return value isn't used.
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNameToString(pub usize);
impl_resolver_singleton!(FNameToString, |ctx| async {
    let string = async {
        // Locates either variant by searching for a string ref and finding the first function
        // call directly above it. Which variant depends on how much inlining has occured
        let s = Pattern::from_bytes(
            "  DrivingBone: %s\nDrivenParamet"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect(),
        )
        .unwrap();
        let strings = ctx.scan(s).await;

        let refs = join_all(
            strings
                .iter()
                .map(|s| ctx.scan(Pattern::new(format!("48 8d 15 X0x{s:x}")).unwrap())),
        )
        .await;

        let fn_gather_debug_data = ensure_one(
            refs.into_iter()
                .flatten()
                .map(|r| -> Result<_> {
                    Ok(ctx.image().get_root_function(r)?.map(|f| f.range.start..r))
                })
                .collect::<Result<Vec<_>>>()? // TODO avoid this collect?
                .into_iter()
                .flatten(),
        )?;

        let bytes = ctx.image().memory.range(fn_gather_debug_data.clone())?;

        let mut decoder = Decoder::with_ip(
            64,
            bytes,
            fn_gather_debug_data.start as u64,
            DecoderOptions::NONE,
        );

        let addr = decoder
            .iter()
            .filter_map(|i| {
                (i.code() == Code::Call_rel32_64).then_some(i.memory_displacement64() as usize)
            })
            .last()
            .context("did not find CALL instruction")?;

        let res: Result<usize> = Ok(addr);

        res
    };

    let any = join!(
        ctx.resolve(FNameToStringFString::resolver()),
        ctx.resolve(FNameToStringVoid::resolver()),
        string,
    );

    Ok(FNameToString(
        any.0.map(|r| r.0).or(any.1.map(|r| r.0)).or(any.2)?,
    ))
});

/// public: class FString __cdecl FName::ToString(void) const
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNameToStringVoid(pub usize);
impl_resolver_singleton!(FNameToStringVoid, |ctx| async {
    let patterns = [
        "E8 | ?? ?? ?? ?? ?? 01 00 00 00 ?? 39 ?? 48 0F 8E",
        "E8 | ?? ?? ?? ?? BD 01 00 00 00 41 39 6E ?? 0F 8E",
        "E8 | ?? ?? ?? ?? 48 8B 4C 24 ?? 8B FD 48 85 C9",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(FNameToStringVoid(try_ensure_one(
        res.iter()
            .flatten()
            .map(|a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) }),
    )?))
});

/// public: void __cdecl FName::ToString(class FString &) const
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNameToStringFString(pub usize);
impl_resolver!(FNameToStringFString, |ctx| async {
    let patterns =
        ["48 8b 48 ?? 48 89 4c 24 ?? 48 8d 4c 24 ?? e8 | ?? ?? ?? ?? 83 7c 24 ?? 00 48 8d"];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(FNameToStringFString(try_ensure_one(
        res.iter()
            .flatten()
            .map(|a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) }),
    )?))
});

/// private: __cdecl FText::FText(class FString &&)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FTextFString(pub usize);
impl_resolver_singleton!(FTextFString, |ctx| async {
    enum Directness {
        Direct,
        Indirect,
    }
    let patterns = [
        (Directness::Indirect, "40 53 48 83 ec ?? 48 8b d9 e8 | ?? ?? ?? ?? 83 4b ?? 12 48 8b c3 48 83 ?? ?? 5b c3"),
        (Directness::Indirect, "eb 12 48 8d ?? 24 ?? e8 | ?? ?? ?? ?? ?? 02 00 00 00 48 8b 10"),
        (Directness::Direct, "48 89 5C 24 10 48 89 6C 24 18 56 57 41 54 41 56 41 57 48 83 EC 40 45 33 E4 48 8B F1 41 8B DC 4C 8B F2 89 5C 24 70 41 8D 4C 24 70 E8 ?? ?? ?? FF 48 8B F8 48 85 C0 0F 84 ?? 00 00 00 49 63 5E 08 ?? 8B ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 8B ?? EB 2E 45 33 C0 48 8D 4C 24 20 8B D3 E8"),
        (Directness::Direct, "48 89 5C 24 10 48 89 6C 24 18 57 48 83 EC 50 33 ED 48 8D 05 ?? ?? ?? 03 48 8B F9 48 89 6C 24 38 48 8B DA 48 89 6C 24 48 48 89 44 24 30 8D 4D 60 48 89 44 24 40 E8 ?? ?? ?? FF 4C 8B C0 48 85 C0 74 65 0F 10 44 24 30 C7 40 08 01 00 00 00 0F 10 4C 24 40 C7 40 0C 01 00 00 00 48 8D 05 ?? ?? ?? 03 49 89 00"),
        (Directness::Direct, "48 89 5C 24 10 48 89 6C 24 18 56 57 41 54 41 56 41 57 48 83 EC 50 45 33 E4 48 8B F9 41 8B DC 4C 8B F2 89 9C 24 80 00 00 00 41 8D 4C 24 70 E8 ?? ?? ?? ?? 48 8B F0 48 85 C0 0F 84 98 00 00 00 49 63 5E 08 41 8B EC 4D 8B 3E 4C 89 64 24 20 89 5C 24 28 85 DB 75 05 45 8B FC EB 2E 45 33 C0 48 8D 4C 24 20 8B"),
        (Directness::Direct, "48 89 5C 24 ?? 48 89 6C 24 ?? 48 89 74 24 ?? 48 89 7C 24 ?? 41 54 41 56 41 57 48 83 EC 40 4C 8B F1 48 8B F2"),
        (Directness::Direct, "48 89 5C 24 ?? 48 89 6C 24 ?? 56 57 41 54 41 56 41 57 48 83 EC 40 45 33 E4 48 8B F1"),
        (Directness::Direct, "40 53 56 48 83 EC 48 33 DB 48 89 6C 24 68 48 8B F1 48 89 7C 24 70 4C 89 74 24 78 4C 8B F2 89 5C 24 60 8D 4B 70 E8 ?? ?? ?? FF 48 8B F8 48 85 C0 0F 84 9E 00 00 00 49 63 5E 08 33 ED 4C 89 7C 24 40 4D 8B 3E 48 89 6C 24 20 89 5C 24 28 85 DB 75 05 45 33 FF EB 2E 45 33 C0 48 8D 4C 24 20 8B D3 E8"),
        (Directness::Direct, "41 57 41 56 41 54 56 57 55 53 48 83 EC 40 48 89 D7 48 89 CE 48 8B 0D"),
    ];

    let res = join_all(
        patterns
            .iter()
            .map(|(tag, p)| ctx.scan_tagged(tag, Pattern::new(p).unwrap())),
    )
    .await;

    let mem = &ctx.image().memory;

    Ok(FTextFString(try_ensure_one(res.iter().flat_map(
        |(directness, _, a)| match directness {
            Directness::Direct => itertools::Either::Right(a.iter().map(|a| Ok(*a))),
            Directness::Indirect => itertools::Either::Left(a.iter().map(|a| Ok(mem.rip4(*a)?))),
        },
    ))?))
});

/// class UObject * __cdecl StaticConstructObject_Internal(struct FStaticConstructObjectParameters const &)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct StaticConstructObjectInternal(pub usize);
impl_resolver_singleton!(StaticConstructObjectInternal, |ctx| async {
    let patterns = [
        "48 89 44 24 28 C7 44 24 20 00 00 00 00 E8 | ?? ?? ?? ?? 48 8B 5C 24 ?? 48 8B ?? 24",
        "E8 | ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? C0 E9 ?? 32 88 ?? ?? ?? ?? 80 E1 01 30 88 ?? ?? ?? ?? 48",
        "E8 | ?? ?? ?? ?? 48 8B D8 48 39 75 30 74 15",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(StaticConstructObjectInternal(try_ensure_one(
        res.iter()
            .flatten()
            .map(|a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) }),
    )?))
});

/// public: void __cdecl UObject::SkipFunction(struct FFrame &, void *const, class UFunction *)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UObjectSkipFunction(pub usize);
impl_resolver!(UObjectSkipFunction, |ctx| async {
    let patterns = [
        "40 55 41 54 41 55 41 56 41 57 48 83 EC 30 48 8D 6C 24 20 48 89 5D 40 48 89 75 48 48 89 7D 50 48 8B 05 ?? ?? ?? ?? 48 33 C5 48 89 45 00 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 4D 8B ?? ?? 8B ?? 85 ?? 75 05 41 8B FC EB ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 ?? E0",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(UObjectSkipFunction(ensure_one(res.into_iter().flatten())?))
});

// GNatives
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct GNatives(pub usize);
impl_resolver!(GNatives, |ctx| async {
    let skip_function = ctx.resolve(UObjectSkipFunction::resolver()).await?;
    let bytes = ctx.image().memory.range_from(skip_function.0..)?;

    let mut decoder = Decoder::with_ip(
        64,
        &bytes[0..bytes.len().min(500)],
        skip_function.0 as u64,
        DecoderOptions::NONE,
    );

    // TODO recursive decode candidate
    let mut instruction = Instruction::default();
    while decoder.can_decode() {
        decoder.decode_out(&mut instruction);
        if instruction.code() == Code::Lea_r64_m && instruction.memory_base() == Register::RIP {
            return Ok(GNatives(instruction.memory_displacement64() as usize));
        }
    }

    bail_out!("failed to not find LEA instruction");
});

/// public: void __cdecl FFrame::Step(class UObject *, void *const)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FFrameStep(pub usize);
impl_resolver_singleton!(FFrameStep, |ctx| async {
    let patterns = [
        "48 8B 41 20 4C 8B D2 48 8B D1 44 0F B6 08 48 FF C0 48 89 41 20 41 8B C1 4C 8D 0D ?? ?? ?? ?? 49 8B CA 49 FF 24 C1",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(FFrameStep(ensure_one(res.into_iter().flatten())?))
});

/// public: void __cdecl FFrame::StepExplicitProperty(void *const, class FProperty *)
/// public: void __cdecl FFrame::StepExplicitProperty(void *const, class UProperty *)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FFrameStepExplicitProperty(pub usize);
impl_resolver_singleton!(FFrameStepExplicitProperty, |ctx| async {
    let patterns = [
         "41 8B 40 40 4D 8B C8 4C 8B D1 48 0F BA E0 08 73 ?? 48 8B ?? ?? ?? ?? 00 ?? ?? ?? ?? ?? ?? ?? 00 48 8B 40 10 4C 39 08 75 F7 48 8B 48 08 49 89 4A 38 ?? ?? ?? 40 ?? ?? ?? ?? ?? 4C ?? 41 ?? 49",
         "48 89 5C 24 ?? 48 89 ?? 24 ?? 57 48 83 EC 20 41 8B 40 40 49 8B D8 48 8B ?? 48 8B F9 48 0F BA E0 08 73 ?? 48 8B ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 8B 40 10 48 39 18 75 F7 48 8B ?? 08 48 89 ?? 38 48",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(FFrameStepExplicitProperty(ensure_one(
        res.into_iter().flatten(),
    )?))
});

/// public: static void __cdecl UKismetStringLibrary::execLen(class UObject *, struct FFrame &, void *const)
/// public: void __cdecl UKismetStringLibrary::execLen(struct FFrame &, void *const)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FFrameStepViaExec {
    pub step: usize,
    pub step_explicit_property: usize,
}
impl_resolver!(FFrameStepViaExec, |ctx| async {
    let patterns = [
        "48 89 5C 24 08 48 89 74 24 10 57 48 83 EC ?? 33 FF 33 C0 49 8B F0 48 8B DA 48 8B CA 48 89 7C 24 20 48 89 7C 24 28 48 39 42 20 74 10 48 8B 52 18 4C 8D 44 24 20 E8 [ ?? ?? ?? ?? ] EB 1C 4C 8B 82 80 00 00 00 49 8B 40 ?? 48 89 82 80 00 00 00 48 8D 54 24 20 E8 [ ?? ?? ?? ?? ] 48 8B 43 20 48 8D 4C 24 20 48 85 C0",
        "48 89 5C 24 08 48 89 74 24 10 57 48 83 EC ?? 33 FF 49 8B F0 48 8B DA 48 89 7C 24 20 48 ?? ?? ?? ?? ?? ?? ?? 48 39 7A 20 74 10 48 8B 52 18 4C 8D 44 24 20 E8 [ ?? ?? ?? ?? ] EB 1C 4C 8B 82 80 00 00 00 49 8B 40 ?? 48 89 82 80 00 00 00 48 8D 54 24 20 E8 [ ?? ?? ?? ?? ] 48 8B 43 20 48 8D 4C 24 20 48 85 C0 40 0F",
        "48 89 5C 24 08 48 89 74 24 10 57 48 83 EC 30 33 ?? 49 8B F0 48 89 ?? 24 20 48 8B ?? 48 89 ?? 24 28 E8 [ ?? ?? ?? ?? ] 48 8B ?? 48 39 ?? 20 74 10 48 8B ?? 18 4C 8D 44 24 20 E8 ?? ?? ?? ?? EB 1C 4C 8B ?? ?? 00 00 00 48 8D 54 24 20 49 8B 40 20 48 89 ?? ?? 00 00 00 E8 [ ?? ?? ?? ?? ] 48 8B ?? 20 48",
    ];

    let res = join_all(
        patterns
            .iter()
            .map(|p| ctx.scan_tagged((), Pattern::new(p).unwrap())),
    )
    .await;

    ensure_one(
        res.into_iter()
            .flat_map(|(_, pattern, addresses)| -> Result<_> {
                try_ensure_one(addresses.iter().map(|a| -> Result<_> {
                    let caps = ctx.image().memory.captures(&pattern, *a)?.unwrap();
                    Ok(FFrameStepViaExec {
                        step: caps[0].rip(),
                        step_explicit_property: caps[1].rip(),
                    })
                }))
            }),
    )
});

/// public: static bool __cdecl UGameplayStatics::SaveGameToMemory(class USaveGame *, class TArray<unsigned char, class TSizedDefaultAllocator<32> > &)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UGameplayStaticsSaveGameToMemory(pub usize);
impl_resolver_singleton!(UGameplayStaticsSaveGameToMemory, |ctx| async {
    let patterns = [
        "48 89 5C 24 10 48 89 7C 24 18 55 48 8D AC 24 ?? FF FF FF 48 81 EC ?? 01 00 00 48 8B DA 48 8B F9 48 85 C9 0F 84 ?? 02 00 00 0F 57 C0 48 C7 85 ?? 00 00 00 00 00 00 00",
        "48 89 5C 24 10 48 89 7C 24 18 55 48 8D AC 24 20 FF FF FF 48 81 EC E0 01 00 00 48 8B DA 48 8B F9 48 85 C9 0F 84 ?? ?? 00 00 0F 57 C0 48 C7 85 F0 00 00 00 00 00 00 00 33 C0 48 8D 4D 80 0F 11 45 80 48 89 45 10 0F 11 45 90 0F 11 45 A0 0F 11 45 B0 0F 11 45 C0 0F 11 45 D0 0F 11 45 E0 0F 11 45 F0 0F 11 45",
        "48 89 5C 24 10 48 89 7C 24 18 55 48 8D AC 24 ?? FF FF FF 48 81 EC ?? 01 00 00 48 8B DA 48 8B F9 48 85 C9 0F 84 71 01 00 00 33 D2 48 C7 85 ?? 00 00 00 00 00 00 00 41 B8 ?? 00 00 00 48 8D 4D 80 E8 ?? ?? ?? ?? 48 8D 4D 80 E8 ?? ?? ?? ?? 48 8D 05 ?? ?? ?? ?? 48 C7 45 ?? 00 00 00 00 48 89 45 80 48 8D 4D",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(UGameplayStaticsSaveGameToMemory(ensure_one(
        res.into_iter().flatten(),
    )?))
});

/// public: static bool __cdecl UGameplayStatics::SaveGameToSlot(class USaveGame *, class FString const &, int)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UGameplayStaticsSaveGameToSlot(pub usize);
impl_resolver_singleton!(UGameplayStaticsSaveGameToSlot, |ctx| async {
    let patterns = [
        "48 89 5C 24 08 48 89 74 24 10 57 48 83 EC 40 ?? 8B ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? E8 ?? ?? FF FF 84 C0 74 58 E8 ?? ?? ?? ?? 48 8B ?? 48 8B ?? FF 52 ?? 4C 8B D0 48 85 C0 74 42 39 74 24 38 7E 3C 8B 53 08 ?? ?? ?? ?? ?? 0F 44 CE 85 C9 7E 2D",
        "48 89 5C 24 08 48 89 74 24 10 48 89 7C 24 18 55 41 56 41 57 48 8D AC 24 ?? FF FF FF 48 81 EC ?? ?? 00 00 48 8B F1 45 33 FF 48 8B 0D ?? ?? ?? ?? 45 8B F0 48 8B ?? 48 85 C9 75 27 41 8D 4F 08 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 49 8B CF 48 89 0D ?? ?? ?? ?? 48 8B",
        "40 55 56 57 41 54 41 55 41 ?? 48 8D AC 24 ?? ?? FF FF 48 81 EC ?? ?? 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 85 ?? ?? 00 00 4C 8B ?? 45 33 ED 48 8B 0D ?? ?? ?? ?? 45 8B E0 48 8B FA 48 85 C9 75 27 41 8D 4D 08 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 49 8B CD 48 89",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(UGameplayStaticsSaveGameToSlot(ensure_one(
        res.into_iter().flatten(),
    )?))
});

/// public: static class USaveGame * __cdecl UGameplayStatics::LoadGameFromMemory(class TArray<unsigned char, class TSizedDefaultAllocator<32> > const &)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UGameplayStaticsLoadGameFromMemory(pub usize);
impl_resolver_singleton!(UGameplayStaticsLoadGameFromMemory, |ctx| async {
    let patterns = [
        "48 89 5C 24 20 55 48 8D AC 24 10 FF FF FF 48 81 EC F0 01 00 00 83 79 08 00 48 8B D9 75 13 33 C0 48 8B 9C 24 18 02 00 00 48 81 C4 F0 01 00 00 5D C3 0F 57 C0 48 89 ?? 24 ?? 02 00 00 48 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11",
        "40 55 48 8D AC 24 00 FF FF FF 48 81 EC 00 02 00 00 83 79 08 00 75 0B 33 C0 48 81 C4 00 02 00 00 5D C3 48 8B D1 48 89 9C 24 20 02 00 00 48 89 BC 24 28 02 00 00 48 8D 4D A0 41 B0 01 33 FF E8 ?? ?? ?? ?? 0F 57 C0 89 7C 24 50 48 8D 4C 24 50 66 0F 7F 44 24 40 66 89 7C 24 54 89 7C 24 58 48 89 7C 24 60 48",
        "40 55 48 8D AC 24 ?? ?? FF FF 48 81 EC ?? 02 00 00 83 79 08 00 75 0B 33 C0 48 81 C4 ?? 02 00 00 5D C3 48 89 9C 24 ?? 02 00 00 48 8B D1 48 89 B4 24 ?? 02 00 00 48 8D 4D A0 48 89 BC 24 ?? 02 00 00 41 B0 01 33 FF E8 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 24 ?? ?? 89 7C 24",
        "48 89 5C 24 20 55 48 8D AC 24 ?? FF FF FF 48 81 EC ?? ?? 00 00 83 79 08 00 48 8B D9 75 13 33 C0 48 8B 9C 24 ?? 02 00 00 48 81 C4 ?? ?? 00 00 5D C3 48 89 B4 24 ?? 02 00 00 48 8D 4D A0 48 89 BC 24 ?? 02 00 00 33",
        "40 55 53 48 8D AC 24 08 FF FF FF 48 81 EC F8 01 00 00 83 79 08 00 48 8B D9 75 0C 33 C0 48 81 C4 F8 01 00 00 5B 5D C3 48 89 B4 24 18 02 00 00 48 8D 4D A0 48 89 BC 24 20 02 00 00 33 FF 4C 89 B4 24 F0 01 00 00 E8 ?? ?? ?? ?? 48 8D 05 ?? ?? ?? 00 48 89 7D 30 48 89 45 A0 48 8D 4D A0 48 B8 FF FF FF FF FF",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(UGameplayStaticsLoadGameFromMemory(ensure_one(
        res.into_iter().flatten(),
    )?))
});

/// public: static class USaveGame * __cdecl UGameplayStatics::LoadGameFromSlot(class FString const &, int)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UGameplayStaticsLoadGameFromSlot(pub usize);
impl_resolver_singleton!(UGameplayStaticsLoadGameFromSlot, |ctx| async {
    let patterns = [
        "48 8B C4 55 ?? 48 8D A8 ?? FE FF FF 48 81 EC ?? 02 00 00 48 89 ?? 08 33 ?? 48 89 ?? 10 48 8B ?? 4C 89 70 E8 44 8B F2 48 89 ?? 24 40 48 89 ?? 24 48 E8 ?? ?? ?? ?? 48 8B C8 4C 8B 00 41 FF 50 40 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 8D 35",
        "48 89 5C 24 08 48 89 74 24 10 57 48 83 EC 40 33 DB 8B F2 48 89 5C 24 30 48 8B F9 48 89 5C 24 38 E8 ?? ?? ?? ?? 48 8B C8 4C 8B 00 41 FF 50 40 4C 8B D0 48 85 C0 74 4A 8B 57 08 85 D2 8D 4A FF 0F 44 CB 85 C9 7E 3B 85 D2 74 05 4C 8B 07 EB 07 4C 8D 05 ?? ?? ?? ?? 48 8B 00 48 8D 4C 24 30 48 89 4C 24 20 44",
        "48 89 5C 24 10 55 57 41 56 48 8D AC 24 00 FF FF FF 48 81 EC 00 02 00 00 48 8B D9 33 FF 48 8B 0D ?? ?? ?? ?? 44 8B F2 48 85 C9 75 26 8D 4F 08 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 48 8B CF 48 89 0D ?? ?? ?? ?? 48 8B 01",
        "48 89 5C 24 08 55 56 57 48 8D AC 24 ?? FF FF FF 48 81 EC ?? 01 00 00 48 8B D9 ?? ?? ?? ?? ?? ?? ?? ?? ?? 8B F2 48 85 C9 75 26 8D 4F 08 E8 ?? ?? ?? FF 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 48 8B CF 48 89 0D ?? ?? ?? ?? 48 8B 01 FF 50 40 48 8B C8 48 85 C0 0F 84 ?? ?? 00 00 8B 43",
        "48 89 5C 24 18 55 56 57 48 8D AC 24 ?? FF FF FF 48 81 EC ?? ?? 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 85 ?? 00 00 00 48 8B D9 33 FF 48 8B 0D ?? ?? ?? ?? 8B F2 48 85 C9 75 26 8D 4F 08 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 48 8B CF 48 89 0D ?? ?? ?? ?? 48 8B 01",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(UGameplayStaticsLoadGameFromSlot(ensure_one(
        res.into_iter().flatten(),
    )?))
});

/// public: static bool __cdecl UGameplayStatics::DoesSaveGameExist(class FString const &, int)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UGameplayStaticsDoesSaveGameExist(pub usize);
impl_resolver_singleton!(UGameplayStaticsDoesSaveGameExist, |ctx| async {
    let patterns = [
        "48 89 5C 24 08 57 48 83 EC 20 8B FA 48 8B D9 E8 ?? ?? ?? ?? 48 8B C8 4C 8B 00 41 FF 50 ?? 48 85 C0 74 3D 83 7B 08 00 4C 8B 00 4D 8B 48 ?? 74 16 48 8B 13 44 8B C7 48 8B C8 48 8B 5C 24 30 48 83 C4 20 5F 49 FF E1 48 8D 15 ?? ?? ?? ?? 44 8B C7 48 8B C8 48 8B 5C 24 30 48 83 C4 20 5F 49 FF E1 48 8B 5C 24",
        "48 89 5C 24 08 57 48 83 EC 20 8B FA 48 8B D9 E8 ?? ?? ?? ?? 48 8B C8 4C 8B 00 41 FF 50 40 48 8B C8 48 85 C0 74 38 83 7B 08 00 74 17 48 8B 00 44 8B C7 48 8B 13 48 8B 5C 24 30 48 83 C4 20 5F 48 FF 60 08 48 8B 00 48 8D 15 ?? ?? ?? ?? 44 8B C7 48 8B 5C 24 30 48 83 C4 20 5F 48 FF 60 08 48 8B 5C 24 30 48",
        "48 89 5C 24 08 57 48 83 EC 20 48 8B D9 ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 85 C9 75 27 B9 08 00 00 00 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 02 33 C9 48 89 0D ?? ?? ?? ?? 48 8B 01 FF 50 40 48 8B C8 48 85 C0 74 38 83 7B 08 00 74 17 48 8B 00 ?? 8B ?? ?? 8B ?? 48 8B 5C 24 30",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(UGameplayStaticsDoesSaveGameExist(ensure_one(
        res.into_iter().flatten(),
    )?))
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct KismetSystemLibrary(pub HashMap<String, usize>);

impl_resolver!(KismetSystemLibrary, |ctx| async {
    let mem = &ctx.image().memory;

    let s = Pattern::from_bytes(
        "KismetSystemLibrary\x00"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect(),
    )
    .unwrap();
    let strings = ctx.scan(s).await;

    let refs = join_all(strings.iter().map(|s| {
        ctx.scan(
            Pattern::new(format!(
        // fragile (only 4.25-4.27 most likely)
        "4c 8d 0d [ ?? ?? ?? ?? ] 88 4c 24 70 4c 8d 05 ?? ?? ?? ?? 49 89 43 e0 48 8d 15 X0x{:x}",
        s
    ))
            .unwrap(),
        )
    }))
    .await;

    let cap = Pattern::new("4c 8d 0d [ ?? ?? ?? ?? ]").unwrap();

    let register_natives_addr =
        try_ensure_one(refs.iter().flatten().map(|a| -> Result<_> {
            Ok(ctx.image().memory.captures(&cap, *a)?.unwrap()[0].rip())
        }))?;

    let register_natives = Pattern::new("48 83 ec 28 e8 ?? ?? ?? ?? 41 b8 [ ?? ?? ?? ?? ] 48 8d 15 [ ?? ?? ?? ?? ] 48 8b c8 48 83 c4 28 e9 ?? ?? ?? ??").unwrap();

    let captures = ctx
        .image()
        .memory
        .captures(&register_natives, register_natives_addr);

    if let Some([num, data]) = captures?.as_deref() {
        let mut res = HashMap::new();

        let ptr = data.rip();
        for i in 0..(num.u32() as usize) {
            let a = ptr + i * 0x10;
            res.insert(mem.read_string(mem.ptr(a)?)?, mem.ptr(a + 8)?);
        }
        Ok(KismetSystemLibrary(res))
    } else {
        bail_out!("did not match");
    }
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UGameEngineTick(pub usize);

impl_resolver_singleton!(UGameEngineTick, |ctx| async {
    let strings = ctx
        .scan(Pattern::from_bytes(b"EngineTickMisc\x00".to_vec()).unwrap())
        .await;

    let refs = join_all(
        strings
            .iter()
            // TODO maybe mask out specific register
            .map(|s| ctx.scan(Pattern::new(format!("48 8d 0d X0x{s:X}")).unwrap())),
    )
    .await;

    let fns = refs
        .into_iter()
        .flatten()
        .map(|r| -> Result<_> { Ok(ctx.image().get_root_function(r)?.map(|f| f.range.start)) })
        .collect::<Result<Vec<_>>>()? // TODO avoid this collect?
        .into_iter()
        .flatten();

    Ok(UGameEngineTick(ensure_one(fns)?))
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct ConsoleManagerSingleton(usize);

impl_resolver_singleton!(ConsoleManagerSingleton, |ctx| async {
    let strings = join_all([
        ctx.scan(
            Pattern::from_bytes(
                "r.DumpingMovie"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            )
            .unwrap(),
        ),
        ctx.scan(
            Pattern::from_bytes(
                "vr.pixeldensity"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            )
            .unwrap(),
        ),
    ])
    .await;

    let refs = join_all(
        strings
            .into_iter()
            .flatten()
            .map(|addr| ctx.scan(Pattern::new(format!("48 8d 15 X0x{addr:x}")).unwrap())),
    )
    .await;

    let fns = refs
        .into_iter()
        .flatten()
        .map(|r| -> Result<_> { Ok(ctx.image().get_root_function(r)?.map(|f| f.range.start)) })
        .collect::<Result<Vec<_>>>()? // TODO avoid this collect?
        .into_iter()
        .flatten();

    Ok(ConsoleManagerSingleton(ensure_one(fns)?))
});

/// useful for extracting strings from common patterns for analysis
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UtilStringExtractor(pub HashSet<String>);
impl_resolver!(UtilStringExtractor, |ctx| async {
    let strings = ctx
        .scan(
            Pattern::new(
                "48 8d 55 f8 49 8b c8 e8 | ?? ?? ?? ?? 0f 28 45 f0 48 8d 55 f0 44 8b c8 66 0f 7f 45 f0 41 b8 01 00 00 00 48 8d 0d ?? ?? ?? ?? e8 ?? ?? ?? ??",
            )
            .unwrap(),
        )
        .await;

    let mem = &ctx.image().memory;

    Ok(UtilStringExtractor(
        strings
            .into_iter()
            .map(|a| -> Result<_> { Ok(mem.read_wstring(mem.rip4(a)?)?) })
            .filter_map(|s| s.ok())
            .collect::<HashSet<String>>(),
    ))
});

/// useful for extracting strings from common patterns for analysis
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct A(pub HashSet<usize>);
impl_resolver!(A, |ctx| async {
    let strings = ctx
        .scan(
            Pattern::new(
                "48 8d 55 f8 49 8b c8 e8 ?? ?? ?? ?? 0f 28 45 f0 48 8d 55 f0 44 8b c8 66 0f 7f 45 f0 41 b8 01 00 00 00 48 8d 0d ?? ?? ?? ?? e8 | ?? ?? ?? ??",
            )
            .unwrap(),
        )
        .await;

    let mem = &ctx.image().memory;

    Ok(A(strings
        .into_iter()
        .map(|a| Ok(mem.rip4(a)?))
        .collect::<Result<HashSet<_>>>()?))
});
