use anyhow::Result;

use super::{MountedPE, Pattern, PatternConfig, Resolution};

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
    IConsoleManagerSingleton,
}

pub fn get_patterns() -> Result<Vec<PatternConfig>> {
    Ok(vec![
        PatternConfig::new(
            Sig::FNameToString,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? 48 8B 4C 24 ?? 8B FD 48 85 C9")?,
            FNameToStringID::resolve,
        ),
        PatternConfig::new(
            Sig::FNameToString,
            "B".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? BD 01 00 00 00 41 39 6E ?? 0F 8E")?,
            FNameToStringID::resolve,
        ),

        PatternConfig::new(
            Sig::FNameFName,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("40 53 48 83 EC ?? 41 B8 01 00 00 00 48 8D 15 ?? ?? ?? ?? 48 8D 4C 24 ?? E8 ?? ?? ?? ?? B9")?,
            FNameFNameID::resolve_a,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            "V5.1".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("57 48 83 EC 50 41 B8 01 00 00 00 0F 29 74 24 40 48 8D ?? ?? ?? ?? ?? 48 8D 4C 24 60 E8")?,
            FNameFNameID::resolve_v5_1,
        ),

        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("C0 E9 02 32 88 ?? ?? ?? ?? 80 E1 01 30 88 ?? ?? ?? ?? 48")?,
            StaticConstructObjectInternalID::resolve_a_v4_12,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "V4.12".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("89 8E C8 03 00 00 3B 8E CC 03 00 00 7E 0F 41 8B D6 48 8D 8E C0 03 00 00")?,
            StaticConstructObjectInternalID::resolve_v4_16_4_19_v5_0,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "V4.16 - V4.19".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? 0F B6 8F ?? 01 00 00 48 89 87 ?? 01 00 00")?,
            StaticConstructObjectInternalID::resolve_v4_16_4_19_v5_0,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "V5.0".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? 48 8B D8 48 39 75 30 74 15")?,
            StaticConstructObjectInternalID::resolve_v4_16_4_19_v5_0,
        ),

        PatternConfig::new(
            Sig::GMalloc,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 85 C9 74 2E 53 48 83 EC 20 48 8B D9 48 8B ?? ?? ?? ?? ?? 48 85 C9")?,
            resolve_self,
        ),

        PatternConfig::new(
            Sig::GUObjectArray,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 03 ?? ?? ?? ?? ?? 48 8B 10 48 85 D2 74 07")?,
            GUObjectArrayID::resolve_a,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            ">V4.20".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B ?? ?? ?? ?? ?? 48 8B 0C C8 ?? 8B 04 ?? 48 85 C0")?,
            GUObjectArrayID::resolve_v_20,
        ),

        PatternConfig::new(
            Sig::GNatives,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("cc 51 20 01")?,
            GNatives::resolve,
        ),
        PatternConfig::new(
            Sig::Pak,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 89 5c 24 10 48 89 74 24 18 48 89 7c 24 20 55 41 54 41 55 41 56 41 57 48 8d ac 24 20 fe ff ff 48 81 ec e0 02 00 00 48 8b 05 ?? ?? ?? ?? 48 33 c4 48 89 85 d0 01 00 00")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::PatchPak,
            "A".to_string(),
            None,
            Pattern::new("5f 00 50 00 2e 00 70 00 61 00 6b")?,
            resolve_self,
        ),

        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B 0D ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 8B 01 4C 8D 0D")?,
            Test::resolve_a,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "B".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B 0D ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 8B 01 4C 8D 4C 24")?,
            Test::resolve_b,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "C".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 83 EC ?? 48 8B 0D ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? ?? 48 8B 0D")?,
            Test::resolve_c,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "D".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 89 3D ?? ?? ?? ?? 48 85 FF 75 ?? E8 ?? ?? ?? ?? 48 8B 3D ?? ?? ?? ?? 48 8B 07")?,
            Test::resolve_d,
        ),
    ])
}

/// do nothing, return address of pattern
pub fn resolve_self(_memory: &MountedPE, section: String, match_address: usize) -> Resolution {
    Resolution {
        section,
        stages: vec![],
        address: Some(match_address),
    }
}

#[allow(non_snake_case)]
mod FNameToStringID {
    use super::*;
    pub fn resolve(memory: &MountedPE, section: String, match_address: usize) -> Resolution {
        let stages = vec![match_address];
        let n = match_address + 5;
        let rel = i32::from_le_bytes(memory[n - 4..n].try_into().unwrap());
        let address = n.checked_add_signed(rel as isize);
        Resolution {
            section,
            stages,
            address,
        }
    }
}

