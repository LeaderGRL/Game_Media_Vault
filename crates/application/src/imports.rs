use std::{
    fs,
    path::{Path, PathBuf},
};

use game_media_vault_domain::{AssetType, ImportedAsset, LibraryEntry, PersistAsset, SourceId};

use crate::{
    ApplicationError, CatalogPort, ObjectStorePort, PortError, ReferenceCatalogRepositoryPort,
    ReferenceCatalogSourcePort,
};

const REFERENCE_IMPORT_BATCH_SIZE: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportLocalBoxFrontRequest {
    pub existing_game_id: Option<i64>,
    pub game_title: String,
    pub platform: String,
    pub region: String,
    pub edition_name: String,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReferenceCatalogRequest {
    pub source_path: PathBuf,
    pub max_games: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceImportSummary {
    pub imported_releases: usize,
}

pub fn import_reference_catalog(
    catalog: &dyn ReferenceCatalogRepositoryPort,
    source: &dyn ReferenceCatalogSourcePort,
    request: ImportReferenceCatalogRequest,
) -> Result<ReferenceImportSummary, ApplicationError> {
    if request.max_games == 0 {
        return Err(ApplicationError::InvalidReferenceImportLimit);
    }

    let source_path = resolve_source_path(&request.source_path)?;
    let releases = source.read_releases(&source_path, request.max_games)?;
    let mut imported_releases = 0;
    let mut batch = Vec::with_capacity(REFERENCE_IMPORT_BATCH_SIZE);
    for release in releases.into_iter().take(request.max_games) {
        batch.push(release);
        if batch.len() == REFERENCE_IMPORT_BATCH_SIZE {
            imported_releases += catalog
                .persist_reference_releases(std::mem::take(&mut batch))?
                .len();
        }
    }
    if !batch.is_empty() {
        imported_releases += catalog.persist_reference_releases(batch)?.len();
    }

    Ok(ReferenceImportSummary { imported_releases })
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
    let mut source =
        fs::File::open(&resolved_source_path).map_err(|error| PortError(error.to_string()))?;
    let stored = object_store.store_original(&mut source)?;

    Ok(catalog.persist_asset(PersistAsset {
        existing_game_id: request.existing_game_id,
        existing_release_edition_id: None,
        match_decision: None,
        game_title: request.game_title,
        platform: request.platform,
        region: request.region,
        edition_name: request.edition_name,
        asset_type: AssetType::BoxFront,
        object_hash: stored.hash,
        byte_len: stored.byte_len,
        original_filename,
        source_id: SourceId::from("local_import"),
        source_asset_label: None,
        source_location,
    })?)
}

pub fn list_library(catalog: &dyn CatalogPort) -> Result<Vec<LibraryEntry>, ApplicationError> {
    Ok(catalog.list_library()?)
}

/// Canonical absolute path of a user-provided source, without Windows verbatim prefixes, so
/// recorded source locations stay readable and identical across import kinds.
fn resolve_source_path(path: &Path) -> Result<PathBuf, ApplicationError> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| ApplicationError::ResolveSourcePath(error.to_string()))?;
    Ok(without_verbatim_prefix(canonical))
}

#[cfg(windows)]
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let location = path.to_string_lossy();
    if let Some(network_path) = location.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{network_path}"));
    }
    if let Some(local_path) = location.strip_prefix(r"\\?\") {
        return PathBuf::from(local_path);
    }
    path
}

#[cfg(not(windows))]
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
}

fn source_location(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
