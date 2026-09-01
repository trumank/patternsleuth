use std::{
    collections::BTreeSet,
    fmt::{Debug, Display},
    str::FromStr,
};

use futures::future::join_all;

use itertools::Itertools;
use patternsleuth_scanner::Pattern;

use crate::resolvers::ensure_one;
use crate::resolvers::unreal::util;
use crate::{
    Addressable as _, MemoryTrait,
    resolvers::{
        AsyncContext, ResolveError, bail_out, impl_resolver, impl_resolver_singleton,
        try_ensure_one,
    },
};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
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

#[rustfmt::skip]
const VERSION_PATTERNS: &[&str] = &[
    "C7 47 20 | 04 00 ?? 00 66 89 6F 24",
    "C7 4? 20 | 04 00 ?? ?? 66 4? 89 ?? 24",
    "C7 ?? 24 20 | 04 00 ?? ?? 48 8D 45 F0",
    "C7 05 ?? ?? ?? ?? | 04 00 ?? 00 66 89 ?? ?? ?? ?? ?? C7 05",
    "C7 05 ?? ?? ?? ?? | 04 00 ?? 00 66 89 ?? ?? ?? ?? ?? 89",
    "41 C7 ?? | 04 00 ?? 00 ?? ?? 00 00 00 66 41 89",
    "41 C7 ?? | 04 00 ?? 00 66 41 89 ?? 04",
    "41 C7 04 24 | 04 00 ?? 00 66 ?? 89 ?? 24",
    "41 C7 04 24 | 04 00 ?? 00 B9 ?? 00 00 00",
    "41 C7 44 24 20 | 04 00 ?? 00 66 ?? 89 ?? 24",
    "41 C7 ?? 20 | 04 00 ?? 00 41 89 ?? 28",
    "41 C7 ?? | 04 00 ?? 00 66 41 C7 4? 04",
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
    "0F 57 C0 0F 11 43 10 C7 03 | 05 ?? ?? ?? 66 C7 43 04 ?? ??", // <- last one is patch
    "48 89 2? 48 89 6? 08 C7 0? | 05 00 ?? ?? 66",
    "49 89 2? 49 89 6? 08 C7 0? | 05 00 ?? ?? 66",
    "C7 46 20 | 05 00 ?? ?? 66 89 ?? 24",
    "C7 43 20 | 05 00 ?? ?? 48 3B F0",
    "C7 46 20 | 05 00 ?? ?? 48 8D 44 24 20",
    "C7 4? 20 | 05 00 ?? ?? 66 44 89 ?? 24",
    "C7 ?? 24 20 | 05 00 ?? ?? 48 8D 45 F0",
    "C7 06 | 05 00 ?? 00 66 C7 46 04",
    "0F B6 D8 C1 E3 1F E8 ?? ?? ?? ?? 0B C3 C7 06 | 05 00 ?? 00",
    "0F B6 D8 C1 E3 1F E8 ?? ?? ?? ?? 0B C3 C7 06 | 04 00 ?? 00",
    "0F B6 D8 C1 E3 1F E8 ?? ?? ?? ?? 33 ED C7 06 | 05 00 ?? 00",
    "89 2E 89 6E 08 48 8D 4E 0C 89 29 41 C7 07 | 05 00 ?? 00",
];

#[rustfmt::skip]
const MOV_RM32_IMM32: &[&str] = &[
    "C7 000000??",              // [reg]
    "C7 0000011?",              // [reg]
    "C7 04 ??",                 // [reg+reg*n]
    "C7 05 ?? ?? ?? ??",        // [rip+disp32]
    "C7 010000?? ??",           // [reg+disp8]
    "C7 01000101 ??",           // [rbp+disp8]
    "C7 0100011? ??",           // [reg+disp8]
    "C7 44 ?? ??",              // [reg+reg*n+disp8]
    "C7 100000?? ?? ?? ?? ??",  // [reg+disp32]
    "C7 10000101 ?? ?? ?? ??",  // [rbp+disp32]
    "C7 1000011? ?? ?? ?? ??",  // [reg+disp32]
    "C7 84 ?? ?? ?? ?? ??",     // [reg+reg*n+disp32]
];

fn plausible(ver: &EngineVersion) -> bool {
    match ver.major {
        4 => (0..=27).contains(&ver.minor),
        5 => (0..=20).contains(&ver.minor),
        _ => false,
    }
}

async fn scan_version_stores(ctx: &crate::resolvers::AsyncContext<'_>) -> Vec<EngineVersion> {
    use crate::disassemble::disassemble_single;
    use iced_x86::{Code, Instruction, OpKind};

    // the `66` is the operand size prefix of the `Patch` store that must follow
    let patterns = MOV_RM32_IMM32
        .iter()
        .flat_map(|s| {
            [
                format!("{s} 0? 00 ?? 00 66"),
                format!("41 {s} 0? 00 ?? 00 66"),
            ]
        })
        .collect_vec();

    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    let img = ctx.image();
    res.iter()
        .flatten()
        .filter_map(|&addr| {
            let store = disassemble_single(img, addr).ok()??;
            if store.code() != Code::Mov_rm32_imm32 || store.op0_kind() != OpKind::Memory {
                return None;
            }
            let (base, index, disp) = (
                store.memory_base(),
                store.memory_index(),
                store.memory_displacement64(),
            );
            let writes = |inst: &Instruction, field: u64| {
                inst.op0_kind() == OpKind::Memory
                    && inst.memory_base() == base
                    && inst.memory_index() == index
                    && inst.memory_displacement64() == disp.wrapping_add(field)
            };

            // FEngineVersionBase is 4 byte aligned
            if disp % 4 != 0 {
                return None;
            }

            let mut ip = addr + store.len() as u64;
            let patch = disassemble_single(img, ip).ok()??;
            if !writes(&patch, 4)
                || match patch.code() {
                    Code::Mov_rm16_r16 => false,
                    Code::Mov_rm16_imm16 => patch.immediate16() > 30,
                    _ => true,
                }
            {
                return None;
            }

            // `Changelist` follows shortly after, but not always adjacently.
            // Without it every `mov dword ptr [X], 5` in the binary looks like a 5.0 candidate.
            ip += patch.len() as u64;
            if !(0..4).any(|_| match disassemble_single(img, ip).ok().flatten() {
                Some(inst) if writes(&inst, 8) => true,
                Some(inst) => {
                    ip += inst.len() as u64;
                    false
                }
                None => false,
            }) {
                return None;
            }

            let imm = store.immediate32();
            Some(EngineVersion {
                major: imm as u16,
                minor: (imm >> 16) as u16,
            })
        })
        .filter(|ver| plausible(ver) && ver.minor != 0)
        .collect()
}

