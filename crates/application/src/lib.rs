use std::{
    fs,
    path::{Path, PathBuf},
};

use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRun, AcquisitionRunStatus, AcquisitionWorkItem, AssetType,
    ImportedAsset, LibraryEntry, PersistAsset, SourceKind, StoredObject,
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
