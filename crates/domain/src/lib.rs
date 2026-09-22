use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "values", rename_all = "snake_case")]
pub enum SourceSelection {
    Auto,
    Explicit(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "values", rename_all = "snake_case")]
pub enum GameSelection {
    All,
    Explicit(Vec<String>),
    PlatformBound(Vec<PlatformBoundGameSelector>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformBoundGameSelector {
    pub game: String,
    pub platform: String,
}

impl GameSelection {
    fn fixes_platforms_explicitly(&self) -> bool {
        matches!(
            self,
            Self::PlatformBound(values)
                if !values.is_empty()
                    && values.iter().all(|value| !value.platform.trim().is_empty())
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetTypeSelector {
    BoxFront,
    BoxBack,
    Spine,
    InnerCover,
    BoxTexture,
    #[serde(rename = "box_3d_render")]
    Box3dRender,
    #[serde(rename = "box_3d_model")]
    Box3dModel,
    SlipcoverSleeve,
    Insert,
    Cartridge,
    CartridgeFront,
    CartridgeBack,
    CartridgeLabel,
    Disc,
    DiscFront,
    DiscBack,
    DiscLabel,
    Pcb,
    CassetteTape,
    FloppyDisk,
    Manual,
    ManualPage,
    StrategyGuide,
    Map,
    ReferenceCard,
    RegistrationCard,
    WarrantySafetyInsert,
    Screenshot,
    TitleScreen,
    GameplayVideo,
    Trailer,
    Logo,
    Icon,
    WallpaperArtwork,
    Flyer,
    Advertisement,
    Poster,
    PromotionalArtwork,
    PressMaterial,
    MagazineScan,
    ArcadeCabinet,
    ControlPanel,
    Marquee,
    Bezel,
    Controller,
    Accessory,
    Soundtrack,
    Texture,
    #[serde(rename = "3d_model")]
    Model3d,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionPolicy {
    KeepEverything,
    KeepBestPerType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct QualityRequirements {
    pub min_width: Option<u32>,
    pub min_height: Option<u32>,
    pub min_longest_edge: Option<u32>,
    pub min_pixel_count: Option<u64>,
    pub original_only: bool,
    pub accepted_mime_types: Vec<String>,
    pub max_compression_ratio: Option<u32>,
    pub min_bitrate_kbps: Option<u32>,
    pub preferred_scan_type: Option<String>,
    pub preferred_source_priority: Vec<String>,
    pub best_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcquisitionLimits {
    pub max_games: Option<u32>,
    pub max_downloads: Option<u32>,
    pub max_concurrent_downloads: Option<u16>,
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AcquisitionRequest {
    sources: SourceSelection,
    platforms: Vec<String>,
    games: GameSelection,
    regions: Vec<String>,
    languages: Vec<String>,
    asset_types: Vec<AssetTypeSelector>,
    quality: Option<QualityRequirements>,
    retention: RetentionPolicy,
    limits: AcquisitionLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcquisitionRequestDraft {
    pub sources: SourceSelection,
    pub platforms: Vec<String>,
    pub games: GameSelection,
    pub regions: Vec<String>,
    pub languages: Vec<String>,
    pub asset_types: Vec<AssetTypeSelector>,
    pub quality: Option<QualityRequirements>,
    pub retention: RetentionPolicy,
    pub limits: AcquisitionLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AcquisitionRequestValidationError {
    MissingSources,
    MissingAssetTypes,
    MissingPlatforms,
}

impl fmt::Display for AcquisitionRequestValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSources => {
                formatter.write_str("acquisition request must include at least one source")
            }
            Self::MissingAssetTypes => {
                formatter.write_str("acquisition request must include at least one asset type")
            }
            Self::MissingPlatforms => {
                formatter.write_str("acquisition request must include at least one platform")
            }
        }
    }
}

impl std::error::Error for AcquisitionRequestValidationError {}

impl AcquisitionRequest {
    pub fn try_from_draft(
        draft: AcquisitionRequestDraft,
    ) -> Result<Self, AcquisitionRequestValidationError> {
        let sources = match draft.sources {
            SourceSelection::Auto => SourceSelection::Auto,
            SourceSelection::Explicit(values) => {
                let values: Vec<_> = values
                    .into_iter()
                    .filter(|value| !value.trim().is_empty())
                    .collect();
                if values.is_empty() {
                    return Err(AcquisitionRequestValidationError::MissingSources);
                }
                SourceSelection::Explicit(values)
            }
        };
        if draft.asset_types.is_empty() {
            return Err(AcquisitionRequestValidationError::MissingAssetTypes);
        }
        if draft.platforms.is_empty() && !draft.games.fixes_platforms_explicitly() {
            return Err(AcquisitionRequestValidationError::MissingPlatforms);
        }

        Ok(Self {
            sources,
            platforms: draft.platforms,
            games: draft.games,
            regions: draft.regions,
            languages: draft.languages,
            asset_types: draft.asset_types,
            quality: draft.quality,
            retention: draft.retention,
            limits: draft.limits,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    BoxFront,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    LocalImport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub hash: String,
    pub byte_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistAsset {
    pub existing_game_id: Option<i64>,
    pub game_title: String,
    pub platform: String,
    pub region: String,
    pub edition_name: String,
    pub asset_type: AssetType,
    pub object_hash: String,
    pub byte_len: u64,
    pub original_filename: String,
    pub source_kind: SourceKind,
    pub source_location: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedAsset {
    pub game_id: i64,
    pub release_edition_id: i64,
    pub asset_id: i64,
    pub object_hash: String,
    pub byte_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetProvenance {
    pub source_kind: SourceKind,
    pub source_location: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub game_id: i64,
    pub game_title: String,
    pub release_edition_id: i64,
    pub platform: String,
    pub region: String,
    pub edition_name: String,
    pub asset_id: i64,
    pub asset_type: AssetType,
    pub object_hash: String,
    pub byte_len: u64,
    pub original_filename: String,
    pub provenance: Vec<AssetProvenance>,
}
