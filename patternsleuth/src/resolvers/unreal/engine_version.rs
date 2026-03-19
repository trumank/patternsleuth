use std::{
    collections::BTreeSet,
    fmt::{Debug, Display},
    str::FromStr,
};

use futures::future::join_all;

use itertools::Itertools;
use patternsleuth_scanner::Pattern;

use crate::{
    Addressable as _, MemoryTrait,
    resolvers::{ResolveError, bail_out, impl_resolver, impl_resolver_singleton, try_ensure_one},
};
use crate::resolvers::ensure_one;
use crate::resolvers::unreal::util;

#[derive(PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct EngineVersion {
    pub major: u16,
    pub minor: u16,
}
impl Display for EngineVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}
impl Debug for EngineVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EngineVersion({}.{})", self.major, self.minor)
    }
}
impl FromStr for EngineVersion {
    type Err = ResolveError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let (major, minor) = s
            .split_once('.')
            .ok_or(ResolveError::new_msg("expected <major>.<minor e.g.: 5.4"))?;
        let major = major
            .parse::<u16>()
            .map_err(|e| ResolveError::new_msg(e.to_string()))?;
        let minor = minor
            .parse::<u16>()
            .map_err(|e| ResolveError::new_msg(e.to_string()))?;
        Ok(Self { major, minor })
    }
}

