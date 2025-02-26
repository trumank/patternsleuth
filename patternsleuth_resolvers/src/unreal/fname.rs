use std::fmt::Debug;

use futures::future::join_all;

use patternsleuth_scanner::Pattern;

use crate::{Result, ensure_one, impl_resolver_singleton, try_ensure_one, unreal::util};

/// public: __cdecl FName::FName(wchar_t const *, enum EFindName)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNameCtorWchar(pub usize);
impl_resolver_singleton!(collect, FNameCtorWchar);

// for linux we find a function caontains following strings
/*
FEngineLoop::LoadPreInitModules:
 FModuleManager::LoadModule called with following FName
    Engine
    Renderer
    AnimGraphRuntime
    Landscape
    RenderCore
*/
impl_resolver_singleton!(ElfImage, FNameCtorWchar, |ctx| async {
    use crate::ResolveError;
    use std::collections::HashSet;

    let strings = [
        "\0Engine\0",
        "\0Renderer\0",
        "\0AnimGraphRuntime\0",
        "\0Landscape\0",
        "\0RenderCore\0",
    ];

    // find the strings
    let strings = join_all(strings.iter().map(|s| ctx.scan(util::utf16_pattern(s)))).await;
    let strings: Vec<Vec<_>> = strings
        .into_iter()
        .map(|pats| pats.into_iter().map(|addr| addr + 2).collect())
        .collect();
    //eprintln!("Find each pattern @ {:?}", strings);
    // find refs to them
    let refs: Vec<_> = join_all(strings.iter().map(|addr| util::scan_xrefs(ctx, addr))).await;
    //eprintln!("Find pattern refs @ {:?}", refs);
    let fns: Vec<_> = refs
        .into_iter()
        .flat_map(|addr| util::root_functions(ctx, &addr).ok())
        .collect();
    //eprintln!("Find pattern fns @ {:?}", fns);
    //strings.into_iter().map(|addr| async move { util::root_functions(ctx, &util::scan_xrefs(ctx, &addr).await ) } ).collect();

    // find fns of these refs
    let fns = fns
        .into_iter()
        .reduce(|x, y| {
            let x: HashSet<usize> = HashSet::from_iter(x.into_iter());
            let y: HashSet<usize> = HashSet::from_iter(y.into_iter());
            x.intersection(&y).cloned().collect::<Vec<_>>()
        })
        .unwrap();

    // output fns
    //eprintln!("Found all fns at {:?}", fns);
    let fnLoadPreInitModules = ensure_one(fns)?;
    let pattern = Pattern::new("ba 01 00 00 00 e8 | ?? ?? ?? ??").unwrap();
    // found fLoadPreInitModules, try find target
    /*
        03f30310 53              PUSH       RBX
        03f30311 48  83  ec       SUB        RSP ,0x30
                 30
        03f30315 e8  c6  25       CALL       FUN_06c928e0                                     undefined FUN_06c928e0()
                 d6  02
        03f3031a 48  89  c3       MOV        RBX ,RAX
        03f3031d 48  8d  7c       LEA        RDI => local_10 ,[RSP  + 0x28 ]
                 24  28
        03f30322 be  38  8a       MOV        ESI ,u_Engine_00868a38                           = u"Engine"
                 86  00
        03f30327 ba  01  00       MOV        EDX ,0x1 <--- pat
                 00  00
        03f3032c e8  af  71       CALL       FName::FName     <- call                                void FName(undefined8 * this, us
                 dc  02
    */
    let mem = ctx
        .image()
        .memory
        .get_section_containing(fnLoadPreInitModules)
        .unwrap();
    let index = fnLoadPreInitModules - mem.address();
    let mut result = None;
    for i in 0..48 {
        if pattern.is_match(mem.data(), mem.address(), index + i) {
            result = ctx
                .image()
                .memory
                .rip4(fnLoadPreInitModules + i + pattern.custom_offset)
                .ok();
        }
    }
    // how to scan code from X?
    let result = result.ok_or(ResolveError::Msg("cannot find address".into()))?;
    /*
    Post check
    if util::root_functions(ctx, &[result]).unwrap()[0] == result {
        eprintln!("ok!!");
    }
    */
    Ok(Self(result))
});