#[allow(non_snake_case)]
mod FNameFNameID {
    use super::*;
    pub fn resolve_a(memory: &MountedPE, section: String, match_address: usize) -> Resolution {
        let stages = vec![match_address];
        let n = match_address.checked_add_signed(0x18 + 5).unwrap();
        let address =
            n.checked_add_signed(i32::from_le_bytes(memory[n - 4..n].try_into().unwrap()) as isize);
        Resolution {
            section,
            stages,
            address,
        }
    }
    pub fn resolve_v5_1(memory: &MountedPE, section: String, match_address: usize) -> Resolution {
        let stages = vec![match_address];
        let n = match_address.checked_add_signed(0x1C + 5).unwrap();
        let address =
            n.checked_add_signed(i32::from_le_bytes(memory[n - 4..n].try_into().unwrap()) as isize);
        Resolution {
            section,
            stages,
            address,
        }
    }
}

#[allow(non_snake_case)]
mod StaticConstructObjectInternalID {
    use super::*;
    pub fn resolve_a_v4_12(
        memory: &MountedPE,
        section: String,
        match_address: usize,
    ) -> Resolution {
        let stages = vec![match_address];
        let n = match_address - 0x0e;
        let address =
            n.checked_add_signed(i32::from_le_bytes(memory[n - 4..n].try_into().unwrap()) as isize);
        Resolution {
            section,
            stages,
            address,
        }
    }
    pub fn resolve_v4_16_4_19_v5_0(
        memory: &MountedPE,
        section: String,
        match_address: usize,
    ) -> Resolution {
        let stages = vec![match_address];
        let n = match_address + 5;
        let address =
            n.checked_add_signed(i32::from_le_bytes(memory[n - 4..n].try_into().unwrap()) as isize);
        Resolution {
            section,
            stages,
            address,
        }
    }
}

#[allow(non_snake_case)]
mod GUObjectArrayID {
    use super::*;
    pub fn resolve_a(_memory: &MountedPE, section: String, match_address: usize) -> Resolution {
        Resolution {
            // TODO
            section,
            stages: vec![],
            address: Some(match_address),
        }
    }
    pub fn resolve_v_20(memory: &MountedPE, section: String, match_address: usize) -> Resolution {
        let stages = vec![match_address];
        let n = match_address + 3;
        let address = n
            .checked_add_signed(i32::from_le_bytes(memory[n..n + 4].try_into().unwrap()) as isize)
            .map(|a| a - 0xc);
        Resolution {
            section,
            stages,
            address,
        }
    }
}

#[allow(non_snake_case)]
mod GNatives {
    use super::*;
    pub fn resolve(memory: &MountedPE, section: String, match_address: usize) -> Resolution {
        let stages = vec![match_address];
        for i in match_address..match_address + 400 {
            if memory[i] == 0x4c
                && memory[i + 1] == 0x8d
                && (memory[i + 2] & 0xc7 == 5 && memory[i + 2] > 0x20)
            {
                let address = (i + 7).checked_add_signed(i32::from_le_bytes(
                    memory[i + 3..i + 3 + 4].try_into().unwrap(),
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
}

#[allow(non_snake_case)]
mod Test {
    use super::*;
    pub fn resolve_a(memory: &MountedPE, section: String, match_address: usize) -> Resolution {
        let stages = vec![match_address];
        let n = match_address + 3;
        let address = n
            .checked_add_signed(i32::from_le_bytes(memory[n..n + 4].try_into().unwrap()) as isize)
            .map(|a| a - 4);
        Resolution {
            section,
            stages,
            address,
        }
    }
    pub fn resolve_b(memory: &MountedPE, section: String, match_address: usize) -> Resolution {
        let stages = vec![match_address];
        let n = match_address + 3;
        let address = n
            .checked_add_signed(i32::from_le_bytes(memory[n..n + 4].try_into().unwrap()) as isize)
            .map(|a| a - 4);
        Resolution {
            section,
            stages,
            address,
        }
    }
    pub fn resolve_c(memory: &MountedPE, section: String, match_address: usize) -> Resolution {
        let stages = vec![match_address];
        let n = match_address + 7;
        let address = n
            .checked_add_signed(i32::from_le_bytes(memory[n..n + 4].try_into().unwrap()) as isize)
            .map(|a| a - 4);
        Resolution {
            section,
            stages,
            address,
        }
    }
    pub fn resolve_d(memory: &MountedPE, section: String, match_address: usize) -> Resolution {
        let stages = vec![match_address];
        let n = match_address + 3;
        let address = n
            .checked_add_signed(i32::from_le_bytes(memory[n..n + 4].try_into().unwrap()) as isize)
            .map(|a| a - 4);
        Resolution {
            section,
            stages,
            address,
        }
    }
}
