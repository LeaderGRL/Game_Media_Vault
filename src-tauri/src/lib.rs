use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use game_media_vault_application::{
    AcquisitionPlan, AcquisitionRequestInput, ApiKey, ApplicationError, ConnectorPort,
    DerivationSummary, DescribedReferenceReviewItem, DownloadLimits, ErrorKind,
    ImportReferenceCatalogRequest, LibraryPage, LibraryQuery, Machine, PackagingModelSummary,
    PortError, ReferenceCatalogSourcePort, ReferenceImportSummary, SourceDescription,
    SourceFailureSummary, VaultReport,
    acquire_run_with_connectors as acquire_run_with_connectors_use_case,
    build_acquisition_request as build_acquisition_request_use_case,
    cancel_acquisition_run as cancel_acquisition_run_use_case,
    clear_source_credential as clear_source_credential_use_case,
    derive_assets as derive_assets_use_case,
    derive_packaging_models as derive_packaging_models_use_case, describe_sources,
    import_reference_catalog as import_reference_catalog_use_case,
    keep_reference_review_item_apart as keep_reference_review_item_apart_use_case,
    link_reference_review_item as link_reference_review_item_use_case,
    list_acquisition_runs as list_acquisition_runs_use_case, list_library as list_library_use_case,
    list_reference_review_items as list_reference_review_items_use_case,
    list_review_items as list_review_items_use_case,
    load_acquisition_run as load_acquisition_run_use_case,
    load_review_preview as load_review_preview_use_case, machine_registry,
    pause_acquisition_run as pause_acquisition_run_use_case,
    plan_acquisition as plan_acquisition_use_case,
    resolve_review_item as resolve_review_item_use_case,
    resume_acquisition_run as resume_acquisition_run_use_case,
    search_library as search_library_use_case,
    set_source_credential as set_source_credential_use_case,
    set_source_enabled as set_source_enabled_use_case, start_acquisition_run_with_connectors,
    summarize_source_failures, verify_vault as verify_vault_use_case,
};
use game_media_vault_connectors::{
    MameSoftwareListCatalog, NoIntroReferenceCatalog, RedumpReferenceCatalog, registered_connectors,
};
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRun, DerivationRecipe, LibraryRelease, MatchingPolicy,
    ReviewDecision, ReviewItem,
};
use game_media_vault_infrastructure::{
    ContentAddressedStore, GltfPackagingBuilder, KeyringCredentialStore, MediaTransformers,
    SqliteCatalog, inspect_media, machine_settings,
};
use serde::{Deserialize, Serialize};
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
            &MediaTransformers::machine(),
            &DerivationRecipe::Thumbnail { max_edge },
        )?)
    })
    .await
    .map_err(|error| CommandError::worker_failed("thumbnail rendering", error))?
}

/// Builds the packaging model every complete release lacks, on a blocking worker since
/// decoding scans and encoding textures takes a while.
pub async fn derive_packaging_models_in_vault_async(
    vault_root: PathBuf,
) -> Result<PackagingModelSummary, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = open_existing_catalog(&vault_root)?;
        Ok(derive_packaging_models_use_case(
            &catalog,
            &catalog,
            &ContentAddressedStore::new(&vault_root),
            &GltfPackagingBuilder::new(),
        )?)
    })
    .await
    .map_err(|error| CommandError::worker_failed("packaging model generation", error))?
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