impl_resolver!(all, EngineVersion, |ctx| async {
    let patterns = [
        "C7 47 20 | 04 00 ?? 00 66 89 6F 24",
        "C7 4? 20 | 04 00 ?? ?? 66 4? 89 ?? 24",
        "C7 ?? 24 20 | 04 00 ?? ?? 48 8D 45 F0",
        "C7 05 ?? ?? ?? ?? | 04 00 ?? 00 66 89 ?? ?? ?? ?? ?? C7 05",
        "C7 05 ?? ?? ?? ?? | 04 00 ?? 00 66 89 ?? ?? ?? ?? ?? 89",
        "41 C7 ?? | 04 00 ?? 00 ?? ?? 00 00 00 66 41 89",
        "41 C7 ?? | 04 00 18 00 66 41 89 ?? 04",
        "41 C7 04 24 | 04 00 ?? 00 66 ?? 89 ?? 24",
        "41 C7 04 24 | 04 00 ?? 00 B9 ?? 00 00 00",
        "41 C7 44 24 20 | 04 00 ?? 00 66 ?? 89 ?? 24",
        "C7 05 ?? ?? ?? ?? | 04 00 ?? 00 89 3D ?? ?? ?? ?? 85 FF",
        "C7 05 ?? ?? ?? ?? | 04 00 ?? 00 89 05 ?? ?? ?? ?? E8",
        "C7 05 ?? ?? ?? ?? | 04 00 ?? 00 66 89 ?? ?? ?? ?? ??",
        "C7 46 20 | 04 00 ?? 00 66 44 89 76 24 44 89 76 28 48 39 C7",
        "C7 03 | 04 00 ?? 00 66 44 89 63 04 C7 43 08 C1 5C 08 80 E8",
        "C7 47 20 | 04 00 ?? 00 66 89 6F 24 C7 47 28 ?? ?? ?? ?? 49",
        "C7 03 | 04 00 ?? 00 66 89 6B 04 89 7B 08 48 83 C3 10",
        "41 C7 06 | 05 00 ?? ?? 48 8B 5C 24 ?? 49 8D 76 ?? 33 ED 41 89 46",
        "C7 06 | 05 00 ?? ?? 48 8B 5C 24 20 4C 8D 76 10 33 ED",
        "11 76 30 c7 46 20 | 04 00 ?? 00",
        // maybe better go from BuildSettings::GetBranchName -> FGlobalEngineVersions::FGlobalEngineVersions
        "0F 57 C0 0F 11 43 10 C7 03 | 05 ?? ?? ?? 66 C7 43 04 ?? ??", // <- last one is patch
        "48 89 2? 48 89 6? 08 C7 0? | 05 00 ?? ?? 66",
        "49 89 2? 49 89 6? 08 C7 0? | 05 00 ?? ?? 66",
        "C7 46 20 | 05 00 ?? ?? 66 89 ?? 24",
        "C7 43 20 | 05 00 ?? ?? 48 3B F0",
        "C7 46 20 | 05 00 ?? ?? 48 8D 44 24 20",
        "C7 4? 20 | 05 00 ?? ?? 66 44 89 ?? 24",
        "C7 ?? 24 20 | 05 00 ?? ?? 48 8D 45 F0",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    try_ensure_one(
        res.iter()
            .flatten()
            .map(|a| {
                Ok(EngineVersion {
                    major: ctx.image().memory.u16_le(*a)?,
                    minor: ctx.image().memory.u16_le(a + 2)?,
                })
            })
            .filter_ok(|ver| match ver.major {
                // TODO 4.0 can false positive so ignore it. need to harden if this is to work on 4.0 games
                4 if (1..=27).contains(&ver.minor) => true,
                5 if (0..).contains(&ver.minor) => true,
                _ => false,
            }),
    )
});

/// currently seems to be 4.22+
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct EngineVersionStrings {
    pub branch_name: String,
    pub build_date: String,
    pub build_version: String,
}
impl FromStr for EngineVersionStrings {
    type Err = ResolveError;
    fn from_str(_s: &str) -> std::result::Result<Self, Self::Err> {
        Err(ResolveError::new_msg("unimplemented"))
    }
}
impl_resolver!(collect, EngineVersionStrings);
// "++UE5+Release-{}.{}"
impl_resolver!(ElfImage, EngineVersionStrings, |ctx| async {
    use crate::resolvers::{ensure_one, unreal::util};

    let pattern_name = util::utf16_pattern("++UE5+Release-");
    let name_scan = ctx.scan(pattern_name).await;

    let mut name_scan: Vec<_> = name_scan
        .iter()
        .flat_map(|&addr| ctx.image().memory.read_wstring(addr))
        .collect();

    if name_scan.len() != 2 {
        bail_out!("not found");
    }

    name_scan.sort();
    let (branch_name, build_version) = (name_scan[0].clone(), name_scan[1].clone());

    let build_date = join_all(
        [
            "Jan ", "Feb ", "Mar ", "Apr ", "May ", "Jun ", "Jul ", "Aug ", "Sep ", "Oct ", "Nov ",
            "Dec ",
        ]
        .map(|p| ctx.scan(util::utf16_pattern(p))),
    )
    .await
    .into_iter()
    .flatten()
    .flat_map(|addr| ctx.image().memory.read_wstring(addr))
    .filter(|p| {
        let sp = p.split_whitespace().collect_vec();
        if sp.len() == 3 {
            let (dd, yyyy) = (
                sp[1].parse::<u32>().unwrap_or(0),
                sp[2].parse::<u32>().unwrap_or(0),
            );
            !(dd >= 32 || yyyy >= 2100 || yyyy <= 2000)
        } else {
            false
        }
    });

    let build_date = ensure_one(build_date)?;

    Ok(Self {
        branch_name,
        build_date,
        build_version,
    })
});

impl_resolver!(PEImage, EngineVersionStrings, |ctx| async {
    use crate::MemoryTrait;
    use std::collections::HashSet;

    let patterns = [
        "48 8D 05 [ ?? ?? ?? ?? ] C3 CC CC CC CC CC CC CC CC 48 8D 05 [ ?? ?? ?? ?? ] C3 CC CC CC CC CC CC CC CC 48 8D 05 [ ?? ?? ?? ?? ] C3 CC CC CC CC CC CC CC CC",
    ];

    let res = join_all(
        patterns
            .iter()
            .map(|p| ctx.scan_tagged((), Pattern::new(p).unwrap())),
    )
    .await;

    let mem = &ctx.image().memory;

    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .into_iter()
    .map(|month| month.encode_utf16().flat_map(u16::to_le_bytes).collect())
    .collect::<HashSet<Vec<u8>>>();

    for (_, pattern, addresses) in res {
        for a in addresses {
            let caps = mem.captures(&pattern, a)?.unwrap();
            let date = caps[1].rip();
            if mem
                .range(date..date + 6)
                .ok()
                .filter(|r| months.contains(&r[..]))
                .is_some()
            {
                return Ok(EngineVersionStrings {
                    branch_name: mem.read_wstring(caps[0].rip())?,
                    build_date: mem.read_wstring(caps[1].rip())?,
                    build_version: mem.read_wstring(caps[2].rip())?,
                });
            }
        }
    }

    bail_out!("not found");
});

/// Detects the build configuration (DebugGame/Development vs Shipping)
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub enum BuildConfiguration {
    Shipping,
    Development, // Includes DebugGame, Test, Dev, Development
}
impl FromStr for BuildConfiguration {
    type Err = ResolveError;
    fn from_str(_s: &str) -> std::result::Result<Self, Self::Err> {
        Err(ResolveError::new_msg("unimplemented"))
    }
}

impl_resolver!(all, BuildConfiguration, |ctx| async {
    use crate::resolvers::unreal::util;

    // This debug string only appears in non-shipping builds
    let debug_string =
        "Size,Name,PSysSize,ModuleSize,ComponentSize,ComponentCount,CompResSize,CompTrueResSize\0";

    let pattern = util::utf16_pattern(debug_string);
    let results = ctx.scan(pattern).await;

    if !results.is_empty() {
        // Found the debug string - this is a development build
        Ok(BuildConfiguration::Development)
    } else {
        // No debug string found - assume shipping build
        Ok(BuildConfiguration::Shipping)
    }
});

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct CustomVersionRegistry(u64);
impl_resolver_singleton!(all, CustomVersionRegistry, |ctx| async {
    let patterns = [
        "75 de 48 8d 1d | ?? ?? ?? ?? 48 8b cb ff 15 ?? ?? ?? ?? 66 0f 6f 05 ?? ?? ?? ?? 48 8d 0d ?? ?? ?? ?? 33 c0",
    ];

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap())))
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    Ok(Self(try_ensure_one(
        res.into_iter().map(|a| Ok(ctx.image().memory.rip4(a)?)),
    )?))
});

