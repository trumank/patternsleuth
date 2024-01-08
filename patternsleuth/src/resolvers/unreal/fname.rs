use std::fmt::Debug;

use futures::{future::join_all, join};
use iced_x86::{Code, Decoder, DecoderOptions};

use patternsleuth_scanner::Pattern;

use crate::{
    resolvers::{
        ensure_one, impl_resolver, impl_resolver_singleton, try_ensure_one, unreal::util, Context,
        Result,
    },
    MemoryAccessorTrait, MemoryTrait,
};

/// public: __cdecl FName::FName(wchar_t const *, enum EFindName)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNameCtorWchar(pub usize);
impl_resolver_singleton!(FNameCtorWchar, |ctx| async {
    let strings = async {
        let strings = ["TGPUSkinVertexFactoryUnlimited\0", "MovementComponent0\0"];
        join_all(strings.iter().map(|s| ctx.scan(util::utf16_pattern(s)))).await
    };
    let patterns = async {
        ctx.scan(Pattern::new("EB 07 48 8D 15 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 41 B8 01 00 00 00 E8 | ?? ?? ?? ??").unwrap()).await
    };
    let (patterns, strings) = join!(patterns, strings);

    // sometimes the call gets inlined so use patterns if any match
    if !patterns.is_empty() {
        return Ok(Self(try_ensure_one(
            patterns
                .iter()
                .map(|a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) }),
        )?));
    }

    #[derive(Clone, Copy)]
    enum Tag {
        Direct,
        FirstCall,
    }

    let refs = join_all(strings.iter().flatten().flat_map(|s| {
        [
            (
                Tag::FirstCall,
                format!("48 8d 15 X0x{s:x} 4c 8d 05 ?? ?? ?? ?? 41 b1 01 e8 | ?? ?? ?? ??"),
            ),
            (
                Tag::Direct,
                format!("48 8d 15 X0x{s:x} 48 8d 0d ?? ?? ?? ?? e8 | ?? ?? ?? ??"),
            ),
            (
                Tag::Direct,
                format!(
                    "41 b8 01 00 00 00 48 8d 15 X0x{s:x} 48 8d 0d ?? ?? ?? ?? e9 | ?? ?? ?? ??"
                ),
            ),
        ]
        .into_iter()
        .map(|(t, p)| ctx.scan_tagged2(t, Pattern::new(p).unwrap()))
    }))
    .await;

    Ok(Self(try_ensure_one(refs.iter().flatten().map(
        |(tag, address)| {
            let f = ctx.image().memory.rip4(*address)?;
            match tag {
                Tag::Direct => Ok(f),
                Tag::FirstCall => {
                    let bytes = ctx.image().memory.range(f..f + 0x200)?;
                    let mut decoder = Decoder::with_ip(64, bytes, f as u64, DecoderOptions::NONE);

                    decoder
                        .iter()
                        .find_map(|i| {
                            (i.code() == Code::Call_rel32_64)
                                .then_some(i.memory_displacement64() as usize)
                        })
                        .context("did not find CALL instruction")
                }
            }
        },
    ))?))
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
    let patterns = async {
        let patterns = ["56 57 48 83 EC 28 48 89 D6 48 89 CF 83 79 ?? 00 74"];

        join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap())))
            .await
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
    };

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
        patterns,
    );

    // use pattern if found
    if !any.3.is_empty() {
        return Ok(Self(ensure_one(any.3)?));
    }

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