/// The failures the vault's executions recorded, by Source, with the `latest` of each.
pub fn load_source_failures(
    vault_root: &Path,
    latest: usize,
) -> Result<Vec<SourceFailureSummary>, CommandError> {
    Ok(summarize_source_failures(
        &open_existing_catalog(vault_root)?,
        latest,
    )?)
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

/// The reference records awaiting a human, with the editions each names.
pub fn load_reference_review_items(
    vault_root: &Path,
) -> Result<Vec<DescribedReferenceReviewItem>, CommandError> {
    Ok(list_reference_review_items_use_case(
        &open_existing_catalog(vault_root)?,
    )?)
}

/// Decides that the record of a Reference Review Item describes one of its candidates, and
/// returns the items still pending.
pub fn link_reference_review_item_in_vault(
    vault_root: &Path,
    item_id: i64,
    release_edition_id: i64,
) -> Result<Vec<DescribedReferenceReviewItem>, CommandError> {
    Ok(link_reference_review_item_use_case(
        &open_existing_catalog(vault_root)?,
        item_id,
        release_edition_id,
    )?)
}

/// Decides that the record of a Reference Review Item describes none of its candidates, and
/// returns the items still pending.
pub fn keep_reference_review_item_apart_in_vault(
    vault_root: &Path,
    item_id: i64,
) -> Result<Vec<DescribedReferenceReviewItem>, CommandError> {
    Ok(keep_reference_review_item_apart_use_case(
        &open_existing_catalog(vault_root)?,
        item_id,
    )?)
}

/// The connectors of a registry, one per Source, as the use cases take them.
fn registry_refs(registry: &[Box<dyn ConnectorPort>]) -> Vec<&dyn ConnectorPort> {
    registry
        .iter()
        .map(|connector| connector.as_ref() as &dyn ConnectorPort)
        .collect()
}

/// The connector of every implemented Source, reading the API keys they need from this
/// machine's credential store.
fn machine_registry_of_sources() -> Vec<Box<dyn ConnectorPort>> {
    registered_connectors(Arc::new(KeyringCredentialStore::machine()))
}

/// The registered connectors, one per Source, as the settings of this machine leave them.
fn machine_connectors() -> Result<Vec<Box<dyn ConnectorPort>>, CommandError> {
    Ok(machine_registry(
        machine_registry_of_sources(),
        &machine_settings().disabled_sources()?,
    ))
}

/// Describes every registered Source from the capabilities planning uses and what this
/// `machine` keeps; no vault is needed, no Source is consulted, and no credential is shown.
pub fn list_sources_on_machine(
    machine: Machine<'_>,
) -> Result<Vec<SourceDescription>, CommandError> {
    Ok(describe_sources(
        &registry_refs(&machine_registry_of_sources()),
        machine,
    )?)
}

/// Enables or disables a registered Source on this `machine`, for every vault.
pub fn set_source_enabled_on_machine(
    machine: Machine<'_>,
    source_id: &str,
    enabled: bool,
) -> Result<Vec<SourceDescription>, CommandError> {
    Ok(set_source_enabled_use_case(
        machine,
        &registry_refs(&machine_registry_of_sources()),
        source_id,
        enabled,
    )?)
}

/// Stores on this `machine`, for every vault, the credential `field` a registered Source asks
/// for, or its API key when no field is named.
pub fn set_source_credential_on_machine(
    machine: Machine<'_>,
    source_id: &str,
    field: Option<&str>,
    key: &str,
) -> Result<Vec<SourceDescription>, CommandError> {
    Ok(set_source_credential_use_case(
        machine,
        &registry_refs(&machine_registry_of_sources()),
        source_id,
        field,
        &ApiKey::new(key)?,
    )?)
}

/// Forgets the credential `field` this `machine` stores for a registered Source, or all of its
/// credentials when no field is named.
pub fn clear_source_credential_on_machine(
    machine: Machine<'_>,
    source_id: &str,
    field: Option<&str>,
) -> Result<Vec<SourceDescription>, CommandError> {
    Ok(clear_source_credential_use_case(
        machine,
        &registry_refs(&machine_registry_of_sources()),
        source_id,
        field,
    )?)
}

/// Runs `action` with what this machine keeps: its settings and its OS credential store.
fn on_this_machine<R>(action: impl FnOnce(Machine<'_>) -> R) -> R {
    let settings = machine_settings();
    action(Machine {
        settings: settings.as_ref(),
        credentials: &KeyringCredentialStore::machine(),
    })
}

#[tauri::command]
fn list_sources() -> Result<Vec<SourceDescription>, CommandError> {
    on_this_machine(list_sources_on_machine)
}

#[tauri::command(rename_all = "snake_case")]
fn set_source_enabled(
    source_id: String,
    enabled: bool,
) -> Result<Vec<SourceDescription>, CommandError> {
    on_this_machine(|machine| set_source_enabled_on_machine(machine, &source_id, enabled))
}

/// The key reaches the backend over the local IPC and goes straight to the OS credential store.
#[tauri::command(rename_all = "snake_case")]
fn set_source_api_key(
    source_id: String,
    field: Option<String>,
    key: String,
) -> Result<Vec<SourceDescription>, CommandError> {
    on_this_machine(|machine| {
        set_source_credential_on_machine(machine, &source_id, field.as_deref(), &key)
    })
}

#[tauri::command(rename_all = "snake_case")]
fn clear_source_api_key(
    source_id: String,
    field: Option<String>,
) -> Result<Vec<SourceDescription>, CommandError> {
    on_this_machine(|machine| {
        clear_source_credential_on_machine(machine, &source_id, field.as_deref())
    })
}

#[tauri::command(rename_all = "snake_case")]
fn list_source_failures(
    session: State<'_, VaultSession>,
    latest: usize,
) -> Result<Vec<SourceFailureSummary>, CommandError> {
    load_source_failures(&session.root()?, latest)
}

#[tauri::command]
async fn import_reference_catalog(
    session: State<'_, VaultSession>,
    input: ReferenceImportInput,
) -> Result<ReferenceImportSummary, CommandError> {
    import_reference_catalog_in_vault_async(session.root()?, input).await
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
    registry: Vec<Box<dyn ConnectorPort>>,
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
async fn derive_packaging_models(
    session: State<'_, VaultSession>,
) -> Result<PackagingModelSummary, CommandError> {
    derive_packaging_models_in_vault_async(session.root()?).await
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
fn list_reference_review_items(
    session: State<'_, VaultSession>,
) -> Result<Vec<DescribedReferenceReviewItem>, CommandError> {
    load_reference_review_items(&session.root()?)
}

#[tauri::command(rename_all = "snake_case")]
fn link_reference_review_item(
    session: State<'_, VaultSession>,
    item_id: i64,
    release_edition_id: i64,
) -> Result<Vec<DescribedReferenceReviewItem>, CommandError> {
    link_reference_review_item_in_vault(&session.root()?, item_id, release_edition_id)
}

#[tauri::command(rename_all = "snake_case")]
fn keep_reference_review_item_apart(
    session: State<'_, VaultSession>,
    item_id: i64,
) -> Result<Vec<DescribedReferenceReviewItem>, CommandError> {
    keep_reference_review_item_apart_in_vault(&session.root()?, item_id)
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
    let bytes =
        load_review_preview_in_vault_async(session.root()?, review_item_id, machine_connectors()?)
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
    registry: Vec<Box<dyn ConnectorPort>>,
) -> Result<AcquisitionPlan, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        plan_acquisition_with_connectors(request, &registry_refs(&registry))
    })
    .await
    .map_err(|error| CommandError::worker_failed("acquisition planning", error))?
}

/// The kinds of reference catalog files the desktop imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceCatalogKind {
    NoIntro,
    Redump,
    MameSoftwareList,
}