#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Guid([u8; 16]);

impl std::fmt::Debug for Guid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, byte) in self.0.iter().enumerate() {
            if i == 4 || i == 6 || i == 8 || i == 10 {
                f.write_str("-")?;
            }
            write!(f, "{:02x}", byte)?;
        }
        write!(f, "}}")
    }
}

#[cfg(feature = "serde-resolvers")]
impl serde::Serialize for Guid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let hex_string = self
            .0
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect::<String>();

        serializer.serialize_str(&hex_string)
    }
}

#[cfg(feature = "serde-resolvers")]
impl<'de> serde::Deserialize<'de> for Guid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct GuidVisitor;

        impl<'de> serde::de::Visitor<'de> for GuidVisitor {
            type Value = Guid;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a hex string of 32 characters")
            }

            fn visit_str<E>(self, value: &str) -> Result<Guid, E>
            where
                E: serde::de::Error,
            {
                let clean = value.replace("-", "");

                if clean.len() != 32 {
                    return Err(E::custom(format!(
                        "expected 32 hex characters, got {}",
                        clean.len()
                    )));
                }

                let mut bytes = [0u8; 16];
                for i in 0..16 {
                    let byte_str = &clean[i * 2..i * 2 + 2];
                    bytes[i] =
                        u8::from_str_radix(byte_str, 16).map_err(|_| E::custom("invalid GUID"))?;
                }

                Ok(Guid(bytes))
            }
        }

        deserializer.deserialize_str(GuidVisitor)
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
struct CustomVersion {
    guid: Guid,
    version: u32,
    name: String,
}
impl std::fmt::Debug for CustomVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CustomVersion({:?}, {:>3}, {})",
            self.guid, self.version, self.name
        )
    }
}

