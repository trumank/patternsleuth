use crate::resolvers::{ensure_one, impl_resolver_singleton, unreal::util};

/// ```
/// class UObject * __cdecl StaticFindObjectFast(class UClass *, class UObject *, class FName, bool, bool, enum EObjectFlags, enum EInternalObjectFlags)
/// class UObject * __cdecl StaticFindObjectFast(class UClass *, class UObject *, class FName, bool, enum EObjectFlags, enum EInternalObjectFlags)
/// class UObject * __cdecl StaticFindObjectFast(class UClass *, class UObject *, class FName, bool, bool, enum EObjectFlags)
/// ```
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct StaticFindObjectFast(pub u64);
impl_resolver_singleton!(collect, StaticFindObjectFast);

impl_resolver_singleton!(PEImage, StaticFindObjectFast, |ctx| async {
    let strings = ctx.scan(util::utf16_pattern("Illegal call to StaticFindObjectFast() while serializing object data or garbage collecting!\0")).await;

    let refs = util::scan_xrefs(ctx, &strings).await;
    let fns = util::root_functions(ctx, &refs)?;
    Ok(Self(ensure_one(fns)?))
});

impl_resolver_singleton!(ElfImage, StaticFindObjectFast, |ctx| async {
    let strings = ctx.scan(util::utf16_pattern("Illegal call to StaticFindObjectFast() while serializing object data or garbage collecting!\0")).await;

    // Clang commonly outlines the fatal/logging path which owns the diagnostic
    // string. The string xrefs consequently identify one or more cold helpers,
    // not StaticFindObjectFast itself. Find the functions which call those
    // helpers and follow their final direct branch. The checked wrappers have
    // different error paths but converge on the same implementation.
    let refs = util::scan_xrefs(ctx, &strings).await;
    let cold_helpers = util::root_functions(ctx, &refs)?;
    let wrapper_refs = util::scan_xcalls(ctx, &cold_helpers).await;
    let mut wrappers = util::root_functions(ctx, &wrapper_refs)?;
    wrappers.sort_unstable();
    wrappers.dedup();

    let tail_targets = wrappers
        .into_iter()
        .filter_map(|wrapper| {
            let calls = util::find_calls(ctx.image(), wrapper).ok()?;
            calls.last().copied()
        })
        .map(|call| call.callee)
        .filter(|target| !cold_helpers.contains(target))
        .collect::<Vec<_>>();

    // There can be unrelated users of an outlined assertion helper. The real
    // fast lookup is the converging tail target used by multiple checked
    // wrappers; one-off tail targets are other assertion users.
    let converging_targets = tail_targets.iter().copied().filter(|target| {
        tail_targets
            .iter()
            .filter(|candidate| *candidate == target)
            .count()
            > 1
    });

    Ok(Self(ensure_one(converging_targets)?))
});