async fn scan_version_pattern_matches(
    ctx: &crate::resolvers::AsyncContext<'_>,
) -> Vec<EngineVersion> {
    let res = join_all(
        VERSION_PATTERNS
            .iter()
            .map(|p| ctx.scan(Pattern::new(p).unwrap())),
    )
    .await;

    let mem = &ctx.image().memory;
    res.iter()
        .flatten()
        .filter_map(|a| {
            Some(EngineVersion {
                major: mem.u16_le(*a).ok()?,
                minor: mem.u16_le(a + 2).ok()?,
            })
        })
        // TODO 4.0 can false positive so ignore it. need to harden if this is to work on 4.0 games
        .filter(|ver| plausible(ver) && !(ver.major == 4 && ver.minor == 0))
        .collect()
}

async fn scan_engine_version_string(
    ctx: &crate::resolvers::AsyncContext<'_>,
) -> Vec<EngineVersion> {
    // wide "<digit>.<digit(s)>." at the start of a string, i.e. preceded by the NUL terminating whatever came before it
    let patterns = [
        "00 00 | 0011???? 00 2E 00 0011???? 00 2E 00",
        "00 00 | 0011???? 00 2E 00 0011???? 00 0011???? 00 2E 00",
    ];
    let res = join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap()))).await;

    let mem = &ctx.image().memory;
    res.iter()
        .flatten()
        .filter_map(|&a| {
            let s = mem.read_wstring(a).ok()?;
            let (major, rest) = s.split_once('.')?;
            let (minor, rest) = rest.split_once('.')?;
            let (patch, build_version) = rest.split_once('-')?;
            patch.parse::<u16>().ok()?;
            // "<branch>-CL-<changelist>" or "<changelist>+<branch>"
            if !build_version.contains('+') && !build_version.contains("-CL-") {
                return None;
            }
            Some(EngineVersion {
                major: major.parse().ok()?,
                minor: minor.parse().ok()?,
            })
        })
        .filter(plausible)
        .collect()
}

async fn scan_branch_name(ctx: &crate::resolvers::AsyncContext<'_>) -> Vec<EngineVersion> {
    const BRANCHES: &[&str] = &["++UE4+Release-", "++UE5+Release-", "++depot+UE4-Releases+"];
    let res = join_all(
        BRANCHES
            .iter()
            .map(|b| async move { (b.len(), ctx.scan(util::utf16_pattern(b)).await) }),
    )
    .await;

    let mem = &ctx.image().memory;
    res.iter()
        .flat_map(|(prefix, addresses)| addresses.iter().map(move |a| (*prefix, *a)))
        .filter_map(|(prefix, a)| {
            let s = mem.read_wstring(a).ok()?;
            let version = s.get(prefix..)?;
            // trailing junk is expected, e.g. "++UE4+Release-4.25Plus"
            let (major, rest) = split_number(version)?;
            let (minor, _) = split_number(rest.strip_prefix('.')?)?;
            Some(EngineVersion { major, minor })
        })
        .filter(plausible)
        .collect()
}

fn split_number(s: &str) -> Option<(u16, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    Some((s[..end].parse().ok()?, &s[end..]))
}

impl_resolver!(all, EngineVersion, |ctx| async {
    let (version_string, stores, patterns, branch_name) = futures::join!(
        scan_engine_version_string(ctx),
        scan_version_stores(ctx),
        scan_version_pattern_matches(ctx),
        scan_branch_name(ctx),
    );

    let patterns = ensure_one(patterns);
    for candidates in [version_string, stores] {
        if let Ok(version) = ensure_one(candidates) {
            return Ok(version);
        }
    }
    if patterns.is_ok() {
        return patterns;
    }
    ensure_one(branch_name).or(patterns)
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
        ctx.scan(util::utf16_pattern("UE-%s\0")),  // UE5
    ])
    .await
    .into_iter()
    .flatten()
    .collect_vec();

    // Old UE4 games have UE4-%s literal in 2 places, once in FGenericCrashContext and once in legacy WER crash report handler
    let patterns = strings
        .into_iter()
        .flat_map(|str_addr| {
            [
        format!("4c 8d 05 | ?? ?? ?? ?? 48 8d 15 X0x{str_addr:08x} 48 8d 4c 24 ?? e8"), // no frame pointer
        format!("4c 8d 05 | ?? ?? ?? ?? 48 8d 15 X0x{str_addr:08x} 48 8d 4d ?? e8"), // frame pointer
        format!("4c 8d 05 | ?? ?? ?? ?? 88 05 ?? ?? ?? ?? 48 8d 15 X0x{str_addr:08x} 48 8d 4d ?? e8"), // frame pointer/legacy (has mov [rip+123], al in the middle of function register filling)
        format!("4c 8d 0d | ?? ?? ?? ?? 4c 8d 05 X0x{str_addr:08x} 48 8d 4c 24 ?? ba 00 04 00 00"), // frame pointer/legacy WER crash reporter
        format!("4c 8d 0d | ?? ?? ?? ?? ba 00 04 00 00 4c 8d 05 X0x{str_addr:08x} 48 8b d8 48 8d 4c 24"), // another variation of legacy WER crash reporter with slightly different argument ordering
    ].into_iter()
        })
        .collect_vec();
    let internal_project_name_list =
        join_all(patterns.iter().map(|p| ctx.scan(Pattern::new(p).unwrap())))
            .await
            .into_iter()
            .flatten()
            .map(|a| Ok(ctx.image().memory.rip4(a)?))
            .collect::<Result<Vec<u64>, ResolveError>>()?;

    let internal_project_name = ensure_one(internal_project_name_list)?;
    let is_bss_section = ctx
        .image()
        .memory
        .get_section_containing(internal_project_name)
        .map(|x| x.kind.is_bss())
        .unwrap_or(true);
    let compiled_in_project_name = if !is_bss_section {
        Some(ctx.image().memory.read_wstring(internal_project_name)?)
    } else {
        None
    };
    Ok(Self {
        internal_project_name,
        compiled_in_project_name,
    })
});
impl FromStr for InternalProjectName {
    type Err = ResolveError;
    fn from_str(_s: &str) -> std::result::Result<Self, Self::Err> {
        Err(ResolveError::new_msg("unimplemented"))
    }
}

const fn v(major: u16, minor: u16) -> EngineVersion {
    EngineVersion { major, minor }
}

