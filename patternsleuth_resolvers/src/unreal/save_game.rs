use std::fmt::Debug;

use futures::future::join_all;

use patternsleuth_scanner::Pattern;

use crate::{ensure_one, impl_resolver_singleton};

/// public: static bool __cdecl UGameplayStatics::SaveGameToMemory(class USaveGame *, class TArray<unsigned char, class TSizedDefaultAllocator<32> > &)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UGameplayStaticsSaveGameToMemory(pub usize);
impl_resolver_singleton!(all, UGameplayStaticsSaveGameToMemory, |ctx| async {
    let patterns = [
        "48 89 5C 24 10 48 89 7C 24 18 55 48 8D AC 24 ?? FF FF FF 48 81 EC ?? 01 00 00 48 8B DA 48 8B F9 48 85 C9 0F 84 ?? 02 00 00 0F 57 C0 48 C7 85 ?? 00 00 00 00 00 00 00",
        "48 89 5C 24 10 48 89 7C 24 18 55 48 8D AC 24 20 FF FF FF 48 81 EC E0 01 00 00 48 8B DA 48 8B F9 48 85 C9 0F 84 ?? ?? 00 00 0F 57 C0 48 C7 85 F0 00 00 00 00 00 00 00 33 C0 48 8D 4D 80 0F 11 45 80 48 89 45 10 0F 11 45 90 0F 11 45 A0 0F 11 45 B0 0F 11 45 C0 0F 11 45 D0 0F 11 45 E0 0F 11 45 F0 0F 11 45",
        "48 89 5C 24 10 48 89 7C 24 18 55 48 8D AC 24 ?? FF FF FF 48 81 EC ?? 01 00 00 48 8B DA 48 8B F9 48 85 C9 0F 84 71 01 00 00 33 D2 48 C7 85 ?? 00 00 00 00 00 00 00 41 B8 ?? 00 00 00 48 8D 4D 80 E8 ?? ?? ?? ?? 48 8D 4D 80 E8 ?? ?? ?? ?? 48 8D 05 ?? ?? ?? ?? 48 C7 45 ?? 00 00 00 00 48 89 45 80 48 8D 4D",
        //linux
        "41 57 41 56 53 01001??? 81 ec b0 01 00 00 01001??? 89 fb 01001??? 85 ff 0f 84 ?? ?? ?? ?? 01001??? 89 f7 0f 57 c0 0f 29 84 ??100100 80 00 00 00 0f 29 44 ??100100 70 0f 29 44 ??100100 60 0f 29 44 ??100100 50 0f 29 44 ??100100 40 0f 29 44 ??100100 30 0f 29 44 ??100100 20 0f 29 44 ??100100 10 0f 29 04 ??100100 01001??? c7 84 ??100100 90 00 00 00 00 00 00 00 01001??? 89 e6",
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
impl_resolver_singleton!(all, UGameplayStaticsSaveGameToSlot, |ctx| async {
    let patterns = [
        "48 89 5C 24 08 48 89 74 24 10 57 48 83 EC 40 ?? 8B ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? E8 ?? ?? FF FF 84 C0 74 58 E8 ?? ?? ?? ?? 48 8B ?? 48 8B ?? FF 52 ?? 4C 8B D0 48 85 C0 74 42 39 74 24 38 7E 3C 8B 53 08 ?? ?? ?? ?? ?? 0F 44 CE 85 C9 7E 2D",
        "48 89 5C 24 08 48 89 74 24 10 48 89 7C 24 18 55 41 56 41 57 48 8D AC 24 ?? FF FF FF 48 81 EC ?? ?? 00 00 48 8B F1 45 33 FF 48 8B 0D ?? ?? ?? ?? 45 8B F0 48 8B ?? 48 85 C9 75 27 41 8D 4F 08 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 49 8B CF 48 89 0D ?? ?? ?? ?? 48 8B",
        "40 55 56 57 41 54 41 55 41 ?? 48 8D AC 24 ?? ?? FF FF 48 81 EC ?? ?? 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 85 ?? ?? 00 00 4C 8B ?? 45 33 ED 48 8B 0D ?? ?? ?? ?? 45 8B E0 48 8B FA 48 85 C9 75 27 41 8D 4D 08 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 49 8B CD 48 89",
        // linux
        "55 53 01001??? 83 ec 18 89 d5 01001??? 89 f3 0f 57 c0 0f 29 04 ??100??? 01001??? 89 e6 e8 ?? ?? ?? ?? 84 c0 74 ?? 01001??? 8b 3d ?? ?? ?? ?? 01001??? 85 ff 74 ?? 01001??? 8b 07 ff 50 48 01001??? 85 c0 75 ?? eb ??",
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
impl_resolver_singleton!(all, UGameplayStaticsLoadGameFromMemory, |ctx| async {
    let patterns = [
        "48 89 5C 24 20 55 48 8D AC 24 10 FF FF FF 48 81 EC F0 01 00 00 83 79 08 00 48 8B D9 75 13 33 C0 48 8B 9C 24 18 02 00 00 48 81 C4 F0 01 00 00 5D C3 0F 57 C0 48 89 ?? 24 ?? 02 00 00 48 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11",
        "40 55 48 8D AC 24 00 FF FF FF 48 81 EC 00 02 00 00 83 79 08 00 75 0B 33 C0 48 81 C4 00 02 00 00 5D C3 48 8B D1 48 89 9C 24 20 02 00 00 48 89 BC 24 28 02 00 00 48 8D 4D A0 41 B0 01 33 FF E8 ?? ?? ?? ?? 0F 57 C0 89 7C 24 50 48 8D 4C 24 50 66 0F 7F 44 24 40 66 89 7C 24 54 89 7C 24 58 48 89 7C 24 60 48",
        "40 55 48 8D AC 24 ?? ?? FF FF 48 81 EC ?? 02 00 00 83 79 08 00 75 0B 33 C0 48 81 C4 ?? 02 00 00 5D C3 48 89 9C 24 ?? 02 00 00 48 8B D1 48 89 B4 24 ?? 02 00 00 48 8D 4D A0 48 89 BC 24 ?? 02 00 00 41 B0 01 33 FF E8 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 24 ?? ?? 89 7C 24",
        "48 89 5C 24 20 55 48 8D AC 24 ?? FF FF FF 48 81 EC ?? ?? 00 00 83 79 08 00 48 8B D9 75 13 33 C0 48 8B 9C 24 ?? 02 00 00 48 81 C4 ?? ?? 00 00 5D C3 48 89 B4 24 ?? 02 00 00 48 8D 4D A0 48 89 BC 24 ?? 02 00 00 33",
        "40 55 53 48 8D AC 24 08 FF FF FF 48 81 EC F8 01 00 00 83 79 08 00 48 8B D9 75 0C 33 C0 48 81 C4 F8 01 00 00 5B 5D C3 48 89 B4 24 18 02 00 00 48 8D 4D A0 48 89 BC 24 20 02 00 00 33 FF 4C 89 B4 24 F0 01 00 00 E8 ?? ?? ?? ?? 48 8D 05 ?? ?? ?? 00 48 89 7D 30 48 89 45 A0 48 8D 4D A0 48 B8 FF FF FF FF FF",
        // linux
        "41 57 41 56 53 01001??? 81 ec c0 01 00 00 83 7f 08 00 0f 84 ?? ?? ?? ?? 01001??? 89 fb 0f 57 c0 0f 29 84 ??100100 e0 00 00 00 0f 29 84 ??100100 d0 00 00 00 0f 29 84 ??100100 c0 00 00 00 0f 29 84 ??100100 b0 00 00 00 0f 29 84 ??100100 a0 00 00 00 0f 29 84 ??100100 90 00 00 00 0f 29 84 ??100100 80 00 00 00 0f 29 44 ??100100 70 0f 29 44 ??100100 60 01001??? c7 84 ??100100 f0 00 00 00 00 00 00 00 01001??? 8d 74 ??100100 60 01001??? 89 f7 e8 ?? ?? ?? ?? 01001??? c7 84 ??100100 f8 00 00 00 00 00 00 00 01001??? c7 44 ??100100 60 ?? ?? ?? ?? 01001??? 89 9c ??100100 00 01 00 00 ",
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
impl_resolver_singleton!(all, UGameplayStaticsLoadGameFromSlot, |ctx| async {
    let patterns = [
        "48 8B C4 55 ?? 48 8D A8 ?? FE FF FF 48 81 EC ?? 02 00 00 48 89 ?? 08 33 ?? 48 89 ?? 10 48 8B ?? 4C 89 70 E8 44 8B F2 48 89 ?? 24 40 48 89 ?? 24 48 E8 ?? ?? ?? ?? 48 8B C8 4C 8B 00 41 FF 50 40 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 8D 35",
        "48 89 5C 24 08 48 89 74 24 10 57 48 83 EC 40 33 DB 8B F2 48 89 5C 24 30 48 8B F9 48 89 5C 24 38 E8 ?? ?? ?? ?? 48 8B C8 4C 8B 00 41 FF 50 40 4C 8B D0 48 85 C0 74 4A 8B 57 08 85 D2 8D 4A FF 0F 44 CB 85 C9 7E 3B 85 D2 74 05 4C 8B 07 EB 07 4C 8D 05 ?? ?? ?? ?? 48 8B 00 48 8D 4C 24 30 48 89 4C 24 20 44",
        "48 89 5C 24 10 55 57 41 56 48 8D AC 24 00 FF FF FF 48 81 EC 00 02 00 00 48 8B D9 33 FF 48 8B 0D ?? ?? ?? ?? 44 8B F2 48 85 C9 75 26 8D 4F 08 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 48 8B CF 48 89 0D ?? ?? ?? ?? 48 8B 01",
        "48 89 5C 24 08 55 56 57 48 8D AC 24 ?? FF FF FF 48 81 EC ?? 01 00 00 48 8B D9 ?? ?? ?? ?? ?? ?? ?? ?? ?? 8B F2 48 85 C9 75 26 8D 4F 08 E8 ?? ?? ?? FF 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 48 8B CF 48 89 0D ?? ?? ?? ?? 48 8B 01 FF 50 40 48 8B C8 48 85 C0 0F 84 ?? ?? 00 00 8B 43",
        "48 89 5C 24 18 55 56 57 48 8D AC 24 ?? FF FF FF 48 81 EC ?? ?? 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 85 ?? 00 00 00 48 8B D9 33 FF 48 8B 0D ?? ?? ?? ?? 8B F2 48 85 C9 75 26 8D 4F 08 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 48 8B CF 48 89 0D ?? ?? ?? ?? 48 8B 01",
        // linux
        "55 53 48 83 ec 18 89 f5 01001??? 89 fb 0f 57 c0 0f 29 04 ??100100 01001??? 8b 3d ?? ?? ?? ?? 01001??? 85 ff 74 ?? 01001??? 8b 07 ff 50 48 01001??? 85 c0 75 ?? eb ?? bf 08 00 00 00 e8 ?? ?? ?? ?? 01001??? 89 c7 01001??? c7 00 ?? ?? ?? ?? 01001??? 89 05 ?? ?? ?? ?? 01001??? 8b 07 ff 01010??? ?? 01001??? 85 c0 74 ??"
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(UGameplayStaticsLoadGameFromSlot(ensure_one(
        res.into_iter().flatten(),
    )?))
});

// not exists on linux
/// public: static bool __cdecl UGameplayStatics::DoesSaveGameExist(class FString const &, int)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UGameplayStaticsDoesSaveGameExist(pub usize);
impl_resolver_singleton!(all, UGameplayStaticsDoesSaveGameExist, |ctx| async {
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
