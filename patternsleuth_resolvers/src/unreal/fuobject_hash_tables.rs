use std::fmt::Debug;

use futures::future::join_all;

use patternsleuth_scanner::Pattern;

use crate::{Result, impl_resolver_singleton, try_ensure_one};

/// public: static class FUObjectHashTables & __cdecl FUObjectHashTables::Get(void)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FUObjectHashTablesGet(pub usize);
// try find u16"Hash efficiency statistics for the Outer Object Hash"
// LogHashOuterStatistics(FOutputDevice& Ar, const bool bShowHashBucketCollisionInfo)
// FHashTableLock HashLock(FUObjectHashTables::Get());
impl_resolver_singleton!(all, FUObjectHashTablesGet, |ctx| async {
    let patterns = [
        "48 89 5C 24 08 48 89 6C 24 10 48 89 74 24 18 57 48 83 EC 40 41 0F B6 F9 49 8B D8 48 8B F2 48 8B E9 E8 | ?? ?? ?? ?? 44 8B 84 24 80 00 00 00 4C 8B CB 44 ?? ?? 24 ?? 48 8B D5 44 ?? 44 24 ?? ?? ?? ?? ?? ?? 44 ?? ?? 44 ?? ?? ?? ?? ?? 44 ?? ?? 24 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48",
        "48 89 5C 24 08 48 89 74 24 10 4C 89 44 24 18 57 48 83 EC 40 41 0F B6 D9 48 8B FA 48 8B F1 E8 | ?? ?? ?? ?? 44 8B 84 24 80 00 00 00 48 8B D6 ?? 8B ?? 24 ?? 48 8B C8 ?? ?? ?? 24 ?? ?? ?? ?? ?? ?? 44 89 44 24 ?? 44 0F B6 44 24 70 44 ?? ?? 24 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 8B",
        "48 89 5C 24 08 48 89 6C 24 10 48 89 74 24 18 57 48 83 EC 40 41 0F B6 F9 49 8B D8 48 8B F2 48 8B E9 E8 | ?? ?? ?? ?? 44 8B 44 24 78 4C 8B CB 44 89 44 24 38 48 8B D5 44 8B 44 24 70 48 8B C8 44 89 44 24 30 4C 8B C6 C6 44 24 28 00 40 88 7C 24 20 E8 ?? ?? ?? ?? 48 8B 5C 24 50 48 8B 6C 24 58 48 8B 74 24 60",
        "e8 | ?? ?? ?? ?? 45 33 ff 48 8b f0 33 c0 f0 44 0f b1 3d",
        // linux pattern
        "0f 84 ?? ?? ?? ?? e8 | ?? ?? ?? ?? 84 c0 74 18 e8 ?? ?? ?? ?? 84 c0 74 0f b0 01 89 44 24 0c 31 c0 48 89 44 24 10 eb",
    ];
    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(Self(try_ensure_one(res.iter().flatten().map(
        |a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) },
    ))?))
});
