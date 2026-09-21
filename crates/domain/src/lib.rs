use serde::{Deserialize, Serialize};

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
