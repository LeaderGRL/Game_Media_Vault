use std::{
    fs,
    path::{Path, PathBuf},
};

use game_media_vault_domain::{
    AssetType, ImportedAsset, LibraryRelease, PersistAsset, ReferenceReviewItem, SourceId,
};

use crate::{
    ApplicationError, CatalogPort, ObjectStorePort, PortError, ReferenceCatalogRepositoryPort,
    ReferenceCatalogSourcePort, ReferenceReviewRepositoryPort,
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReferenceImportSummary {
    pub imported_releases: usize,
    /// Records of the file too malformed to read, which the import skipped.
    pub skipped_records: usize,
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
    let location = source_location(&source_path);
    let read = source.read_releases(&source_path, request.max_games)?;
    let mut imported_releases = 0;
    let mut batch = Vec::with_capacity(REFERENCE_IMPORT_BATCH_SIZE);
    for mut release in read.releases.into_iter().take(request.max_games) {
        for assertion in &mut release.assertions {
            assertion.source_location.clone_from(&location);
        }
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

    Ok(ReferenceImportSummary {
        imported_releases,
        skipped_records: read.skipped_records,
    })
}

/// The reference records awaiting a human to tell which Release Edition, if any, they describe:
/// those whose evidence pointed at several editions of other sources when they were imported.
pub fn list_reference_review_items(
    reviews: &dyn ReferenceReviewRepositoryPort,
) -> Result<Vec<ReferenceReviewItem>, ApplicationError> {
    Ok(reviews.list_reference_review_items()?)
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
        fs::File::open(&resolved_source_path).map_err(|error| PortError::new(error.to_string()))?;
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
        media: stored.media,
        original_filename,
        source_id: SourceId::from("local_import"),
        source_asset_label: None,
        source_location,
    })?)
}

/// Lists the library with the Canonical Values derived from each release's assertions.
pub fn list_library(catalog: &dyn CatalogPort) -> Result<Vec<LibraryRelease>, ApplicationError> {
    Ok(catalog
        .list_library()?
        .into_iter()
        .map(LibraryRelease::from)
        .collect())
}

/// Canonical absolute path of a user-provided source, used for reading it. On Windows it keeps
/// the verbatim prefix so long or unusual file names stay addressable.
fn resolve_source_path(path: &Path) -> Result<PathBuf, ApplicationError> {
    let canonical = fs::canonicalize(path).map_err(source_path_error)?;
    // Metadata failures keep their own classification; only a readable non-file is invalid.
    if !fs::metadata(&canonical)
        .map_err(source_path_error)?
        .is_file()
    {
        return Err(ApplicationError::ResolveSourcePath(format!(
            "{} is not a file",
            path.display()
        )));
    }
    Ok(canonical)
}

/// A path that names no file is an invalid request; other failures, such as denied
/// permissions or unavailable storage, are environmental and may succeed when retried.
fn source_path_error(error: std::io::Error) -> ApplicationError {
    match error.kind() {
        std::io::ErrorKind::NotFound
        | std::io::ErrorKind::InvalidInput
        | std::io::ErrorKind::InvalidFilename
        | std::io::ErrorKind::NotADirectory => {
            ApplicationError::ResolveSourcePath(error.to_string())
        }
        _ => ApplicationError::Port(PortError::new(format!(
            "failed to resolve source path: {error}"
        ))),
    }
}

/// Readable location recorded for a resolved source, identical across import kinds: Windows
/// verbatim prefixes are dropped.
#[cfg(windows)]
fn source_location(path: &Path) -> String {
    let location = path.to_string_lossy();
    if let Some(network_path) = location.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{network_path}");
    }
    if let Some(local_path) = location.strip_prefix(r"\\?\") {
        return local_path.to_owned();
    }
    location.into_owned()
}

#[cfg(not(windows))]
fn source_location(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind as IoErrorKind};

    use super::source_path_error;
    use crate::ErrorKind;

    #[test]
    fn only_paths_naming_no_file_are_invalid_requests() {
        assert_eq!(
            source_path_error(Error::from(IoErrorKind::NotFound)).kind(),
            ErrorKind::InvalidRequest
        );
        assert_eq!(
            source_path_error(Error::from(IoErrorKind::PermissionDenied)).kind(),
            ErrorKind::External
        );
    }
}
