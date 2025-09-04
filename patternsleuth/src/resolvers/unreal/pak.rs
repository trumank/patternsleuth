use std::fmt::Debug;

use futures::future::join_all;

use crate::resolvers::{ensure_one, impl_resolver_singleton, unreal::util};

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FPakPlatformFileInitialize(pub u64);
impl_resolver_singleton!(collect, FPakPlatformFileInitialize);
impl_resolver_singleton!(PEImage, FPakPlatformFileInitialize, |ctx| async {
    let string_xrefs = |strings: &'static [&'static str]| async {
        let strings: Vec<_> = join_all(strings.iter().map(|s| ctx.scan(util::utf16_pattern(s))))
            .await
            .into_iter()
            .flatten()
            .collect();
        let refs = util::scan_xrefs(ctx, &strings).await;
        ensure_one(util::root_functions(ctx, &refs)?)
    };

    let (a, b) = futures::join!(
        string_xrefs(&["%sPaks/%s-\0"]),
        string_xrefs(&[
            "ushaderbytecode\0",
            "%sPaks/global\0",
            "fileopenlog\0",
            "Signedpak\0"
        ]),
    );
    Ok(Self(b.or(a)?))
});
impl_resolver_singleton!(ElfImage, FPakPlatformFileInitialize, |_ctx| async {
    super::bail_out!("ElfImage unimplemented");
});