impl_resolver_singleton!(PEImage, FNameCtorWchar, |ctx| async {
    use crate::Context;
    use futures::join;
    use iced_x86::{Code, Decoder, DecoderOptions};

    let strings = async {
        let strings = ["TGPUSkinVertexFactoryUnlimited\0", "MovementComponent0\0"];
        join_all(strings.iter().map(|s| ctx.scan(util::utf16_pattern(s)))).await
    };
    let patterns = async {
        ctx.scan(Pattern::new("EB 07 48 8D 15 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 41 B8 01 00 00 00 E8 | ?? ?? ?? ??").unwrap()).await
    };
    let (patterns, strings) = join!(patterns, strings);

    // sometimes the call gets inlined so use patterns if any match
    if !patterns.is_empty() {
        return Ok(Self(try_ensure_one(
            patterns
                .iter()
                .map(|a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) }),
        )?));
    }

    #[derive(Clone, Copy)]
    enum Tag {
        Direct,
        FirstCall,
    }

    let refs = join_all(strings.iter().flatten().flat_map(|s| {
        [
            (
                Tag::FirstCall,
                format!("48 8d 15 X0x{s:x} 4c 8d 05 ?? ?? ?? ?? 41 b1 01 e8 | ?? ?? ?? ??"),
            ),
            (
                Tag::Direct,
                format!("48 8d 15 X0x{s:x} 48 8d 0d ?? ?? ?? ?? e8 | ?? ?? ?? ??"),
            ),
            (
                Tag::Direct,
                format!(
                    "41 b8 01 00 00 00 48 8d 15 X0x{s:x} 48 8d 0d ?? ?? ?? ?? e9 | ?? ?? ?? ??"
                ),
            ),
        ]
        .into_iter()
        .map(|(t, p)| ctx.scan_tagged2(t, Pattern::new(p).unwrap()))
    }))
    .await;

    Ok(Self(try_ensure_one(refs.iter().flatten().map(
        |(tag, address)| {
            let f = ctx.image().memory.rip4(*address)?;
            match tag {
                Tag::Direct => Ok(f),
                Tag::FirstCall => {
                    let bytes = ctx.image().memory.range(f..f + 0x200)?;
                    let mut decoder = Decoder::with_ip(64, bytes, f as u64, DecoderOptions::NONE);

                    decoder
                        .iter()
                        .find_map(|i| {
                            (i.code() == Code::Call_rel32_64)
                                .then_some(i.memory_displacement64() as usize)
                        })
                        .context("did not find CALL instruction")
                }
            }
        },
    ))?))
});

/// Can be either of the following:
/// `public: class FString __cdecl FName::ToString(void) const`
/// `public: void __cdecl FName::ToString(class FString &) const`
///
/// They take the same arguments and either can be used as long as the return value isn't used.
///
/// !! Be aware anyone try play with this code in Linux, they're different and you should stick with the
/// second one.
///
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNameToString(pub usize);
impl_resolver_singleton!(collect, FNameToString);

impl_resolver_singleton!(ElfImage, FNameToString, |ctx| async {
    let strings = ctx.scan(util::utf16_pattern("SkySphereMesh\0")).await;
    let str_addr = ensure_one(strings)?;
    let pattern = Pattern::new(format!(
        "e8 | ?? ?? ?? ?? 49 8b 5f 10 48 8d 7c 24 30 be 0x{str_addr:08x}"
    ))
    .unwrap();
    let refs = ctx.scan(pattern).await;
    Ok(Self(try_ensure_one(
        refs.into_iter().map(|a| Ok(ctx.image().memory.rip4(a)?)),
    )?))
});