impl FromStr for StaticCustomVersions {
    type Err = ResolveError;
    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        Err(ResolveError::new_msg("unimplemented"))
    }
}

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct StaticCustomVersions(BTreeSet<CustomVersion>);
impl_resolver!(all, StaticCustomVersions, |ctx| async {
    enum V {
        A,
        B,
        C,
        D,
        E,
        F,
        G,
    }
    #[rustfmt::skip]
    let patterns = [
        (V::A, "0f 10 05 [ ?? ?? ?? ?? ] 45 33 c9 4c 8d 05 [ ?? ?? ?? ?? ] 48 8d 4c 24 20 0f 29 44 24 20 0f 11 05 ?? ?? ?? ?? 41 8d 51 [ ?? ] e8 [ ?? ?? ?? ?? ]"),
        (V::A, "0f 10 35 [ ?? ?? ?? ?? ] 4c 8d 05 [ ?? ?? ?? ?? ] 48 8d 4c 24 20 41 8d 51 [ ?? ] 0f 29 74 24 20 0f 11 35 ?? ?? ?? ?? e8 [ ?? ?? ?? ?? ]"),
        (V::A, "0f 10 05 [ ?? ?? ?? ?? ] 45 33 c9 4c 8d 05 [ ?? ?? ?? ?? ] 48 8d 4c 24 20 0f 29 44 24 20 41 8d 51 [ ?? ] e8 [ ?? ?? ?? ?? ]"),
        (V::A, "0f 10 35 [ ?? ?? ?? ?? ] 4c 8d 05 [ ?? ?? ?? ?? ] 48 8d 4c 24 20 41 8d 51 [ ?? ] 0f 29 74 24 20 e8 [ ?? ?? ?? ?? ]"),
        (V::B, "0f 10 05 [ ?? ?? ?? ?? ] 45 33 c9 4c 8d 05 [ ?? ?? ?? ?? ] 33 d2 48 8d 4c 24 20 0f 29 44 24 20 0f 11 05 ?? ?? ?? ?? e8 [ ?? ?? ?? ?? ]"),
        (V::B, "0f 10 05 [ ?? ?? ?? ?? ] 45 33 c9 4c 8d 05 [ ?? ?? ?? ?? ] 33 d2 48 8d 4c 24 20 0f 29 44 24 20 e8 [ ?? ?? ?? ?? ]"),
        (V::C, "4c 8d 05 [ ?? ?? ?? ?? ] 0f 10 35 [ ?? ?? ?? ?? ] 45 33 c9 48 8d 4c 24 20 33 d2 0f 29 74 24 20 e8 [ ?? ?? ?? ?? ]"),
        (V::D, "48 8d 15 [ ?? ?? ?? ?? ] 0f 10 35 [ ?? ?? ?? ?? ] c7 44 24 28 ff ff ff ff 48 8d 4c 24 60 41 b9 01 00 00 00 c6 44 24 20 01 45 33 c0 e8 ?? ?? ?? ?? 4c 8b 4c 24 60 48 8d 54 24 30 41 b8 [ ?? ?? ?? ?? ]"),
        (V::E, "48 8d 15 [ ?? ?? ?? ?? ] 0f 10 35 [ ?? ?? ?? ?? ] c7 44 24 28 ff ff ff ff 48 8d 4c 24 60 41 b9 01 00 00 00 c6 44 24 20 01 45 33 c0 e8 ?? ?? ?? ?? 4c 8b 4c 24 60 48 8d 54 24 30 45 33 c0"),
        (V::F, "0f 10 05 [ ?? ?? ?? ?? ] 41 b8 01 00 00 00 48 8d 15 [ ?? ?? ?? ?? ] 48 8d 4c 24 40 0f 29 44 24 20 e8 ?? ?? ?? ?? 41 b8 [ ?? ?? ?? ?? ] 48"),
        (V::G, "0f 10 05 [ ?? ?? ?? ?? ] 41 b8 01 00 00 00 48 8d 15 [ ?? ?? ?? ?? ] 48 8d 4c 24 40 0f 29 44 24 20 e8 ?? ?? ?? ?? 45 33 c0 48 ?? ?? ?? ?? 48"),
    ];
    let res = join_all(
        patterns
            .iter()
            .map(|(v, p)| ctx.scan_tagged(v, Pattern::new(p).unwrap())),
    )
    .await;

    let mut versions = BTreeSet::new();

    let mem = &ctx.image().memory;

    for (v, pattern, addresses) in res {
        for a in addresses {
            let caps = mem.captures(&pattern, a)?.unwrap();

            match v {
                V::A => {
                    versions.insert(CustomVersion {
                        guid: Guid(mem.array(caps[0].rip()).unwrap_or_default()),
                        version: caps[2].data[0] as u32,
                        name: mem.read_wstring(caps[1].rip())?,
                    });
                }
                V::B => {
                    versions.insert(CustomVersion {
                        guid: Guid(mem.array(caps[0].rip()).unwrap_or_default()),
                        version: 0,
                        name: mem.read_wstring(caps[1].rip())?,
                    });
                }
                V::C => {
                    versions.insert(CustomVersion {
                        guid: Guid(mem.array(caps[1].rip()).unwrap_or_default()),
                        version: 0,
                        name: mem.read_wstring(caps[0].rip())?,
                    });
                }
                V::D => {
                    versions.insert(CustomVersion {
                        guid: Guid(mem.array(caps[1].rip()).unwrap_or_default()),
                        version: caps[2].u32(),
                        name: mem.read_wstring(caps[0].rip())?,
                    });
                }
                V::E => {
                    versions.insert(CustomVersion {
                        guid: Guid(mem.array(caps[1].rip()).unwrap_or_default()),
                        version: 0,
                        name: mem.read_wstring(caps[0].rip())?,
                    });
                }
                V::F => {
                    versions.insert(CustomVersion {
                        guid: Guid(mem.array(caps[0].rip()).unwrap_or_default()),
                        version: caps[2].u32(),
                        name: mem.read_wstring(caps[1].rip())?,
                    });
                }
                V::G => {
                    versions.insert(CustomVersion {
                        guid: Guid(mem.array(caps[0].rip()).unwrap_or_default()),
                        version: 0,
                        name: mem.read_wstring(caps[1].rip())?,
                    });
                }
            }
        }
    }

    if versions.is_empty() {
        Err(ResolveError::new_msg("expected at least one value"))
    } else {
        Ok(Self(versions))
    }
});

