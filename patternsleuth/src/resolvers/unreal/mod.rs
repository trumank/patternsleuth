pub mod aes;
pub mod blueprint_library;
pub mod engine_version;
pub mod fname;
pub mod ftext;
pub mod fuobject_hash_tables;
pub mod game_loop;
pub mod gengine;
pub mod gmalloc;
pub mod guobject_array;
pub mod kismet;
pub mod pak;
pub mod save_game;
pub mod static_construct_object;
pub mod static_find_object;

use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
};

use futures::future::join_all;
use iced_x86::FlowControl;
use itertools::Itertools;
use patternsleuth_scanner::Pattern;

use crate::{
    disassemble::{disassemble, Control},
    resolvers::{
        bail_out, ensure_one, impl_resolver, impl_resolver_singleton, try_ensure_one, Result,
    },
    Addressable, Image, Matchable, MemoryTrait,
};

#[allow(unused)]
pub mod util {
    use crate::resolvers::AsyncContext;

    use super::*;

    #[derive(Debug, Clone, Copy)]
    pub struct Call {
        pub index: usize,
        pub ip: usize,
        pub callee: usize,
    }

    pub fn utf16(string: &str) -> Vec<u8> {
        string.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }
    pub fn utf8_pattern(string: &str) -> Pattern {
        Pattern::from_bytes(string.as_bytes().to_vec()).unwrap()
    }
    pub fn utf16_pattern(string: &str) -> Pattern {
        Pattern::from_bytes(utf16(string)).unwrap()
    }
    pub async fn scan_xrefs(
        ctx: &AsyncContext<'_>,
        addresses: impl IntoIterator<Item = &usize> + Copy,
    ) -> Vec<usize> {
        let refs_indirect = join_all(
            addresses
                .into_iter()
                .map(|s| ctx.scan(Pattern::from_bytes(usize::to_le_bytes(*s).into()).unwrap())),
        )
        .await;

        let refs = join_all(
            addresses
                .into_iter()
                .copied()
                .chain(refs_indirect.into_iter().flatten())
                .flat_map(|s| {
                    let mut scans =
                        vec![format!("48 8d ?? X0x{s:X}"), format!("4c 8d ?? X0x{s:X}")];
                    if TryInto::<u32>::try_into(s).is_ok() {
                        // mov reg, imm32 if address is 32 bit
                        scans.extend([
                            format!("b8 0x{s:X}"),
                            format!("b9 0x{s:X}"),
                            format!("ba 0x{s:X}"),
                            format!("bb 0x{s:X}"),
                            format!("bc 0x{s:X}"),
                            format!("bd 0x{s:X}"),
                            format!("be 0x{s:X}"),
                            format!("bf 0x{s:X}"),
                        ]);
                    }
                    scans
                })
                .map(|p| ctx.scan(Pattern::new(p).unwrap())),
        )
        .await;

        refs.into_iter().flatten().collect()
    }

    pub async fn scan_xcalls(
        ctx: &AsyncContext<'_>,
        addresses: impl IntoIterator<Item = &usize> + Copy,
    ) -> Vec<usize> {
        let refs_indirect = join_all(
            addresses
                .into_iter()
                .map(|s| ctx.scan(Pattern::from_bytes(usize::to_le_bytes(*s).into()).unwrap())),
        )
        .await;

        let refs = join_all(
            addresses
                .into_iter()
                .copied()
                .chain(refs_indirect.into_iter().flatten())
                .flat_map(|s| {
                    [
                        //ctx.scan(Pattern::new(format!("10111??? 0x{s:X}")).unwrap()), // mov reg, imm32
                        ctx.scan(Pattern::new(format!("e8 X0x{s:X}")).unwrap()),
                        ctx.scan(Pattern::new(format!("e9 X0x{s:X}")).unwrap()),
                    ]
                }),
        )
        .await;

        refs.into_iter().flatten().collect()
    }

