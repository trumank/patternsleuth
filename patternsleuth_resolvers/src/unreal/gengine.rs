use std::fmt::Debug;

use iced_x86::Register;
use itertools::Itertools as _;

use crate::{
    disassemble::{Control, disassemble},
    image::Image,
    {Result, impl_resolver_singleton, try_ensure_one, unreal::util},
};

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct GEngine(pub usize);
impl_resolver_singleton!(collect, GEngine);
impl_resolver_singleton!(PEImage, GEngine, |ctx| async {
    let strings = ctx.scan(util::utf16_pattern("rhi.DumpMemory\0")).await;
    let refs = util::scan_xrefs(ctx, &strings).await;

    fn for_each(img: &Image, addr: usize) -> Result<Option<usize>> {
        let Some(root) = img.get_root_function(addr)? else {
            return Ok(None);
        };
        let f = root.range().start;

        let mut is_match = false;
        let mut rcx = None;

        disassemble(img, f, |inst| {
            let cur = inst.ip() as usize;
            if !(f..=addr).contains(&cur) {
                return Ok(Control::Break);
            }
            if addr == cur && inst.op0_register() == Register::R8 {
                is_match = true;
                return Ok(Control::Break);
            }

            if inst.op0_register() == Register::RCX && inst.is_ip_rel_memory_operand() {
                rcx = Some(inst.ip_rel_memory_address() as usize);
            }

            Ok(Control::Continue)
        })?;

        Ok(is_match.then_some(rcx).flatten())
    }

    Ok(Self(try_ensure_one(
        refs.into_iter()
            .map(|addr| for_each(ctx.image(), addr))
            .flatten_ok(),
    )?))
});
impl_resolver_singleton!(ElfImage, GEngine, |_ctx| async {
    super::bail_out!("ElfImage unimplemented");
});
