use anyhow::Result;

use super::{
    MountedPE, Pattern, PatternConfig, ResolutionAction, ResolutionType, ResolveContext,
    ResolveStages, Scan, Xref,
};

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
    Custom(String),
    AllowCheats,
    CameraWriteTransform,     // Only in older engines (4.19 and earlier)
    CameraWriteTransformMenu, // Only in older engines (4.17 and earlier)
    CameraARCorrectionFMinimalViewInfo,
    CameraARCorrectionCameraComponent,
    CameraARCorrectionPlayerCameraManager,
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
    #[strum(serialize = "FPakPlatformFile::Initialize")]
    FPakPlatformFileInitialize,
    #[strum(serialize = "FPakPlatformFile::~FPakPlatformFile")]
    FPakPlatformFileDtor,
    FCustomVersionContainer,
    #[strum(serialize = "SViewport::OnPaint call of SCompoundWidget::onPaint")]
    SViewportOnPaintCallToSCompoundWidgetOnPaint,

    StringFTagMetaData,
    SigningKey,

    AES,
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
        PatternConfig::new(
            Sig::GetEffectiveTimeDilation,
            "UUU5_Alternative0".to_string(),
            None,
            Pattern::new("F3 0F 10 81 ?? ?? 00 00 F3 0F 59 81 ?? ?? 00 00 F3 0F 59 81 ?? ?? 00 00 C3")?,
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
        PatternConfig::new(
            Sig::UWorldIsPaused,
            "UUU5_Alternative0".to_string(),
            None,
            Pattern::new("80 BB ?? ?? ?? ?? 00 7C 08 | ?? C0 48 83 C4 20 5B C3 B0 01")?,
            resolve_self,
        ),

        //===============================[ProcessEventFromCall]=============================================================================================
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("F0 0F B1 0D ?? ?? ?? ?? 75 0E ?? ?? ?? 48 ?? ?? 48 ?? ?? E8 | ?? ?? ?? ?? 48 8B ?? 24 ?? 48 8B ?? 24 38 48 8B ?? 24 40 48 83")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("F0 0F B1 11 75 0E ?? ?? ?? 48 ?? ?? 48 ?? ?? E8 | ?? ?? ?? ?? 48 8B ?? 24 ?? 48 8B ?? 24 38 48 8B ?? 24 40 48 83")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("84 C0 75 0E ?? ?? ?? 48 ?? ?? 48 ?? ?? E8 | ?? ?? ?? ?? 48 8B ?? 24 ?? 48 8B ?? 24 38 48 8B ?? 24 40 48 83")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("4C 8B C5 48 8B D7 48 8B CB E8 | ?? ?? ?? ?? 48 8B 5C 24 30 48 8B 6C 24 38 48 8B 74 24 40 48 8B 7C 24 48 48 83 C4 20 41 5E C3")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative4_modular".to_string(),
            None,
            Pattern::new("F0 0F B1 11 75 0F 4C 8B C5 48 8B D7 48 8B CB FF 15 | ?? ?? ?? ?? 48 8B 74 24 30")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative5".to_string(),
            None,
            Pattern::new("48 8B D6 48 8B CB E8 | ?? ?? ?? ?? 8B 83 ?? ?? ?? ?? 85 C0 75 27 87 BB")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::ProcessEventFromCall,
            "UUU4_Alternative6".to_string(),
            None,
            Pattern::new("75 0E ?? ?? ?? 48 ?? ?? 48 ?? ?? E8 | ?? ?? ?? ?? 48 8B ?? 24 ?? 48 8B ?? 24 38 48 8B ?? 24 40 48 83")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
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
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GEngine,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("48 8B 05 | ?? ?? ?? ?? 48 8B 88 ?? 07 00 00 48 85 C9 74 ?? 48 8B 51 50")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GEngine,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("48 8B 05 | ?? ?? ?? ?? 48 8B 88 ?? ?? ?? ?? 48 85 C9 0F 84 ?? ?? ?? ?? 48 89 74 24 50 48 8B 71")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
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
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU5_Alternative0".to_string(),
            None,
            Pattern::new("41 C7 06 | 05 00 ?? ?? 48 8B 5C 24 ?? 49 8D 76 ?? 33 ED 41 89 46")?,
            resolve_engine_version,
        ),
        PatternConfig::new(
            Sig::EngineVersion,
            "UUU5_Alternative1".to_string(),
            None,
            Pattern::new("C7 06 | 05 00 ?? ?? 48 8B 5C 24 20 4C 8D 76 10 33 ED")?,
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
        PatternConfig::new(
            Sig::UWorldSpawnActor,
            "UUU5_Alternative5".to_string(),
            None,
            Pattern::new("40 53 56 57 48 81 EC ?? ?? ?? ?? 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 84 24 ?? ?? ?? ?? 0F 28 0D")?,
            resolve_self,
        ),

        //===============================[UWorldSpawnActorFromCall]=============================================================================================
        PatternConfig::new(
            Sig::UWorldSpawnActorFromCall,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("48 89 44 24 ?? E8 | ?? ?? ?? ?? 48 85 C0 0F 85 ?? ?? 00 00 48 8D 05")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::UWorldSpawnActorFromCall,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("4C 8D 4D B7 4C 8D 44 24 ?? 48 8B D7 E8 | ?? ?? ?? ?? 48 85 C0 0F 85")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::UWorldSpawnActorFromCall,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("4C 8D 44 24 ?? ?? 8B ?? E8 | ?? ?? ?? ?? 48 85 C0 0F 85 ?? ?? 00 00 48 8D 05")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::UWorldSpawnActorFromCall,
            "UUU5_Alternative0".to_string(),
            None,
            Pattern::new("48 8B C8 4C 8D 45 80 E8 | ?? ?? ?? ?? 48 85 C0 74 ?? B3 01")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::UWorldSpawnActorFromCall,
            "UUU5_Alternative1".to_string(),
            None,
            Pattern::new("48 8B C8 4C 8D 44 24 50 E8 | ?? ?? ?? ?? 48 85 C0 75")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
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
        PatternConfig::new(
            Sig::WidgetOpacityBlendMultiply,
            "UUU5_Alternative2".to_string(),
            None,
            Pattern::new("F3 0F 59 C3 F3 0F 11 44 24 ?? 40 84 F6 74 0C 48 8D 55 ?? FF 90")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::WidgetOpacityBlendMultiply,
            "UUU5_Alternative3".to_string(),
            None,
            Pattern::new("0F 10 83 ?? ?? 00 00 0F 11 4D 8C 0F 59 F0 0F 11 45 ?? 0F 11")?,
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

        //===============================[SViewportOnPaintCallToSCompoundWidgetOnPaint]=============================================================================================
        PatternConfig::new(
            Sig::SViewportOnPaintCallToSCompoundWidgetOnPaint,
            "UUU5_Alternative0".to_string(),
            None,
            Pattern::new("E8 ?? ?? ?? ?? 89 44 24 54 8B D8 BF FF FF FF FF")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::SViewportOnPaintCallToSCompoundWidgetOnPaint,
            "UUU5_Alternative1".to_string(),
            None,
            Pattern::new("E8 ?? ?? ?? ?? 89 44 24 5C 8B F0 BF FF FF FF FF 4D 85 E4")?,
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
        PatternConfig::new(
            Sig::FMinimalViewInfoCTor,
            "UUU5_Alternative0".to_string(),
            None,
            Pattern::new("0F 10 02 ?? ?? ?? ?? ?? ?? | 0F 11 01 F2 0F 10 4A ?? F2 0F 11 49 ?? 0F 10 42 ?? 0F 11 41 ?? F2 0F 10 4A ?? F2 0F 11 49 ?? 8B 42 ?? 89 41 ?? 8B 42 ?? 89 41 ?? 8B 42 ?? 89 41")?,
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
        PatternConfig::new(
            Sig::CameraARCorrectionCameraComponent,
            "UUU5_Alternative1".to_string(),
            None,
            Pattern::new("33 4F 4C 83 E1 01 33 4F 4C 89 4F 4C 0F B6 83 ?? ?? 00 00 33 C1")?,
            resolve_self,
        ),

        //===============================[CameraARCorrectionPlayerCameraManager]=============================================================================================
        PatternConfig::new(
            Sig::CameraARCorrectionPlayerCameraManager,
            "UUU5_Alternative0".to_string(),
            None,
            Pattern::new("0F B6 81 BC 02 00 00 41 33 45 5C 83 E0 01 | 41 31 45 5C")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::CameraARCorrectionPlayerCameraManager,
            "UUU5_Alternative1".to_string(),
            None,
            Pattern::new("41 33 40 5C 83 E0 01 48 89 7C 24 60 | 41 31 40 5C 8B")?,
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
        PatternConfig::new(
            Sig::FNameToString,//419-427
            "SetEnums".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("0f 84 ?? ?? ?? ?? 48 8b ?? e8 ?? ?? ?? ?? 84 c0 0f 85 ?? ?? ?? ?? 48 8d ?? 24 ?? 48 8b ?? e8 ?? ?? ?? ??")?,
            FNameToStringID::setenums,
        ),
        PatternConfig::new(
            Sig::FNameToString,
            "LW".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8b 48 ?? 48 89 4c 24 ?? 48 8d 4c 24 ?? e8 | ?? ?? ?? ?? 83 7c 24 ?? 00 48 8d")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameToString,
            "Bnew3".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? ?? 01 00 00 00 ?? 39 ?? 48 0F 8E")?,
            FNameToStringID::resolve,
        ),
        PatternConfig::new(
            Sig::FNameToString,
            "C".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? 83 7D C8 00 48 8D 15 ?? ?? ?? ?? 0F 5A DE")?,
            FNameToStringID::resolve,
        ),
        PatternConfig::new(
            Sig::FNameToString,
            "Dnew".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? 83 7D C8 00 48 8D 15 ?? ?? ?? ?? 48 8D 0D ?? ?? ?? ?? 48 0f")?,
            FNameToStringID::resolve,
        ),
        PatternConfig::new(
            Sig::FNameToString,
            "KH3".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 89 5C 24 ?? 48 89 ?? 24 ?? 48 89 ?? 24 ?? 41 56 48 83 EC ?? 48 8B DA 4C 8B F1 e8 ?? ?? ?? ?? 4C 8B C8 41 8B 06 99")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::FNameToString,
            "FullyLoad".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("c6 ?? 2a 01 48 89 44 24 ?? e8 | ?? ?? ?? ?? 83 7c 24 ?? 00")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameToString,
            "FMemoryArchive".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 89 0f eb 15 48 8b cf e8 | ?? ?? ?? ?? 48 8d ?? 24 ?? 48 8b cb e8 ?? ?? ?? ??  48 8b ?? 24 ?? 48 85 c9 74 05")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameToString,
            "FLoadTimeTracker".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 63 c9 48 c1 ?? 05 48 03 ?? e8 | ?? ?? ?? ?? 48 8b ?? ?? 48 85 c9 74 05")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameToString,
            "ISlateStyleJoin".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("e8 ?? ?? ?? ?? 48 ?? ?? 24 ?? 48 ?? ?? 24 98 00 00 00 e8 | ?? ?? ?? ?? 8b 48 ?? 83 f9 01")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameToString,
            "UClassRename".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("00 74 ?? 48 8d ?? 24 ?? 48 8b ?? e8 ?? ?? ?? ?? 48 8b c8 48 8d ?? 24 ?? e8 | ?? ?? ?? ?? 83 78 08 00 74 ?? ?? ?? ?? eb")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameToString,
            "LinkerManagerExec".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8d 0c c1 e8 | ?? ?? ?? ?? 83 78 08 00")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "UUU5_Alternative0".to_string(),
            None,
            Pattern::new("E8 | ?? ?? ?? ?? 48 83 7C 24 60 00 48 8B D8 74 ?? 48 8B 54 24")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
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
            "<=V4.22".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("40 53 48 83 EC ?? 48 8B D9 48 85 D2 74 21 45 8B C8 C7 44 24 ?? FF FF FF FF 45 33 C0 C6 44 24 ?? 01 E8")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            "V4.23".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("40 53 48 83 EC ?? 45 33 D2 48 89 54 24 ?? 48 8B D9 48 8B C2 48 85 D2 74 2C 44 0F B7")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            ">=V4.24".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 89 5C 24 ?? 57 48 83 EC ?? 48 8B D9 48 89 54 24 ?? 33 C9 41 8B F8 4C 8B ?? 44 8B")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            "V5.1".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("57 48 83 EC 50 41 B8 01 00 00 00 0F 29 74 24 40 48 8D ?? ?? ?? ?? ?? 48 8D 4C 24 60 E8")?,
            FNameFNameID::resolve_v5_1,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            "LW0".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("41 b8 01 00 00 00 48 8d 15 ?? ?? ?? ?? 48 8d 4c 24 ?? e8 | ?? ?? ?? ??")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            "LW01".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 83 ec ?? 41 b8 01 00 00 00 48 8d 15 ?? ?? ?? ?? 48 8d 4c 24 ?? e8 | ?? ?? ?? ?? 48 8b")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            "LW1".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("41 b8 01 00 00 00 48 8d 15 ?? ?? ?? ?? 48 8d 4c 24 ?? e8 | ?? ?? ?? ?? 48 8b ?? e8")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            "LW11".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("e8 ?? ?? ?? ?? 41 b8 01 00 00 00 48 8d 15 ?? ?? ?? ?? 48 8d 4c 24 ?? e8 | ?? ?? ?? ?? 48 8b 44")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            "LW2".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("ff 50 ?? 41 b8 01 00 00 00 48 8d 15 ?? ?? ?? ?? 48 8d 4c 24 ?? e8 | ?? ?? ?? ?? ")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            "LW3".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("41 ?? 01 00 00 00 48 8d ?? 24 ?? 48 0f 45 ?? 24 ?? e8 | ?? ?? ?? ??")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            "LW4".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("41 ?? 01 00 00 00 48 8d ?? ?? 48 0f 45 ?? ?? e8 | ?? ?? ?? ??")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            "LW5".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("75 ?? 41 ?? 01 00 00 00 48 8d ?? ?? ?? ?? ?? 48 8d 0d ?? ?? ?? ?? e8 | ?? ?? ?? ?? 48 8d ?? ?? ?? ?? ?? e8")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::FNameFName,
            "LW51".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("ff 0f 85 ?? ?? ?? ?? 41 ?? 01 00 00 00 48 8d ?? ?? ?? ?? ?? 48 8d 0d ?? ?? ?? ?? e8 | ?? ?? ?? ?? 48 8d ?? ?? ?? ?? ?? e8 ?? ?? ?? ?? e9")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),

        //===============================[StaticConstructObjectInternal]=============================================================================================
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "V4.12".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("89 8E C8 03 00 00 3B 8E CC 03 00 00 7E 0F 41 8B D6 48 8D 8E C0 03 00 00")?,
            StaticConstructObjectInternalID::resolve_a_v4_20,
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
            ">=V4.20".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("C0 E9 ?? 32 88 ?? ?? ?? ?? 80 E1 01 30 88 ?? ?? ?? ?? 48")?,
            StaticConstructObjectInternalID::resolve_a_v4_20,
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
            "LW".to_string(),
            None,
            Pattern::new("48 89 ?? 24 ?? c7 ?? 24 ?? 00 00 00 00 E8 | ?? ?? ?? ?? 48 8B ?? 24 ?? 48 8b ?? 24")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "LW1".to_string(),
            None,
            Pattern::new("00 48 89 ?? 24 ?? c7 ?? 24 ?? 00 00 00 00 E8 | ?? ?? ?? ?? 48 8B ?? 24 ?? 48 8b ?? 24")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "LW2".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? 48 89 47 ?? 40 38 35 ?? ?? ?? ?? 75 09 40 38 35")?,
            StaticConstructObjectInternalID::resolve_v4_16_4_19_v5_0,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("48 8B 84 24 ?? ?? 00 00 48 89 44 24 ?? C7 44 24 ?? 00 00 00 00 E8 | ?? ?? ?? ?? 48 8B 5C 24")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("48 8B C8 89 7C 24 ?? E8 | ?? ?? ?? ?? 48 8B 5C 24 ?? 48 83 C4 ?? 5F C3")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("48 89 ?? 24 30 89 ?? 24 38 E8 | ?? ?? ?? ?? 48 ?? ?? 24 70 48 ?? ?? 24 78")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("E8 | ?? ?? ?? ?? 48 8B 4C 24 30 48 89 ?? ?? ?? ?? ?? 48 85 C9 74 05 E8 ?? ?? ?? ?? 48 8D 4D 48 E8")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::StaticConstructObjectInternal,
            "UUU4_Alternative4".to_string(),
            None,
            Pattern::new("E8 | ?? ?? ?? ?? 49 8B D6 48 8B C8 48 8B D8 E8 ?? ?? ?? ?? 4C 8D 9C 24 90 00 00 00")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),

        //===============================[GMalloc]=============================================================================================
        PatternConfig::new(
            Sig::GMalloc,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 85 C9 74 2E 53 48 83 EC 20 48 8B D9 48 8B ?? | ?? ?? ?? ?? 48 85 C9")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GMalloc,
            "B".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("E8 ?? ?? ?? ?? 48 8b 0d | ?? ?? ?? ?? 48 8b ?? 48 8b ?? ff 50 ?? 48 83 c4 ?? ?? c3 cc")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GMalloc,
            "alt".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 85 C9 74 ?? 4C 8B 05 | ?? ?? ?? ?? 4D 85 C0 0F 84 ?? ?? ?? ?? 49 8B 00 48")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),

        //===============================[GUObjectArray]=============================================================================================
        PatternConfig::new(
            Sig::GUObjectArray,
            "<=V4.12".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 03 ?? ?? ?? ?? ?? 48 8B 10 48 85 D2 74 07")?,
            GUObjectArrayID::resolve_v_20,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "<=V4.13".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 83 79 10 00 74 F6 48 8B D1 48 8D | ??")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "<=V4.19".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B ?? ?? ?? ?? ?? 4C 8B 04 C8 4D 85 C0 74 07")?,
            GUObjectArrayID::resolve_v_20,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            ">=V4.20".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B ?? ?? ?? ?? ?? 48 8B 0C C8 ?? 8B 04 ?? 48 85 C0")?,
            GUObjectArrayID::resolve_v_20,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "B_Ext".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B C8 48 89 05 ?? ?? ?? ?? E8 ?? ?? ?? ?? ?? ?? ?? 0F 84")?,
            GUObjectArrayID::resolve_b_ext,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative0".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? C6 05 ?? ?? ?? ?? 01 E8 ?? ?? ?? ?? C6 05 ?? ?? ?? ?? 01 C6 05 ?? ?? ?? ?? 00 80 3D")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative0_LW".to_string(),
            None,
            Pattern::new("74 ?? 48 8D 0D | ?? ?? ?? ?? C6 05 ?? ?? ?? ?? 01 E8 ?? ?? ?? ?? C6 05 ?? ?? ?? ?? 01")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 48 89 74 24 70 48 89 7C 24 78 45 33 C9")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative2".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? 44 8B 84 24 ?? ?? ?? ?? 8B 94 24 ?? ?? ?? ?? E8 ?? ?? ?? ?? E8")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative3".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 84 C0 74 ?? 48 8D 0D ?? ?? ?? ?? E8 ?? ?? ?? ?? E8")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative4".to_string(),
            None,
            Pattern::new("84 D2 48 C7 41 10 00 00 00 00 B8 FF FF FF FF 4C 8D 05 | ?? ?? ?? ?? 89 41 08")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative5".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 45 ?? C9 4C 89 74 24")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative5_LW".to_string(),
            None,
            Pattern::new("75 ?? 48 ?? ?? 48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 45 33 C9 4C 89 74 24")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative6".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 4C 89 64 ?? 28 4C 89 7C ?? 30 45 33 C9")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative7".to_string(),
            None,
            Pattern::new("48 8D 0D | ?? ?? ?? ?? E8 ?? ?? ?? ?? 48 8D 4D 80 E8 ?? ?? ?? ?? 48 8D 4D 80 E8 ?? ?? ?? ?? F3")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative8".to_string(),
            None,
            Pattern::new("89 ?? | ?? ?? ?? ?? 45 85 E4 7F ?? 4C 8D 05 ?? ?? ?? ?? BA ?? 00 00 00")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative9".to_string(),
            None,
            Pattern::new("89 1D | ?? ?? ?? ?? 48 8B 9C 24 80 00 00 00 48 89 05 ?? ?? ?? ?? 48")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative10".to_string(),
            None,
            Pattern::new("89 0D ?? ?? ?? ?? 89 15 | ?? ?? ?? ?? 85 FF 7F ?? 4C 8D 05 ?? ?? ?? ?? BA ?? 00 00 00")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative12".to_string(),
            None,
            Pattern::new("89 05 | ?? ?? ?? ?? 85 D2 7F 39 4C 8D 05 ?? ?? ?? ?? BA 67 00 00 00 48 8D 0D")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative13".to_string(),
            None,
            Pattern::new("89 05 | ?? ?? ?? ?? 45 85 E4 7F 46 48 8D 05 ?? ?? ?? ?? 44 8B CE 4C 8D 05")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU4_Alternative14".to_string(),
            None,
            Pattern::new("89 05 | ?? ?? ?? ?? 85 DB 7F 24 4C 8D 05 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 81 F2")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU5_Alternative0".to_string(),
            None,
            Pattern::new("89 ?? | ?? ?? ?? ?? 85 FF 7F ?? 4C 8D 05 ?? ?? ?? ?? 48 8D 15 ?? ?? ?? ?? 48")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU5_Alternative1".to_string(),
            None,
            Pattern::new("89 ?? | ?? ?? ?? ?? 85 FF 7F ?? 48 8D 15 ?? ?? ?? ?? 48 8D 8C 24 ?? ?? ?? ?? E8")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU5_Alternative2".to_string(),
            None,
            Pattern::new("89 ?? | ?? ?? ?? ?? 85 FF 7F ?? 48 8D 15 ?? ?? ?? ?? 48 8D 0D ?? ?? ?? ?? E8")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::GUObjectArray,
            "UUU5_Alternative3".to_string(),
            None,
            Pattern::new("89 15 | ?? ?? ?? ?? 85 FF 7F ?? 48 8D 8C 24 88 00 00 00 E8")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
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
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "B".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B 0D | ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 8B 01 4C 8D 4C 24")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "C".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 83 EC ?? 48 8B 0D | ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? ?? 48 8B 0D")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "D".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 89 3D | ?? ?? ?? ?? 48 85 FF 75 ?? E8 ?? ?? ?? ?? 48 8B 3D ?? ?? ?? ?? 48 8B 07")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "UUU5_Alternative0".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B 0D | ?? ?? ?? ?? 48 0F 45 1D ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? ?? 48 8B 0D")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "UUU5_Alternative1".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B 0D | ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 8B 01")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "UUU5_Alternative2".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B 0D | ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 8B 01 4C 8D 0D")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "UUU5_Alternative3".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 8B 0D | ?? ?? ?? ?? 48 85 C9 75 ?? E8 ?? ?? ?? ?? 48 8B 0D ?? ?? ?? ?? 48 8B 01 4C 8D 4C 24")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::IConsoleManagerSingleton,
            "UUU5_Alternative4".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 89 3D | ?? ?? ?? ?? 48 85 FF 75 ?? E8 ?? ?? ?? ?? 48 8B 3D ?? ?? ?? ?? 48 8B 07")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
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
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::NamePoolData,
            "UUU4_Alternative1".to_string(),
            None,
            Pattern::new("48 8D 05 | ?? ?? ?? ?? EB 27 48 8D 05 ?? ?? ?? ?? 48")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
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
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),
        PatternConfig::new(
            Sig::NamePoolData,
            "UUU4_Alternative4".to_string(),
            None,
            Pattern::new("E8 ?? ?? ?? ?? 48 89 D8 48 89 1D | ?? ?? ?? ?? 48 8B 5C 24 20 48 83 C4 28 C3 48 8B 5C")?,
            RIPRelativeResolvers::resolve_RIP_offset::<4>,
        ),

        //===============================[FPakPlatformFile]=============================================================================================
        PatternConfig::new(
            Sig::FPakPlatformFileInitialize,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 89 5c 24 10 48 89 74 24 18 48 89 7c 24 20 55 41 54 41 55 41 56 41 57 48 8d ac 24 20 fe ff ff 48 81 ec e0 02 00 00 48 8b 05 ?? ?? ?? ?? 48 33 c4 ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 48 8d 4c 24 78")?,
            FPakPlatformFile::resolve_initialize,
        ),
        PatternConfig::new(
            Sig::FPakPlatformFileDtor,
            "A".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("40 53 56 57 48 83 ec 20 48 8d 05 ?? ?? ?? ?? 4c 89 74 24 50 48 89 01 48 8b f9 e8 ?? ?? ?? ?? 48 8b c8")?,
            FPakPlatformFile::resolve_dtor,
        ),

        //===============================[FCustomVersionContainer]=============================================================================================
        PatternConfig::new(
            Sig::FCustomVersionContainer,
            "Direct".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 89 5c 24 ?? 48 89 74 24 ?? 57 48 83 ec ?? 48 8b f9 e8 ?? ?? ?? ?? 48 8b c8 48 8b d8 ff 15")?,
            resolve_self,
        ),


        //===============================[Xrefs]=============================================================================================
        PatternConfig::new(
            Sig::StringFTagMetaData,
            "FTagMetaData".to_string(),
            Some(object::SectionKind::ReadOnlyData),
            Pattern::from_bytes("FTagMetaData".encode_utf16().flat_map(u16::to_le_bytes).collect())?,
            xref::resolve,
        ),
        PatternConfig::new(
            Sig::SigningKey,
            "delegate".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("40 53 48 83 EC 50 E8 | ?? ?? ?? ?? 48 8B")?,
            resolve_self,
        ),
        PatternConfig::new(
            Sig::SigningKey,
            "delegate + call".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("40 53 48 83 EC 50 E8 | ?? ?? ?? ?? 48 8B")?,
            signing_key::resolve_follow_delegate,
        ),
        PatternConfig::new(
            Sig::SigningKey,
            "delegate + call + xref".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("40 53 48 83 EC 50 E8 | ?? ?? ?? ?? 48 8B")?,
            signing_key::resolve_follow_delegate_xref,
        ),
        PatternConfig::new(
            Sig::SigningKey,
            "delegate + call + xref + func".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("40 53 48 83 EC 50 E8 | ?? ?? ?? ?? 48 8B")?,
            signing_key::resolve_follow_delegate_xref_func,
        ),
        PatternConfig::new(
            Sig::SigningKey,
            "direct".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("48 89 5C 24 08 57 48 83 EC 20 65 48 8B 04 25 58 00 00 00 48 8B F9 8B ?? ?? ?? ?? ?? B9 BC 04 00 00 48 8B 14 D0 8B 04 11 39 ?? ?? ?? ?? ?? 0F 8F ?? ?? ?? ?? 8B")?,
            resolve_self,
        ),

        PatternConfig::new(
            Sig::AES,
            "AES".to_string(),
            Some(object::SectionKind::Text),
            Pattern::new("c7 45 d0 ?? ?? ?? ?? c7 45 d4 ?? ?? ?? ?? ?? ?? ?? ?? c7 45 d8 ?? ?? ?? ?? c7 45 dc ?? ?? ?? ?? c7 45 e0 ?? ?? ?? ?? c7 45 e4 ?? ?? ?? ?? c7 45 e8 ?? ?? ?? ?? c7 45 ec ?? ?? ?? ??")?,
            aes::resolve,
        ),
    ])
}