#[rustfmt::skip]
const VERSIONS: &[EngineVersion] = &[
    v(4, 7), v(4, 8), v(4, 9), v(4, 10), v(4, 11), v(4, 12),
    v(4, 13), v(4, 14), v(4, 15), v(4, 16), v(4, 17), v(4, 18),
    v(4, 19), v(4, 20), v(4, 21), v(4, 22), v(4, 23), v(4, 24),
    v(4, 25), v(4, 26), v(4, 27), v(5, 0), v(5, 1), v(5, 2),
    v(5, 3), v(5, 4), v(5, 5), v(5, 6), v(5, 7), v(5, 8),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub enum BuildConfiguration {
    /// Also Debug and DebugGame
    Development,
    Test,
    Shipping,
}
impl FromStr for BuildConfiguration {
    type Err = ResolveError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "Development" => Ok(Self::Development),
            "Test" => Ok(Self::Test),
            "Shipping" => Ok(Self::Shipping),
            _ => Err(ResolveError::new_msg(
                "expected Development, Test or Shipping",
            )),
        }
    }
}

/// The `STATS` macro. Defaults off in Test and Shipping, but any target can
/// force it back on, so it doesn't follow from the configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub enum Stats {
    Off,
    On,
}
impl FromStr for Stats {
    type Err = ResolveError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "Off" => Ok(Self::Off),
            "On" => Ok(Self::On),
            _ => Err(ResolveError::new_msg("expected On or Off")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Build {
    version: Option<&'static EngineVersion>,
    config: BuildConfiguration,
    stats: Stats,
}

/// A preprocessor condition a marker sits behind. Another axis costs a variant
/// here, not a column on every row.
#[derive(Clone, Copy, PartialEq)]
enum Gate {
    /// `#if !UE_BUILD_SHIPPING`
    NotShipping,
    /// `#if !(UE_BUILD_SHIPPING || UE_BUILD_TEST)`
    DevOnly,
    /// `#if UE_BUILD_TEST || UE_BUILD_SHIPPING`
    TestOrShipping,
    /// `#if UE_BUILD_SHIPPING`
    ShippingOnly,
    /// `#if STATS`
    WithStats,
}
use Gate::*;

impl Gate {
    fn allows(self, build: &Build) -> bool {
        use BuildConfiguration::*;
        match self {
            NotShipping => build.config != Shipping,
            DevOnly => build.config == Development,
            TestOrShipping => build.config != Development,
            ShippingOnly => build.config == Shipping,
            WithStats => build.stats == Stats::On,
        }
    }
}

struct Marker {
    string: &'static str,
    gates: &'static [Gate],
    first: Option<EngineVersion>,
    last: Option<EngineVersion>,
}

impl Marker {
    fn covers(&self, build: &Build) -> bool {
        let after_first = match (&self.first, build.version) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(first), Some(version)) => version >= first,
        };
        let before_last = match (&self.last, build.version) {
            (None, _) | (_, None) => true,
            (Some(last), Some(version)) => version <= last,
        };
        after_first && before_last && self.gates.iter().all(|gate| gate.allows(build))
    }
}

