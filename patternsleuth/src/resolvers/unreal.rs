use std::{collections::HashMap, sync::Arc};

use futures::{future::join_all, try_join};
use patternsleuth_scanner::Pattern;

use crate::{
    resolvers::{bail_out, ensure_one, impl_resolver},
    Addressable, Matchable, MemoryAccessorTrait,
};

#[derive(Debug)]
pub struct GUObjectArray(pub usize);
impl_resolver!(GUObjectArray, |ctx| async {
    let patterns = [
        "74 ?? 48 8D 0D | ?? ?? ?? ?? C6 05 ?? ?? ?? ?? 01 E8 ?? ?? ?? ?? C6 05 ?? ?? ?? ?? 01",
        "75 ?? 48 ?? ?? 48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 45 33 C9 4C 89 74 24",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(GUObjectArray(ensure_one(
        res.iter().flatten().map(|a| ctx.image().memory.rip4(*a)),
    )?))
});

#[derive(Debug)]
pub struct FNameToString(pub usize);
impl_resolver!(FNameToString, |ctx| async {
    let patterns = [
        "E8 | ?? ?? ?? ?? ?? 01 00 00 00 ?? 39 ?? 48 0F 8E",
        "E8 | ?? ?? ?? ?? BD 01 00 00 00 41 39 6E ?? 0F 8E",
        "E8 | ?? ?? ?? ?? 48 8B 4C 24 ?? 8B FD 48 85 C9",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(FNameToString(ensure_one(
        res.iter().flatten().map(|a| ctx.image().memory.rip4(*a)),
    )?))
});

/// public: void __cdecl FFrame::Step(class UObject *, void *const)
#[derive(Debug)]
pub struct FFrameStep(pub usize);
impl_resolver!(FFrameStep, |ctx| async {
    let patterns = [
        "48 8B 41 20 4C 8B D2 48 8B D1 44 0F B6 08 48 FF C0 48 89 41 20 41 8B C1 4C 8D 0D ?? ?? ?? ?? 49 8B CA 49 FF 24 C1",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(FFrameStep(ensure_one(res.into_iter().flatten())?))
});

/// public: void __cdecl FFrame::StepExplicitProperty(void *const, class FProperty *)
/// public: void __cdecl FFrame::StepExplicitProperty(void *const, class UProperty *)
#[derive(Debug)]
pub struct FFrameStepExplicitProperty(pub usize);
impl_resolver!(FFrameStepExplicitProperty, |ctx| async {
    let patterns = [
         "41 8B 40 40 4D 8B C8 4C 8B D1 48 0F BA E0 08 73 ?? 48 8B ?? ?? ?? ?? 00 ?? ?? ?? ?? ?? ?? ?? 00 48 8B 40 10 4C 39 08 75 F7 48 8B 48 08 49 89 4A 38 ?? ?? ?? 40 ?? ?? ?? ?? ?? 4C ?? 41 ?? 49",
         "48 89 5C 24 ?? 48 89 ?? 24 ?? 57 48 83 EC 20 41 8B 40 40 49 8B D8 48 8B ?? 48 8B F9 48 0F BA E0 08 73 ?? 48 8B ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 8B 40 10 48 39 18 75 F7 48 8B ?? 08 48 89 ?? 38 48",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(FFrameStepExplicitProperty(ensure_one(
        res.into_iter().flatten(),
    )?))
});

#[derive(Debug)]
pub struct KismetSystemLibrary(HashMap<String, usize>);

impl_resolver!(KismetSystemLibrary, |ctx| async {
    let mem = &ctx.image().memory;

    let s = Pattern::from_bytes(
        "KismetSystemLibrary\x00"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect(),
    )
    .unwrap();
    let strings = ctx.scan(s).await;

    let refs = join_all(strings.iter().map(|s| {
        ctx.scan(
            Pattern::new(format!(
        // fragile (only 4.25-4.27 most likely)
        "4c 8d 0d [ ?? ?? ?? ?? ] 88 4c 24 70 4c 8d 05 ?? ?? ?? ?? 49 89 43 e0 48 8d 15 X0x{:x}",
        s
    ))
            .unwrap(),
        )
    }))
    .await;
    dbg!(&refs);

    let cap = Pattern::new("4c 8d 0d [ ?? ?? ?? ?? ]").unwrap();

    let register_natives_addr = ensure_one(
        refs.iter()
            .flatten()
            .map(|a| ctx.image().memory.captures(&cap, *a).unwrap()[0].rip()),
    )?;

    let register_natives = Pattern::new("48 83 ec 28 e8 ?? ?? ?? ?? 41 b8 [ ?? ?? ?? ?? ] 48 8d 15 [ ?? ?? ?? ?? ] 48 8b c8 48 83 c4 28 e9 ?? ?? ?? ??").unwrap();

    let captures = ctx
        .image()
        .memory
        .captures(&register_natives, register_natives_addr);

    if let Some([num, data]) = captures.as_deref() {
        let mut res = HashMap::new();

        let ptr = data.rip();
        for i in 0..(num.u32() as usize) {
            let a = ptr + i * 0x10;
            res.insert(mem.read_string(mem.ptr(a)), mem.ptr(a + 8));
        }
        Ok(KismetSystemLibrary(res))
    } else {
        bail_out!("did not match");
    }
});

#[derive(Debug)]
pub struct UGameEngineTick(pub usize);

impl_resolver!(UGameEngineTick, |ctx| async {
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
        .flat_map(|r| ctx.image().get_root_function(r).map(|f| f.range.start));

    Ok(UGameEngineTick(ensure_one(fns)?))
});

#[derive(Debug)]
pub struct ConsoleManagerSingleton(usize);

impl_resolver!(ConsoleManagerSingleton, |ctx| async {
    let strings = join_all([
        ctx.scan(
            Pattern::from_bytes(
                "r.DumpingMovie"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            )
            .unwrap(),
        ),
        ctx.scan(
            Pattern::from_bytes(
                "vr.pixeldensity"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            )
            .unwrap(),
        ),
    ])
    .await;

    let refs = join_all(
        strings
            .into_iter()
            .flatten()
            .map(|addr| ctx.scan(Pattern::new(format!("48 8d 15 X0x{addr:x}")).unwrap())),
    )
    .await;

    Ok(ConsoleManagerSingleton(ensure_one(
        refs.iter()
            .flatten()
            .map(|r| ctx.image().get_root_function(*r).unwrap().range().start),
    )?))
});

#[derive(Debug)]
pub struct Compound {
    pub kismet_system_library: Arc<KismetSystemLibrary>,
    pub console_manager_singleton: Arc<ConsoleManagerSingleton>,
}

impl_resolver!(Compound, |ctx| async {
    let (kismet_system_library, console_manager_singleton) = try_join!(
        ctx.resolve(KismetSystemLibrary::resolver()),
        ctx.resolve(ConsoleManagerSingleton::resolver()),
    )?;
    Ok(Compound {
        kismet_system_library,
        console_manager_singleton,
    })
});
