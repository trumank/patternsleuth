use anyhow::Result;

use crate::ResolutionType;

use super::{MountedPE, Pattern, PatternConfig, Resolution, ResolveContext};

#[derive(
    Debug,
    Clone,
    Hash,
    Eq,
    PartialEq,
    PartialOrd,
    strum::Display,
    strum::EnumString,
    strum::EnumIter,
)]
pub enum Sig {
    AllowCheats,
    CameraWriteTransform,     // Only in older engines (4.19 and earlier)
    CameraWriteTransformMenu, // Only in older engines (4.17 and earlier)
    CameraARCorrectionFMinimalViewInfo,
    CameraARCorrectionCameraComponent,
    EngineVersion,
    #[strum(serialize = "FMinimalViewInfo::FMinimalViewInfo")]
    FMinimalViewInfoCTor, // FMinimalViewInfo::FMinimalViewInfo and operator= are equal in code but called at different locations. One of the matches is the ctor the other is the = operator.
    FMinimalViewInfoLockFOV,
    #[strum(serialize = "FName::ToString")]
    FNameToString,
    #[strum(serialize = "FName::FName")]
    FNameFName,
    GEngine,
    #[strum(serialize = "AWorldSettings::GetEffectiveTimeDilation")]
    GetEffectiveTimeDilation,
    GMalloc,
    GNatives,
    GUObjectArray,
    IConsoleManagerSingleton,
    NamePoolData,
    #[strum(serialize = "FSlateApplication::OnApplicationActivationChanged")]
    OnApplicationActivationChanged,
    Pak,
    PatchPak,
    #[strum(serialize = "UObject::ProcessEvent")]
    ProcessEvent,
    #[strum(serialize = "UObject::ProcessEvent (from call)")]
    ProcessEventFromCall,
    //ProcessInternal, // not found by pattern scan
    //ProcessLocalScriptFunction, // not found by pattern scan
    #[strum(serialize = "StaticConstructObject_Internal")]
    StaticConstructObjectInternal,
    #[strum(serialize = "UWorld::IsPaused")]
    UWorldIsPaused,
    #[strum(serialize = "UWorld::SpawnActor")]
    UWorldSpawnActor,
    #[strum(serialize = "UWorld::SpawnActor (from call)")]
    UWorldSpawnActorFromCall,
    WidgetOpacityBlendMultiply, // In SCompoundWidget::OnPaint
    WidgetPaintOpacityRead,
}

