use std::fmt::Debug;

use futures::future::join_all;

use patternsleuth_scanner::Pattern;

use crate::resolvers::ResolveError;
use crate::{
    MemoryTrait,
    resolvers::{Result, ensure_one, impl_resolver_singleton, try_ensure_one, unreal::util},
};

/// public: __cdecl FName::FName(wchar_t const *, enum EFindName)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNameCtorWchar(pub u64);
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
    use crate::resolvers::ResolveError;
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
            let x: HashSet<u64> = HashSet::from_iter(x.into_iter());
            let y: HashSet<u64> = HashSet::from_iter(y.into_iter());
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
        if pattern.is_match(mem.data(), mem.address() as usize, (index + i) as usize) {
            result = ctx
                .image()
                .memory
                .rip4(fnLoadPreInitModules + i + pattern.custom_offset as u64)
                .ok();
        }
    }
    // how to scan code from X?
    let result = result.ok_or(ResolveError::new_msg("cannot find address"))?;
    /*
    Post check
    if util::root_functions(ctx, &[result]).unwrap()[0] == result {
        eprintln!("ok!!");
    }
    */
    Ok(Self(result))
});

impl_resolver_singleton!(PEImage, FNameCtorWchar, |ctx| async {
    use crate::{MemoryTrait, resolvers::Context};
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
                .map(|a| -> Result<_> { Ok(ctx.image().memory.rip4(*a)?) }),
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
                    let mut decoder = Decoder::with_ip(64, bytes, f, DecoderOptions::NONE);

                    decoder
                        .iter()
                        .find_map(|i| {
                            (i.code() == Code::Call_rel32_64).then_some(i.memory_displacement64())
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
pub struct FNameToString(pub u64);
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
    use crate::{MemoryTrait, resolvers::Context};
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
            .filter_map(|i| (i.code() == Code::Call_rel32_64).then_some(i.memory_displacement64()))
            .last()
            .context("did not find CALL instruction")?;

        let res: Result<u64> = Ok(addr);

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

    Ok(Self(any.0.map(|r| r.0).or(any.1.map(|r| r.0)).or(any.2)?))
});

/// public: class FString __cdecl FName::ToString(void) const
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNameToStringVoid(pub u64);
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
            .map(|a| -> Result<_> { Ok(ctx.image().memory.rip4(*a)?) }),
    )?))
});

/// public: void __cdecl FName::ToString(class FString &) const
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNameToStringFString(pub u64);
impl_resolver_singleton!(all, FNameToStringFString, |ctx| async {
    let patterns =
        ["48 8b 48 ?? 48 89 4c 24 ?? 48 8d 4c 24 ?? e8 | ?? ?? ?? ?? 83 7c 24 ?? 00 48 8d"];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(FNameToStringFString(try_ensure_one(
        res.iter()
            .flatten()
            .map(|a| -> Result<_> { Ok(ctx.image().memory.rip4(*a)?) }),
    )?))
});

