use std::path::{Path, PathBuf};

use game_media_vault_application::{
    AcquisitionRequestInput, ApplicationError, ConnectorPort, ErrorKind, PortError,
    acquire_run_with_connector as acquire_run_with_connector_use_case,
    build_acquisition_request as build_acquisition_request_use_case,
    cancel_acquisition_run as cancel_acquisition_run_use_case,
    list_acquisition_runs as list_acquisition_runs_use_case, list_library as list_library_use_case,
    list_review_items as list_review_items_use_case,
    load_acquisition_run as load_acquisition_run_use_case,
    load_review_preview as load_review_preview_use_case,
    pause_acquisition_run as pause_acquisition_run_use_case,
    resolve_review_item as resolve_review_item_use_case,
    resume_acquisition_run as resume_acquisition_run_use_case,
    start_acquisition_run as start_acquisition_run_use_case,
};
use game_media_vault_connectors::LibretroThumbnailsConnector;
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRun, LibraryEntry, MatchingPolicy, ReviewDecision, ReviewItem,
};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use serde::Serialize;

/// Error returned by every command: a stable `kind` the frontend can branch on and a
/// human-readable `message`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandError {
    pub kind: &'static str,
    pub message: String,
}

impl From<ApplicationError> for CommandError {
    fn from(error: ApplicationError) -> Self {
        Self {
            kind: error.kind().as_str(),
            message: error.to_string(),
        }
    }
}

impl From<PortError> for CommandError {
    fn from(error: PortError) -> Self {
        ApplicationError::from(error).into()
    }
}

impl CommandError {
    fn worker_failed(worker: &str, error: impl std::fmt::Display) -> Self {
        Self {
            kind: ErrorKind::External.as_str(),
            message: format!("{worker} worker failed: {error}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewPreviewPayload {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

fn open_existing_catalog(vault_root: &Path) -> Result<SqliteCatalog, CommandError> {
    Ok(SqliteCatalog::open_existing(
        vault_root.join("catalog.sqlite3"),
    )?)
}

pub fn validate_acquisition_request(
    request: AcquisitionRequestInput,
) -> Result<AcquisitionRequest, CommandError> {
    build_acquisition_request_use_case(request)
        .map_err(|error| ApplicationError::Validation(error).into())
}

#[tauri::command(rename_all = "snake_case")]
fn build_acquisition_request(
    request: AcquisitionRequestInput,
) -> Result<AcquisitionRequest, CommandError> {
    validate_acquisition_request(request)
}

pub fn load_library(vault_root: &Path) -> Result<Vec<LibraryEntry>, CommandError> {
    Ok(list_library_use_case(&open_existing_catalog(vault_root)?)?)
}

pub fn load_review_items(vault_root: &Path) -> Result<Vec<ReviewItem>, CommandError> {
    Ok(list_review_items_use_case(&open_existing_catalog(
        vault_root,
    )?)?)
}

pub fn resolve_review_item_in_vault(
    vault_root: &Path,
    review_item_id: i64,
    decision: ReviewDecision,
) -> Result<ReviewItem, CommandError> {
    let catalog = open_existing_catalog(vault_root)?;
    Ok(resolve_review_item_use_case(
        &catalog,
        review_item_id,
        decision,
    )?)
}

pub fn load_review_preview_in_vault_with_connector(
    vault_root: &Path,
    review_item_id: i64,
    connector: &dyn ConnectorPort,
) -> Result<ReviewPreviewPayload, CommandError> {
    let catalog = open_existing_catalog(vault_root)?;
    let preview = load_review_preview_use_case(&catalog, connector, review_item_id)?;
    Ok(ReviewPreviewPayload {
        media_type: preview_media_type(&preview.original_filename).to_owned(),
        bytes: preview.bytes,
    })
}

pub async fn load_review_preview_in_vault_with_connector_async(
    vault_root: PathBuf,
    review_item_id: i64,
    connector: Box<dyn ConnectorPort + Send>,
) -> Result<ReviewPreviewPayload, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        load_review_preview_in_vault_with_connector(&vault_root, review_item_id, connector.as_ref())
    })
    .await
    .map_err(|error| CommandError::worker_failed("review preview", error))?
}

#[tauri::command(rename_all = "snake_case")]
fn list_library(vault_root: String) -> Result<Vec<LibraryEntry>, CommandError> {
    load_library(Path::new(&vault_root))
}

#[tauri::command(rename_all = "snake_case")]
fn list_review_items(vault_root: String) -> Result<Vec<ReviewItem>, CommandError> {
    load_review_items(Path::new(&vault_root))
}

#[tauri::command(rename_all = "snake_case")]
fn resolve_review_item(
    vault_root: String,
    review_item_id: i64,
    decision: ReviewDecision,
) -> Result<ReviewItem, CommandError> {
    resolve_review_item_in_vault(Path::new(&vault_root), review_item_id, decision)
}

#[tauri::command(rename_all = "snake_case")]
async fn load_review_preview(
    vault_root: String,
    review_item_id: i64,
) -> Result<ReviewPreviewPayload, CommandError> {
    load_review_preview_in_vault_with_connector_async(
        PathBuf::from(vault_root),
        review_item_id,
        Box::new(LibretroThumbnailsConnector::new()),
    )
    .await
}

pub fn start_acquisition_run_in_vault(
    vault_root: &Path,
    request: AcquisitionRequestInput,
) -> Result<AcquisitionRun, CommandError> {
    let catalog = SqliteCatalog::open(vault_root.join("catalog.sqlite3"))?;
    Ok(start_acquisition_run_use_case(&catalog, request)?)
}

pub fn execute_acquisition_run_in_vault_with_connector(
    vault_root: &Path,
    run_id: i64,
    connector: &dyn ConnectorPort,
    matching_policy: MatchingPolicy,
) -> Result<AcquisitionRun, CommandError> {
    let catalog = open_existing_catalog(vault_root)?;
    let object_store = ContentAddressedStore::new(vault_root);
    acquire_run_with_connector_use_case(
        &catalog,
        &catalog,
        &catalog,
        &object_store,
        connector,
        run_id,
        matching_policy,
    )?;
    Ok(load_acquisition_run_use_case(&catalog, run_id)?)
}

pub async fn execute_acquisition_run_in_vault_with_connector_async(
    vault_root: PathBuf,
    run_id: i64,
    connector: Box<dyn ConnectorPort + Send>,
    matching_policy: MatchingPolicy,
) -> Result<AcquisitionRun, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        execute_acquisition_run_in_vault_with_connector(
            &vault_root,
            run_id,
            connector.as_ref(),
            matching_policy,
        )
    })
    .await
    .map_err(|error| CommandError::worker_failed("acquisition", error))?
}

