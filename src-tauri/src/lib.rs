use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use game_media_vault_application::{
    AcquisitionPlan, AcquisitionRequestInput, ApplicationError, ConnectorPort, DerivationSummary,
    ErrorKind, LibraryPage, LibraryQuery, PortError, SourceDescription, VaultReport,
    acquire_run_with_connectors as acquire_run_with_connectors_use_case,
    build_acquisition_request as build_acquisition_request_use_case,
    cancel_acquisition_run as cancel_acquisition_run_use_case,
    derive_assets as derive_assets_use_case, describe_sources,
    list_acquisition_runs as list_acquisition_runs_use_case, list_library as list_library_use_case,
    list_review_items as list_review_items_use_case,
    load_acquisition_run as load_acquisition_run_use_case,
    load_review_preview as load_review_preview_use_case,
    pause_acquisition_run as pause_acquisition_run_use_case,
    plan_acquisition as plan_acquisition_use_case,
    resolve_review_item as resolve_review_item_use_case,
    resume_acquisition_run as resume_acquisition_run_use_case,
    search_library as search_library_use_case, start_acquisition_run_with_connectors,
    verify_vault as verify_vault_use_case,
};
use game_media_vault_connectors::registered_connectors;
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRun, DerivationRecipe, LibraryRelease, MatchingPolicy,
    ReviewDecision, ReviewItem,
};
use game_media_vault_infrastructure::{
    ContentAddressedStore, ImageTransformer, SqliteCatalog, inspect_media,
};
use serde::Serialize;
use tauri::{Manager, State, http, ipc::Response};

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

/// A canonical vault path as an identity: as is when it is Unicode, else in its debug form,
/// quoted with its other bytes escaped. That form is lossless, so distinct paths never share an
/// identity, and no canonical path written as is starts with a quote.
fn vault_identity(canonical: &Path) -> String {
    canonical
        .to_str()
        .map_or_else(|| format!("{canonical:?}"), str::to_owned)
}

/// The vault opened by the desktop user. Commands act on it instead of trusting a path sent by
/// the webview with every call.
#[derive(Debug, Default)]
pub struct VaultSession {
    root: Mutex<Option<PathBuf>>,
}

impl VaultSession {
    /// Opens an existing vault, or initializes one when `create` is set, and returns its
    /// identity: the canonical form of its path, which every spelling of that path shares. A
    /// failed open closes the previous vault so commands cannot silently keep acting on it.
    pub fn open(&self, vault_root: &Path, create: bool) -> Result<String, CommandError> {
        let mut root = self.lock();
        *root = None;
        let catalog_path = vault_root.join("catalog.sqlite3");
        if create {
            SqliteCatalog::open(catalog_path)?;
        } else {
            SqliteCatalog::open_existing(catalog_path)?;
        }
        // An identity that is not canonical would split one vault in two, so the open fails
        // rather than fall back to the path as given.
        let canonical = fs::canonicalize(vault_root).map_err(|error| CommandError {
            kind: ErrorKind::External.as_str(),
            message: format!(
                "failed to resolve the vault path {}: {error}",
                vault_root.display()
            ),
        })?;
        *root = Some(vault_root.to_path_buf());
        Ok(vault_identity(&canonical))
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

pub fn load_library(vault_root: &Path) -> Result<Vec<LibraryRelease>, CommandError> {
    Ok(list_library_use_case(&open_existing_catalog(vault_root)?)?)
}

/// Renders the thumbnail of every retained original lacking one, on a blocking worker since
/// decoding and scaling images takes a while.
pub async fn derive_thumbnails_in_vault_async(
    vault_root: PathBuf,
    max_edge: u32,
) -> Result<DerivationSummary, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = open_existing_catalog(&vault_root)?;
        Ok(derive_assets_use_case(
            &catalog,
            &ContentAddressedStore::new(&vault_root),
            &ImageTransformer::new(),
            &DerivationRecipe::Thumbnail { max_edge },
        )?)
    })
    .await
    .map_err(|error| CommandError::worker_failed("thumbnail rendering", error))?
}

/// Compares the catalog with the stored bytes, on a blocking worker since every referenced
/// object is hashed again. Nothing is repaired.
pub async fn verify_vault_async(vault_root: PathBuf) -> Result<VaultReport, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = open_existing_catalog(&vault_root)?;
        Ok(verify_vault_use_case(
            &catalog,
            &ContentAddressedStore::new(&vault_root),
        )?)
    })
    .await
    .map_err(|error| CommandError::worker_failed("vault verification", error))?
}