/// do nothing, return address of pattern
pub fn resolve_self(ctx: ResolveContext, _stages: &mut ResolveStages) -> ResolutionAction {
    ResolutionType::Address(ctx.match_address).into()
}

/// return containing function via exception table lookup
pub fn resolve_function(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
    stages.0.push(ctx.match_address);
    ctx.exe
        .get_function(ctx.match_address)
        .map(|f| f.range.start)
        .into()
}

/// do nothing, but return a constant so it's squashing all multiple matches to 1 value: 0x12345678
pub fn resolve_multi_self(_ctx: ResolveContext, _stages: &mut ResolveStages) -> ResolutionAction {
    ResolutionType::Count.into()
}

/// simply returns 0x1 as constant address so the scanner will pack multiple instances together as 1 and mention the amount.
pub fn resolve_engine_version(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
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
    stages.0.push(ctx.match_address);
    ResolutionType::String(format!("{}.{}", version_major, version_minor)).into()
}

#[allow(non_snake_case)]
mod RIPRelativeResolvers {
    use super::*;

    pub fn calc_rip(ctx: &ResolveContext, address: usize) -> Option<usize> {
        address
            .checked_add_signed(i32::from_le_bytes(
                ctx.memory[address..address + 4].try_into().unwrap(),
            ) as isize)
            .map(|a| a + 4)
    }

