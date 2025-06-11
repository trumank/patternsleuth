use std::fmt::Debug;

use futures::{future::join_all, join};

use patternsleuth_scanner::Pattern;

use crate::{
    resolvers::{ensure_one, impl_resolver, impl_resolver_singleton, Context},
    Addressable, Matchable,
};

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct BlueprintLibraryInit {
    pub uclass_compiled_in_defer: usize,
    pub uobject_compiled_in_defer: usize,
    pub construct_uclass: usize,
    pub get_private_static_class_body: usize,
    pub uobject_static_class: usize,
    pub ublueprint_function_library_static_class: usize,
}

impl_resolver!(all, BlueprintLibraryInit, |ctx| async {
    let mem = &ctx.image().memory;

    let class_str = Pattern::from_bytes(
        "UKismetStringTableLibrary\x00"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect(),
    )
    .unwrap();

    let class_str = ctx.scan(class_str).await;

    let pattern_dyn_init_class = |s: usize| {
        Pattern::new(format!("41 b9 ?? ?? ?? ?? 48 8d 15 X0x{s:x} 41 b8 28 00 00 00 48 8d 0d [ ?? ?? ?? ?? ] e9 [ ?? ?? ?? ?? ]")).unwrap()
    };
    let pattern_dyn_init_object = |s: usize| {
        Pattern::new(format!(
            "
                48 83 ec 48
                33 c0
                4c 8d 0d ?? ?? ?? ??
                48 89 44 24 30
                4c 8d 05 X0x{s:x}
                48 89 44 24 28
                48 8d 15 [ ?? ?? ?? ?? ]
                48 8d 0d [ ?? ?? ?? ?? ]
                88 44 24 20
                e8 [ ?? ?? ?? ?? ]
                48 83 c4 48
                c3
            "
        ))
        .unwrap()
    };

    let construct_uclass_pattern = Pattern::new(
        "
            48 83 ec 28
            48 8b 05 ?? ?? ?? ??
            48 85 c0
            75 1a
            48 8d 15 ?? ?? ?? ??
            48 8d 0d ?? ?? ?? ??
            e8 [ ?? ?? ?? ?? ]
            48 8b 05 ?? ?? ?? ??
            48 83 c4 28
            c3
        ",
    )
    .unwrap();

    let (init_class_refs, init_object_refs) = join!(
        join_all(
            class_str
                .iter()
                .map(|s| ctx.scan_tagged((), pattern_dyn_init_class(*s)))
        ),
        join_all(
            class_str
                .iter()
                .map(|s| ctx.scan_tagged((), pattern_dyn_init_object(*s)))
        )
    );

    let uclass_compiled_in_defer =
        ensure_one(init_class_refs.into_iter().flat_map(|(_, p, a)| {
            a.into_iter()
                .flat_map(move |a| mem.captures(&p, a).ok().flatten().map(|c| c[1].rip()))
        }))?;
    let (get_private_static_class_wrapper, construct_uclass_wrapper, uobject_compiled_in_defer) =
        ensure_one(init_object_refs.into_iter().flat_map(|(_, p, a)| {
            a.into_iter().flat_map(move |a| {
                mem.captures(&p, a)
                    .ok()
                    .flatten()
                    .map(|c| (c[0].rip(), c[1].rip(), c[2].rip()))
            })
        }))?;

    let construct_uclass = mem
        .captures(&construct_uclass_pattern, construct_uclass_wrapper)?
        .context("Construct_UClass pattern did not match")?[0]
        .rip();

    let get_private_static_class_pattern = Pattern::new(
        "
            4c 8b dc
            48 81 ec 88 00 00 00
            48 8b 05 ?? ?? ?? ??
            48 85 c0
            0f 85 90 00 00 00
            33 c9
            48 8d 05 [ ?? ?? ?? ?? ]
            49 89 4b f0
            4c 8d 0d ?? ?? ?? ??
            88 4c 24 70
            4c 8d 05 ?? ?? ?? ??
            49 89 43 e0
            48 8d 15 ?? ?? ?? ??
            48 8d 05 [ ?? ?? ?? ?? ]
            49 89 43 d8
            48 8d 05 ?? ?? ?? ??
            49 89 43 d0
            48 8d 05 ?? ?? ?? ??
            49 89 43 c8
            48 8d 05 ?? ?? ?? ??
            49 89 43 c0
            48 8d 05 ?? ?? ?? ??
            49 89 43 b8
            49 89 4b b0
            48 8d 0d ?? ?? ?? ??
            c7 44 24 30 ?? ?? ?? ??
            c7 44 24 28 ?? ?? ?? ??
            c7 44 24 20 ?? ?? ?? ??
            e8 [ ?? ?? ?? ?? ]
            48 8b 05 ?? ?? ?? ??
            48 81 c4 88 00 00 00
            c3
    ",
    )
    .unwrap();

    let (
        uobject_static_class,
        ublueprint_function_library_static_class,
        get_private_static_class_body,
    ) = {
        let captures = mem
            .captures(
                &get_private_static_class_pattern,
                get_private_static_class_wrapper,
            )?
            .context("Construct_UClass pattern did not match")?;

        (captures[0].rip(), captures[1].rip(), captures[2].rip())
    };

    Ok(Self {
        uclass_compiled_in_defer,
        uobject_compiled_in_defer,
        construct_uclass,
        get_private_static_class_body,
        uobject_static_class,
        ublueprint_function_library_static_class,
    })
});

/// UFunction::Bind
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct UFunctionBind(pub usize);
impl_resolver_singleton!(collect, UFunctionBind);
impl_resolver_singleton!(PEImage, UFunctionBind, |ctx| async {
    use crate::resolvers::unreal::util;

    let string = async {
        let strings = ctx
            .scan(util::utf16_pattern(
                "Failed to bind native function %s.%s\0",
            ))
            .await;
        let refs = util::scan_xrefs(ctx, &strings).await;
        util::root_functions(ctx, &refs)
    };

    let pattern = async {
        let patterns = [
            "48 89 5C 24 ?? 57 48 83 EC 20 33 D2 48 8B F9 48 8B D9 48 85 C9 74 3D 48 85 D2 75 38 48 85 DB 74 28 E8",
        ];

        join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await
    };

    let (string, pattern) = join!(string, pattern);

    Ok(Self(ensure_one(
        string?.into_iter().chain(pattern.into_iter().flatten()),
    )?))
});

impl_resolver_singleton!(ElfImage, UFunctionBind, |ctx| async {
    // maybe find symbol of vtable?
    let pattern = Pattern::new("41 56 53 50 49 89 fe 48 89 fb 66 0f 1f 44 00 00 e8 ?? ?? ?? ?? 48 8b 4b 10 48 63 50 38 3b 51 38 7e ?? 31 c0 48 8b 5b 20 48 85 db 75 ?? eb ?? 90 48 83 c0 30").unwrap();
    let fns = ctx.scan(pattern).await;
    Ok(Self(ensure_one(fns)?))
});