/// Searches the vault's Library with the query every frontend shares.
pub fn search_library_in_vault(
    vault_root: &Path,
    query: &LibraryQuery,
) -> Result<LibraryPage, CommandError> {
    let catalog = open_existing_catalog(vault_root)?;
    Ok(search_library_use_case(&catalog, &catalog, query)?)
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

/// The connectors of a registry, one per Source, as the use cases take them.
fn registry_refs(registry: &[Box<dyn ConnectorPort + Send>]) -> Vec<&dyn ConnectorPort> {
    registry
        .iter()
        .map(|connector| connector.as_ref() as &dyn ConnectorPort)
        .collect()
}

/// Describes every registered Source from the capabilities planning uses; no vault is needed and
/// no Source is consulted.
pub fn list_registered_sources() -> Vec<SourceDescription> {
    describe_sources(&registry_refs(&registered_connectors()))
}

#[tauri::command]
fn list_sources() -> Vec<SourceDescription> {
    list_registered_sources()
}

pub fn load_review_preview_in_vault(
    vault_root: &Path,
    review_item_id: i64,
    connectors: &[&dyn ConnectorPort],
) -> Result<Vec<u8>, CommandError> {
    let catalog = open_existing_catalog(vault_root)?;
    Ok(load_review_preview_use_case(&catalog, connectors, review_item_id)?.bytes)
}

pub async fn load_review_preview_in_vault_async(
    vault_root: PathBuf,
    review_item_id: i64,
    registry: Vec<Box<dyn ConnectorPort + Send>>,
) -> Result<Vec<u8>, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        load_review_preview_in_vault(&vault_root, review_item_id, &registry_refs(&registry))
    })
    .await
    .map_err(|error| CommandError::worker_failed("review preview", error))?
}

#[tauri::command(rename_all = "snake_case")]
fn list_library(session: State<'_, VaultSession>) -> Result<Vec<LibraryRelease>, CommandError> {
    load_library(&session.root()?)
}

#[tauri::command(rename_all = "snake_case")]
async fn derive_thumbnails(
    session: State<'_, VaultSession>,
    max_edge: u32,
) -> Result<DerivationSummary, CommandError> {
    derive_thumbnails_in_vault_async(session.root()?, max_edge).await
}

#[tauri::command(rename_all = "snake_case")]
async fn verify_vault(session: State<'_, VaultSession>) -> Result<VaultReport, CommandError> {
    verify_vault_async(session.root()?).await
}

#[tauri::command(rename_all = "snake_case")]
fn search_library(
    session: State<'_, VaultSession>,
    query: LibraryQuery,
) -> Result<LibraryPage, CommandError> {
    search_library_in_vault(&session.root()?, &query)
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
) -> Result<String, CommandError> {
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
    let bytes = load_review_preview_in_vault_async(
        session.root()?,
        review_item_id,
        registered_connectors(),
    )
    .await?;
    Ok(Response::new(bytes))
}

/// Starts a run only if the registered `connectors`, which the desktop executes it with, can
/// plan it.
pub fn start_acquisition_run_in_vault(
    vault_root: &Path,
    request: AcquisitionRequestInput,
    connectors: &[&dyn ConnectorPort],
) -> Result<AcquisitionRun, CommandError> {
    let catalog = SqliteCatalog::open(vault_root.join("catalog.sqlite3"))?;
    Ok(start_acquisition_run_with_connectors(
        &catalog, request, connectors,
    )?)
}

/// Explains which of the registered `connectors` `request` would contact and what each
/// acquires; no vault is needed.
pub fn plan_acquisition_with_connectors(
    request: AcquisitionRequestInput,
    connectors: &[&dyn ConnectorPort],
) -> Result<AcquisitionPlan, CommandError> {
    let request = validate_acquisition_request(request)?;
    Ok(plan_acquisition_use_case(&request, connectors)?)
}

/// Plans on a blocking worker, since connectors may consult their Source.
pub async fn plan_acquisition_async(
    request: AcquisitionRequestInput,
    registry: Vec<Box<dyn ConnectorPort + Send>>,
) -> Result<AcquisitionPlan, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        plan_acquisition_with_connectors(request, &registry_refs(&registry))
    })
    .await
    .map_err(|error| CommandError::worker_failed("acquisition planning", error))?
}

/// Starts a run on a blocking worker, since checking the plan may reach the Source.
pub async fn start_acquisition_run_in_vault_async(
    vault_root: PathBuf,
    request: AcquisitionRequestInput,
    registry: Vec<Box<dyn ConnectorPort + Send>>,
) -> Result<AcquisitionRun, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        start_acquisition_run_in_vault(&vault_root, request, &registry_refs(&registry))
    })
    .await
    .map_err(|error| CommandError::worker_failed("acquisition start", error))?
}