    fn resolve_RIP(
        memory: &MountedPE,
        match_address: usize,
        next_opcode_offset: usize,
        stages: &mut ResolveStages,
    ) -> ResolutionAction {
        stages.0.push(match_address);
        let rip_relative_value_address = match_address;
        // calculate the absolute address from the RIP relative value.
        let address = rip_relative_value_address
            .checked_add_signed(i32::from_le_bytes(
                memory[rip_relative_value_address..rip_relative_value_address + 4]
                    .try_into()
                    .unwrap(),
            ) as isize)
            .map(|a| a + next_opcode_offset);
        address.into()
    }

    pub fn resolve_RIP_offset<const N: usize>(
        ctx: ResolveContext,
        stages: &mut ResolveStages,
    ) -> ResolutionAction {
        resolve_RIP(ctx.memory, ctx.match_address, N, stages)
    }
}

#[allow(non_snake_case)]
mod FNameToStringID {
    use super::*;
    pub fn resolve(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
        stages.0.push(ctx.match_address);
        let n = ctx.match_address + 5;
        let rel = i32::from_le_bytes(ctx.memory[n - 4..n].try_into().unwrap());
        let address = n.checked_add_signed(rel as isize);
        address.into()
    }
    pub fn setenums(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
        stages.0.push(ctx.match_address);
        let n = ctx.match_address + 35;
        let rel = i32::from_le_bytes(ctx.memory[n - 4..n].try_into().unwrap());
        let address = n.checked_add_signed(rel as isize).unwrap();

        for i in address..address + 400 {
            if ctx.memory[i] == 0xe8 {
                stages.0.push(i.checked_add_signed(0).unwrap());
                let n = i.checked_add_signed(5).unwrap();
                let address = n.checked_add_signed(i32::from_le_bytes(
                    ctx.memory[n - 4..n].try_into().unwrap(),
                ) as isize);
                return address.into();
            }
        }
        address.into()
    }
}