pub fn get_patterns() -> Result<Vec<PatternConfig>> {
    Ok(vec![
        //===============================[OnApplicationActivationChanged]=============================================================================================
        PatternConfig::new(
            Sig::OnApplicationActivationChanged,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("48 83 EC 28 | 48 81 C1 ?? ?? FF FF E8 ?? ?? ?? ?? B0 01 48 83 C4 28 C3")?,
            resolve_self,
        ),

        //===============================[GetEffectiveTimeDilation]=============================================================================================
        PatternConfig::new(
            Sig::GetEffectiveTimeDilation,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("F3 0F 10 81 ?? ?? 00 00 F3 0F 59 81 ?? ?? 00 00 F3 0F 59 81 ?? ?? 00 00 0F 28 D0")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::GetEffectiveTimeDilation,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("F3 0F 10 81 ?? 03 00 00 F3 0F 59 81 ?? 03 00 00 F3 0F 59 81 ?? 03 00 00 C3")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::GetEffectiveTimeDilation,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("F3 0F 10 81 ?? 04 00 00 F3 0F 59 81 ?? 04 00 00 F3 0F 59 81 ?? 04 00 00 C3")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::GetEffectiveTimeDilation,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("F3 0F 10 81 ?? ?? 00 00 F3 0F 59 81 ?? ?? 00 00 F3 0F 59 81 ?? ?? 00 00 C3")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::GetEffectiveTimeDilation,
            "UUU4_Alternative4".to_string(),
            None,
            Pattern::new("F3 0F 10 81 ?? ?? 00 00 F3 0F 59 81 ?? ?? 00 00 F3 0F 59 81 ?? ?? 00 00")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::GetEffectiveTimeDilation,
            "UUU4_Alternative5".to_string(),
            None,
            Pattern::new("F3 0F 10 89 ?? ?? 00 00 F3 0F 59 89 ?? ?? 00 00 F3 0F 59 89 ?? ?? 00 00 0F 28 C1")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::GetEffectiveTimeDilation,
            "UUU4_Alternative6".to_string(),
            None,
            Pattern::new("C5 FA 10 81 ?? ?? 00 00 C5 FA 59 89 ?? ?? 00 00 C5 F2 59 81 ?? ?? 00 00 C3")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::GetEffectiveTimeDilation,
            "UUU4_Alternative7_411".to_string(),
            None,
            Pattern::new("F3 41 0F 10 86 ?? 04 00 00 F3 41 0F 59 86 ?? 04 00 00 F3 41 0F 59 86 ?? 04 00 00 F3 0F 59 F0")?,
            resolve_self,
        ),

        //===============================[UWorldIsPaused]=============================================================================================
        PatternConfig::new(
            Sig::UWorldIsPaused,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("F7 83 ?? ?? 00 00 ?? ?? 00 00 75 ?? ?? C0 48 83 C4 20 5B C3 B0 01 48 83 C4 20 5B C3")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldIsPaused,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("80 BB ?? ?? 00 00 00 7C ?? ?? C0 48 83 C4 20 5B C3 B0 01 48 83 C4 20 5B C3")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldIsPaused,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("F7 83 ?? ?? 00 00 ?? ?? 00 00 75 ?? ?? C0 48 8B 5C 24 30 48 83 C4 20 5F C3 48 8B 5C")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldIsPaused,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("F7 83 30 08 00 00 00 10 00 00 75 ?? 30 C0")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldIsPaused,
            "UUU4_Alternative4".to_string(),
            None,
            Pattern::new("F7 83 ?? ?? 00 00 00 10 00 00 75 ?? | 30 C0 48 83 C4 20 ?? C3 B0 01 48 83 C4 20 ?? C3")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldIsPaused,
            "UUU4_Alternative5".to_string(),
            None,
            Pattern::new("F7 83 ?? ?? 00 00 00 10 00 00 74 08 B0 01 48 83 C4 20 5B C3 30 C0")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldIsPaused,
            "UUU4_Alternative6".to_string(),
            None,
            Pattern::new("F7 83 ?? ?? 00 00 00 10 00 00 74 08 B0 01 48 83 C4 20 5B C3 32 C0")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldIsPaused,
            "UUU4_Alternative7".to_string(),
            None,
            Pattern::new("F6 83 ?? 01 00 00 01 75 08 | ?? C0 48 83 C4 20 5B C3 B0 01")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldIsPaused,
            "UUU4_Alternative8".to_string(),
            None,
            Pattern::new("F6 83 ?? 01 00 00 10 75 ?? ?? C0 48 8B 5C 24 30 48 83 C4")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldIsPaused,
            "UUU4_Alternative9".to_string(),
            None,
            Pattern::new("40 38 BB ?? 01 00 00 7C ?? ?? C0 48 8B 5C 24 30 48 83 C4 20 5F C3 48 8B 5C 24 30 B0 01")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldIsPaused,
            "UUU4_Alternative10".to_string(),
            None,
            Pattern::new("F6 83 25 01 00 00 40 74 08 B0 01 48 83 C4 20 5B C3 ?? C0 48 83 C4 20 5B C3")?,
            resolve_self,
        ),

        //===============================[ProcessEventFromCall]=============================================================================================
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("F0 0F B1 0D ?? ?? ?? ?? 75 0E ?? ?? ?? 48 ?? ?? 48 ?? ?? E8 | ?? ?? ?? ?? 48 8B ?? 24 ?? 48 8B ?? 24 38 48 8B ?? 24 40 48 83")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("F0 0F B1 11 75 0E ?? ?? ?? 48 ?? ?? 48 ?? ?? E8 | ?? ?? ?? ?? 48 8B ?? 24 ?? 48 8B ?? 24 38 48 8B ?? 24 40 48 83")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("84 C0 75 0E ?? ?? ?? 48 ?? ?? 48 ?? ?? E8 | ?? ?? ?? ?? 48 8B ?? 24 ?? 48 8B ?? 24 38 48 8B ?? 24 40 48 83")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("4C 8B C5 48 8B D7 48 8B CB E8 | ?? ?? ?? ?? 48 8B 5C 24 30 48 8B 6C 24 38 48 8B 74 24 40 48 8B 7C 24 48 48 83 C4 20 41 5E C3")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative4_modular".to_string(),
            None,
            Pattern::new("F0 0F B1 11 75 0F 4C 8B C5 48 8B D7 48 8B CB FF 15 | ?? ?? ?? ?? 48 8B 74 24 30")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative5".to_string(),
            None,
            Pattern::new("48 8B D6 48 8B CB E8 | ?? ?? ?? ?? 8B 83 ?? ?? ?? ?? 85 C0 75 27 87 BB")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative6".to_string(),
            None,
            Pattern::new("75 0E ?? ?? ?? 48 ?? ?? 48 ?? ?? E8 | ?? ?? ?? ?? 48 8B ?? 24 ?? 48 8B ?? 24 38 48 8B ?? 24 40 48 83")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),

        //===============================[ProcessEvent]=============================================================================================
        PatternConfig::new(
            Sig::ProcessEvent,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("55 56 57 41 54 41 55 41 56 41 57 48 81 EC F0 00 00 00 48 8D 6C 24 30")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::ProcessEvent,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("55 56 57 41 54 41 55 41 56 41 57 48 81 EC ?? 00 00 00 48 8D 6C 24 ?? 48 89 9D 18 01 00 00")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::ProcessEvent,
            "UUU4_Alternative2_push_alt".to_string(),
            None,
            Pattern::new("48 55 56 57 41 54 41 55 41 56 41 57 48 81 EC F0 00 00 00 48 8D 6C 24 30")?,
            resolve_self,
        ),

        //===============================[GEngine]=============================================================================================
        PatternConfig::new(
            Sig::GEngine,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("48 8B 05 | ?? ?? ?? ?? 48 8B 88 ?? ?? 00 00 48 85 C9 74 ?? 48 8B 49 ?? 48 85 C9")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GEngine,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("48 8B 05 | ?? ?? ?? ?? 48 8B 88 ?? 07 00 00 48 85 C9 74 ?? 48 8B 51 50")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GEngine,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("48 8B 05 | ?? ?? ?? ?? 48 8B 88 ?? ?? ?? ?? 48 85 C9 0F 84 ?? ?? ?? ?? 48 89 74 24 50 48 8B 71")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),

        //===============================[EngineVersion]=============================================================================================
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("C7 03 | 04 00 ?? 00 66 89 4B 04 48 3B F8 74 ?? 48")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("C7 05 ?? ?? ?? ?? | 04 00 ?? 00 66 89 ?? ?? ?? ?? ?? C7 05")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("C7 05 ?? ?? ?? ?? | 04 00 ?? 00 66 89 ?? ?? ?? ?? ?? 89")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("41 C7 ?? | 04 00 ?? 00 ?? ?? 00 00 00 66 41 89")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative4".to_string(),
            None,
            Pattern::new("41 C7 ?? | 04 00 18 00 66 41 89 ?? 04")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative5".to_string(),
            None,
            Pattern::new("41 C7 04 24 | 04 00 ?? 00 66 ?? 89 ?? 24")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative6".to_string(),
            None,
            Pattern::new("41 C7 04 24 | 04 00 ?? 00 B9 ?? 00 00 00")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative7".to_string(),
            None,
            Pattern::new("C7 05 ?? ?? ?? ?? | 04 00 ?? 00 89 05 ?? ?? ?? ?? E8")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative8".to_string(),
            None,
            Pattern::new("C7 05 ?? ?? ?? ?? | 04 00 ?? 00 66 89 ?? ?? ?? ?? ?? 89 05")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative9".to_string(),
            None,
            Pattern::new("C7 46 20 | 04 00 ?? 00 66 44 89 76 24 44 89 76 28 48 39 C7")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative10".to_string(),
            None,
            Pattern::new("C7 03 | 04 00 ?? 00 66 44 89 63 04 C7 43 08 C1 5C 08 80 E8")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative11".to_string(),
            None,
            Pattern::new("C7 47 20 | 04 00 ?? 00 66 89 6F 24 C7 47 28 ?? ?? ?? ?? 49")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU4_Alternative12".to_string(),
            None,
            Pattern::new("C7 03 | 04 00 ?? 00 66 89 6B 04 89 7B 08 48 83 C3 10")?,
            resolve_engine_version,
        ),

        //===============================[UWorldSpawnActor]=============================================================================================
        PatternConfig::new(
            Sig::UWorldSpawnActor,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("40 53 48 83 EC ?? 48 8B 05 ?? ?? ?? ?? 48 ?? ?? 48 89 44 24 60 0F 28 1D ?? ?? ?? ?? 0F 57 D2")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldSpawnActor,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("40 53 56 57 48 83 EC ?? 48 8B 05 ?? ?? ?? ?? 48 ?? ?? 48 89 44 24 ?? 0F 28 1D")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldSpawnActor,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("53 56 57 48 83 EC ?? 48 8B 05 ?? ?? ?? ?? 48 ?? ?? 48 89 44 24 ?? 0F 28 1D")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::UWorldSpawnActor,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("40 53 56 57 48 83 EC 70 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 44 24 60 48 8B B4 24 B0 00 00 00")?,
            resolve_self,
        ),

        //===============================[UWorldSpawnActorFromCall]=============================================================================================
        PatternConfig::new(
            Sig::UWorldSpawnActorFromCall,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("48 89 44 24 ?? E8 | ?? ?? ?? ?? 48 85 C0 0F 85 ?? ?? 00 00 48 8D 05")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::UWorldSpawnActorFromCall,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("4C 8D 4D B7 4C 8D 44 24 ?? 48 8B D7 E8 | ?? ?? ?? ?? 48 85 C0 0F 85")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::UWorldSpawnActorFromCall,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("4C 8D 44 24 ?? ?? 8B ?? E8 | ?? ?? ?? ?? 48 85 C0 0F 85 ?? ?? 00 00 48 8D 05")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),

        //===============================[WidgetPaintOpacityRead]=============================================================================================
        PatternConfig::new(
            Sig::WidgetPaintOpacityRead,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("F3 0F 59 83 ?? ?? ?? ?? F3 0F 58 E6 0F C6 F6 FF 0F C6 C9 93")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::WidgetPaintOpacityRead,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("F3 0F 59 83 ?? ?? ?? ?? 0F 11 7D ?? 0F C6 C9 93 F3 0F 10 C8")?,
            resolve_self,
        ),

        //===============================[WidgetOpacityBlendMultiply]=============================================================================================
        PatternConfig::new(
            Sig::WidgetOpacityBlendMultiply,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("F3 0F 59 8F ?? ?? 00 00 F3 0F 11 44 24 ?? F3 0F 11 4C 24 ?? E8 ?? ?? ?? ?? 0F B6")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::WidgetOpacityBlendMultiply,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("F3 0F 59 40 ?? F3 0F 11 74 24 ?? F3 0F 11 4C 24 ?? F3 0F 11 44 24 ?? E8 ?? ?? ?? ?? 0F 28")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::WidgetOpacityBlendMultiply,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("F3 0F 59 87 ?? ?? 00 00 F3 0F 11 44 24 ?? F3 0F 11 4C 24 ?? E8 ?? ?? ?? ?? 0F B6")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::WidgetOpacityBlendMultiply,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("F3 0F 10 4C 24 ?? F3 0F 59 8F ?? ?? 00 00 F3 0F 11 4C 24 ?? 41 0F")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::WidgetOpacityBlendMultiply,
            "UUU4_Alternative4".to_string(),
            None,
            Pattern::new("F3 0F 10 4C 24 ?? F3 0F 59 8F ?? ?? 00 00 F3 0F 11 4C 24 ?? 48 8D 55")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::WidgetOpacityBlendMultiply,
            "UUU4_Alternative5".to_string(),
            None,
            Pattern::new("F3 0F 59 8F ?? 03 00 00 F3 0F 11 4C 24 4C 83 7D ?? 00 74 ?? 48 85 C9 74")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::WidgetOpacityBlendMultiply,
            "UUU4_Alternative6".to_string(),
            None,
            Pattern::new("F3 0F 59 8F ?? ?? 00 00 F3 0F 11 4C 24 4C 48 8D 54 24 70")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::WidgetOpacityBlendMultiply,
            "UUU4_Alternative7".to_string(),
            None,
            Pattern::new("F3 0F 59 8F ?? ?? 00 00 F3 0F 11 44 24 48 F3 0F 11 4C 24 4C E8 ?? ?? ?? ?? 48")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::WidgetOpacityBlendMultiply,
            "UUU4_Alternative8_avx".to_string(),
            None,
            Pattern::new("C5 F0 59 8F ?? ?? ?? ?? 41 0F B6 CF 4C 8B BC 24 B0 02 00 00 C5 F8 11 4C ?? 40")?,
            resolve_self,
        ),

        //===============================[FMinimalViewInfoCTor]=============================================================================================
        PatternConfig::new(
            Sig::FMinimalViewInfoCTor,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("F2 0F 10 02 48 ?? ?? | F2 0F 11 01 48 ?? ?? 8B 42 08 89 41 08 F2 0F 10 42 0C F2 0F 11 41 0C 8B 42 14 89 41 14 8B 42 18 89 41 18 8B 42 1C 89 41 1C 8B 42 20 89 41 20 8B 42 24 89 41 24")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::FMinimalViewInfoCTor,
            "UUU4_Alternative0_avx".to_string(),
            None,
            Pattern::new("C5 FB 11 01 8B 42 08 48 8B ?? 89 41 08 48 8B ?? C5 FB 10 42 0C C5 FB 11 41 0C 8B 42 14 89 41 14 8B 42 18 89 41 18 8B 42 1C 89 41 1C 8B 42 20 89 41 20 8B 42 24 89 41 24")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::FMinimalViewInfoCTor,
            "UUU4_Alternative0_411".to_string(),
            None,
            Pattern::new("F2 0F 10 02 48 8B D9 F2 0F 11 01 4C 8B C2 8B 42 08 89 41 08 F2 0F 10 42 0C F2 0F 11 41 0C 8B 42 14")?,
            resolve_self,
        ),

        //===============================[FMinimalViewInfoLockFOV]=============================================================================================
        PatternConfig::new(
            Sig::FMinimalViewInfoLockFOV,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("F3 0F 10 B1 ?? ?? 00 00 ?? C0 0F 57 C0")?,
            resolve_self,
        ),

        //===============================[CameraWriteTransform]=============================================================================================
        PatternConfig::new(
            Sig::CameraWriteTransform,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("83 A7 ?? ?? 00 00 FC F2 0F 11 87 ?? ?? 00 00 F2 0F 10 44 24 ?? F2 0F 11 87 ?? ?? 00 00")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::CameraWriteTransform,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("83 A7 ?? ?? 00 00 F0 |  F2 0F 11 87 ?? ?? 00 00 F2 0F 10 44 24 ?? F2 0F 11 87 ?? ?? 00 00")?,
            resolve_self,
        ),

        //===============================[CameraWriteTransformMenu]=============================================================================================
        PatternConfig::new(
            Sig::CameraWriteTransformMenu,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("F2 0F 11 83 ?? ?? 00 00 F2 0F 10 44 24 ?? F2 0F 11 83 ?? ?? 00 00 0F 10 44 24 ?? 89 83 ?? ?? 00 00 8B 44 24")?,
            resolve_self,
        ),

        //===============================[CameraARCorrectionFMinimalViewInfo]=============================================================================================
        PatternConfig::new(
            Sig::CameraARCorrectionFMinimalViewInfo,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("8B 42 ?? 89 41 ?? 8B 41 ?? 33 42 ?? 83 E0 01 31 41 ?? 8B ?? ?? 33 ?? ?? 83 ?? 02 31")?,
            resolve_self,
        ),

        //===============================[CameraARCorrectionCameraComponent]=============================================================================================
        PatternConfig::new(
            Sig::CameraARCorrectionCameraComponent,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("33 47 ?? 83 E0 01 31 47 ?? 0F B6 8B ?? ?? ?? ?? 33 4F ?? 83 E1 02 31 4F")?,
            resolve_self,
        ),

        //===============================[FNameToString]=============================================================================================
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

        //===============================[FNameFName]=============================================================================================
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

        //===============================[StaticConstructObjectInternal]=============================================================================================
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
            Sig::StaticConstructObjectInternal,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("48 8B 84 24 ?? ?? 00 00 48 89 44 24 ?? C7 44 24 ?? 00 00 00 00 E8 | ?? ?? ?? ?? 48 8B 5C 24")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("48 8B C8 89 7C 24 ?? E8 | ?? ?? ?? ?? 48 8B 5C 24 ?? 48 83 C4 ?? 5F C3")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("48 89 ?? 24 30 89 ?? 24 38 E8 | ?? ?? ?? ?? 48 ?? ?? 24 70 48 ?? ?? 24 78")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("E8 | ?? ?? ?? ?? 48 8B 4C 24 30 48 89 ?? ?? ?? ?? ?? 48 85 C9 74 05 E8 ?? ?? ?? ?? 48 8D 4D 48 E8")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "UUU4_Alternative4".to_string(),
            None,
            Pattern::new("E8 | ?? ?? ?? ?? 49 8B D6 48 8B C8 48 8B D8 E8 ?? ?? ?? ?? 4C 8D 9C 24 90 00 00 00")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),

        //===============================[GMalloc]=============================================================================================
        PatternConfig::new(
            Sig::GMalloc,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 85 C9 74 2E 53 48 83 EC 20 48 8B D9 48 8B ?? ?? ?? ?? ?? 48 85 C9")?,
            resolve_self,
        ),

        //===============================[GUObjectArray]=============================================================================================
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
            Sig::GUObjectArray,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? C6 05 ?? ?? ?? ?? 01 E8 ?? ?? ?? ?? C6 05 ?? ?? ?? ?? 01 C6 05 ?? ?? ?? ?? 00 80 3D")?,
            RIPRelativeResolvers::resolve_RIP4,
       ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 48 89 74 24 70 48 89 7C 24 78 45 33 C9")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? 44 8B 84 24 ?? ?? ?? ?? 8B 94 24 ?? ?? ?? ?? E8 ?? ?? ?? ?? E8")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 84 C0 74 ?? 48 8D 0D ?? ?? ?? ?? E8 ?? ?? ?? ?? E8")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative4".to_string(),
            None,
            Pattern::new("84 D2 48 C7 41 10 00 00 00 00 B8 FF FF FF FF 4C 8D 05 | ?? ?? ?? ?? 89 41 08")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative5".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 45 ?? C9 4C 89 74 24")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative6".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 4C 89 64 ?? 28 4C 89 7C ?? 30 45 33 C9")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative7".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 48 8D 4D 80 E8 ?? ?? ?? ?? 48 8D 4D 80 E8 ?? ?? ?? ?? F3")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative8".to_string(),
            None,
            Pattern::new("89 ?? | ?? ?? ?? ?? 45 85 E4 7F ?? 4C 8D 05 ?? ?? ?? ?? BA ?? 00 00 00")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative9".to_string(),
            None,
            Pattern::new("89 1D | ?? ?? ?? ?? 48 8B 9C 24 80 00 00 00 48 89 05 ?? ?? ?? ?? 48")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative10".to_string(),
            None,
            Pattern::new("89 0D ?? ?? ?? ?? 89 15 | ?? ?? ?? ?? 85 FF 7F ?? 4C 8D 05 ?? ?? ?? ?? BA ?? 00 00 00")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative12".to_string(),
            None,
            Pattern::new("89 05 | ?? ?? ?? ?? 85 D2 7F 39 4C 8D 05 ?? ?? ?? ?? BA 67 00 00 00 48 8D 0D")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative13".to_string(),
            None,
            Pattern::new("89 05 | ?? ?? ?? ?? 45 85 E4 7F 46 48 8D 05 ?? ?? ?? ?? 44 8B CE 4C 8D 05")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative14".to_string(),
            None,
            Pattern::new("89 05 | ?? ?? ?? ?? 85 DB 7F 24 4C 8D 05 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 81 F2")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),

        //===============================[GNatives]=============================================================================================
        PatternConfig::new(
            Sig::GNatives,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("cc 51 20 01")?,
            GNatives::resolve,
        ),

        //===============================[Pak]=============================================================================================
        PatternConfig::new(
            Sig::Pak,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 89 5c 24 10 48 89 74 24 18 48 89 7c 24 20 55 41 54 41 55 41 56 41 57 48 8d ac 24 20 fe ff ff 48 81 ec e0 02 00 00 48 8b 05 ?? ?? ?? ?? 48 33 c4 48 89 85 d0 01 00 00")?,
            resolve_self,
        ),

        //===============================[PatchPak]=============================================================================================
        PatternConfig::new(
            Sig::PatchPak,
            "A".to_string(),
            None,
            Pattern::new("5f 00 50 00 2e 00 70 00 61 00 6b")?,
            resolve_self,
        ),

        //===============================[IConsoleManagerSingleton]=============================================================================================
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B 0D | ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 8B 01 4C 8D 0D")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "B".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B 0D | ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 8B 01 4C 8D 4C 24")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "C".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 83 EC ?? 48 8B 0D | ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? ?? 48 8B 0D")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "D".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 89 3D | ?? ?? ?? ?? 48 85 FF 75 ?? E8 ?? ?? ?? ?? 48 8B 3D ?? ?? ?? ?? 48 8B 07")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),

        //===============================[AllowCheats]=============================================================================================
        PatternConfig::new(
            Sig::AllowCheats,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("74 ?? 48 8B 01 48 8B ?? FF 90 ?? ?? 00 00 84 C0 75 ?? 40")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::AllowCheats,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("FF 90 ?? 08 00 00 84 C0 75 ?? 84 DB 0F 84 ?? ?? 00 00")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::AllowCheats,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("FF 90 ?? 07 00 00 84 C0 75 05 40 84 F6")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::AllowCheats,
            "UUU4_Alternative4".to_string(),
            None,
            Pattern::new("FF 90 ?? ?? 00 00 84 C0 75 ?? 84 DB 0F 84")?,
            resolve_self,
        ),

        //===============================[NamePoolData]=============================================================================================
        PatternConfig::new(
            Sig::NamePoolData,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("48 8D 15 | ?? ?? ?? ?? EB 16 48 8D 0D ?? ?? ?? ?? E8")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::NamePoolData,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("48 8D 05 | ?? ?? ?? ?? EB 27 48 8D 05 ?? ?? ?? ?? 48")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::NamePoolData,
            "UUU4_Alternative2_422-".to_string(),
            None,
            Pattern::new("E8 ?? ?? ?? ?? 48 8B C3 48 89 1D | ?? ?? ?? ?? 48 8B 5C 24")?,
            resolve_multi_self,
        ),
        PatternConfig::new(
            Sig::NamePoolData,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("E8 ?? ?? ?? ?? 48 89 D8 48 89 1D | ?? ?? ?? ?? 48 8B 5C 24 20 48 83 C4 28 C3 31 DB 48 89 1D")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
        PatternConfig::new(
            Sig::NamePoolData,
            "UUU4_Alternative4".to_string(),
            None,
            Pattern::new("E8 ?? ?? ?? ?? 48 89 D8 48 89 1D | ?? ?? ?? ?? 48 8B 5C 24 20 48 83 C4 28 C3 48 8B 5C")?,
            RIPRelativeResolvers::resolve_RIP4,
        ),
    ])
}

