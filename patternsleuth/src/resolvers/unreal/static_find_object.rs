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
pub struct StaticFindObjectFast(pub usize);
impl_resolver_singleton!(all, StaticFindObjectFast, |ctx| async {
    let strings = ctx.scan(util::utf16_pattern("Illegal call to StaticFindObjectFast() while serializing object data or garbage collecting!\0")).await;

    let refs = util::scan_xrefs(ctx, &strings).await;
    let fns = util::root_functions(ctx, &refs)?;
    Ok(Self(ensure_one(fns)?))
});