#[allow(non_snake_case)]
mod FNameFNameID {
    use super::*;
    pub fn resolve_a(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
        stages.0.push(ctx.match_address);
        let n = ctx.match_address.checked_add_signed(0x18 + 5).unwrap();
        let address = n.checked_add_signed(i32::from_le_bytes(
            ctx.memory[n - 4..n].try_into().unwrap(),
        ) as isize);
        address.into()
    }
    pub fn resolve_v5_1(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
        stages.0.push(ctx.match_address);
        let n = ctx.match_address.checked_add_signed(0x1C + 5).unwrap();
        let address = n.checked_add_signed(i32::from_le_bytes(
            ctx.memory[n - 4..n].try_into().unwrap(),
        ) as isize);
        address.into()
    }
}

#[allow(non_snake_case)]
mod StaticConstructObjectInternalID {
    use super::*;
    pub fn resolve_a_v4_20(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
        stages.0.push(ctx.match_address);
        let n = ctx.match_address - 0x0e;
        let address = n.checked_add_signed(i32::from_le_bytes(
            ctx.memory[n - 4..n].try_into().unwrap(),
        ) as isize);
        address.into()
    }
    pub fn resolve_v4_16_4_19_v5_0(
        ctx: ResolveContext,
        stages: &mut ResolveStages,
    ) -> ResolutionAction {
        stages.0.push(ctx.match_address);
        let n = ctx.match_address + 5;
        let address = n.checked_add_signed(i32::from_le_bytes(
            ctx.memory[n - 4..n].try_into().unwrap(),
        ) as isize);
        address.into()
    }
}