/// Build changelist version string (e.g., "main-CL-89524" or "++UE4+Release-4.22-CL-0")
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct BuildChangeList(pub String);

impl FromStr for BuildChangeList {
    type Err = ResolveError;
    fn from_str(_s: &str) -> std::result::Result<Self, Self::Err> {
        Err(ResolveError::new_msg("unimplemented"))
    }
}

impl_resolver!(all, BuildChangeList, |ctx| async {
    use crate::{
        disassemble::{Control, disassemble},
        resolvers::unreal::util,
    };
    use iced_x86::{Code, OpKind, Register};

    // Pattern: call GetBuildVersion + mov r8, rax + lea rdx, [rip+"Build: %s"] + lea rcx, [rsp+offset] + call PrintfImpl
    let patterns = [
        "e8 [ ?? ?? ?? ?? ] 4c 8b c0 48 8d 15 [ ?? ?? ?? ?? ] 48 8d 4c 24 ?? e8",
        "e8 [ ?? ?? ?? ?? ] 4c 8b c0 48 8d 15 [ ?? ?? ?? ?? ] 48 8d ?? ?? e8",
    ];

    // Find all "Build: %s" strings
    let build_pattern_str = util::utf16_pattern("Build: %s");
    let build_str_addrs: BTreeSet<_> = ctx.scan(build_pattern_str).await.into_iter().collect();

    if build_str_addrs.is_empty() {
        bail_out!("'Build: %s' string not found");
    }

    let mem = &ctx.image().memory;
    let img = ctx.image();

    let res = join_all(
        patterns
            .iter()
            .map(|p| ctx.scan_tagged((), Pattern::new(p).unwrap())),
    )
    .await;

    for (_, pattern, addresses) in res {
        for a in addresses {
            let caps = mem.captures(&pattern, a)?.unwrap();

            // Check if caps[1] (the lea rdx offset) points to a "Build: %s" string
            if build_str_addrs.contains(&caps[1].rip()) {
                let call_target = caps[0].rip();
                let mut result: Option<String> = None;
                let mut num_inst = 0;

                disassemble(img, call_target, |inst| {
                    // Look for lea reg, [rip+offset]
                    if matches!(inst.code(), Code::Lea_r64_m | Code::Lea_r32_m)
                        && inst.memory_base() == Register::RIP
                        && inst.op1_kind() == OpKind::Memory
                        && let Ok(s) = mem.read_wstring(inst.ip_rel_memory_address())
                    {
                        result = Some(s);
                        return Ok(Control::Exit);
                    }

                    num_inst += 1;
                    if num_inst > 100 {
                        Ok(Control::Exit)
                    } else {
                        // depth first
                        Ok(Control::Follow)
                    }
                })?;

                if let Some(s) = result {
                    return Ok(BuildChangeList(s));
                }
            }
        }
    }

    bail_out!("Build changelist not found");
});