/// FNamePool: resolved address of the global FName storage.
///
/// On UE 4.23+ this is the FNamePool struct's static address. On pre-4.23
/// this is the address of the static pointer to
/// `TStaticIndirectArrayThreadSafeRead<FNameEntry>` (i.e. `&GNames`).
/// Consumers branch on the engine version to interpret the value correctly.
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct FNamePool(pub u64);
impl_resolver_singleton!(all, FNamePool, |ctx| async {
    use futures::join;
    use std::collections::HashSet;

    // Strategy 1: post-4.23 FNamePool::FNamePool
    //
    // Two independent fingerprints, intersected at the end so disagreement
    // surfaces as an error rather than a silent miscatch:
    //
    // (a) Body shape: the FNameEntry-buffer memset at the top of the ctor.
    //     MSVC : `mov [r?+8], r?` immediately followed by `mov r8d, 0x10000`
    //            — the +0x8 zero-store sits right before the 64KB memset.
    //     Clang: `lea rcx, [r?+8]; mov r8d, 0x10008` — Clang folds the +0x8
    //            field into the memset (size 0x10008) and skips the
    //            separate zero-store.
    //     Both land in `FNameEntryAllocator::FNameEntryAllocator`, which is
    //     inlined into FNamePool::FNamePool in some builds (monolithic) and
    //     a separate callee in others (split — Clang/PGO, some MSVC LTCG).
    //     Caller-walking covers the split case; the LEA-CALL filter at the
    //     end drops whichever candidate isn't the FNamePool ctor.
    //
    // (b) String intersection: xrefs to the EName-class narrow strings the
    //     constructor registers. Bracketing nulls discriminate from
    //     `UByteProperty`/etc. and the wide-char EName table. Fails on
    //     builds where the ctor reads names indirectly via a `GetEName(idx)`
    //     trampoline table — body shape covers those.
    let post = async {
        let body = async {
            let body_matches: Vec<u64> = join_all([
                ctx.scan(Pattern::new("4? 89 ?? 08 41 b8 00 00 01 00").unwrap()),
                ctx.scan(Pattern::new("4? 8d 4? 08 41 b8 08 00 01 00").unwrap()),
            ])
            .await
            .into_iter()
            .flatten()
            .collect();
            let body_fns: HashSet<u64> = util::root_functions(ctx, &body_matches)?
                .into_iter()
                .collect();

            let caller_sites: Vec<u64> = join_all(body_fns.iter().flat_map(|f| {
                [
                    ctx.scan(Pattern::new(format!("e8 X0x{f:X}")).unwrap()),
                    ctx.scan(Pattern::new(format!("e9 X0x{f:X}")).unwrap()),
                ]
            }))
            .await
            .into_iter()
            .flatten()
            .collect();
            let caller_fns: HashSet<u64> = util::root_functions(ctx, &caller_sites)?
                .into_iter()
                .collect();

            Ok::<HashSet<u64>, ResolveError>(body_fns.union(&caller_fns).copied().collect())
        };

        let strings = async {
            let strs = [
                "\0ByteProperty\0",
                "\0IntProperty\0",
                "\0BoolProperty\0",
                "\0FloatProperty\0",
            ];
            let str_addrs: Vec<Vec<u64>> =
                join_all(strs.iter().map(|s| ctx.scan(util::utf8_pattern(s))))
                    .await
                    .into_iter()
                    .map(|matches| matches.into_iter().map(|a| a + 1).collect())
                    .collect();
            let xrefs: Vec<Vec<u64>> =
                join_all(str_addrs.iter().map(|addrs| util::scan_xrefs(ctx, addrs))).await;
            let fn_groups: Vec<HashSet<u64>> = xrefs
                .iter()
                .map(|rs| util::root_functions(ctx, rs).map(|fs| fs.into_iter().collect()))
                .collect::<Result<_>>()?;
            Ok::<HashSet<u64>, ResolveError>(
                fn_groups
                    .into_iter()
                    .reduce(|a, b| a.intersection(&b).copied().collect())
                    .unwrap_or_default(),
            )
        };

        let (body_set, str_set) = join!(body, strings);
        let mut candidates = body_set?;
        candidates.extend(str_set?);

        // Strict singleton-init shape: `lea rcx, [pool]; call ctor; mov byte
        // [init_flag], 1`. The trailing `c6 05 ?? ?? ?? ?? 01` is the C++
        // static-local "ran ctor" flag set — present after every Meyers
        // singleton init and absent from arbitrary `lea; call` adjacencies,
        // so it filters out spurious LEA-CALL coincidences. Two variants:
        // adjacent, or with 3 intermediate bytes (`mov rsi/rbx, rax` saving
        // the ctor's return value before storing the flag).
        //
        // The strict filter is what makes caller-walking safe: in split-ctor
        // builds the inner allocator candidate produces 0 strict matches
        // (no singleton init wraps it), and in monolithic builds the
        // singleton-getter wrapper added by caller-walking also produces 0
        // strict matches (it is itself the wrapper, not constructed by
        // one), so only the real FNamePool ctor's static-init sites remain.
        let pool_matches = join_all(candidates.iter().flat_map(|c| {
            [
                ctx.scan(
                    Pattern::new(format!(
                        "48 8d 0d | ?? ?? ?? ?? e8 X0x{c:X} c6 05 ?? ?? ?? ?? 01"
                    ))
                    .unwrap(),
                ),
                ctx.scan(
                    Pattern::new(format!(
                        "48 8d 0d | ?? ?? ?? ?? e8 X0x{c:X} ?? ?? ?? c6 05 ?? ?? ?? ?? 01"
                    ))
                    .unwrap(),
                ),
            ]
        }))
        .await;
        let pool_addrs: HashSet<u64> = pool_matches
            .into_iter()
            .flatten()
            .map(|a| ctx.image().memory.rip4(a))
            .collect::<std::result::Result<_, _>>()?;
        ensure_one(pool_addrs)
    };

    // Strategy 2: pre-4.23 GNames getter
    //
    // GNames is a static pointer to `TStaticIndirectArrayThreadSafeRead<
    // FNameEntry>`, lazily allocated. The getter has a distinctive prologue:
    //   sub rsp, 0x28
    //   mov rax, [rip+disp]    <- captures &GNames
    //   test rax, rax
    //   jne short already_init
    //   mov ecx, <size>         <- size of the indirect-array struct
    // Size depends on template params: UE 4.07–4.21 use 0x408 (128-ptr chunk
    // array), UE 4.22 doubled it to 0x808 (256 ptrs).
    let gnames = async {
        let matches: Vec<u64> = join_all([
            ctx.scan(
                Pattern::new("48 83 EC 28 48 8B 05 | ?? ?? ?? ?? 48 85 C0 75 ?? B9 08 04 00 00")
                    .unwrap(),
            ),
            ctx.scan(
                Pattern::new("48 83 EC 28 48 8B 05 | ?? ?? ?? ?? 48 85 C0 75 ?? B9 08 08 00 00")
                    .unwrap(),
            ),
        ])
        .await
        .into_iter()
        .flatten()
        .collect();
        try_ensure_one(matches.into_iter().map(|a| Ok(ctx.image().memory.rip4(a)?)))
    };

    let (post_result, gnames_result) = join!(post, gnames);
    Ok(Self(post_result.or(gnames_result)?))
});

/// StaticFNameConst
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct StaticFNameConst(pub u64);
impl_resolver_singleton!(all, StaticFNameConst, |ctx| async {
    let strings = ctx.scan(util::utf16_pattern("GLSL_ES3_1_ANDROID\0")).await;
    let str_addr = ensure_one(strings)?;
    let pattern = Pattern::new(format!(
        "41 b8 01 00 00 00 48 8d 15 X0x{str_addr:08x} 48 8d 0d | ?? ?? ?? ?? e9"
    ))
    .unwrap();
    let refs = ctx.scan(pattern).await;
    match refs.len() {
        0 => Err(ResolveError::new_msg("expected at least one value")),
        _ => Ok(StaticFNameConst(ctx.image().memory.rip4(refs[0])?)),
    }
});
