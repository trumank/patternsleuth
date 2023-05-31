use anyhow::Result;

use super::{Pattern, PatternConfig, Resolution};

#[derive(
    Debug, Hash, Eq, PartialEq, PartialOrd, strum::Display, strum::EnumString, strum::EnumIter,
)]
pub enum Sig {
    #[strum(serialize = "FName::ToString")]
    FNameToString,
    #[strum(serialize = "FName::FName")]
    FNameFName,
    GMalloc,
    GUObjectArray,
    GNatives,
    //ProcessInternal, // not found by pattern scan
    //ProcessLocalScriptFunction, // not found by pattern scan
    #[strum(serialize = "StaticConstructObject_Internal")]
    StaticConstructObjectInternal,
    Pak,
    PatchPak,
    ConsoleManager,
}

#[derive(Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub enum PatternID {
    FNameToString(FNameToStringID),
    FNameFname(FNameFNameID),
    StaticConstructObjectInternal(StaticConstructObjectInternalID),
    GMalloc,
    GUObjectArray(GUObjectArrayID),
    GNatives,
    Pak,
    PatchPak,
    ConsoleManager,
}

impl PatternID {
    pub fn sig(&self) -> Sig {
        match self {
            Self::FNameToString(_) => Sig::FNameToString,
            Self::FNameFname(_) => Sig::FNameFName,
            Self::StaticConstructObjectInternal(_) => Sig::StaticConstructObjectInternal,
            Self::GMalloc => Sig::GMalloc,
            Self::GUObjectArray(_) => Sig::GUObjectArray,
            Self::GNatives => Sig::GNatives,
            Self::Pak => Sig::Pak,
            Self::PatchPak => Sig::PatchPak,
            Self::ConsoleManager => Sig::ConsoleManager,
        }
    }
    pub fn resolve(&self, data: &[u8], section: String, base: usize, m: usize) -> Resolution {
        match self {
            Self::FNameToString(f) => f.resolve(data, section, base, m),
            Self::FNameFname(f) => f.resolve(data, section, base, m),
            Self::StaticConstructObjectInternal(f) => f.resolve(data, section, base, m),
            Self::GMalloc => Resolution {
                section,
                stages: vec![],
                address: Some(m),
            },
            Self::GUObjectArray(f) => f.resolve(data, section, base, m),
            Self::GNatives => {
                let stages = vec![m];
                for i in m - base..m - base + 400 {
                    if data[i] == 0x4c
                        && data[i + 1] == 0x8d
                        && (data[i + 2] & 0xc7 == 5 && data[i + 2] > 0x20)
                    {
                        let address = (base + i + 7)
                            .checked_add_signed(i32::from_le_bytes(
                                data[i + 3..i + 3 + 4].try_into().unwrap(),
                            ) as isize);
                        return Resolution {
                            section,
                            stages,
                            address,
                        };
                    }
                }
                Resolution {
                    section,
                    stages,
                    address: None,
                }
            }
            Self::Pak => Resolution {
                section,
                stages: vec![],
                address: Some(m),
            },
            Self::PatchPak => Resolution {
                section,
                stages: vec![],
                address: Some(m),
            },
            Self::ConsoleManager => Resolution {
                section,
                stages: vec![],
                address: Some(m),
            },
        }
    }
}