/// do nothing, return address of pattern
pub fn resolve_self(ctx: ResolveContext) -> Resolution {
    Resolution {
        stages: vec![],
        res: ResolutionType::Address(ctx.match_address),
    }
}

/// do nothing, but return a constant so it's squashing all multiple matches to 1 value: 0x12345678
pub fn resolve_multi_self(_ctx: ResolveContext) -> Resolution {
    Resolution {
        stages: vec![],
        res: ResolutionType::Count,
    }
}

/// simply returns 0x1 as constant address so the scanner will pack multiple instances together as 1 and mention the amount.
pub fn resolve_engine_version(ctx: ResolveContext) -> Resolution {
    let version_value_address = ctx.match_address;
    let version_major = i16::from_le_bytes(
        ctx.memory[version_value_address..version_value_address + 2]
            .try_into()
            .unwrap(),
    );
    let version_minor = i16::from_le_bytes(
        ctx.memory[version_value_address + 2..version_value_address + 4]
            .try_into()
            .unwrap(),
    );
    Resolution {
        stages: vec![ctx.match_address],
        res: ResolutionType::String(format!("{}.{}", version_major, version_minor)),
    }
}

#[allow(non_snake_case)]
mod RIPRelativeResolvers {
    use super::*;

    fn resolve_RIP(
        memory: &MountedPE,
        match_address: usize,
        next_opcode_offset: usize,
    ) -> Resolution {
        let stages = vec![match_address];
        let rip_relative_value_address = match_address;
        // calculate the absolute address from the RIP relative value.
        let address = rip_relative_value_address
            .checked_add_signed(i32::from_le_bytes(
                memory[rip_relative_value_address..rip_relative_value_address + 4]
                    .try_into()
                    .unwrap(),
            ) as isize)
            .map(|a| a + next_opcode_offset);
        Resolution {
            stages,
            res: address.into(),
        }
    }