/// A reference catalog file to import into the opened vault.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReferenceImportInput {
    pub kind: ReferenceCatalogKind,
    pub file: String,
    pub max_games: usize,
    /// The MAME release a software list came with, which the list does not record itself.
    #[serde(default)]
    pub mame_version: Option<String>,
}

/// Imports up to `input.max_games` releases of a reference catalog file, as the CLI imports.
pub fn import_reference_catalog_in_vault(
    vault_root: &Path,
    input: ReferenceImportInput,
) -> Result<ReferenceImportSummary, CommandError> {
    let catalog = open_existing_catalog(vault_root)?;
    let source: Box<dyn ReferenceCatalogSourcePort> = match input.kind {
        ReferenceCatalogKind::NoIntro => Box::new(NoIntroReferenceCatalog::new()),
        ReferenceCatalogKind::Redump => Box::new(RedumpReferenceCatalog::new()),
        ReferenceCatalogKind::MameSoftwareList => Box::new(
            input
                .mame_version
                .as_deref()
                .map(MameSoftwareListCatalog::with_mame_version)
                .unwrap_or_default(),
        ),
    };
    Ok(import_reference_catalog_use_case(
        &catalog,
        source.as_ref(),
        ImportReferenceCatalogRequest {
            source_path: PathBuf::from(input.file),
            max_games: input.max_games,
        },
    )?)
}

/// Imports on a blocking worker, since reading a large catalog takes a while.
pub async fn import_reference_catalog_in_vault_async(
    vault_root: PathBuf,
    input: ReferenceImportInput,
) -> Result<ReferenceImportSummary, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        import_reference_catalog_in_vault(&vault_root, input)
    })
    .await
    .map_err(|error| CommandError::worker_failed("reference import", error))?
}

/// Starts a run on a blocking worker, since checking the plan may reach the Source.
pub async fn start_acquisition_run_in_vault_async(
    vault_root: PathBuf,
    request: AcquisitionRequestInput,
    registry: Vec<Box<dyn ConnectorPort>>,
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
        DownloadLimits::default(),
    )?;
    Ok(load_acquisition_run_use_case(&catalog, run_id)?)
}

pub async fn execute_acquisition_run_in_vault_async(
    vault_root: PathBuf,
    run_id: i64,
    registry: Vec<Box<dyn ConnectorPort>>,
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
    plan_acquisition_async(request, machine_connectors()?).await
}

#[tauri::command(rename_all = "snake_case")]
async fn start_acquisition_run(
    session: State<'_, VaultSession>,
    request: AcquisitionRequestInput,
) -> Result<AcquisitionRun, CommandError> {
    start_acquisition_run_in_vault_async(session.root()?, request, machine_connectors()?).await
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
        machine_connectors()?,
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
        // Opens in the system browser the sites the default capability names, such as RAWG's,
        // whose terms ask for a link back.
        .plugin(tauri_plugin_opener::init())
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
            derive_packaging_models,
            verify_vault,
            list_review_items,
            resolve_review_item,
            load_review_preview,
            list_reference_review_items,
            link_reference_review_item,
            keep_reference_review_item_apart,
            build_acquisition_request,
            plan_acquisition,
            start_acquisition_run,
            execute_acquisition_run,
            get_acquisition_run,
            list_acquisition_runs,
            pause_acquisition_run,
            resume_acquisition_run,
            cancel_acquisition_run,
            list_sources,
            set_source_enabled,
            set_source_api_key,
            clear_source_api_key,
            list_source_failures,
            import_reference_catalog
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Game Media Vault");
}
