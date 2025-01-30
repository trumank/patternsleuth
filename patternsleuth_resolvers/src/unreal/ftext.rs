use std::fmt::Debug;

use futures::future::join_all;

use patternsleuth_scanner::Pattern;

use crate::{
    MemoryTrait, {impl_resolver_singleton, try_ensure_one},
};

/// private: __cdecl FText::FText(class FString &&)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FTextFString(pub usize);
impl_resolver_singleton!(all, FTextFString, |ctx| async {
    #[derive(Debug)]
    enum Directness {
        Direct,
        Indirect,
    }
    let patterns = [
        (Directness::Indirect, "40 53 48 83 ec ?? 48 8b d9 e8 | ?? ?? ?? ?? 83 4b ?? 12 48 8b c3 48 83 ?? ?? 5b c3"),
        (Directness::Indirect, "eb 12 48 8d ?? 24 ?? e8 | ?? ?? ?? ?? ?? 02 00 00 00 48 8b 10 48 89 17"),
        (Directness::Indirect, "eb 12 48 8d ?? 24 ?? e8 | ?? ?? ?? ?? ?? 02 00 00 00 48 8b 10 89"),
        (Directness::Direct, "48 89 5C 24 10 48 89 6C 24 18 56 57 41 54 41 56 41 57 48 83 EC 40 45 33 E4 48 8B F1 41 8B DC 4C 8B F2 89 5C 24 70 41 8D 4C 24 70 E8 ?? ?? ?? FF 48 8B F8 48 85 C0 0F 84 ?? 00 00 00 49 63 5E 08 ?? 8B ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 8B ?? EB 2E 45 33 C0 48 8D 4C 24 20 8B D3 E8"),
        (Directness::Direct, "48 89 5C 24 10 48 89 6C 24 18 57 48 83 EC 50 33 ED 48 8D 05 ?? ?? ?? 03 48 8B F9 48 89 6C 24 38 48 8B DA 48 89 6C 24 48 48 89 44 24 30 8D 4D 60 48 89 44 24 40 E8 ?? ?? ?? FF 4C 8B C0 48 85 C0 74 65 0F 10 44 24 30 C7 40 08 01 00 00 00 0F 10 4C 24 40 C7 40 0C 01 00 00 00 48 8D 05 ?? ?? ?? 03 49 89 00"),
        (Directness::Direct, "48 89 5C 24 10 48 89 6C 24 18 56 57 41 54 41 56 41 57 48 83 EC 50 45 33 E4 48 8B F9 41 8B DC 4C 8B F2 89 9C 24 80 00 00 00 41 8D 4C 24 70 E8 ?? ?? ?? ?? 48 8B F0 48 85 C0 0F 84 98 00 00 00 49 63 5E 08 41 8B EC 4D 8B 3E 4C 89 64 24 20 89 5C 24 28 85 DB 75 05 45 8B FC EB 2E 45 33 C0 48 8D 4C 24 20 8B"),
        (Directness::Direct, "48 89 5C 24 ?? 48 89 6C 24 ?? 48 89 74 24 ?? 48 89 7C 24 ?? 41 54 41 56 41 57 48 83 EC 40 4C 8B F1 48 8B F2"),
        (Directness::Direct, "48 89 5C 24 ?? 48 89 6C 24 ?? 56 57 41 54 41 56 41 57 48 83 EC 40 45 33 E4 48 8B F1"),
        (Directness::Direct, "40 53 56 48 83 EC 48 33 DB 48 89 6C 24 68 48 8B F1 48 89 7C 24 70 4C 89 74 24 78 4C 8B F2 89 5C 24 60 8D 4B 70 E8 ?? ?? ?? FF 48 8B F8 48 85 C0 0F 84 9E 00 00 00 49 63 5E 08 33 ED 4C 89 7C 24 40 4D 8B 3E 48 89 6C 24 20 89 5C 24 28 85 DB 75 05 45 33 FF EB 2E 45 33 C0 48 8D 4C 24 20 8B D3 E8"),
        (Directness::Direct, "41 57 41 56 41 54 56 57 55 53 48 83 EC 40 48 89 D7 48 89 CE 48 8B 0D"),
        // FText::AsCultureInvariant on Linux
        (Directness::Indirect, "48 85 c9 74 13 f0 83 41 08 01 eb 0c 48 89 df e8 | ?? ?? ?? ?? 48 8d 43 10"),
        // FText::FText signature
        (Directness::Direct, "41 57 41 56 53 48 83 ec 20 49 89 f7 49 89 fe 0f 57 c0 0f 29 44 24 10 0f 29 04 24 48 8d 5c 24 10 48 89 e7 e8 ?? ?? ?? ?? 48 89 df e8 ?? ?? ?? ?? bf 60 00 00 00 e8"),
        // UE 5.4
        (Directness::Direct, "48 89 5C 24 ?? 48 89 74 24 ?? 57 48 83 EC ?? 48 8D 05 ?? ?? ?? ?? 33 F6 48 8B D9 48 89 44 24"),
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