#[rustfmt::skip]
const MARKERS: &[Marker] = &[
    Marker { string: "FOpenGLRHILongGPUTaskPS",                            gates: &[], first: None,            last: Some(v(4, 7)) },
    Marker { string: "FPostProcessTonemapPS15",                            gates: &[], first: None,            last: Some(v(4, 7)) },
    Marker { string: "USE_VIGNETTE_COLOR",                                 gates: &[], first: None,            last: Some(v(4, 7)) },
    Marker { string: "p.BounceThresholdVelocity",                          gates: &[], first: None,            last: Some(v(4, 7)) },
    Marker { string: "r.AmbientOcclusionSampleSetQuality",                 gates: &[], first: None,            last: Some(v(4, 8)) },
    Marker { string: "r.LegacySingleThreadedRelevance",                    gates: &[], first: None,            last: Some(v(4, 8)) },
    Marker { string: "r.Shadow.DistanceFieldPenumbraSize",                 gates: &[], first: None,            last: Some(v(4, 8)) },
    Marker { string: "r.HalfResReflections",                               gates: &[], first: None,            last: Some(v(4, 9)) },
    Marker { string: "r.DepthOfFieldNearBlurSizeThreshold",                gates: &[], first: None,            last: Some(v(4, 10)) },
    Marker { string: "r.Editor.MovingPattern",                             gates: &[], first: None,            last: Some(v(4, 10)) },
    Marker { string: "r.OptimizeForUAVPerformance",                        gates: &[], first: None,            last: Some(v(4, 13)) },
    Marker { string: "r.PS4DumpShaderSDB",                                 gates: &[], first: None,            last: Some(v(4, 20)) },
    Marker { string: "r.PS4MixedModeShaderDebugInfo",                      gates: &[], first: None,            last: Some(v(4, 27)) },
    Marker { string: "ReceiveUninitializeComponent",                       gates: &[], first: Some(v(4, 7)),   last: Some(v(4, 7)) },
    Marker { string: "r.Upscale.Cylinder",                                 gates: &[], first: Some(v(4, 7)),   last: Some(v(4, 8)) },
    Marker { string: "bAnimBranchingPointNeedsSort",                       gates: &[], first: Some(v(4, 7)),   last: Some(v(4, 10)) },
    Marker { string: "AvoidanceConsiderationRadius",                       gates: &[], first: Some(v(4, 7)),   last: Some(v(4, 17)) },
    Marker { string: "K2_SetActorRelativeTransform",                       gates: &[], first: Some(v(4, 7)),   last: Some(v(4, 17)) },
    Marker { string: "PeripheralVisionAngleDegrees",                       gates: &[], first: Some(v(4, 7)),   last: Some(v(4, 17)) },
    Marker { string: "RequestStimuliListenerUpdate",                       gates: &[], first: Some(v(4, 7)),   last: Some(v(4, 17)) },
    Marker { string: "SubduedSelectionOutlineColor",                       gates: &[], first: Some(v(4, 7)),   last: Some(v(4, 17)) },
    Marker { string: "r.RHIDeferredContextWidth",                          gates: &[], first: Some(v(4, 8)),   last: Some(v(4, 8)) },
    Marker { string: "MatineeScreenshotOptions",                           gates: &[], first: Some(v(4, 8)),   last: Some(v(4, 9)) },
    Marker { string: "r.MotionBlurDilate",                                 gates: &[], first: Some(v(4, 8)),   last: Some(v(4, 10)) },
    Marker { string: "r.MotionBlurSmoothMax",                              gates: &[], first: Some(v(4, 8)),   last: Some(v(4, 10)) },
    Marker { string: "r.MotionBlurNew",                                    gates: &[], first: Some(v(4, 8)),   last: Some(v(4, 13)) },
    Marker { string: "r.AOFillGapsHighQuality",                            gates: &[], first: Some(v(4, 8)),   last: Some(v(4, 15)) },
    Marker { string: "r.OcclusionQueryLocation",                           gates: &[], first: Some(v(4, 8)),   last: Some(v(4, 16)) },
    Marker { string: "r.CheckSRVTransitions",                              gates: &[], first: Some(v(4, 8)),   last: None },
    Marker { string: "MovieSceneBoundObjectInfo",                          gates: &[], first: Some(v(4, 9)),   last: Some(v(4, 9)) },
    Marker { string: "UMovieSceneObjectManager",                           gates: &[], first: Some(v(4, 9)),   last: Some(v(4, 9)) },
    Marker { string: "r.AOInnerGlobalDFClipmapDistance",                   gates: &[], first: Some(v(4, 9)),   last: Some(v(4, 11)) },
    Marker { string: "r.MobileOnChipMSAA",                                 gates: &[], first: Some(v(4, 9)),   last: Some(v(4, 14)) },
    Marker { string: "r.AOVisualizeGlobalDistanceField",                   gates: &[], first: Some(v(4, 9)),   last: Some(v(4, 15)) },
    Marker { string: "s.PreloadPackageDependencies",                       gates: &[], first: Some(v(4, 9)),   last: Some(v(4, 15)) },
    Marker { string: "s.AsyncIOBandwidthLimit",                            gates: &[], first: Some(v(4, 9)),   last: Some(v(4, 16)) },
    Marker { string: "r.D3D12GraphicsAdapter",                             gates: &[], first: Some(v(4, 9)),   last: Some(v(4, 19)) },
    Marker { string: "r.MobileHDR32bppMode",                               gates: &[], first: Some(v(4, 9)),   last: Some(v(4, 24)) },
    Marker { string: "r.MobileDynamicPointLightsUseStaticBranch",          gates: &[], first: Some(v(4, 9)),   last: Some(v(4, 27)) },
    Marker { string: "r.MobileNumDynamicPointLights",                      gates: &[], first: Some(v(4, 9)),   last: Some(v(5, 0)) },
    Marker { string: "ULevelSequenceInstance",                             gates: &[], first: Some(v(4, 10)),  last: Some(v(4, 10)) },
    Marker { string: "FORWARD_QL_FORCE_FULLY_ROUGH",                       gates: &[], first: Some(v(4, 10)),  last: Some(v(4, 12)) },
    Marker { string: "UHapticFeedbackEffect",                              gates: &[], first: Some(v(4, 10)),  last: Some(v(4, 12)) },
    Marker { string: "UAutomatedLevelSequenceCapture",                     gates: &[], first: Some(v(4, 10)),  last: Some(v(4, 19)) },
    Marker { string: "D3D12.AllowDrawClears",                              gates: &[], first: Some(v(4, 11)),  last: Some(v(4, 11)) },
    Marker { string: "a.UseBakedAdditiveAnimations",                       gates: &[], first: Some(v(4, 11)),  last: Some(v(4, 11)) },
    Marker { string: "r.DetectAndWarnOfBadDrivers",                        gates: &[], first: Some(v(4, 11)),  last: Some(v(4, 11)) },
    Marker { string: "r.Tonemapper.ScreenPercentage",                      gates: &[], first: Some(v(4, 11)),  last: Some(v(4, 12)) },
    Marker { string: "r.CapsuleIndirectShadowMinVisibility",               gates: &[], first: Some(v(4, 11)),  last: Some(v(4, 13)) },
    Marker { string: "r.TargetPrecompileFrameTime",                        gates: &[], first: Some(v(4, 11)),  last: Some(v(4, 21)) },
    Marker { string: "r.UseAsyncShaderPrecompilation",                     gates: &[], first: Some(v(4, 11)),  last: Some(v(4, 21)) },
    Marker { string: "r.EyeAdaptation.MethodOveride",                      gates: &[], first: Some(v(4, 11)),  last: Some(v(4, 23)) },
    Marker { string: "r.Tonemapper.GrainQuantization",                     gates: &[], first: Some(v(4, 11)),  last: Some(v(5, 2)) },
    Marker { string: "r.HighResScreenshotDelay",                           gates: &[], first: Some(v(4, 11)),  last: None },
    Marker { string: "r.DistanceFieldSpecularOcclusion",                   gates: &[], first: Some(v(4, 12)),  last: Some(v(4, 12)) },
    Marker { string: "r.Shaders.AvoidFlowControl",                         gates: &[], first: Some(v(4, 12)),  last: Some(v(4, 12)) },
    Marker { string: "r.Streaming.ShowWantedMips",                         gates: &[], first: Some(v(4, 12)),  last: Some(v(4, 12)) },
    Marker { string: "r.Streaming.AnalysisIndex",                          gates: &[], first: Some(v(4, 12)),  last: Some(v(4, 13)) },
    Marker { string: "r.RHICmdStateCacheEnable",                           gates: &[], first: Some(v(4, 12)),  last: Some(v(4, 15)) },
    Marker { string: "r.AllReceiveDynamicCSM",                             gates: &[], first: Some(v(4, 12)),  last: Some(v(4, 18)) },
    Marker { string: "r.DriverDetectionMethod",                            gates: &[], first: Some(v(4, 12)),  last: None },
    Marker { string: "r.Mobile.EnableStaticAndCSMShadowReceivers",         gates: &[], first: Some(v(4, 12)),  last: None },
    Marker { string: "r.Tonemapper2084",                                   gates: &[], first: Some(v(4, 13)),  last: Some(v(4, 14)) },
    Marker { string: "r.TonemapperACESInversion",                          gates: &[], first: Some(v(4, 13)),  last: Some(v(4, 14)) },
    Marker { string: "r.TonemapperOutputGamut",                            gates: &[], first: Some(v(4, 13)),  last: Some(v(4, 14)) },
    Marker { string: "r.Mobile.Shadow.CSMShaderCulling",                   gates: &[], first: Some(v(4, 13)),  last: Some(v(4, 18)) },
    Marker { string: "r.UseProgramBinaryCache",                            gates: &[], first: Some(v(4, 13)),  last: Some(v(4, 20)) },
    Marker { string: "r.Tonemapper.ConfigIndexOverride",                   gates: &[], first: Some(v(4, 13)),  last: Some(v(4, 23)) },
    Marker { string: "r.Android.DisableVulkanSupport",                     gates: &[], first: Some(v(4, 13)),  last: None },
    Marker { string: "r.CapsuleIndirectShadowSelfShadowIntensity",         gates: &[], first: Some(v(4, 14)),  last: Some(v(4, 14)) },
    Marker { string: "r.Streaming.ScaleTexturesByGlobalMyBias",            gates: &[], first: Some(v(4, 14)),  last: Some(v(4, 15)) },
    Marker { string: "enablehighdpi",                                      gates: &[], first: Some(v(4, 14)),  last: Some(v(4, 17)) },
    Marker { string: "r.Photography.PersistEffects",                       gates: &[], first: Some(v(4, 14)),  last: Some(v(4, 18)) },
    Marker { string: "r.HLOD.DistanceScale",                               gates: &[], first: Some(v(4, 14)),  last: Some(v(4, 19)) },
    Marker { string: "FMonoscopicFarFieldMaskPS",                          gates: &[], first: Some(v(4, 15)),  last: Some(v(4, 15)) },
    Marker { string: "r.SceneAlpha",                                       gates: &[], first: Some(v(4, 15)),  last: Some(v(4, 15)) },
    Marker { string: "r.Mobile.AllowMovableDirectionalLights",             gates: &[], first: Some(v(4, 15)),  last: Some(v(5, 4)) },
    Marker { string: "r.PostProcessingColorFormat",                        gates: &[], first: Some(v(4, 15)),  last: None },
    Marker { string: "r.CopySceneColorOncePerViewOnly",                    gates: &[], first: Some(v(4, 16)),  last: Some(v(4, 16)) },
    Marker { string: "r.FastVRamLightAttenuation",                         gates: &[], first: Some(v(4, 16)),  last: Some(v(4, 16)) },
    Marker { string: "AACF_DriveAttribute_DEPRECATED",                     gates: &[], first: Some(v(4, 16)),  last: Some(v(4, 17)) },
    Marker { string: "AACF_DriveMaterial_DEPRECATED",                      gates: &[], first: Some(v(4, 16)),  last: Some(v(4, 17)) },
    Marker { string: "r.AOHistoryMinConfidenceScale",                      gates: &[], first: Some(v(4, 16)),  last: Some(v(4, 19)) },
    Marker { string: "SMU_OnlyTickPoseWhenRendered",                       gates: &[], first: Some(v(4, 16)),  last: Some(v(4, 20)) },
    Marker { string: "r.BinaryShaderCacheLogging",                         gates: &[], first: Some(v(4, 16)),  last: Some(v(4, 21)) },
    Marker { string: "r.TransientResourceAliasing.RenderTargets",          gates: &[], first: Some(v(4, 17)),  last: Some(v(4, 17)) },
    Marker { string: "r.UseUserShaderCache",                               gates: &[], first: Some(v(4, 17)),  last: Some(v(4, 21)) },
    Marker { string: "r.VT.NumMipsToExpandRequests",                       gates: &[], first: Some(v(4, 17)),  last: Some(v(4, 22)) },
    Marker { string: "r.Mobile.SceneColorFormat",                          gates: &[], first: Some(v(4, 17)),  last: None },
    Marker { string: "r.ConsoleTextScale",                                 gates: &[], first: Some(v(4, 18)),  last: Some(v(4, 18)) },
    Marker { string: "r.FastVRam.DistanceFieldAOConfidence",               gates: &[], first: Some(v(4, 18)),  last: Some(v(4, 19)) },
    Marker { string: "r.SaveShaderCache",                                  gates: &[], first: Some(v(4, 18)),  last: Some(v(4, 21)) },
    Marker { string: "r.AndroidDisableThreadedRenderingFirstLoad",         gates: &[], first: Some(v(4, 18)),  last: None },
    Marker { string: "r.ViewDistanceScaleNoScalability",                   gates: &[], first: Some(v(4, 19)),  last: Some(v(4, 19)) },
    Marker { string: "r.vulkan.CpuWaitForFence",                           gates: &[], first: Some(v(4, 19)),  last: Some(v(4, 19)) },
    Marker { string: "r.DefaultFeature.SpotLightUnits",                    gates: &[], first: Some(v(4, 19)),  last: Some(v(4, 20)) },
    Marker { string: "r.Mobile.EnableMovableLightCSMShaderCulling",        gates: &[], first: Some(v(4, 19)),  last: None },
    Marker { string: "r.Mobile.SeparateMaskedPass",                        gates: &[], first: Some(v(4, 20)),  last: Some(v(4, 21)) },
    Marker { string: "r.Mobile.Shadow.CSMDebugHint",                       gates: &[], first: Some(v(4, 20)),  last: Some(v(5, 0)) },
    Marker { string: "r.Android.DisableASTCSupport",                       gates: &[], first: Some(v(4, 20)),  last: None },
    Marker { string: "r.CookOutUnusedDetailModeComponents",                gates: &[], first: Some(v(4, 20)),  last: None },
    Marker { string: "r.ViewDistanceScale.SecondaryScale",                 gates: &[], first: Some(v(4, 20)),  last: None },
    Marker { string: "r.VT.TLSTranscodeCodecCacheSize",                    gates: &[], first: Some(v(4, 21)),  last: Some(v(4, 22)) },
    Marker { string: "r.Mobile.ForceFullPrecisionInPS",                    gates: &[], first: Some(v(4, 21)),  last: Some(v(4, 27)) },
    Marker { string: "r.Mobile.SkyLightPermutation",                       gates: &[], first: Some(v(4, 21)),  last: Some(v(5, 4)) },
    Marker { string: "r.Mobile.AllowDitheredLODTransition",                gates: &[], first: Some(v(4, 21)),  last: None },
    Marker { string: "au.BypassVirtualizeWhenSilent",                      gates: &[], first: Some(v(4, 22)),  last: Some(v(4, 22)) },
    Marker { string: "r.Vulkan.EnableTessellation",                        gates: &[], first: Some(v(4, 22)),  last: Some(v(4, 22)) },
    Marker { string: "p.CullPhiVisualizeDistance",                         gates: &[], first: Some(v(4, 22)),  last: Some(v(4, 23)) },
    Marker { string: "TaskGraph.EnablePowerSavingThreadPriorityReduction", gates: &[], first: Some(v(4, 22)),  last: Some(v(4, 25)) },
    Marker { string: "net.MaxNetStringSize",                               gates: &[], first: Some(v(4, 22)),  last: None },
    Marker { string: "p.ComputeConstraintsUseAny",                         gates: &[], first: Some(v(4, 23)),  last: Some(v(4, 23)) },
    Marker { string: "p.GatherVerbosePhysicsStats",                        gates: &[], first: Some(v(4, 23)),  last: Some(v(4, 23)) },
    Marker { string: "bRestrictLocalization",                              gates: &[], first: Some(v(4, 23)),  last: Some(v(4, 25)) },
    Marker { string: "r.Mobile.AllowPixelDepthOffset",                     gates: &[], first: Some(v(4, 23)),  last: None },
    Marker { string: "r.Mobile.SupportGPUScene",                           gates: &[], first: Some(v(4, 23)),  last: None },
    Marker { string: "r.VT.EvictFileCache",                                gates: &[], first: Some(v(4, 23)),  last: None },
    Marker { string: "p.Chaos.ImmPhys.DeltaTime",                          gates: &[], first: Some(v(4, 24)),  last: Some(v(4, 24)) },
    Marker { string: "p.ChaosParticleParallelFor",                         gates: &[], first: Some(v(4, 24)),  last: Some(v(4, 24)) },
    Marker { string: "r.Water.ConstantWaterDepth",                         gates: &[], first: Some(v(4, 24)),  last: Some(v(4, 24)) },
    Marker { string: "r.Mobile.UseGPUSceneTexture",                        gates: &[], first: Some(v(4, 24)),  last: Some(v(4, 27)) },
    Marker { string: "log.flushInterval",                                  gates: &[], first: Some(v(4, 24)),  last: None },
    Marker { string: "TestLockFreeWorker",                                 gates: &[], first: Some(v(4, 25)),  last: Some(v(4, 25)) },
    Marker { string: "r.AnisotropicBRDF",                                  gates: &[], first: Some(v(4, 25)),  last: Some(v(4, 25)) },
    Marker { string: "fc.NumFileCacheBlocks",                              gates: &[], first: Some(v(4, 25)),  last: Some(v(4, 27)) },
    Marker { string: "Freezing_bWithRayTracing",                           gates: &[], first: Some(v(4, 25)),  last: Some(v(5, 0)) },
    Marker { string: "r.Android.DisableVulkanSM5Support",                  gates: &[], first: Some(v(4, 25)),  last: None },
    Marker { string: "fx.Niagara.BatchGPUTickSubmit",                      gates: &[], first: Some(v(4, 26)),  last: Some(v(4, 26)) },
    Marker { string: "fx.Niagara.ConcurrentGPUTickInit",                   gates: &[], first: Some(v(4, 26)),  last: Some(v(4, 26)) },
    Marker { string: "p.CollisionCullDistance",                            gates: &[], first: Some(v(4, 26)),  last: Some(v(4, 26)) },
    Marker { string: "r.SupportAnisotropicMaterials",                      gates: &[], first: Some(v(4, 26)),  last: Some(v(4, 26)) },
    Marker { string: "D3D12.GlobalViewHeapBlockSize",                      gates: &[], first: Some(v(4, 26)),  last: Some(v(5, 0)) },
    Marker { string: "r.VolumetricCloud.HzbCulling",                       gates: &[], first: Some(v(4, 26)),  last: Some(v(5, 2)) },
    Marker { string: "p.Chaos.Solver.SleepEnabled",                        gates: &[], first: Some(v(4, 26)),  last: Some(v(5, 3)) },
    Marker { string: "r.ContactShadows.NonShadowCastingIntensity",         gates: &[], first: Some(v(4, 26)),  last: None },
    Marker { string: "r.Mobile.ShadingPath",                               gates: &[], first: Some(v(4, 26)),  last: None },
    Marker { string: "r.FASTBuild.Shader.BatchSize",                       gates: &[], first: Some(v(4, 27)),  last: Some(v(4, 27)) },
    Marker { string: "r.WPOPrimitivesOutputVelocity",                      gates: &[], first: Some(v(4, 27)),  last: Some(v(4, 27)) },
    Marker { string: "r.ShaderCompiler.JobCache",                          gates: &[], first: Some(v(4, 27)),  last: Some(v(5, 5)) },
    Marker { string: "s.EnforcePackageCompatibleVersionCheck",             gates: &[], first: Some(v(4, 27)),  last: None },
    Marker { string: "r.Lumen.ProbeHierarchy.Depth",                       gates: &[], first: Some(v(5, 0)),   last: Some(v(5, 0)) },
    Marker { string: "r.Nanite.SphereCullingFrustum",                      gates: &[], first: Some(v(5, 0)),   last: Some(v(5, 1)) },
    Marker { string: "r.MaterialEnableControlFlow",                        gates: &[], first: Some(v(5, 0)),   last: Some(v(5, 2)) },
    Marker { string: "r.Nanite.OptimizedRelevance",                        gates: &[], first: Some(v(5, 0)),   last: Some(v(5, 3)) },
    Marker { string: "r.Lumen.IrradianceFieldGather",                      gates: &[], first: Some(v(5, 0)),   last: Some(v(5, 7)) },
    Marker { string: "r.DemotedLocalMemoryWarning",                        gates: &[], first: Some(v(5, 0)),   last: None },
    Marker { string: "r.Nanite.AllowComputeMaterial",                      gates: &[], first: Some(v(5, 1)),   last: Some(v(5, 1)) },
    Marker { string: "r.Strata.AsyncClassification",                       gates: &[], first: Some(v(5, 1)),   last: Some(v(5, 1)) },
    Marker { string: "r.Strata.Debug.VisualizeMode",                       gates: &[], first: Some(v(5, 1)),   last: Some(v(5, 1)) },
    Marker { string: "gc.LockBehavior",                                    gates: &[], first: Some(v(5, 1)),   last: Some(v(5, 2)) },
    Marker { string: "p.Chaos.Solver.ValidateGraph",                       gates: &[], first: Some(v(5, 1)),   last: Some(v(5, 2)) },
    Marker { string: "r.GlobalDistanceField.Debug",                        gates: &[], first: Some(v(5, 1)),   last: Some(v(5, 3)) },
    Marker { string: "p.net.TargetNumBufferedCmds",                        gates: &[], first: Some(v(5, 1)),   last: Some(v(5, 5)) },
    Marker { string: "r.Mobile.ShadingModelsMask",                         gates: &[], first: Some(v(5, 1)),   last: None },
    Marker { string: "s.LargeMemoryDataMaxPoolLength",                     gates: &[], first: Some(v(5, 1)),   last: None },
    Marker { string: "s.RemoveUnreachableObjectsOnGT",                     gates: &[], first: Some(v(5, 1)),   last: None },
    Marker { string: "gc.DumpMemoryStats",                                 gates: &[], first: Some(v(5, 2)),   last: Some(v(5, 2)) },
    Marker { string: "r.RectLightAtlas.Translucent",                       gates: &[], first: Some(v(5, 2)),   last: Some(v(5, 3)) },
    Marker { string: "r.DynamicRes.DynamicFrameTime",                      gates: &[], first: Some(v(5, 2)),   last: Some(v(5, 5)) },
    Marker { string: "r.SubstrateBackCompatibility",                       gates: &[], first: Some(v(5, 2)),   last: Some(v(5, 6)) },
    Marker { string: "net.BitReader.EnsureOnOverflow",                     gates: &[], first: Some(v(5, 2)),   last: None },
    Marker { string: "r.MaterialEditor.LWCTruncateMode",                   gates: &[], first: Some(v(5, 2)),   last: None },
    Marker { string: "s.SkipChangelistCompatibilityVersionCheck",          gates: &[], first: Some(v(5, 2)),   last: None },
    Marker { string: "s.IasMaxHttpConnectionCount",                        gates: &[], first: Some(v(5, 3)),   last: Some(v(5, 3)) },
    Marker { string: "r.DX11NVAfterMathDumpWaitTime",                      gates: &[], first: Some(v(5, 3)),   last: Some(v(5, 4)) },
    Marker { string: "r.DX12NVAfterMathDumpWaitTime",                      gates: &[], first: Some(v(5, 3)),   last: Some(v(5, 4)) },
    Marker { string: "r.PathTracing.Override.Depth",                       gates: &[], first: Some(v(5, 3)),   last: Some(v(5, 4)) },
    Marker { string: "r.ManyLights.HairVoxelTraces",                       gates: &[], first: Some(v(5, 4)),   last: Some(v(5, 4)) },
    Marker { string: "r.ManyLights.LightFunctions",                        gates: &[], first: Some(v(5, 4)),   last: Some(v(5, 4)) },
    Marker { string: "r.ManyLights.WorldSpaceTraces",                      gates: &[], first: Some(v(5, 4)),   last: Some(v(5, 4)) },
    Marker { string: "D3D12.SamplerWarningThreshold",                      gates: &[], first: Some(v(5, 4)),   last: Some(v(5, 5)) },
    Marker { string: "r.PathTracing.CloudMapEnable",                       gates: &[], first: Some(v(5, 5)),   last: Some(v(5, 5)) },
    Marker { string: "r.Nanite.SkinningBuffers.Defrag",                    gates: &[], first: Some(v(5, 5)),   last: Some(v(5, 6)) },
    Marker { string: "r.MegaLights.Volume.Debug",                          gates: &[], first: Some(v(5, 5)),   last: Some(v(5, 7)) },
    Marker { string: "r.Vulkan.Bindless.BlockSize",                        gates: &[], first: Some(v(5, 5)),   last: Some(v(5, 7)) },
    Marker { string: "net.QueuedBatchTimeoutSeconds",                      gates: &[], first: Some(v(5, 6)),   last: Some(v(5, 6)) },
    Marker { string: "r.LensFlareBlurComputeShader",                       gates: &[], first: Some(v(5, 6)),   last: Some(v(5, 6)) },
    Marker { string: "r.MegaLights.DownsampleFactor",                      gates: &[], first: Some(v(5, 6)),   last: Some(v(5, 6)) },
    Marker { string: "r.Substrate.BlendableGBuffer",                       gates: &[], first: Some(v(5, 6)),   last: Some(v(5, 6)) },
    Marker { string: "au.DirectProceduralRendering",                       gates: &[], first: Some(v(5, 7)),   last: Some(v(5, 7)) },
    Marker { string: "p.Chaos.MinParallelTaskSize",                        gates: &[], first: Some(v(5, 7)),   last: Some(v(5, 7)) },
    Marker { string: "p.Chaos.SingleThreadPushData",                       gates: &[], first: Some(v(5, 7)),   last: Some(v(5, 7)) },
    Marker { string: "s.ImportTypeHierarchyEnabled",                       gates: &[], first: Some(v(5, 7)),   last: Some(v(5, 7)) },
    Marker { string: "D3D12.ResidencyDebugBudgetMB",                       gates: &[], first: Some(v(5, 8)),   last: None },
    Marker { string: "D3D12.ResourcesStartResident",                       gates: &[], first: Some(v(5, 8)),   last: None },
    Marker { string: "au.metasound.dump_poly_types",                       gates: &[], first: Some(v(5, 8)),   last: None },
    Marker { string: "gc.PauseGCFreeMemThresholdMB",                       gates: &[], first: Some(v(5, 8)),   last: None },

    Marker { string: "AudioComponent Dump",                                                                            gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "CLEANSCREENSHOTS",                                                                               gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "CONTENTCOMPARISON",                                                                              gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "CompressionState",                                                                               gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "ConsoleHelp.html",                                                                               gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "DUMPMATERIALSTATS",                                                                              gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "DUMPPARTICLECOUNTS",                                                                             gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "DebugTrackedTextures",                                                                           gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "Dry audio isolated",                                                                             gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "Dump Shadow Setup:",                                                                             gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "DumpBTUsageStats",                                                                               gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "FATALSCRIPTWARNINGS",                                                                            gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "INTRINSICCLASSES",                                                                               gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "Memory.StaleTest",                                                                               gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "Memory.UsePurgatory",                                                                            gates: &[NotShipping],  first: None,           last: None },
    Marker { string: "gc.FindStaleClusters",                                                                           gates: &[NotShipping],  first: None,           last: None },

    Marker { string: "Debug.OOMMemReport",                                                                             gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "GameplayTags.PackingTest",                                                                       gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "GameplayTags.PrintReport",                                                                       gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "Reattach.MaterialInstances",                                                                     gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "Reattach.Materials",                                                                             gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "TaskGraph.Randomize",                                                                            gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "TimedMemReport.Delay",                                                                           gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "gc.StressTestGC",                                                                                gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "net.TestObjRefSerialize",                                                                        gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "p.DebugTimeDiscrepancy",                                                                         gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "p.ShowInitialOverlaps",                                                                          gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "p.VisualizeMovement",                                                                            gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "r.LimitRenderingFeatures",                                                                       gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "r.MotionBlurFiltering",                                                                          gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "r.RenderTimeFrozen",                                                                             gates: &[DevOnly],       first: None,           last: None },
    Marker { string: "r.Shadow.FreezeCamera",                                                                          gates: &[DevOnly],       first: None,           last: None },

    Marker { string: "Time slicing cannot be disabled in Test or Shipping builds.  SetAllowTimeSlicing does nothing.", gates: &[TestOrShipping], first: Some(v(4, 11)), last: Some(v(4, 21)) },
    Marker { string: "Debug viewmodes not allowed in Test or Shipping builds.",                                        gates: &[TestOrShipping], first: Some(v(4, 26)), last: Some(v(5, 5)) },
    Marker { string: "TransientUserTexture",                                                                           gates: &[TestOrShipping], first: Some(v(5, 5)),  last: None },

    Marker { string: "Plugin commandlet disabled in shipping mode.",                                                   gates: &[ShippingOnly],      first: None,           last: None },
    Marker { string: "Logging interval in shipping. If set, this overrides archive.FlushInterval",                     gates: &[ShippingOnly],      first: Some(v(4, 24)), last: None },
    Marker { string: "log.flushInterval.Shipping",                                                                     gates: &[ShippingOnly],      first: Some(v(4, 24)), last: None },
    Marker { string: "Engine.VerifyLoadMapWorldCleanup.Severity.Shipping",                                             gates: &[ShippingOnly],      first: Some(v(5, 1)),  last: None },
    Marker { string: "Engine.VerifyLoadMapWorldCleanup.TraceMode.Shipping",                                            gates: &[ShippingOnly],      first: Some(v(5, 1)),  last: None },
    Marker { string: "GameFeaturePlugin.LeakedAssetTrace.Severity.Shipping",                                           gates: &[ShippingOnly],      first: Some(v(5, 2)),  last: None },
    Marker { string: "GameFeaturePlugin.LeakedAssetTrace.TraceMode.Shipping",                                          gates: &[ShippingOnly],      first: Some(v(5, 2)),  last: None },
    Marker { string: "LevelStreaming.Profiling.Enabled.Shipping",                                                      gates: &[ShippingOnly],      first: Some(v(5, 3)),  last: None },
    Marker { string: "PluginManager.LeakedAssetTrace.Severity.Shipping",                                               gates: &[ShippingOnly],      first: Some(v(5, 4)),  last: None },
    Marker { string: "PluginManager.LeakedAssetTrace.TraceMode.Shipping",                                              gates: &[ShippingOnly],      first: Some(v(5, 4)),  last: None },

    Marker { string: "Cycle counters (flat)",                              gates: &[WithStats], first: None, last: None },
    Marker { string: "Cycle counters (hierarchy)",                         gates: &[WithStats], first: None, last: None },
    Marker { string: "Dumps RHI memory stats to the log",                  gates: &[WithStats], first: None, last: None },
    Marker { string: "Empty stat command!",                                gates: &[WithStats], first: None, last: None },
    Marker { string: "FSimpleDelegateGraphTask.StatCmd",                   gates: &[WithStats], first: None, last: None },
    Marker { string: "FSimpleDelegateGraphTask.StatsToGame",               gates: &[WithStats], first: None, last: None },
    Marker { string: "Frame Messages Condensed",                           gates: &[WithStats], first: None, last: None },
    Marker { string: "Here is the brief list of stats console commands",   gates: &[WithStats], first: None, last: None },
    Marker { string: "If true, dump stat packets.",                        gates: &[WithStats], first: None, last: None },
    Marker { string: "Particle Dynamic Memory Stats",                      gates: &[WithStats], first: None, last: None },
    Marker { string: "ParticleData,Total(Bytes),FMath::Max(Bytes)",        gates: &[WithStats], first: None, last: None },
    Marker { string: "RHI resource memory (not tracked by our allocator)", gates: &[WithStats], first: None, last: None },
];

