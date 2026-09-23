use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRun, AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate,
    AssetType, ConnectorCapabilities, ImportedAsset, ImportedReleaseEdition, LibraryEntry,
    PersistAsset, ReferenceReleaseRecord, RetentionPolicy, SourceId, StoredObject,
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

    fn store_original_reader(&self, reader: &mut dyn Read) -> Result<StoredObject, PortError>;

    fn store_original_bytes(&self, bytes: &[u8]) -> Result<StoredObject, PortError> {
        let mut reader = Cursor::new(bytes);
        self.store_original_reader(&mut reader)
    }
}

pub trait CatalogPort {
    fn persist_asset(&self, record: PersistAsset) -> Result<ImportedAsset, PortError>;

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError>;
}

pub trait RunRepositoryPort {
    fn create_run(&self, request: AcquisitionRequest) -> Result<AcquisitionRun, PortError>;

    fn get_run(&self, run_id: i64) -> Result<Option<AcquisitionRun>, PortError>;

    fn list_runs(&self) -> Result<Vec<AcquisitionRun>, PortError>;

    fn queue_work(&self, run_id: i64, work_key: String) -> Result<(), PortError>;

    fn next_queued_work(&self, run_id: i64) -> Result<Option<AcquisitionWorkItem>, PortError>;

    fn complete_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError>;

    fn compare_and_set_run_status(
        &self,
        run_id: i64,
        expected: AcquisitionRunStatus,
        target: AcquisitionRunStatus,
    ) -> Result<bool, PortError>;
}

pub trait ConnectorPort {
    fn source_id(&self) -> &'static str;

    fn capabilities(&self) -> ConnectorCapabilities;

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError>;

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError>;
}

pub trait ReferenceCatalogSourcePort {
    fn read_releases(
        &self,
        source_path: &Path,
        max_games: usize,
    ) -> Result<Vec<ReferenceReleaseRecord>, PortError>;
}

pub trait ReferenceCatalogRepositoryPort {
    fn persist_reference_release(
        &self,
        record: ReferenceReleaseRecord,
    ) -> Result<ImportedReleaseEdition, PortError>;

    fn persist_reference_releases(
        &self,
        records: Vec<ReferenceReleaseRecord>,
    ) -> Result<Vec<ImportedReleaseEdition>, PortError> {
        records
            .into_iter()
            .map(|record| self.persist_reference_release(record))
            .collect()
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReferenceCatalogRequest {
    pub source_path: PathBuf,
    pub max_games: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceImportSummary {
    pub imported_releases: usize,
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

pub fn load_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    runs.get_run(run_id)?
        .ok_or(ApplicationError::RunNotFound(run_id))
}

pub fn list_acquisition_runs(
    runs: &dyn RunRepositoryPort,
) -> Result<Vec<AcquisitionRun>, ApplicationError> {
    Ok(runs.list_runs()?)
}

pub fn queue_acquisition_work(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
    work_key: String,
) -> Result<(), ApplicationError> {
    let run = load_acquisition_run(runs, run_id)?;
    if matches!(
        run.status,
        AcquisitionRunStatus::Cancelled | AcquisitionRunStatus::Completed
    ) {
        return Err(ApplicationError::RunNotAcceptingWork { status: run.status });
    }
    if work_key.trim().is_empty() {
        return Err(ApplicationError::InvalidWorkKey);
    }
    Ok(runs.queue_work(run_id, work_key)?)
}

pub fn next_acquisition_work(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<Option<AcquisitionWorkItem>, ApplicationError> {
    let run = load_acquisition_run(runs, run_id)?;
    if run.status != AcquisitionRunStatus::Running {
        return Ok(None);
    }
    Ok(runs.next_queued_work(run_id)?)
}

pub fn complete_acquisition_work(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
    work_key: &str,
) -> Result<(), ApplicationError> {
    load_acquisition_run(runs, run_id)?;
    Ok(runs.complete_work(run_id, work_key)?)
}

pub fn pause_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    transition_acquisition_run(runs, run_id, AcquisitionRunStatus::Paused)
}

pub fn resume_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    transition_acquisition_run(runs, run_id, AcquisitionRunStatus::Running)
}

pub fn cancel_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    transition_acquisition_run(runs, run_id, AcquisitionRunStatus::Cancelled)
}

pub fn complete_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    transition_acquisition_run(runs, run_id, AcquisitionRunStatus::Completed)
}