/// InternalProjectName. GInternalProjectName in UE code, compiled_in_project_name will be unset if executable is a game agnostic executable
#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct InternalProjectName {
    pub internal_project_name: u64,
    pub compiled_in_project_name: Option<String>,
}
impl_resolver!(all, InternalProjectName, |ctx| async {
    let strings = join_all([
        ctx.scan(util::utf16_pattern("UE4-%s\0")), // UE4
        ctx.scan(util::utf16_pattern("UE-%s\0")) // UE5
    ]).await.into_iter().flatten().collect_vec();

    // Old UE4 games have UE4-%s literal in 2 places, once in FGenericCrashContext and once in legacy WER crash report handler
    let patterns = strings.into_iter().flat_map(|str_addr| [
        format!("4c 8d 05 | ?? ?? ?? ?? 48 8d 15 X0x{str_addr:08x} 48 8d 4c 24 ?? e8"), // no frame pointer
        format!("4c 8d 05 | ?? ?? ?? ?? 48 8d 15 X0x{str_addr:08x} 48 8d 4d ?? e8"), // frame pointer
        format!("4c 8d 05 | ?? ?? ?? ?? 88 05 ?? ?? ?? ?? 48 8d 15 X0x{str_addr:08x} 48 8d 4d ?? e8"), // frame pointer/legacy (has mov [rip+123], al in the middle of function register filling)
        format!("4c 8d 0d | ?? ?? ?? ?? 4c 8d 05 X0x{str_addr:08x} 48 8d 4c 24 ?? ba 00 04 00 00"), // frame pointer/legacy WER crash reporter
        format!("4c 8d 0d | ?? ?? ?? ?? ba 00 04 00 00 4c 8d 05 X0x{str_addr:08x} 48 8b d8 48 8d 4c 24"), // another variation of legacy WER crash reporter with slightly different argument ordering
    ].into_iter()).collect_vec();
    let internal_project_name_list = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap())))
        .await
        .into_iter()
        .flatten()
        .map(|a| Ok(ctx.image().memory.rip4(a)?))
        .collect::<Result<Vec<u64>, ResolveError>>()?;

    let internal_project_name = ensure_one(internal_project_name_list)?;
    let is_bss_section = ctx.image().memory.get_section_containing(internal_project_name).map(|x| x.kind.is_bss()).unwrap_or(true);
    let compiled_in_project_name = if !is_bss_section { Some(ctx.image().memory.read_wstring(internal_project_name)?) } else { None };
    Ok(Self{internal_project_name, compiled_in_project_name})
});
impl FromStr for InternalProjectName {
    type Err = ResolveError;
    fn from_str(_s: &str) -> std::result::Result<Self, Self::Err> {
        Err(ResolveError::new_msg("unimplemented"))
    }
}
