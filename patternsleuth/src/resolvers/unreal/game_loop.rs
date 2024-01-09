use std::fmt::Debug;

use futures::future::join_all;

use patternsleuth_scanner::Pattern;

use crate::resolvers::{ensure_one, impl_resolver_singleton, unreal::util, Result};

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

/// int32_t FEngineLoop::Init(class FEngineLoop* this)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FEngineLoopInit(pub usize);
impl_resolver_singleton!(FEngineLoopInit, |ctx| async {
    let search_strings = [
        "FEngineLoop::Init\0",
        "Failed to load UnrealEd Engine class '%s'.",
        "One or more modules failed PostEngineInit",
    ];
    let strings = join_all(
        search_strings
            .into_iter()
            .map(|s| ctx.scan(util::utf16_pattern(s))),
    )
    .await
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    let refs = util::scan_xrefs(ctx, &strings).await;
    let fns = util::root_functions(ctx, &refs)?;
    Ok(Self(ensure_one(fns)?))
});
