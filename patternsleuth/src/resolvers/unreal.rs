use std::{collections::HashMap, sync::Arc};

use futures::{future::join_all, join, try_join, FutureExt};
use patternsleuth_scanner::Pattern;

use crate::{
    resolvers::{bail_out, impl_resolver, Context, DynResolverFactory, Result},
    Addressable, Matchable, MemoryAccessorTrait,
};

pub fn all() -> &'static [(&'static str, fn() -> &'static DynResolverFactory)] {
    macro_rules! inc {
        ( $( $name:ident , )* ) => {
            &[$( ( stringify!($name), $name::dyn_resolver ), )*]
        };
    }
    inc!(KismetSystemLibrary, ConsoleManagerSingleton,)
}

/// Given an iterator of values, returns Ok(value) if all values are equal or Err
pub fn ensure_one<T: PartialEq>(data: impl IntoIterator<Item = T>) -> Result<T> {
    let mut iter = data.into_iter();
    let first = iter.next().context("expected at least one value")?;
    for value in iter {
        if value != first {
            bail_out!("iter returned multiple unique values");
        }
    }
    Ok(first)
}

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

    dbg!(&strings);

    let refs = join_all(
        strings
            .into_iter()
            .flatten()
            .map(|addr| ctx.scan(Pattern::new(format!("48 8d 15 X0x{addr:x}")).unwrap())),
    )
    .await;

    dbg!(&refs);

    for r in refs.into_iter().flatten() {
        let f = ctx.image().get_root_function(r).unwrap().range().start;
        println!("singleton = {:x}", f);
        return Ok(ConsoleManagerSingleton(f));
    }

    bail_out!("failed");
});

#[derive(Debug)]
pub struct Compound {
    kismet_system_library: Arc<KismetSystemLibrary>,
    console_manager_singleton: Arc<ConsoleManagerSingleton>,
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

#[derive(Debug)]
pub struct FNameToStringFString(usize);
impl_resolver!(FNameToStringFString, |ctx| async {
    let scans = join!(
        ctx.scan(Pattern::new("E8 | ?? ?? ?? ?? 48 8B 4C 24 ?? 8B FD 48 85 C9").unwrap())
            .map(|r| ("asdf", r)),
        ctx.scan(Pattern::new("E8 | ?? ?? ?? ?? BD 01 00 00 00 41 39 6E ?? 0F 8E").unwrap())
            .map(|r| ("asdf2", r)),
    );
    dbg!(scans);

    let scans = join_all([
        ctx.scan_tagged(
            "asdf",
            Pattern::new("E8 | ?? ?? ?? ?? 48 8B 4C 24 ?? 8B FD 48 85 C9").unwrap(),
        ),
        ctx.scan_tagged(
            "asdf",
            Pattern::new("E8 | ?? ?? ?? ?? BD 01 00 00 00 41 39 6E ?? 0F 8E").unwrap(),
        ),
    ])
    .await;
    dbg!(scans);

    /*
    "A".to_string(),
    Pattern::new("E8 ?? ?? ?? ?? 48 8B 4C 24 ?? 8B FD 48 85 C9")?,
    FNameToStringID::resolve,

    "B".to_string(),
    Pattern::new("E8 ?? ?? ?? ?? BD 01 00 00 00 41 39 6E ?? 0F 8E")?,
    FNameToStringID::resolve,

    Sig::FNameToStringFString,//419-427
    "SetEnums".to_string(),
    Pattern::new("0f 84 ?? ?? ?? ?? 48 8b ?? e8 ?? ?? ?? ?? 84 c0 0f 85 ?? ?? ?? ?? 48 8d ?? 24 ?? 48 8b ?? e8 ?? ?? ?? ??")?,
    FNameToStringID::setenums,

    Sig::FNameToStringFString,
    "Bnew3".to_string(),
    Pattern::new("E8 ?? ?? ?? ?? ?? 01 00 00 00 ?? 39 ?? 48 0F 8E")?,
    FNameToStringID::resolve,

    Sig::FNameToStringFString,
    "KH3".to_string(),
    Pattern::new("48 89 5C 24 ?? 48 89 ?? 24 ?? 48 89 ?? 24 ?? 41 56 48 83 EC ?? 48 8B DA 4C 8B F1 e8 ?? ?? ?? ?? 4C 8B C8 41 8B 06 99")?,
    resolve_self,
    */

    Ok(FNameToStringFString(0))
});