    pub fn root_functions<'a, I>(ctx: &AsyncContext<'_>, addresses: I) -> Result<Vec<usize>>
    where
        I: IntoIterator<Item = &'a usize> + Copy,
    {
        Ok(addresses
            .into_iter()
            .map(|r| -> Result<_> { Ok(ctx.image().get_root_function(*r)?.map(|f| f.range.start)) })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect())
    }

    pub fn find_calls(img: &Image<'_>, f: usize) -> Result<Vec<Call>> {
        let mut calls = vec![];

        disassemble(img, f, |inst| {
            let cur = inst.ip() as usize;
            if Some(f) != img.get_root_function(cur)?.map(|f| f.range.start) {
                return Ok(Control::Break);
            }

            match inst.flow_control() {
                FlowControl::Call
                | FlowControl::ConditionalBranch
                | FlowControl::UnconditionalBranch => {
                    let call = inst.near_branch_target() as usize;
                    if Some(f) != img.get_root_function(call)?.map(|f| f.range.start) {
                        calls.push(Call {
                            index: 0,
                            ip: inst.ip() as usize,
                            callee: call,
                        });
                    }
                }
                _ => {}
            }

            Ok(Control::Continue)
        })?;

        calls.sort_by_key(|c| c.ip);
        for (i, call) in calls.iter_mut().enumerate() {
            call.index = i;
        }
        Ok(calls)
    }

    pub fn find_path(
        img: &Image<'_>,
        f: usize,
        depth: usize,
        searched: &mut HashSet<usize>,
        path: &mut Vec<Call>,
        needle: usize,
    ) -> Result<Vec<String>> {
        searched.insert(f);

        let mut result = vec![];
        let mut calls = vec![];

        disassemble(img, f, |inst| {
            let cur = inst.ip() as usize;
            if !(f..f + 1000).contains(&cur)
                && Some(f) != img.get_root_function(cur)?.map(|f| f.range.start)
            {
                println!("bailing at {:x}", inst.ip());
                return Ok(Control::Break);
            }

            match inst.flow_control() {
                FlowControl::Call
                | FlowControl::ConditionalBranch
                | FlowControl::UnconditionalBranch => {
                    let call = inst.near_branch_target() as usize;
                    println!("{:x} {:x}", inst.ip(), call);
                    if Some(f) != img.get_root_function(call)?.map(|f| f.range.start) {
                        calls.push(Call {
                            index: 0, // unknown for now
                            ip: cur,
                            callee: call,
                        });
                    }
                }
                _ => {}
            }

            Ok(Control::Continue)
        })?;

        for (i, call) in calls.iter_mut().sorted_by_key(|c| c.ip).enumerate() {
            call.index = i;
            if !searched.contains(&call.callee) {
                path.push(*call);
                if call.callee == needle {
                    println!("{path:x?}");
                    result.push(format!("{path:x?}"));
                }
                if depth > 0 {
                    result.extend(find_path(
                        img,
                        call.callee,
                        depth - 1,
                        searched,
                        path,
                        needle,
                    )?);
                }
                path.pop();
            }
        }
        Ok(result)
    }
}

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct KismetSystemLibrary(pub HashMap<String, usize>);

impl_resolver!(all, KismetSystemLibrary, |ctx| async {
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
        "4c 8d 0d [ ?? ?? ?? ?? ] 88 4c 24 70 4c 8d 05 ?? ?? ?? ?? 49 89 43 e0 48 8d 15 X0x{s:x}"
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
pub struct ConsoleManagerSingleton(pub usize);

impl_resolver_singleton!(all, ConsoleManagerSingleton, |ctx| async {
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

/// void UObjectBaseUtility::GetPathName(class UObjectBaseUtility const* this, class UObject const* StopOuter, class FString* ResultString)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UObjectBaseUtilityGetPathName(pub usize);
impl_resolver_singleton!(all, UObjectBaseUtilityGetPathName, |ctx| async {
    let patterns = [
        "40 53 48 81 EC 50 02 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 84 24 ?? ?? ?? ?? 48 8D 44 24",
    ];
    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;
    Ok(Self(ensure_one(res.into_iter().flatten())?))
});

/// useful for extracting strings from common patterns for analysis
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UtilStringExtractor(pub HashSet<String>);
impl_resolver!(all, UtilStringExtractor, |ctx| async {
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
impl_resolver!(all, A, |ctx| async {
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
