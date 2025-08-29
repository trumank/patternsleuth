use std::fmt::{Debug, Display};

use futures::future::join_all;

use itertools::Itertools;
use patternsleuth_scanner::Pattern;

use crate::{
    MemoryTrait,
    resolvers::{bail_out, impl_resolver, try_ensure_one},
};

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
    use crate::{Addressable, Matchable, MemoryTrait};
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