const MIN_MARKERS_PRESENT: usize = 10;
const CONTRADICTION: usize = 5;
const OMISSION: usize = 1;
const MIN_MARGIN: usize = 4;

fn candidates() -> impl Iterator<Item = Build> {
    use BuildConfiguration::*;
    std::iter::once(None)
        .chain(VERSIONS.iter().map(Some))
        .cartesian_product([Development, Test, Shipping])
        .cartesian_product([Stats::Off, Stats::On])
        .map(|((version, config), stats)| Build {
            version,
            config,
            stats,
        })
}

fn costs(present: &[bool]) -> Vec<usize> {
    candidates()
        .map(|build| {
            MARKERS
                .iter()
                .zip(present)
                .map(|(marker, present)| match (marker.covers(&build), present) {
                    (false, true) => CONTRADICTION,
                    (true, false) => OMISSION,
                    _ => 0,
                })
                .sum::<usize>()
        })
        .collect_vec()
}

fn resolve_axis<T: PartialEq>(
    costs: &[usize],
    axis: impl Fn(&Build) -> T,
    what: &'static str,
) -> crate::resolvers::Result<T> {
    let best = costs.iter().position_min().unwrap();
    let value = axis(&candidates().nth(best).unwrap());
    let runner_up = candidates()
        .zip(costs)
        .filter(|(build, _)| axis(build) != value)
        .map(|(_, cost)| *cost)
        .min()
        .unwrap();
    if runner_up - costs[best] < MIN_MARGIN {
        return Err(ResolveError::new_msg(format!(
            "no {what} explains the markers better than its neighbour"
        )));
    }
    Ok(value)
}

