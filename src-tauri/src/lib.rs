use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

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
use tauri::{State, ipc::Response};

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

/// The vault opened by the desktop user. Commands act on it instead of trusting a path sent by
/// the webview with every call.
#[derive(Debug, Default)]
pub struct VaultSession {
    root: Mutex<Option<PathBuf>>,
}

impl VaultSession {
    /// Opens an existing vault, or initializes one when `create` is set. A failed open closes
    /// the previous vault so commands cannot silently keep acting on it.
    pub fn open(&self, vault_root: &Path, create: bool) -> Result<(), CommandError> {
        let mut root = self.lock();
        *root = None;
        let catalog_path = vault_root.join("catalog.sqlite3");
        if create {
            SqliteCatalog::open(catalog_path)?;
        } else {
            SqliteCatalog::open_existing(catalog_path)?;
        }
        *root = Some(vault_root.to_path_buf());
        Ok(())
    }

    pub fn root(&self) -> Result<PathBuf, CommandError> {
        self.lock().clone().ok_or_else(|| CommandError {
            kind: ErrorKind::InvalidRequest.as_str(),
            message: "no vault is open".to_owned(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<PathBuf>> {
        // The guarded path stays consistent even if a holder panicked.
        self.root
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
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
) -> Result<Vec<u8>, CommandError> {
    let catalog = open_existing_catalog(vault_root)?;
    Ok(load_review_preview_use_case(&catalog, connector, review_item_id)?.bytes)
}

pub async fn load_review_preview_in_vault_with_connector_async(
    vault_root: PathBuf,
    review_item_id: i64,
    connector: Box<dyn ConnectorPort + Send>,
) -> Result<Vec<u8>, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        load_review_preview_in_vault_with_connector(&vault_root, review_item_id, connector.as_ref())
    })
    .await
    .map_err(|error| CommandError::worker_failed("review preview", error))?
}

#[tauri::command(rename_all = "snake_case")]
fn list_library(session: State<'_, VaultSession>) -> Result<Vec<LibraryEntry>, CommandError> {
    load_library(&session.root()?)
}

#[tauri::command(rename_all = "snake_case")]
fn list_review_items(session: State<'_, VaultSession>) -> Result<Vec<ReviewItem>, CommandError> {
    load_review_items(&session.root()?)
}

#[tauri::command(rename_all = "snake_case")]
fn open_vault(
    session: State<'_, VaultSession>,
    vault_root: String,
    create: bool,
) -> Result<(), CommandError> {
    session.open(Path::new(&vault_root), create)
}

#[tauri::command(rename_all = "snake_case")]
fn resolve_review_item(
    session: State<'_, VaultSession>,
    review_item_id: i64,
    decision: ReviewDecision,
) -> Result<ReviewItem, CommandError> {
    resolve_review_item_in_vault(&session.root()?, review_item_id, decision)
}

#[tauri::command(rename_all = "snake_case")]
async fn load_review_preview(
    session: State<'_, VaultSession>,
    review_item_id: i64,
) -> Result<Response, CommandError> {
    // Raw bytes reach the webview as an ArrayBuffer instead of a JSON number array.
    let bytes = load_review_preview_in_vault_with_connector_async(
        session.root()?,
        review_item_id,
        Box::new(LibretroThumbnailsConnector::new()),
    )
    .await?;
    Ok(Response::new(bytes))
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
    session: State<'_, VaultSession>,
    request: AcquisitionRequestInput,
) -> Result<AcquisitionRun, CommandError> {
    start_acquisition_run_in_vault(&session.root()?, request)
}

#[tauri::command(rename_all = "snake_case")]
async fn execute_acquisition_run(
    session: State<'_, VaultSession>,
    run_id: i64,
    matching_policy: MatchingPolicy,
) -> Result<AcquisitionRun, CommandError> {
    execute_acquisition_run_in_vault_with_connector_async(
        session.root()?,
        run_id,
        Box::new(LibretroThumbnailsConnector::new()),
        matching_policy,
    )
    .await
}

#[tauri::command(rename_all = "snake_case")]
fn get_acquisition_run(
    session: State<'_, VaultSession>,
    run_id: i64,
) -> Result<AcquisitionRun, CommandError> {
    load_acquisition_run_from_vault(&session.root()?, run_id)
}

#[tauri::command(rename_all = "snake_case")]
fn list_acquisition_runs(
    session: State<'_, VaultSession>,
) -> Result<Vec<AcquisitionRun>, CommandError> {
    list_acquisition_runs_from_vault(&session.root()?)
}

#[tauri::command(rename_all = "snake_case")]
fn pause_acquisition_run(
    session: State<'_, VaultSession>,
    run_id: i64,
) -> Result<AcquisitionRun, CommandError> {
    pause_acquisition_run_in_vault(&session.root()?, run_id)
}

#[tauri::command(rename_all = "snake_case")]
fn resume_acquisition_run(
    session: State<'_, VaultSession>,
    run_id: i64,
) -> Result<AcquisitionRun, CommandError> {
    resume_acquisition_run_in_vault(&session.root()?, run_id)
}

#[tauri::command(rename_all = "snake_case")]
fn cancel_acquisition_run(
    session: State<'_, VaultSession>,
    run_id: i64,
) -> Result<AcquisitionRun, CommandError> {
    cancel_acquisition_run_in_vault(&session.root()?, run_id)
}

pub fn run() {
    tauri::Builder::default()
        .manage(VaultSession::default())
        .invoke_handler(tauri::generate_handler![
            open_vault,
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
