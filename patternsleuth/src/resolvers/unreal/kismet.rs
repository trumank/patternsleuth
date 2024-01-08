use std::fmt::Debug;

use futures::future::join_all;
use iced_x86::{Code, Decoder, DecoderOptions, Instruction, Register};
use patternsleuth_scanner::Pattern;

use crate::{
    resolvers::{
        bail_out, ensure_one, impl_resolver, impl_resolver_singleton, try_ensure_one, Result,
    },
    Addressable, Matchable, MemoryTrait,
};

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
