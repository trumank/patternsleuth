use std::fmt::Debug;

use futures::{future::join_all, try_join};

use patternsleuth_scanner::Pattern;

use crate::{
    resolvers::{ensure_one, impl_resolver_singleton, try_ensure_one, unreal::util, Result},
    MemoryAccessorTrait,
};

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct GUObjectArray(pub usize);
impl_resolver_singleton!(@all GUObjectArray, |ctx| async {
    let patterns = [
        "74 ?? 48 8D 0D | ?? ?? ?? ?? C6 05 ?? ?? ?? ?? 01 E8 ?? ?? ?? ?? C6 05 ?? ?? ?? ?? 01",
        "75 ?? 48 ?? ?? 48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 45 33 C9 4C 89 74 24",
        "45 84 c0 48 c7 41 10 00 00 00 00 b8 ff ff ff ff 4c 8d 1d | ?? ?? ?? ?? 89 41 08 4c 8b d1 4c 89 19 0f 45 05 ?? ?? ?? ?? ff c0 89 41 08 3b 05",
        "81 ce 00 00 00 02 83 e0 fb 89 47 08 48 8d 0d | ?? ?? ?? ?? 48 89 fa 45 31 c0 e8 ?? ?? ?? ??",
    ];
    // mov imm32 pattern for linux
    let patterns1 = [
        /*
        06f97b32 41  39  ee       CMP        R14D ,EBP
        06f97b35 0f  8e  7e       JLE        LAB_06f97cb9
                 01  00  00
        06f97b3b bf  d8  c5       MOV        param_1 ,GUObjectArray
                 ac  0b
        06f97b40 48  8b  74       MOV        param_2 ,qword ptr [RSP  + local_68 ]
                 24  10
        06f97b45 e8  16  d2       CALL       FUN_06f64d60                                     undefined FUN_06f64d60()
                 fc  ff
        06f97b4a e9  6a  01       JMP        LAB_06f97cb9
                 00  00
         */
        "41 39 ee 0f 8e ?? ?? ?? ?? bf | ?? ?? ?? ?? 48 8b 74 24 10 e8 ?? ?? ?? ?? e9",
        /*
        06fa15c8 8b  6f  3c       MOV        EBP ,dword ptr [RDI  + 0x3c ]
        06fa15cb 4c  89  f7       MOV        RDI ,R14
        06fa15ce 31  f6           XOR        ESI ,ESI
        06fa15d0 e8  1b  09       CALL       FUN_06fa1ef0                                     undefined FUN_06fa1ef0()
                 00  00
        06fa15d5 41  39  ef       CMP        R15D ,EBP
        06fa15d8 7e  0d           JLE        LAB_06fa15e7
        06fa15da bf  d8  c5       MOV        EDI ,GUObjectArray
                 ac  0b
        06fa15df 48  89  de       MOV        RSI ,RBX
        06fa15e2 e8  79  37       CALL       FUN_06f64d60                                     undefined FUN_06f64d60()
                 fc  ff
         */
        "8b 6f ?? 4c 89 f7 31 f6 e8 ?? ?? ?? ?? 41 39 ef 7e 0d bf | ?? ?? ?? ?? 48 89 de e8",
    ];
    let res0 = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;
    let res1 = join_all(patterns1.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;
    let res1 = res1.iter().flatten().map(|a| -> Result<usize> {Ok(ctx.image().memory.u32_le(*a)? as usize)} );
    Ok(GUObjectArray(try_ensure_one(res0.iter().flatten().map(
        |a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) },
    ).chain(res1))?))
});

/// public: void __cdecl FUObjectArray::AllocateUObjectIndex(class UObjectBase *, bool)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FUObjectArrayAllocateUObjectIndex(pub usize);
impl_resolver_singleton!(@all FUObjectArrayAllocateUObjectIndex, |ctx| async {
    let strings = ctx
        .scan(util::utf16_pattern(
            "Unable to add more objects to disregard for GC pool (Max: %d)\0",
        ))
        .await;
    let refs = util::scan_xrefs(ctx, &strings).await;
    let fns = util::root_functions(ctx, &refs)?;
    Ok(Self(ensure_one(fns)?))
});

/// public: void __cdecl FUObjectArray::FreeUObjectIndex(class UObjectBase *)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FUObjectArrayFreeUObjectIndex(pub usize);
impl_resolver_singleton!(@all FUObjectArrayFreeUObjectIndex, |ctx| async {
    let refs_future = async {
        let search_strings = [
            "Removing object (0x%016llx) at index %d but the index points to a different object (0x%016llx)!",
            "Unexpected concurency while adding new object",
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
        Ok(util::scan_xrefs(ctx, &strings).await)
    };

    // same string is present in both functions so resolve the other so we can filter it out
    let (allocate_uobject, refs) = try_join!(
        ctx.resolve(FUObjectArrayAllocateUObjectIndex::resolver()),
        refs_future,
    )?;

    let fns = refs
        .into_iter()
        .map(|r| -> Result<_> { Ok(ctx.image().get_root_function(r)?.map(|f| f.range.start)) })
        .collect::<Result<Vec<_>>>()? // TODO avoid this collect?
        .into_iter()
        .flatten()
        .filter(|f| *f != allocate_uobject.0);

    Ok(FUObjectArrayFreeUObjectIndex(ensure_one(fns)?))
});

/// void __cdecl UObjectBaseShutdown(void)
/// could be used to determine object listener offsets, but only for recent UE versions
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UObjectBaseShutdown(pub usize);
impl_resolver_singleton!(@all UObjectBaseShutdown, |ctx| async {
    let strings = ctx
        .scan(util::utf16_pattern(
                "All UObject delete listeners should be unregistered when shutting down the UObject array\0"
        ))
        .await;
    let refs = util::scan_xrefs(ctx, &strings).await;
    let fns = util::root_functions(ctx, &refs)?;
    #[cfg(target_os="linux")]
    let fns = {
        // on linux both functions are not inlined, we need to find the caller
        let callsites = util::scan_xcalls(ctx, &fns).await;
        util::root_functions(ctx, &callsites)?
    };
    Ok(UObjectBaseShutdown(ensure_one(fns)?))
});
