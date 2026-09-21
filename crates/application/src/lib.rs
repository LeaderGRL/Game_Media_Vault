use std::path::Path;

use game_media_vault_domain::{
    AssetType, ImportedAsset, LibraryEntry, PersistLocalBoxFront, SourceKind, StoredObject,
};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct PortError(pub String);

pub trait ObjectStorePort {
    fn store_original(&self, source: &Path) -> Result<StoredObject, PortError>;
}

pub trait CatalogPort {
    fn persist_local_box_front(
        &self,
        record: PersistLocalBoxFront,
    ) -> Result<ImportedAsset, PortError>;

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportLocalBoxFrontRequest {
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
    let source_location = request.source_path.to_string_lossy().into_owned();
    let stored = object_store.store_original(&request.source_path)?;

    Ok(catalog.persist_local_box_front(PersistLocalBoxFront {
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
