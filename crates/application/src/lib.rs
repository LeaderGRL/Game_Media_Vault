use std::{
    fs,
    path::{Path, PathBuf},
};

use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRun, AssetType, ImportedAsset, LibraryEntry, PersistAsset,
    SourceKind, StoredObject,
};
use thiserror::Error;

pub use game_media_vault_domain::{
    AcquisitionRequestDraft as AcquisitionRequestInput, AcquisitionRequestValidationError,
};

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

pub trait RunRepositoryPort {
    fn create_run(&self, request: AcquisitionRequest) -> Result<AcquisitionRun, PortError>;
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

pub fn build_acquisition_request(
    input: AcquisitionRequestInput,
) -> Result<AcquisitionRequest, AcquisitionRequestValidationError> {
    AcquisitionRequest::try_from_draft(input)
}

pub fn start_acquisition_run(
    runs: &dyn RunRepositoryPort,
    input: AcquisitionRequestInput,
) -> Result<AcquisitionRun, ApplicationError> {
    let request = build_acquisition_request(input)?;
    Ok(runs.create_run(request)?)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApplicationError {
    #[error("source path does not contain a file name")]
    MissingSourceFileName,
    #[error("failed to resolve source path: {0}")]
    ResolveSourcePath(String),
    #[error("{0}")]
    Port(#[from] PortError),
    #[error("{0}")]
    Validation(#[from] AcquisitionRequestValidationError),
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
    let resolved_source_path = resolve_source_path(&request.source_path)?;
    let source_location = source_location(&resolved_source_path);
    let stored = object_store.store_original(&resolved_source_path)?;

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

fn resolve_source_path(path: &Path) -> Result<PathBuf, ApplicationError> {
    fs::canonicalize(path).map_err(|error| ApplicationError::ResolveSourcePath(error.to_string()))
}

fn source_location(path: &Path) -> String {
    let location = path.to_string_lossy();

    #[cfg(windows)]
    {
        if let Some(network_path) = location.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{network_path}");
        }
        if let Some(local_path) = location.strip_prefix(r"\\?\") {
            return local_path.to_owned();
        }
    }

    location.into_owned()
}

pub fn list_library(catalog: &dyn CatalogPort) -> Result<Vec<LibraryEntry>, ApplicationError> {
    Ok(catalog.list_library()?)
}
