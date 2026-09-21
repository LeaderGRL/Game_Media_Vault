use std::path::Path;

use game_media_vault_domain::{
    AssetType, ImportedAsset, LibraryEntry, PersistAsset, SourceKind, StoredObject,
};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct PortError(pub String);

pub trait ObjectStorePort {
    fn store_original(&self, source: &Path) -> Result<StoredObject, PortError>;
}

pub trait CatalogPort {
    fn persist_asset(&self, record: PersistAsset) -> Result<ImportedAsset, PortError>;

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportLocalBoxFrontRequest {
    pub existing_game_id: Option<i64>,
    pub game_title: String,
    pub platform: String,
    pub region: String,
    pub edition_name: String,
    pub source_path: std::path::PathBuf,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApplicationError {
    #[error("source path does not contain a file name")]
    MissingSourceFileName,
    #[error("failed to resolve source path: {0}")]
    ResolveSourcePath(String),
    #[error("{0}")]
    Port(#[from] PortError),
}

pub fn import_local_box_front(
    catalog: &dyn CatalogPort,
    object_store: &dyn ObjectStorePort,
    request: ImportLocalBoxFrontRequest,
) -> Result<ImportedAsset, ApplicationError> {
    let original_filename = request
        .source_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or(ApplicationError::MissingSourceFileName)?;
    let source_location = std::path::absolute(&request.source_path)
        .map_err(|error| ApplicationError::ResolveSourcePath(error.to_string()))?
        .to_string_lossy()
        .into_owned();
    let stored = object_store.store_original(&request.source_path)?;

    Ok(catalog.persist_asset(PersistAsset {
        existing_game_id: request.existing_game_id,
        game_title: request.game_title,
        platform: request.platform,
        region: request.region,
        edition_name: request.edition_name,
        asset_type: AssetType::BoxFront,
        object_hash: stored.hash,
        byte_len: stored.byte_len,
        original_filename,
        source_kind: SourceKind::LocalImport,
        source_location,
    })?)
}

pub fn list_library(catalog: &dyn CatalogPort) -> Result<Vec<LibraryEntry>, ApplicationError> {
    Ok(catalog.list_library()?)
}
