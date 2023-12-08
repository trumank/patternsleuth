use std::{collections::HashMap, sync::Arc};

use futures::{future::join_all, try_join};
use iced_x86::{Code, Decoder, DecoderOptions, Instruction, Register};
use patternsleuth_scanner::Pattern;

use crate::{
    resolvers::{bail_out, ensure_one, impl_resolver},
    Addressable, Matchable, MemoryAccessorTrait, MemoryTrait,
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

/// public: void __cdecl UObject::SkipFunction(struct FFrame &, void *const, class UFunction *)
#[derive(Debug)]
pub struct UObjectSkipFunction(pub usize);
impl_resolver!(UObjectSkipFunction, |ctx| async {
    let patterns = [
        "40 55 41 54 41 55 41 56 41 57 48 83 EC 30 48 8D 6C 24 20 48 89 5D 40 48 89 75 48 48 89 7D 50 48 8B 05 ?? ?? ?? ?? 48 33 C5 48 89 45 00 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 4D 8B ?? ?? 8B ?? 85 ?? 75 05 41 8B FC EB ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 ?? E0",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(UObjectSkipFunction(ensure_one(res.into_iter().flatten())?))
});

// GNatives
#[derive(Debug)]
pub struct GNatives(pub usize);
impl_resolver!(GNatives, |ctx| async {
    let skip_function = ctx.resolve(UObjectSkipFunction::resolver()).await?;
    let bytes = ctx.image().memory.range_from(skip_function.0..);

    let mut decoder = Decoder::with_ip(
        64,
        &bytes[0..bytes.len().min(500)],
        skip_function.0 as u64,
        DecoderOptions::NONE,
    );

    // TODO recursive decode candidate
    let mut instruction = Instruction::default();
    while decoder.can_decode() {
        decoder.decode_out(&mut instruction);
        if instruction.code() == Code::Lea_r64_m && instruction.memory_base() == Register::RIP {
            return Ok(GNatives(instruction.memory_displacement64() as usize));
        }
    }

    bail_out!("failed to not find LEA instruction");
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

/// public: static bool __cdecl UGameplayStatics::SaveGameToSlot(class USaveGame *, class FString const &, int)
#[derive(Debug)]
pub struct UGameplayStaticsSaveGameToSlot(pub usize);
impl_resolver!(UGameplayStaticsSaveGameToSlot, |ctx| async {
    let patterns = [
        "48 89 5C 24 08 48 89 74 24 10 57 48 83 EC 40 ?? 8B ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? E8 ?? ?? FF FF 84 C0 74 58 E8 ?? ?? ?? ?? 48 8B ?? 48 8B ?? FF 52 ?? 4C 8B D0 48 85 C0 74 42 39 74 24 38 7E 3C 8B 53 08 ?? ?? ?? ?? ?? 0F 44 CE 85 C9 7E 2D",
        "48 89 5C 24 08 48 89 74 24 10 48 89 7C 24 18 55 41 56 41 57 48 8D AC 24 ?? FF FF FF 48 81 EC ?? ?? 00 00 48 8B F1 45 33 FF 48 8B 0D ?? ?? ?? ?? 45 8B F0 48 8B ?? 48 85 C9 75 27 41 8D 4F 08 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 49 8B CF 48 89 0D ?? ?? ?? ?? 48 8B",
        "40 55 56 57 41 54 41 55 41 ?? 48 8D AC 24 ?? ?? FF FF 48 81 EC ?? ?? 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 85 ?? ?? 00 00 4C 8B ?? 45 33 ED 48 8B 0D ?? ?? ?? ?? 45 8B E0 48 8B FA 48 85 C9 75 27 41 8D 4D 08 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 49 8B CD 48 89",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(UGameplayStaticsSaveGameToSlot(ensure_one(
        res.into_iter().flatten(),
    )?))
});

/// public: static class USaveGame * __cdecl UGameplayStatics::LoadGameFromMemory(class TArray<unsigned char, class TSizedDefaultAllocator<32> > const &)
#[derive(Debug)]
pub struct UGameplayStaticsLoadGameFromMemory(pub usize);
impl_resolver!(UGameplayStaticsLoadGameFromMemory, |ctx| async {
    let patterns = [
        "48 89 5C 24 20 55 48 8D AC 24 10 FF FF FF 48 81 EC F0 01 00 00 83 79 08 00 48 8B D9 75 13 33 C0 48 8B 9C 24 18 02 00 00 48 81 C4 F0 01 00 00 5D C3 0F 57 C0 48 89 ?? 24 ?? 02 00 00 48 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11 45 ?? 0F 11",
        "40 55 48 8D AC 24 00 FF FF FF 48 81 EC 00 02 00 00 83 79 08 00 75 0B 33 C0 48 81 C4 00 02 00 00 5D C3 48 8B D1 48 89 9C 24 20 02 00 00 48 89 BC 24 28 02 00 00 48 8D 4D A0 41 B0 01 33 FF E8 ?? ?? ?? ?? 0F 57 C0 89 7C 24 50 48 8D 4C 24 50 66 0F 7F 44 24 40 66 89 7C 24 54 89 7C 24 58 48 89 7C 24 60 48",
        "40 55 48 8D AC 24 ?? ?? FF FF 48 81 EC ?? 02 00 00 83 79 08 00 75 0B 33 C0 48 81 C4 ?? 02 00 00 5D C3 48 89 9C 24 ?? 02 00 00 48 8B D1 48 89 B4 24 ?? 02 00 00 48 8D 4D A0 48 89 BC 24 ?? 02 00 00 41 B0 01 33 FF E8 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 24 ?? ?? 89 7C 24",
        "48 89 5C 24 20 55 48 8D AC 24 ?? FF FF FF 48 81 EC ?? ?? 00 00 83 79 08 00 48 8B D9 75 13 33 C0 48 8B 9C 24 ?? 02 00 00 48 81 C4 ?? ?? 00 00 5D C3 48 89 B4 24 ?? 02 00 00 48 8D 4D A0 48 89 BC 24 ?? 02 00 00 33",
        "40 55 53 48 8D AC 24 08 FF FF FF 48 81 EC F8 01 00 00 83 79 08 00 48 8B D9 75 0C 33 C0 48 81 C4 F8 01 00 00 5B 5D C3 48 89 B4 24 18 02 00 00 48 8D 4D A0 48 89 BC 24 20 02 00 00 33 FF 4C 89 B4 24 F0 01 00 00 E8 ?? ?? ?? ?? 48 8D 05 ?? ?? ?? 00 48 89 7D 30 48 89 45 A0 48 8D 4D A0 48 B8 FF FF FF FF FF",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(UGameplayStaticsLoadGameFromMemory(ensure_one(
        res.into_iter().flatten(),
    )?))
});

/// public: static class USaveGame * __cdecl UGameplayStatics::LoadGameFromSlot(class FString const &, int)
#[derive(Debug)]
pub struct UGameplayStaticsLoadGameFromSlot(pub usize);
impl_resolver!(UGameplayStaticsLoadGameFromSlot, |ctx| async {
    let patterns = [
        "48 8B C4 55 ?? 48 8D A8 ?? FE FF FF 48 81 EC ?? 02 00 00 48 89 ?? 08 33 ?? 48 89 ?? 10 48 8B ?? 4C 89 70 E8 44 8B F2 48 89 ?? 24 40 48 89 ?? 24 48 E8 ?? ?? ?? ?? 48 8B C8 4C 8B 00 41 FF 50 40 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 8D 35",
        "48 89 5C 24 08 48 89 74 24 10 57 48 83 EC 40 33 DB 8B F2 48 89 5C 24 30 48 8B F9 48 89 5C 24 38 E8 ?? ?? ?? ?? 48 8B C8 4C 8B 00 41 FF 50 40 4C 8B D0 48 85 C0 74 4A 8B 57 08 85 D2 8D 4A FF 0F 44 CB 85 C9 7E 3B 85 D2 74 05 4C 8B 07 EB 07 4C 8D 05 ?? ?? ?? ?? 48 8B 00 48 8D 4C 24 30 48 89 4C 24 20 44",
        "48 89 5C 24 10 55 57 41 56 48 8D AC 24 00 FF FF FF 48 81 EC 00 02 00 00 48 8B D9 33 FF 48 8B 0D ?? ?? ?? ?? 44 8B F2 48 85 C9 75 26 8D 4F 08 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 48 8B CF 48 89 0D ?? ?? ?? ?? 48 8B 01",
        "48 89 5C 24 08 55 56 57 48 8D AC 24 ?? FF FF FF 48 81 EC ?? 01 00 00 48 8B D9 ?? ?? ?? ?? ?? ?? ?? ?? ?? 8B F2 48 85 C9 75 26 8D 4F 08 E8 ?? ?? ?? FF 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 48 8B CF 48 89 0D ?? ?? ?? ?? 48 8B 01 FF 50 40 48 8B C8 48 85 C0 0F 84 ?? ?? 00 00 8B 43",
        "48 89 5C 24 18 55 56 57 48 8D AC 24 ?? FF FF FF 48 81 EC ?? ?? 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 85 ?? 00 00 00 48 8B D9 33 FF 48 8B 0D ?? ?? ?? ?? 8B F2 48 85 C9 75 26 8D 4F 08 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 03 48 8B CF 48 89 0D ?? ?? ?? ?? 48 8B 01",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(UGameplayStaticsLoadGameFromSlot(ensure_one(
        res.into_iter().flatten(),
    )?))
});

/// public: static bool __cdecl UGameplayStatics::DoesSaveGameExist(class FString const &, int)
#[derive(Debug)]
pub struct UGameplayStaticsDoesSaveGameExist(pub usize);
impl_resolver!(UGameplayStaticsDoesSaveGameExist, |ctx| async {
    let patterns = [
        "48 89 5C 24 08 57 48 83 EC 20 8B FA 48 8B D9 E8 ?? ?? ?? ?? 48 8B C8 4C 8B 00 41 FF 50 ?? 48 85 C0 74 3D 83 7B 08 00 4C 8B 00 4D 8B 48 ?? 74 16 48 8B 13 44 8B C7 48 8B C8 48 8B 5C 24 30 48 83 C4 20 5F 49 FF E1 48 8D 15 ?? ?? ?? ?? 44 8B C7 48 8B C8 48 8B 5C 24 30 48 83 C4 20 5F 49 FF E1 48 8B 5C 24",
        "48 89 5C 24 08 57 48 83 EC 20 8B FA 48 8B D9 E8 ?? ?? ?? ?? 48 8B C8 4C 8B 00 41 FF 50 40 48 8B C8 48 85 C0 74 38 83 7B 08 00 74 17 48 8B 00 44 8B C7 48 8B 13 48 8B 5C 24 30 48 83 C4 20 5F 48 FF 60 08 48 8B 00 48 8D 15 ?? ?? ?? ?? 44 8B C7 48 8B 5C 24 30 48 83 C4 20 5F 48 FF 60 08 48 8B 5C 24 30 48",
        "48 89 5C 24 08 57 48 83 EC 20 48 8B D9 ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 85 C9 75 27 B9 08 00 00 00 E8 ?? ?? ?? ?? 48 8B C8 48 85 C0 74 0C 48 8D 05 ?? ?? ?? ?? 48 89 01 EB 02 33 C9 48 89 0D ?? ?? ?? ?? 48 8B 01 FF 50 40 48 8B C8 48 85 C0 74 38 83 7B 08 00 74 17 48 8B 00 ?? 8B ?? ?? 8B ?? 48 8B 5C 24 30",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    Ok(UGameplayStaticsDoesSaveGameExist(ensure_one(
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