#[allow(non_snake_case)]
mod GUObjectArrayID {
    use super::*;
    pub fn resolve_v_20(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
        stages.0.push(ctx.match_address);
        let n = ctx.match_address + 3;
        let address = n
            .checked_add_signed(
                i32::from_le_bytes(ctx.memory[n..n + 4].try_into().unwrap()) as isize
            )
            .map(|a| a - 0xc);
        address.into()
    }
    pub fn resolve_b_ext(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
        stages.0.push(ctx.match_address);
        let n = ctx.match_address + 6;
        let address = n
            .checked_add_signed(
                i32::from_le_bytes(ctx.memory[n..n + 4].try_into().unwrap()) as isize
            )
            .map(|a| a - 0xc);
        address.into()
    }
}

#[allow(non_snake_case)]
mod GNatives {
    use super::*;
    pub fn resolve(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
        stages.0.push(ctx.match_address - 1);
        for i in ctx.match_address..ctx.match_address + 400 {
            if ctx.memory[i] == 0x4c
                && ctx.memory[i + 1] == 0x8d
                && (ctx.memory[i + 2] & 0xc7 == 5 && ctx.memory[i + 2] > 0x20)
            {
                stages.0.push(i);
                let address = (i + 7).checked_add_signed(i32::from_le_bytes(
                    ctx.memory[i + 3..i + 3 + 4].try_into().unwrap(),
                ) as isize);
                return address.into();
            }
        }
        ResolutionType::Failed.into()
    }
}
#[allow(non_snake_case)]
mod FPakPlatformFile {
    use super::*;
    pub fn resolve_initialize(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
        stages.0.push(ctx.match_address);

        let patterns = [
            Pattern::new("48 8d 15 | ?? ?? ?? ?? 48 8b cf ff 50 40 eb 3e 39 1d ?? ?? ?? ?? 74 36 48 8b 0d ?? ?? ?? ??").unwrap(),
            Pattern::new("39 1d ?? ?? ?? ?? 74 36 48 8b 0d | ?? ?? ?? ??").unwrap(),
            Pattern::new("83 3d ?? ?? ?? ?? 00 74 37 48 8b 0d | ?? ?? ?? ??").unwrap(),
        ];

        let addresses = ctx
            .memory
            .get_section_containing(ctx.match_address)
            .map(|section| {
                let res = crate::scanner::scan_memchr(
                    &patterns.iter().map(|p| (&(), p)).collect::<Vec<_>>(),
                    0,
                    section.data,
                );
                let mut addresses = res
                    .into_iter()
                    .map(|(_, address)| {
                        // TODO allow passing sub-patterns to stages?
                        // TODO rename 'stages' to 'addresses_of_interest' or similar and give them names
                        stages.0.push(section.address + address);
                        section.address
                            + (address + 4)
                                .checked_add_signed(i32::from_le_bytes(
                                    section.data[address..address + 4].try_into().unwrap(),
                                ) as isize)
                                .unwrap()
                    })
                    .collect::<Vec<_>>();
                addresses.dedup();
                // TODO: implement returning multiple addresses
                format!("{:x?}", addresses)
            });

        addresses.into()
    }
    pub fn resolve_dtor(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
        stages.0.push(ctx.match_address);

        let patterns = [Pattern::new("48 8b 0d | ").unwrap()];

        let addresses = ctx
            .memory
            .get_section_containing(ctx.match_address)
            .map(|section| {
                let start = ctx.match_address - section.address;
                let res = crate::scanner::scan_memchr(
                    &patterns.iter().map(|p| (&(), p)).collect::<Vec<_>>(),
                    start,
                    &section.data[start..start + 400],
                );
                let mut addresses = res
                    .into_iter()
                    .map(|(_, address)| {
                        stages.0.push(section.address + address);
                        section.address
                            + (address + 4)
                                .checked_add_signed(i32::from_le_bytes(
                                    section.data[address..address + 4].try_into().unwrap(),
                                ) as isize)
                                .unwrap()
                    })
                    .collect::<Vec<_>>();
                addresses.dedup();
                format!("{:x?}", addresses)
            });

        addresses.into()
    }
}