pub fn get_patterns() -> Result<Vec<PatternConfig>> {
    Ok(vec![
        PatternConfig::new(
            PatternID::FNameToString(FNameToStringID::A),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? 48 8B 4C 24 ?? 8B FD 48 85 C9")?,
        ),
        PatternConfig::new(
            PatternID::FNameToString(FNameToStringID::B),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? BD 01 00 00 00 41 39 6E ?? 0F 8E")?,
        ),

        PatternConfig::new(
            PatternID::FNameFname(FNameFNameID::A),
            Some(object::SectionKind::Text),
            Pattern::new("40 53 48 83 EC ?? 41 B8 01 00 00 00 48 8D 15 ?? ?? ?? ?? 48 8D 4C 24 ?? E8 ?? ?? ?? ?? B9")?
        ),
        PatternConfig::new(
            PatternID::FNameFname(FNameFNameID::V5_1),
            Some(object::SectionKind::Text),
            Pattern::new("57 48 83 EC 50 41 B8 01 00 00 00 0F 29 74 24 40 48 8D ?? ?? ?? ?? ?? 48 8D 4C 24 60 E8")?
        ),

        PatternConfig::new(
            PatternID::StaticConstructObjectInternal(StaticConstructObjectInternalID::A),
            Some(object::SectionKind::Text),
            Pattern::new("C0 E9 02 32 88 ?? ?? ?? ?? 80 E1 01 30 88 ?? ?? ?? ?? 48")?,
        ),
        PatternConfig::new(
            PatternID::StaticConstructObjectInternal(StaticConstructObjectInternalID::V4_12),
            Some(object::SectionKind::Text),
            Pattern::new("89 8E C8 03 00 00 3B 8E CC 03 00 00 7E 0F 41 8B D6 48 8D 8E C0 03 00 00")?,
        ),
        PatternConfig::new(
            PatternID::StaticConstructObjectInternal(StaticConstructObjectInternalID::V4_16_4_19),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? 0F B6 8F ?? 01 00 00 48 89 87 ?? 01 00 00")?,
        ),
        PatternConfig::new(
            PatternID::StaticConstructObjectInternal(StaticConstructObjectInternalID::V5_0),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? 48 8B D8 48 39 75 30 74 15")?,
        ),

        PatternConfig::new(
            PatternID::GMalloc,
            Some(object::SectionKind::Text),
            Pattern::new("48 85 C9 74 2E 53 48 83 EC 20 48 8B D9 48 8B ?? ?? ?? ?? ?? 48 85 C9")?,
        ),

        PatternConfig::new(
            PatternID::GUObjectArray(GUObjectArrayID::A),
            Some(object::SectionKind::Text),
            Pattern::new("48 03 ?? ?? ?? ?? ?? 48 8B 10 48 85 D2 74 07")?,
        ),
        PatternConfig::new(
            PatternID::GUObjectArray(GUObjectArrayID::V4_20),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B ?? ?? ?? ?? ?? 48 8B 0C C8 ?? 8B 04 ?? 48 85 C0")?, // > 4.20
        ),

        PatternConfig::new(
            PatternID::GNatives,
            Some(object::SectionKind::Text),
            Pattern::new("cc 51 20 01")?,
        ),
        PatternConfig::new(
            PatternID::Pak,
            Some(object::SectionKind::Text),
            Pattern::new("48 89 5c 24 10 48 89 74 24 18 48 89 7c 24 20 55 41 54 41 55 41 56 41 57 48 8d ac 24 20 fe ff ff 48 81 ec e0 02 00 00 48 8b 05 ?? ?? ?? ?? 48 33 c4 48 89 85 d0 01 00 00")?,
        ),
        PatternConfig::new(
            PatternID::PatchPak,
            None,
            Pattern::new("5f 00 50 00 2e 00 70 00 61 00 6b")?,
        ),
        PatternConfig::new(
            PatternID::ConsoleManager,
            None,
            Pattern::new("72 00 2e 00 44 00 75 00 6d 00 70 00 69 00 6e 00 67 00 4d 00 6f 00 76 00 69 00 65 00")?,
        ),
    ])
}

#[derive(Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub enum FNameToStringID {
    A,
    B,
}
impl FNameToStringID {
    fn resolve(&self, data: &[u8], section: String, base: usize, m: usize) -> Resolution {
        let stages = vec![m];
        let n = (m - base).checked_add_signed(5).unwrap();
        let rel = i32::from_le_bytes(data[n - 4..n].try_into().unwrap());
        let address = n.checked_add_signed(rel as isize).map(|a| base + a);
        Resolution {
            section,
            stages,
            address,
        }
    }
}
#[derive(Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub enum FNameFNameID {
    A,
    V5_1,
}
impl FNameFNameID {
    fn resolve(&self, data: &[u8], section: String, base: usize, m: usize) -> Resolution {
        let stages = vec![m];
        match self {
            Self::A => {
                let n = (m - base).checked_add_signed(0x18 + 5).unwrap();
                let address = n
                    .checked_add_signed(
                        i32::from_le_bytes(data[n - 4..n].try_into().unwrap()) as isize
                    )
                    .map(|a| base + a);
                Resolution {
                    section,
                    stages,
                    address,
                }
            }
            Self::V5_1 => {
                let n = (m - base).checked_add_signed(0x1C + 5).unwrap();
                let address = n
                    .checked_add_signed(
                        i32::from_le_bytes(data[n - 4..n].try_into().unwrap()) as isize
                    )
                    .map(|a| base + a);
                Resolution {
                    section,
                    stages,
                    address,
                }
            }
        }
    }
}
#[derive(Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub enum StaticConstructObjectInternalID {
    A,
    V4_12,
    V4_16_4_19,
    V5_0,
}
impl StaticConstructObjectInternalID {
    fn resolve(&self, data: &[u8], section: String, base: usize, m: usize) -> Resolution {
        let stages = vec![m];
        match self {
            Self::A | Self::V4_12 => {
                let n = m - base - 0x0e;
                let address = n
                    .checked_add_signed(
                        i32::from_le_bytes(data[n - 4..n].try_into().unwrap()) as isize
                    )
                    .map(|a| base + a);
                Resolution {
                    section,
                    stages,
                    address,
                }
            }
            Self::V4_16_4_19 | Self::V5_0 => {
                let n = m - base + 5;
                let address = n
                    .checked_add_signed(
                        i32::from_le_bytes(data[n - 4..n].try_into().unwrap()) as isize
                    )
                    .map(|a| base + a);
                Resolution {
                    section,
                    stages,
                    address,
                }
            }
        }
    }
}
#[derive(Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub enum GUObjectArrayID {
    A,
    V4_20,
}
impl GUObjectArrayID {
    fn resolve(&self, data: &[u8], section: String, base: usize, m: usize) -> Resolution {
        let stages = vec![m];
        match self {
            Self::A => unimplemented!(),
            Self::V4_20 => {
                let n = m - base + 3;
                let address = n
                    .checked_add_signed(
                        i32::from_le_bytes(data[n..n + 4].try_into().unwrap()) as isize
                    )
                    .map(|a| base + a - 0xc);
                Resolution {
                    section,
                    stages,
                    address,
                }
            }
        }
    }
}