    pub fn resolve_RIP4(ctx: ResolveContext) -> Resolution {
        resolve_RIP(ctx.memory, ctx.match_address, 4)
    }

    pub fn resolve_RIP5(ctx: ResolveContext) -> Resolution {
        resolve_RIP(ctx.memory, ctx.match_address, 5)
    }
}

#[allow(non_snake_case)]
mod FNameToStringID {
    use super::*;
    pub fn resolve(ctx: ResolveContext) -> Resolution {
        let stages = vec![ctx.match_address];
        let n = ctx.match_address + 5;
        let rel = i32::from_le_bytes(ctx.memory[n - 4..n].try_into().unwrap());
        let address = n.checked_add_signed(rel as isize);
        Resolution {
            stages,
            res: address.into(),
        }
    }
}

#[allow(non_snake_case)]
mod FNameFNameID {
    use super::*;
    pub fn resolve_a(ctx: ResolveContext) -> Resolution {
        let stages = vec![ctx.match_address];
        let n = ctx.match_address.checked_add_signed(0x18 + 5).unwrap();
        let address = n.checked_add_signed(i32::from_le_bytes(
            ctx.memory[n - 4..n].try_into().unwrap(),
        ) as isize);
        Resolution {
            stages,
            res: address.into(),
        }
    }
    pub fn resolve_v5_1(ctx: ResolveContext) -> Resolution {
        let stages = vec![ctx.match_address];
        let n = ctx.match_address.checked_add_signed(0x1C + 5).unwrap();
        let address = n.checked_add_signed(i32::from_le_bytes(
            ctx.memory[n - 4..n].try_into().unwrap(),
        ) as isize);
        Resolution {
            stages,
            res: address.into(),
        }
    }
}

