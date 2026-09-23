use std::path::{Path, PathBuf};

use game_media_vault_application::{
    AcquisitionRequestInput, AcquisitionRequestValidationError, ConnectorPort,
    acquire_run_with_connector as acquire_run_with_connector_use_case,
    build_acquisition_request as build_acquisition_request_use_case,
    cancel_acquisition_run as cancel_acquisition_run_use_case,
    list_acquisition_runs as list_acquisition_runs_use_case, list_library as list_library_use_case,
    load_acquisition_run as load_acquisition_run_use_case,
    pause_acquisition_run as pause_acquisition_run_use_case,
    resume_acquisition_run as resume_acquisition_run_use_case,
    start_acquisition_run as start_acquisition_run_use_case,
};
use game_media_vault_connectors::LibretroThumbnailsConnector;
use game_media_vault_domain::{AcquisitionRequest, AcquisitionRun, LibraryEntry, MatchingPolicy};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};

pub fn validate_acquisition_request(
    request: AcquisitionRequestInput,
) -> Result<AcquisitionRequest, AcquisitionRequestValidationError> {
    build_acquisition_request_use_case(request)
}

#[tauri::command(rename_all = "snake_case")]
fn build_acquisition_request(
    request: AcquisitionRequestInput,
) -> Result<AcquisitionRequest, AcquisitionRequestValidationError> {
    validate_acquisition_request(request)
}

pub fn load_library(vault_root: &Path) -> Result<Vec<LibraryEntry>, String> {
    let catalog = SqliteCatalog::open_existing(vault_root.join("catalog.sqlite3"))
        .map_err(|error| error.to_string())?;
    list_library_use_case(&catalog).map_err(|error| error.to_string())
}

#[tauri::command(rename_all = "snake_case")]
fn list_library(vault_root: String) -> Result<Vec<LibraryEntry>, String> {
    load_library(Path::new(&vault_root))
}

pub fn start_acquisition_run_in_vault(
    vault_root: &Path,
    request: AcquisitionRequestInput,
) -> Result<AcquisitionRun, String> {
    let catalog = SqliteCatalog::open(vault_root.join("catalog.sqlite3"))
        .map_err(|error| error.to_string())?;
    start_acquisition_run_use_case(&catalog, request).map_err(|error| error.to_string())
}

pub fn execute_acquisition_run_in_vault_with_connector(
    vault_root: &Path,
    run_id: i64,
    connector: &dyn ConnectorPort,
    matching_policy: MatchingPolicy,
) -> Result<AcquisitionRun, String> {
    let catalog = SqliteCatalog::open_existing(vault_root.join("catalog.sqlite3"))
        .map_err(|error| error.to_string())?;
    let object_store = ContentAddressedStore::new(vault_root);
    acquire_run_with_connector_use_case(
        &catalog,
        &catalog,
        &object_store,
        connector,
        run_id,
        matching_policy,
    )
    .map_err(|error| error.to_string())?;
    load_acquisition_run_use_case(&catalog, run_id).map_err(|error| error.to_string())
}

pub async fn execute_acquisition_run_in_vault_with_connector_async(
    vault_root: PathBuf,
    run_id: i64,
    connector: Box<dyn ConnectorPort + Send>,
    matching_policy: MatchingPolicy,
) -> Result<AcquisitionRun, String> {
    tauri::async_runtime::spawn_blocking(move || {
        execute_acquisition_run_in_vault_with_connector(
            &vault_root,
            run_id,
            connector.as_ref(),
            matching_policy,
        )
    })
    .await
    .map_err(|error| format!("acquisition worker failed: {error}"))?
}

pub fn load_acquisition_run_from_vault(
    vault_root: &Path,
    run_id: i64,
) -> Result<AcquisitionRun, String> {
    let catalog = SqliteCatalog::open_existing(vault_root.join("catalog.sqlite3"))
        .map_err(|error| error.to_string())?;
    load_acquisition_run_use_case(&catalog, run_id).map_err(|error| error.to_string())
}

pub fn list_acquisition_runs_from_vault(vault_root: &Path) -> Result<Vec<AcquisitionRun>, String> {
    let catalog = SqliteCatalog::open_existing(vault_root.join("catalog.sqlite3"))
        .map_err(|error| error.to_string())?;
    list_acquisition_runs_use_case(&catalog).map_err(|error| error.to_string())
}

pub fn pause_acquisition_run_in_vault(
    vault_root: &Path,
    run_id: i64,
) -> Result<AcquisitionRun, String> {
    let catalog = SqliteCatalog::open_existing(vault_root.join("catalog.sqlite3"))
        .map_err(|error| error.to_string())?;
    pause_acquisition_run_use_case(&catalog, run_id).map_err(|error| error.to_string())
}

pub fn resume_acquisition_run_in_vault(
    vault_root: &Path,
    run_id: i64,
) -> Result<AcquisitionRun, String> {
    let catalog = SqliteCatalog::open_existing(vault_root.join("catalog.sqlite3"))
        .map_err(|error| error.to_string())?;
    resume_acquisition_run_use_case(&catalog, run_id).map_err(|error| error.to_string())
}

pub fn cancel_acquisition_run_in_vault(
    vault_root: &Path,
    run_id: i64,
) -> Result<AcquisitionRun, String> {
    let catalog = SqliteCatalog::open_existing(vault_root.join("catalog.sqlite3"))
        .map_err(|error| error.to_string())?;
    cancel_acquisition_run_use_case(&catalog, run_id).map_err(|error| error.to_string())
}

#[tauri::command(rename_all = "snake_case")]
fn start_acquisition_run(
    vault_root: String,
    request: AcquisitionRequestInput,
) -> Result<AcquisitionRun, String> {
    start_acquisition_run_in_vault(Path::new(&vault_root), request)
}

#[tauri::command(rename_all = "snake_case")]
async fn execute_acquisition_run(
    vault_root: String,
    run_id: i64,
    matching_policy: MatchingPolicy,
) -> Result<AcquisitionRun, String> {
    execute_acquisition_run_in_vault_with_connector_async(
        PathBuf::from(vault_root),
        run_id,
        Box::new(LibretroThumbnailsConnector::new()),
        matching_policy,
    )
    .await
}

#[tauri::command(rename_all = "snake_case")]
fn get_acquisition_run(vault_root: String, run_id: i64) -> Result<AcquisitionRun, String> {
    load_acquisition_run_from_vault(Path::new(&vault_root), run_id)
}

#[tauri::command(rename_all = "snake_case")]
fn list_acquisition_runs(vault_root: String) -> Result<Vec<AcquisitionRun>, String> {
    list_acquisition_runs_from_vault(Path::new(&vault_root))
}

#[tauri::command(rename_all = "snake_case")]
fn pause_acquisition_run(vault_root: String, run_id: i64) -> Result<AcquisitionRun, String> {
    pause_acquisition_run_in_vault(Path::new(&vault_root), run_id)
}

#[tauri::command(rename_all = "snake_case")]
fn resume_acquisition_run(vault_root: String, run_id: i64) -> Result<AcquisitionRun, String> {
    resume_acquisition_run_in_vault(Path::new(&vault_root), run_id)
}

#[tauri::command(rename_all = "snake_case")]
fn cancel_acquisition_run(vault_root: String, run_id: i64) -> Result<AcquisitionRun, String> {
    cancel_acquisition_run_in_vault(Path::new(&vault_root), run_id)
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_library,
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