pub fn execute_acquisition_run_in_vault(
    vault_root: &Path,
    run_id: i64,
    connectors: &[&dyn ConnectorPort],
    matching_policy: MatchingPolicy,
) -> Result<AcquisitionRun, CommandError> {
    let catalog = open_existing_catalog(vault_root)?;
    let object_store = ContentAddressedStore::new(vault_root);
    acquire_run_with_connectors_use_case(
        &catalog,
        &catalog,
        &catalog,
        &object_store,
        connectors,
        run_id,
        matching_policy,
    )?;
    Ok(load_acquisition_run_use_case(&catalog, run_id)?)
}

pub async fn execute_acquisition_run_in_vault_async(
    vault_root: PathBuf,
    run_id: i64,
    registry: Vec<Box<dyn ConnectorPort + Send>>,
    matching_policy: MatchingPolicy,
) -> Result<AcquisitionRun, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        execute_acquisition_run_in_vault(
            &vault_root,
            run_id,
            &registry_refs(&registry),
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
async fn plan_acquisition(
    request: AcquisitionRequestInput,
) -> Result<AcquisitionPlan, CommandError> {
    plan_acquisition_async(request, registered_connectors()).await
}

#[tauri::command(rename_all = "snake_case")]
async fn start_acquisition_run(
    session: State<'_, VaultSession>,
    request: AcquisitionRequestInput,
) -> Result<AcquisitionRun, CommandError> {
    start_acquisition_run_in_vault_async(session.root()?, request, registered_connectors()).await
}

#[tauri::command(rename_all = "snake_case")]
async fn execute_acquisition_run(
    session: State<'_, VaultSession>,
    run_id: i64,
    matching_policy: MatchingPolicy,
) -> Result<AcquisitionRun, CommandError> {
    execute_acquisition_run_in_vault_async(
        session.root()?,
        run_id,
        registered_connectors(),
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

/// URI scheme serving original objects of the open vault to the webview.
pub const OBJECT_PROTOCOL: &str = "gmv-object";

/// Answers `gmv-object` requests: the path must be a BLAKE3 object hash of the open vault, so
/// the webview can only read original objects and Derived Assets, never arbitrary files.
pub fn object_response(session: &VaultSession, path: &str) -> http::Response<Vec<u8>> {
    let hash = path.trim_start_matches('/');
    let is_object_hash = hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !is_object_hash {
        return plain_response(http::StatusCode::BAD_REQUEST, "not an object hash");
    }
    let Ok(vault_root) = session.root() else {
        return plain_response(http::StatusCode::CONFLICT, "no vault is open");
    };
    let store = ContentAddressedStore::new(vault_root);
    // Derived Assets are content-addressed too, apart from the originals.
    let bytes = match std::fs::read(store.object_path(hash)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::read(store.derived_path(hash))
        }
        read => read,
    };
    match bytes {
        Ok(bytes) => http::Response::builder()
            .status(http::StatusCode::OK)
            .header(http::header::CONTENT_TYPE, inspect_media(&bytes).media_type)
            // Objects are immutable: the same hash always serves the same bytes.
            .header(
                http::header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            )
            // Unknown bytes stay opaque instead of being sniffed into renderable documents.
            .header(http::header::X_CONTENT_TYPE_OPTIONS, "nosniff")
            .body(bytes)
            .unwrap_or_else(|_| {
                plain_response(http::StatusCode::INTERNAL_SERVER_ERROR, "invalid response")
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            plain_response(http::StatusCode::NOT_FOUND, "object not found")
        }
        Err(_) => plain_response(http::StatusCode::INTERNAL_SERVER_ERROR, "object unreadable"),
    }
}

fn plain_response(status: http::StatusCode, message: &str) -> http::Response<Vec<u8>> {
    let mut response = http::Response::new(message.as_bytes().to_vec());
    *response.status_mut() = status;
    response
}

pub fn run() {
    tauri::Builder::default()
        .manage(VaultSession::default())
        .register_asynchronous_uri_scheme_protocol(
            OBJECT_PROTOCOL,
            |context, request, responder| {
                let app = context.app_handle().clone();
                let path = request.uri().path().to_owned();
                // Reading originals must not block the webview's event loop.
                tauri::async_runtime::spawn_blocking(move || {
                    responder.respond(object_response(&app.state::<VaultSession>(), &path));
                });
            },
        )
        .invoke_handler(tauri::generate_handler![
            open_vault,
            list_library,
            search_library,
            derive_thumbnails,
            verify_vault,
            list_review_items,
            resolve_review_item,
            load_review_preview,
            build_acquisition_request,
            plan_acquisition,
            start_acquisition_run,
            execute_acquisition_run,
            get_acquisition_run,
            list_acquisition_runs,
            pause_acquisition_run,
            resume_acquisition_run,
            cancel_acquisition_run,
            list_sources
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Game Media Vault");
}