mod xref {
    use super::*;

    pub fn resolve(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
        stages.0.push(ctx.match_address);
        ResolutionAction::Continue(Scan {
            section: Some(object::SectionKind::Text),
            scan_type: Xref(ctx.match_address).into(),
            resolve: resolve_self,
        })
    }
}

mod aes {
    use super::*;

    pub fn resolve(ctx: ResolveContext, stages: &mut ResolveStages) -> ResolutionAction {
        stages.0.push(ctx.match_address);
        let mut key = vec![0; 32];
        let data = &ctx.memory[ctx.match_address..ctx.match_address + 60];
        (&mut key[0..4]).copy_from_slice(&data[3..7]);
        (&mut key[4..8]).copy_from_slice(&data[10..14]);
        (&mut key[8..12]).copy_from_slice(&data[21..25]);
        (&mut key[12..16]).copy_from_slice(&data[28..32]);
        (&mut key[16..20]).copy_from_slice(&data[35..39]);
        (&mut key[20..24]).copy_from_slice(&data[42..46]);
        (&mut key[24..28]).copy_from_slice(&data[49..53]);
        (&mut key[28..32]).copy_from_slice(&data[56..60]);

        use std::fmt::Write;
        let mut hex = String::with_capacity(2 + 2 * data.len());
        hex.push_str("0x");
        for b in key {
            write!(&mut hex, "{:02x}", b).unwrap();
        }

        ResolutionType::String(hex).into()
    }
}