#[allow(non_snake_case)]
mod StaticConstructObjectInternalID {
    use super::*;
    pub fn resolve_a_v4_12(ctx: ResolveContext) -> Resolution {
        let stages = vec![ctx.match_address];
        let n = ctx.match_address - 0x0e;
        let address = n.checked_add_signed(i32::from_le_bytes(
            ctx.memory[n - 4..n].try_into().unwrap(),
        ) as isize);
        Resolution {
            stages,
            res: address.into(),
        }
    }
    pub fn resolve_v4_16_4_19_v5_0(ctx: ResolveContext) -> Resolution {
        let stages = vec![ctx.match_address];
        let n = ctx.match_address + 5;
        let address = n.checked_add_signed(i32::from_le_bytes(
            ctx.memory[n - 4..n].try_into().unwrap(),
        ) as isize);
        Resolution {
            stages,
            res: address.into(),
        }
    }
}

#[allow(non_snake_case)]
mod GUObjectArrayID {
    use super::*;
    pub fn resolve_a(ctx: ResolveContext) -> Resolution {
        Resolution {
            // TODO
            stages: vec![],
            res: ctx.match_address.into(),
        }
    }
    pub fn resolve_v_20(ctx: ResolveContext) -> Resolution {
        let stages = vec![ctx.match_address];
        let n = ctx.match_address + 3;
        let address = n
            .checked_add_signed(
                i32::from_le_bytes(ctx.memory[n..n + 4].try_into().unwrap()) as isize
            )
            .map(|a| a - 0xc);
        Resolution {
            stages,
            res: address.into(),
        }
    }
}

#[allow(non_snake_case)]
mod GNatives {
    use super::*;
    pub fn resolve(ctx: ResolveContext) -> Resolution {
        let mut stages = vec![ctx.match_address - 1];
        for i in ctx.match_address..ctx.match_address + 400 {
            if ctx.memory[i] == 0x4c
                && ctx.memory[i + 1] == 0x8d
                && (ctx.memory[i + 2] & 0xc7 == 5 && ctx.memory[i + 2] > 0x20)
            {
                stages.push(i);
                let address = (i + 7).checked_add_signed(i32::from_le_bytes(
                    ctx.memory[i + 3..i + 3 + 4].try_into().unwrap(),
                ) as isize);
                return Resolution {
                    stages,
                    res: address.into(),
                };
            }
        }
        Resolution {
            stages,
            res: ResolutionType::Failed,
        }
    }
}
