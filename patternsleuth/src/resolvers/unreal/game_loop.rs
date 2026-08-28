use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;

use futures::future::join_all;

use crate::resolvers::{bail_out, ensure_one, impl_resolver_singleton, unreal::util};

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct Main(pub u64);
impl_resolver_singleton!(collect, Main);
impl_resolver_singleton!(PEImage, Main, |ctx| async {
    let strings = ctx.scan(util::utf16_pattern("UnrealEngine4\0")).await;
    let refs = util::scan_xrefs(ctx, &strings).await;
    let fns = util::root_functions(ctx, &refs)?;
    Ok(Self(ensure_one(fns)?))
});
impl_resolver_singleton!(ElfImage, Main, |_ctx| async {
    crate::resolvers::bail_out!("ElfImage unimplemented");
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FEngineLoopTick(pub u64);
impl_resolver_singleton!(collect, FEngineLoopTick);
impl_resolver_singleton!(PEImage, FEngineLoopTick, |ctx| async {
    // intersect the always-present anchor with each secondary and union the results
    let primary = "t.IdleWhenNotForeground\0";
    let secondaries = ["r.OneFrameThreadLag\0", "FEngineLoop::Tick.Benchmarking\0"];

    let primary_strings = ctx.scan(util::utf16_pattern(primary)).await;
    let primary_fns = util::root_functions(ctx, &util::scan_xrefs(ctx, &primary_strings).await)?;

    let secondary_fns: Vec<Vec<_>> = join_all(secondaries.map(|s| async move {
        let strings = ctx.scan(util::utf16_pattern(s)).await;
        util::root_functions(ctx, &util::scan_xrefs(ctx, &strings).await)
    }))
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;

    let mut candidates: Vec<u64> = vec![];
    for sec in &secondary_fns {
        for f in &primary_fns {
            if sec.contains(f) && !candidates.contains(f) {
                candidates.push(*f);
            }
        }
    }

    Ok(Self(ensure_one(candidates)?))
});
impl_resolver_singleton!(ElfImage, FEngineLoopTick, |_ctx| async {
    crate::resolvers::bail_out!("ElfImage unimplemented");
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UGameEngineTick(pub u64);
impl_resolver_singleton!(collect, UGameEngineTick);

impl_resolver_singleton!(PEImage, UGameEngineTick, |ctx| async {
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
pub struct FEngineLoopInit(pub u64);
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

/// void UWorld::Tick(ELevelTick TickType, float DeltaSeconds)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UWorldTick(pub u64);
impl_resolver_singleton!(collect, UWorldTick);
impl_resolver_singleton!(PEImage, UWorldTick, |ctx| async {
    const UWORLD_TICK_SPECIFIC: &[&str] = &[
        "TickInGamePerfTrackersRT\0",
        "UWorld_Tick\0",
        "%d PAWN SPAWNS THIS FRAME! \0",
        "World Tick Time\0",
        "Tick Time\0",
        "GT Tickable Time\0",
        "Net Tick Time\0",
        "Nav Tick Time\0",
        "Update Camera Time\0",
        "Finish Async Trace Time\0",
        "Reset Async Trace Time\0",
        "Net Broadcast Tick Time\0",
        "TickGroups\0",
    ];

    const UWORLD_TICK_GENERIC: &[&str] = &[
        "Your connection to the host has been lost.\0",
        "ConnectionFailed\0",
        "Media\0",
    ];

    let n_specific = UWORLD_TICK_SPECIFIC.len();

    let str_results = join_all(
        UWORLD_TICK_SPECIFIC
            .iter()
            .chain(UWORLD_TICK_GENERIC.iter())
            .enumerate()
            .map(|(i, s)| ctx.scan_tagged(i, util::utf16_pattern(s))),
    )
    .await;

    let xref_results = join_all(
        str_results
            .into_iter()
            .map(
                |(idx, _pattern, addrs)| async move { (idx, util::scan_xrefs(ctx, &addrs).await) },
            ),
    )
    .await;

    // bin each xref to its containing root function
    let mut fn_anchors: HashMap<u64, HashSet<usize>> = HashMap::new();
    let mut fn_size: HashMap<u64, u64> = HashMap::new();
    for (idx, refs) in xref_results {
        for r in refs {
            if let Some(f) = ctx.image().get_root_function(r)? {
                fn_anchors.entry(f.range.start).or_default().insert(idx);
                fn_size
                    .entry(f.range.start)
                    .or_insert(f.range.end - f.range.start);
            }
        }
    }
    if fn_anchors.is_empty() {
        bail_out!("no UWorld::Tick anchor string referenced inside a function");
    }

    // rank key: (#specific, #total, #generic, span), all descending.
    let score = |fn_addr: &u64| {
        let set = &fn_anchors[fn_addr];
        let spec = set.iter().filter(|&&i| i < n_specific).count();
        (spec, set.len(), set.len() - spec, fn_size[fn_addr])
    };

    let mut fns = fn_anchors.keys().copied().collect::<Vec<_>>();
    fns.sort_by_key(|f| Reverse(score(f)));
    let best = fns[0];
    let (best_spec, best_total, _best_gen, best_size) = score(&best);

    if best_spec == 0 && (best_total < 2 || best_size < 0x800) {
        bail_out!("only generic net anchors on a small fn");
    }

    // require a strict winner
    if fns.len() > 1 && score(&fns[1]) == score(&best) {
        bail_out!("ambiguous UWorld::Tick candidates");
    }

    Ok(UWorldTick(best))
});
impl_resolver_singleton!(ElfImage, UWorldTick, |_ctx| async {
    crate::resolvers::bail_out!("ElfImage unimplemented");
});