mod signing_key {
    use super::*;

    pub fn resolve_follow_delegate(
        ctx: ResolveContext,
        stages: &mut ResolveStages,
    ) -> ResolutionAction {
        stages.0.push(ctx.match_address);

        if let Some(addr) = ctx.match_address.checked_add_signed(i32::from_le_bytes(
            ctx.memory[ctx.match_address..ctx.match_address + 4]
                .try_into()
                .unwrap(),
        ) as isize)
        {
            let addr = addr + 4 + 39 + 3;
            stages.0.push(addr);
            if let Some(rip) = RIPRelativeResolvers::calc_rip(&ctx, addr) {
                rip.into()
            } else {
                ResolutionType::Failed.into()
            }
        } else {
            ResolutionType::Failed.into()
        }
    }

    pub fn resolve_follow_delegate_xref(
        ctx: ResolveContext,
        stages: &mut ResolveStages,
    ) -> ResolutionAction {
        stages.0.push(ctx.match_address);

        if let Some(addr) = ctx.match_address.checked_add_signed(i32::from_le_bytes(
            ctx.memory[ctx.match_address..ctx.match_address + 4]
                .try_into()
                .unwrap(),
        ) as isize)
        {
            let addr = addr + 4 + 39 + 3;
            stages.0.push(addr);
            if let Some(rip) = RIPRelativeResolvers::calc_rip(&ctx, addr) {
                ResolutionAction::Continue(Scan {
                    section: Some(object::SectionKind::Text),
                    scan_type: Xref(rip + 0x10).into(),
                    resolve: resolve_self,
                })
            } else {
                ResolutionType::Failed.into()
            }
        } else {
            ResolutionType::Failed.into()
        }
    }

    pub fn resolve_follow_delegate_xref_func(
        ctx: ResolveContext,
        stages: &mut ResolveStages,
    ) -> ResolutionAction {
        stages.0.push(ctx.match_address);

        if let Some(addr) = ctx.match_address.checked_add_signed(i32::from_le_bytes(
            ctx.memory[ctx.match_address..ctx.match_address + 4]
                .try_into()
                .unwrap(),
        ) as isize)
        {
            let addr = addr + 4 + 39 + 3;
            stages.0.push(addr);
            if let Some(rip) = RIPRelativeResolvers::calc_rip(&ctx, addr) {
                ResolutionAction::Continue(Scan {
                    section: Some(object::SectionKind::Text),
                    scan_type: Xref(rip + 0x10).into(),
                    resolve: resolve_function,
                })
            } else {
                ResolutionType::Failed.into()
            }
        } else {
            ResolutionType::Failed.into()
        }
    }
}