pub fn load_acquisition_run_from_vault(
    vault_root: &Path,
    run_id: i64,
) -> Result<AcquisitionRun, CommandError> {
    Ok(load_acquisition_run_use_case(
        &open_existing_catalog(vault_root)?,
        run_id,
    )?)
}

pub fn list_acquisition_runs_from_vault(
    vault_root: &Path,
) -> Result<Vec<AcquisitionRun>, CommandError> {
    Ok(list_acquisition_runs_use_case(&open_existing_catalog(
        vault_root,
    )?)?)
}

pub fn pause_acquisition_run_in_vault(
    vault_root: &Path,
    run_id: i64,
) -> Result<AcquisitionRun, CommandError> {
    Ok(pause_acquisition_run_use_case(
        &open_existing_catalog(vault_root)?,
        run_id,
    )?)
}

pub fn resume_acquisition_run_in_vault(
    vault_root: &Path,
    run_id: i64,
) -> Result<AcquisitionRun, CommandError> {
    Ok(resume_acquisition_run_use_case(
        &open_existing_catalog(vault_root)?,
        run_id,
    )?)
}

pub fn cancel_acquisition_run_in_vault(
    vault_root: &Path,
    run_id: i64,
) -> Result<AcquisitionRun, CommandError> {
    Ok(cancel_acquisition_run_use_case(
        &open_existing_catalog(vault_root)?,
        run_id,
    )?)
}

#[tauri::command(rename_all = "snake_case")]
fn start_acquisition_run(
    vault_root: String,
    request: AcquisitionRequestInput,
) -> Result<AcquisitionRun, CommandError> {
    start_acquisition_run_in_vault(Path::new(&vault_root), request)
}

#[tauri::command(rename_all = "snake_case")]
async fn execute_acquisition_run(
    vault_root: String,
    run_id: i64,
    matching_policy: MatchingPolicy,
) -> Result<AcquisitionRun, CommandError> {
    execute_acquisition_run_in_vault_with_connector_async(
        PathBuf::from(vault_root),
        run_id,
        Box::new(LibretroThumbnailsConnector::new()),
        matching_policy,
    )
    .await
}

#[tauri::command(rename_all = "snake_case")]
fn get_acquisition_run(vault_root: String, run_id: i64) -> Result<AcquisitionRun, CommandError> {
    load_acquisition_run_from_vault(Path::new(&vault_root), run_id)
}

#[tauri::command(rename_all = "snake_case")]
fn list_acquisition_runs(vault_root: String) -> Result<Vec<AcquisitionRun>, CommandError> {
    list_acquisition_runs_from_vault(Path::new(&vault_root))
}

#[tauri::command(rename_all = "snake_case")]
fn pause_acquisition_run(vault_root: String, run_id: i64) -> Result<AcquisitionRun, CommandError> {
    pause_acquisition_run_in_vault(Path::new(&vault_root), run_id)
}

#[tauri::command(rename_all = "snake_case")]
fn resume_acquisition_run(vault_root: String, run_id: i64) -> Result<AcquisitionRun, CommandError> {
    resume_acquisition_run_in_vault(Path::new(&vault_root), run_id)
}

#[tauri::command(rename_all = "snake_case")]
fn cancel_acquisition_run(vault_root: String, run_id: i64) -> Result<AcquisitionRun, CommandError> {
    cancel_acquisition_run_in_vault(Path::new(&vault_root), run_id)
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_library,
            list_review_items,
            resolve_review_item,
            load_review_preview,
            build_acquisition_request,
            start_acquisition_run,
            execute_acquisition_run,
            get_acquisition_run,
            list_acquisition_runs,
            pause_acquisition_run,
            resume_acquisition_run,
            cancel_acquisition_run
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Game Media Vault");
}

fn preview_media_type(filename: &str) -> &'static str {
    match Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("avif") => "image/avif",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}
