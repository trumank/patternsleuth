use std::fmt::Debug;

use futures::future::join_all;

use crate::{ensure_one, impl_resolver_singleton, unreal::util};

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct Main(pub usize);
impl_resolver_singleton!(collect, Main);
impl_resolver_singleton!(PEImage, Main, |ctx| async {
    let strings = ctx.scan(util::utf16_pattern("UnrealEngine4\0")).await;
    let refs = util::scan_xrefs(ctx, &strings).await;
    let fns = util::root_functions(ctx, &refs)?;
    Ok(Self(ensure_one(fns)?))
});
impl_resolver_singleton!(ElfImage, Main, |_ctx| async {
    super::bail_out!("ElfImage unimplemented");
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FEngineLoopTick(pub usize);
impl_resolver_singleton!(collect, FEngineLoopTick);
impl_resolver_singleton!(PEImage, FEngineLoopTick, |ctx| async {
    let strings = ["DeferredTickTime\0", "ConcurrentWithSlateTickTasks_Wait\0"];
    let strings: Vec<_> = join_all(strings.map(|s| ctx.scan(util::utf8_pattern(s))))
        .await
        .into_iter()
        .flatten()
        .collect();
    let refs = util::scan_xrefs(ctx, &strings).await;
    let fns = util::root_functions(ctx, &refs)?;
    Ok(Self(ensure_one(fns)?))
});
impl_resolver_singleton!(ElfImage, FEngineLoopTick, |_ctx| async {
    super::bail_out!("ElfImage unimplemented");
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UGameEngineTick(pub usize);
impl_resolver_singleton!(collect, UGameEngineTick);

impl_resolver_singleton!(PEImage, UGameEngineTick, |ctx| async {
    use crate::Result;
    use patternsleuth_scanner::Pattern;

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

// on linux we use u16"causeevent="
impl_resolver_singleton!(ElfImage, UGameEngineTick, |ctx| async {
    let strings = ["causeevent=\0", "CAUSEEVENT \0"];
    let strings: Vec<_> = join_all(strings.map(|s| ctx.scan(util::utf16_pattern(s))))
        .await
        .into_iter()
        .flatten()
        .collect();

    let refs = util::scan_xrefs(ctx, &strings).await;

    let fns = util::root_functions(ctx, &refs)?;

    Ok(UGameEngineTick(ensure_one(fns)?))
});

/// int32_t FEngineLoop::Init(class FEngineLoop* this)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FEngineLoopInit(pub usize);
impl_resolver_singleton!(collect, FEngineLoopInit);

impl_resolver_singleton!(PEImage, FEngineLoopInit, |ctx| async {
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

impl_resolver_singleton!(ElfImage, FEngineLoopInit, |ctx| async {
    let search_strings = [
        util::utf8_pattern("FEngineLoop::Init\0"),
        // this is a standalone function called by FEngineLoopInit
        // util::utf16_pattern("Failed to load UnrealEd Engine class '%s'."),
        util::utf16_pattern("One or more modules failed PostEngineInit"),
    ];
    let strings = join_all(search_strings.into_iter().map(|s| ctx.scan(s)))
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    let refs = util::scan_xrefs(ctx, &strings).await;
    let fns = util::root_functions(ctx, &refs)?;
    Ok(Self(ensure_one(fns)?))
});