fn transition_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
    target: AcquisitionRunStatus,
) -> Result<AcquisitionRun, ApplicationError> {
    let mut current = load_acquisition_run(runs, run_id)?;
    loop {
        if current.status == target {
            return Ok(current);
        }
        if target == AcquisitionRunStatus::Completed && current.queued_work != 0 {
            return Err(ApplicationError::RunHasQueuedWork {
                queued_work: current.queued_work,
            });
        }

        let allowed = matches!(
            (current.status, target),
            (AcquisitionRunStatus::Running, AcquisitionRunStatus::Paused)
                | (AcquisitionRunStatus::Paused, AcquisitionRunStatus::Running)
                | (
                    AcquisitionRunStatus::Running,
                    AcquisitionRunStatus::Cancelled
                )
                | (
                    AcquisitionRunStatus::Paused,
                    AcquisitionRunStatus::Cancelled
                )
                | (
                    AcquisitionRunStatus::Running,
                    AcquisitionRunStatus::Completed
                )
        );
        if !allowed {
            return Err(ApplicationError::InvalidRunTransition {
                from: current.status,
                to: target,
            });
        }

        if runs.compare_and_set_run_status(run_id, current.status, target)? {
            return load_acquisition_run(runs, run_id);
        }
        current = load_acquisition_run(runs, run_id)?;
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApplicationError {
    #[error("source path does not contain a file name")]
    MissingSourceFileName,
    #[error("failed to resolve source path: {0}")]
    ResolveSourcePath(String),
    #[error("reference catalog imports require a positive game limit")]
    InvalidReferenceImportLimit,
    #[error("{0}")]
    Port(#[from] PortError),
    #[error("{0}")]
    Validation(#[from] AcquisitionRequestValidationError),
    #[error("acquisition run #{0} does not exist")]
    RunNotFound(i64),
    #[error("acquisition work key must not be blank")]
    InvalidWorkKey,
    #[error("acquisition run cannot accept new work while {status:?}")]
    RunNotAcceptingWork { status: AcquisitionRunStatus },
    #[error("acquisition run still has {queued_work} queued work item(s)")]
    RunHasQueuedWork { queued_work: u64 },
    #[error("cannot transition acquisition run from {from:?} to {to:?}")]
    InvalidRunTransition {
        from: AcquisitionRunStatus,
        to: AcquisitionRunStatus,
    },
    #[error("connector {source_id} is not selected by this acquisition request")]
    ConnectorNotSelected { source_id: String },
    #[error("connector {source_id} does not support direct media downloads")]
    ConnectorCannotDownload { source_id: String },
    #[error("acquisition run cannot execute connector work while {status:?}")]
    RunNotExecutable { status: AcquisitionRunStatus },
    #[error("connector {source_id} cannot execute this acquisition plan: {reason}")]
    UnsupportedConnectorPlan { source_id: String, reason: String },
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

const REFERENCE_IMPORT_BATCH_SIZE: usize = 256;

pub fn acquire_run_with_connector(
    runs: &dyn RunRepositoryPort,
    catalog: &dyn CatalogPort,
    object_store: &dyn ObjectStorePort,
    connector: &dyn ConnectorPort,
    run_id: i64,
) -> Result<Vec<ImportedAsset>, ApplicationError> {
    let run = load_acquisition_run(runs, run_id)?;
    if run.status == AcquisitionRunStatus::Completed {
        return Ok(Vec::new());
    }
    if run.status != AcquisitionRunStatus::Running {
        return Err(ApplicationError::RunNotExecutable { status: run.status });
    }
    if !run.request.selects_source(connector.source_id()) {
        return Err(ApplicationError::ConnectorNotSelected {
            source_id: connector.source_id().to_owned(),
        });
    }

    let capabilities = connector.capabilities();
    if !capabilities.direct_media_download {
        return Err(ApplicationError::ConnectorCannotDownload {
            source_id: connector.source_id().to_owned(),
        });
    }
    validate_connector_plan(&run.request, connector.source_id(), &capabilities)?;

    let mut candidates_by_work_key = std::collections::HashMap::new();
    for candidate in connector.discover(&run.request)? {
        if !run.request.requests_asset_type(candidate.asset_type)
            || !capabilities.asset_types.contains(&candidate.asset_type)
        {
            continue;
        }

        let work_key = connector_work_key(connector.source_id(), &candidate);
        queue_acquisition_work(runs, run_id, work_key.clone())?;
        candidates_by_work_key.insert(work_key, candidate);
    }

    let mut imported_assets = Vec::new();
    while let Some(work) = next_acquisition_work(runs, run_id)? {
        let Some(candidate) = candidates_by_work_key.get(&work.key) else {
            break;
        };
        let mut stream = connector.download(candidate)?;
        let stored = object_store.store_original_reader(stream.as_mut())?;
        let imported = catalog.persist_asset(PersistAsset {
            existing_game_id: None,
            game_title: candidate.game_title.clone(),
            platform: candidate.platform.clone(),
            region: candidate.region.clone(),
            edition_name: candidate.edition_name.clone(),
            asset_type: candidate.asset_type,
            object_hash: stored.hash,
            byte_len: stored.byte_len,
            original_filename: candidate.original_filename.clone(),
            source_id: candidate.source_id.clone(),
            source_asset_label: candidate.source_asset_label.clone(),
            source_location: candidate.source_url.clone(),
        })?;
        complete_acquisition_work(runs, run_id, &work.key)?;
        imported_assets.push(imported);
    }

    runs.compare_and_set_run_status(
        run_id,
        AcquisitionRunStatus::Running,
        AcquisitionRunStatus::Completed,
    )?;

    Ok(imported_assets)
}

fn connector_work_key(source_id: &str, candidate: &AssetCandidate) -> String {
    let mut key = "connector".to_owned();
    for part in [
        source_id,
        candidate.platform.as_str(),
        candidate.game_title.as_str(),
        candidate.region.as_str(),
        candidate.edition_name.as_str(),
        asset_type_work_key(candidate.asset_type),
        candidate.source_url.as_str(),
    ] {
        push_work_key_part(&mut key, part);
    }
    key
}

fn push_work_key_part(key: &mut String, value: &str) {
    key.push(':');
    key.push_str(&value.len().to_string());
    key.push(':');
    key.push_str(value);
}

fn asset_type_work_key(asset_type: AssetType) -> &'static str {
    match asset_type {
        AssetType::BoxFront => "box_front",
    }
}

fn validate_connector_plan(
    request: &AcquisitionRequest,
    source_id: &str,
    capabilities: &ConnectorCapabilities,
) -> Result<(), ApplicationError> {
    let unsupported = |reason: &str| ApplicationError::UnsupportedConnectorPlan {
        source_id: source_id.to_owned(),
        reason: reason.to_owned(),
    };

    if !request.selects_only_source(source_id) {
        return Err(unsupported(
            "this execution path requires one explicitly selected source",
        ));
    }
    if !request.requested_asset_types_supported_by(&capabilities.asset_types) {
        return Err(unsupported(
            "one or more requested asset types are not supported by this connector",
        ));
    }
    if request.quality().is_some() {
        return Err(unsupported(
            "quality requirements are not supported by this execution path",
        ));
    }
    if request.retention() != RetentionPolicy::KeepEverything {
        return Err(unsupported(
            "Keep Best Per Type is not supported by this execution path",
        ));
    }
    if request.limits() != &game_media_vault_domain::AcquisitionLimits::default() {
        return Err(unsupported(
            "acquisition limits are not supported by this execution path",
        ));
    }
    Ok(())
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
        source_id: SourceId::from("local_import"),
        source_asset_label: None,
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