async fn fingerprint(ctx: &AsyncContext<'_>) -> crate::resolvers::Result<Vec<usize>> {
    let found = join_all(
        MARKERS
            .iter()
            .map(|m| ctx.scan(util::utf16_pattern(m.string))),
    )
    .await;
    let present = found.iter().map(|a| !a.is_empty()).collect_vec();
    if present.iter().filter(|p| **p).count() < MIN_MARKERS_PRESENT {
        bail_out!("too few marker strings to fingerprint");
    }
    Ok(costs(&present))
}

#[derive(Debug, PartialEq)]
#[cfg_attr(
    feature = "serde-resolvers",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct EngineVersionFingerprint(pub EngineVersion);
impl FromStr for EngineVersionFingerprint {
    type Err = ResolveError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

impl_resolver!(all, EngineVersionFingerprint, |ctx| async {
    let costs = fingerprint(ctx).await?;
    let Some(version) = resolve_axis(&costs, |build| build.version.cloned(), "version")? else {
        bail_out!("predates the oldest fingerprinted version");
    };
    Ok(Self(version))
});

impl_resolver!(all, BuildConfiguration, |ctx| async {
    let costs = fingerprint(ctx).await?;
    resolve_axis(&costs, |build| build.config, "build configuration")
});

impl_resolver!(all, Stats, |ctx| async {
    let costs = fingerprint(ctx).await?;
    resolve_axis(&costs, |build| build.stats, "stats setting")
});

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn fingerprint_table_is_well_formed() {
        let mut strings = MARKERS.iter().map(|m| m.string).collect_vec();
        let count = strings.len();
        strings.sort();
        strings.dedup();
        assert_eq!(strings.len(), count, "duplicate marker string");

        for marker in MARKERS {
            assert!(
                marker.first.is_some() || marker.last.is_some() || !marker.gates.is_empty(),
                "{}: present in every build, so it says nothing",
                marker.string
            );
        }

        // Every candidate must be the cheapest explanation of its own markers.
        for build in candidates() {
            let observed = MARKERS.iter().map(|m| m.covers(&build)).collect_vec();
            let costs = costs(&observed);
            assert_eq!(
                resolve_axis(&costs, |b| b.version.cloned(), "version").map_err(|e| e.to_string()),
                Ok(build.version.cloned()),
                "{build:?} does not resolve its own version"
            );
            assert_eq!(
                resolve_axis(&costs, |b| b.config, "configuration").map_err(|e| e.to_string()),
                Ok(build.config),
                "{build:?} does not resolve its own configuration"
            );
            assert_eq!(
                resolve_axis(&costs, |b| b.stats, "stats").map_err(|e| e.to_string()),
                Ok(build.stats),
                "{build:?} does not resolve its own stats setting"
            );
        }
    }
}
