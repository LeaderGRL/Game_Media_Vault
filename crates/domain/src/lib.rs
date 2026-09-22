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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetTypeSelector {
    BoxFront,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionPolicy {
    KeepEverything,
    KeepBestPerType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct QualityRequirements {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcquisitionLimits {}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquisitionRequestValidationError {
    MissingSources,
    MissingAssetTypes,
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
        }
    }
}

impl std::error::Error for AcquisitionRequestValidationError {}

impl AcquisitionRequest {
    pub fn try_from_draft(
        draft: AcquisitionRequestDraft,
    ) -> Result<Self, AcquisitionRequestValidationError> {
        if matches!(&draft.sources, SourceSelection::Explicit(values) if values.is_empty()) {
            return Err(AcquisitionRequestValidationError::MissingSources);
        }
        if draft.asset_types.is_empty() {
            return Err(AcquisitionRequestValidationError::MissingAssetTypes);
        }

        Ok(Self {
            sources: draft.sources,
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
