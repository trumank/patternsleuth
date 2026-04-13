use futures::{future::join_all, join};
use patternsleuth_scanner::Pattern;

use crate::{
    MemoryTrait,
    resolvers::{Result, ensure_one, impl_resolver_singleton, try_ensure_one, unreal::util},
};

/// class UObject * __cdecl StaticLoadObject(class UClass *, class UObject *, wchar_t const *, wchar_t const *, unsigned int, class UPackageMap *, bool, struct FLinkerInstancingContext const *)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct StaticLoadObject(pub u64);
impl_resolver_singleton!(all, StaticLoadObject, |ctx| async {
    let any = join!(
        ctx.resolve(StaticLoadObjectPatterns::resolver()),
        ctx.resolve(StaticLoadObjectString::resolver()),
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
pub struct StaticLoadObjectPatterns(pub u64);
impl_resolver_singleton!(all, StaticLoadObjectPatterns, |ctx| async {
    let patterns = [
        "c6 44 24 30 01 48 8b d5 4c 89 54 24 28 48 8b c8 89 5c 24 20 e8 | ?? ?? ?? ?? 48 8b 5c 24 50 48 8b 6c 24 58 48 8b 74 24 60 48 83 c4 40 5f c3",
        "4c 89 6c 24 38 4c 8b c7 c6 44 24 30 01 49 8b d7 48 89 5c 24 28 48 8b c8 44 89 64 24 20 e8 | ?? ?? ?? ?? 48 85 c0",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(Self(try_ensure_one(res.iter().flatten().map(
        |a| -> Result<_> { Ok(ctx.image().memory.rip4(*a)?) },
    ))?))
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct StaticLoadObjectString(pub u64);
impl_resolver_singleton!(all, StaticLoadObjectString, |ctx| async {
    let strings = join_all(
        [
            "Failed to find object '{ClassName} {OuterName}.{ObjectName}'\0",
            "Failed to find object '{ObjectPath}'\0",
        ]
        .iter()
        .map(|s| ctx.scan(util::utf16_pattern(s))),
    )
    .await
    .concat();

    let refs = util::scan_xrefs(ctx, &strings).await;
    let fns = util::root_functions(ctx, &refs)?;
    Ok(Self(ensure_one(fns)?))
});