impl_resolver_singleton!(PEImage, FNameToString, |ctx| async {
    use crate::Context;
    use futures::join;
    use iced_x86::{Code, Decoder, DecoderOptions};

    let patterns = async {
        let patterns = ["56 57 48 83 EC 28 48 89 D6 48 89 CF 83 79 ?? 00 74"];

        join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap())))
            .await
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
    };

    let string = async {
        // Locates either variant by searching for a string ref and finding the first function
        // call directly above it. Which variant depends on how much inlining has occured
        let s = Pattern::from_bytes(
            "  DrivingBone: %s\nDrivenParamet"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect(),
        )
        .unwrap();
        let strings = ctx.scan(s).await;

        let refs = join_all(
            strings
                .iter()
                .map(|s| ctx.scan(Pattern::new(format!("48 8d 15 X0x{s:x}")).unwrap())),
        )
        .await;

        let fn_gather_debug_data = ensure_one(
            refs.into_iter()
                .flatten()
                .map(|r| -> Result<_> {
                    Ok(ctx.image().get_root_function(r)?.map(|f| f.range.start..r))
                })
                .collect::<Result<Vec<_>>>()? // TODO avoid this collect?
                .into_iter()
                .flatten(),
        )?;

        let bytes = ctx.image().memory.range(fn_gather_debug_data.clone())?;

        let mut decoder = Decoder::with_ip(
            64,
            bytes,
            fn_gather_debug_data.start as u64,
            DecoderOptions::NONE,
        );

        let addr = decoder
            .iter()
            .filter_map(|i| {
                (i.code() == Code::Call_rel32_64).then_some(i.memory_displacement64() as usize)
            })
            .last()
            .context("did not find CALL instruction")?;

        let res: Result<usize> = Ok(addr);

        res
    };

    let any = join!(
        ctx.resolve(FNameToStringFString::resolver()),
        ctx.resolve(FNameToStringVoid::resolver()),
        string,
        patterns,
    );

    // use pattern if found
    if !any.3.is_empty() {
        return Ok(Self(ensure_one(any.3)?));
    }

    Ok(FNameToString(
        any.0.map(|r| r.0).or(any.1.map(|r| r.0)).or(any.2)?,
    ))
});

/// public: class FString __cdecl FName::ToString(void) const
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNameToStringVoid(pub usize);
impl_resolver_singleton!(all, FNameToStringVoid, |ctx| async {
    let patterns = [
        "E8 | ?? ?? ?? ?? ?? 01 00 00 00 ?? 39 ?? 48 0F 8E",
        "E8 | ?? ?? ?? ?? BD 01 00 00 00 41 39 6E ?? 0F 8E",
        "E8 | ?? ?? ?? ?? 48 8B 4C 24 ?? 8B FD 48 85 C9",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(FNameToStringVoid(try_ensure_one(
        res.iter()
            .flatten()
            .map(|a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) }),
    )?))
});

/// public: void __cdecl FName::ToString(class FString &) const
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNameToStringFString(pub usize);
impl_resolver_singleton!(all, FNameToStringFString, |ctx| async {
    let patterns =
        ["48 8b 48 ?? 48 89 4c 24 ?? 48 8d 4c 24 ?? e8 | ?? ?? ?? ?? 83 7c 24 ?? 00 48 8d"];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(FNameToStringFString(try_ensure_one(
        res.iter()
            .flatten()
            .map(|a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) }),
    )?))
});

/// FNamePool
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNamePool(pub usize);
impl_resolver_singleton!(all, FNamePool, |ctx| async {
    let patterns = [
        "74 ?? 4C 8D 05 | ?? ?? ?? ?? EB ?? 48 8D 0D",
        "48 8d 0d | ?? ?? ?? ?? e8 ?? ?? ?? ?? 48 8b d0 c6 05 dc ?? ?? ?? ?? 48 8b 44 24 30 48 c1 e8 20 03 c0 48 03 44 da 10 48 83 c4 20 5b c3",
        "48 8d 2d | ?? ?? ?? ?? ?? ?? ?? ?? 48 bf cd cc cc cc cc cc cc",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(Self(try_ensure_one(res.iter().flatten().map(
        |a| -> Result<usize> { Ok(ctx.image().memory.rip4(*a)?) },
    ))?))
});
