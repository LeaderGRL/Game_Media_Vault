use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use clap::{Args, Parser, Subcommand};
use game_media_vault_application::{
    ACQUISITION_REQUEST_DOCUMENT_VERSION, AcquisitionRequestDocument, AcquisitionRequestInput,
    AcquisitionRequestValidationError, ApplicationError, ConnectorPort, DEFAULT_LIBRARY_PAGE_SIZE,
    ErrorKind, ImportLocalBoxFrontRequest, ImportReferenceCatalogRequest, LibraryQuery,
    LibraryStatus, PortError, ReferenceCatalogSourcePort, RepairActions, RepairSummary,
    VaultReport, acquire_run_with_connectors, build_acquisition_request, cancel_acquisition_run,
    derive_assets, draft_from_document, export_acquisition_request, import_local_box_front,
    import_reference_catalog, list_acquisition_runs, list_library, list_review_items,
    load_acquisition_run, pause_acquisition_run, plan_acquisition, repair_vault,
    resolve_review_item, resume_acquisition_run, search_library,
    start_acquisition_run_with_connectors, verify_vault,
};
use game_media_vault_connectors::{
    LibretroThumbnailsConnector, NoIntroReferenceCatalog, RedumpReferenceCatalog,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRun, AssetTypeSelector, DerivationRecipe,
    GameSelection, MatchingPolicy, PlatformBoundGameSelector, QualityRequirements, RetentionPolicy,
    ReviewDecision, SourceSelection,
};
use game_media_vault_infrastructure::{ContentAddressedStore, ImageTransformer, SqliteCatalog};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("{0}")]
    Parse(#[from] clap::Error),
    #[error("{0}")]
    Application(#[from] ApplicationError),
    #[error("{0}")]
    Port(#[from] PortError),
    #[error("{0}")]
    Serialization(#[from] serde_json::Error),
    #[error("{0}")]
    Validation(#[from] AcquisitionRequestValidationError),
    /// A request document that cannot be read or does not parse.
    #[error("{0}")]
    InvalidDocument(String),
}

impl CliError {
    /// Process exit code for this error: 2 invalid request, 3 not found, 4 conflict,
    /// 5 unsupported, 6 source failure, 1 anything else.
    pub fn exit_code(&self) -> i32 {
        let kind = match self {
            Self::Parse(error) => return error.exit_code(),
            Self::Validation(_) | Self::InvalidDocument(_) => ErrorKind::InvalidRequest,
            Self::Application(error) => error.kind(),
            Self::Port(_) | Self::Serialization(_) => ErrorKind::External,
        };
        match kind {
            ErrorKind::InvalidRequest => 2,
            ErrorKind::NotFound => 3,
            ErrorKind::Conflict => 4,
            ErrorKind::Unsupported => 5,
            ErrorKind::SourceFailure => 6,
            ErrorKind::External => 1,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "game-media-vault")]
#[command(about = "Acquire and browse video-game media assets")]
struct Cli {
    #[arg(long, global = true, default_value = ".game-media-vault")]
    vault: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Acquire(Box<AcquireArgs>),
    /// Explains which Sources an `acquire` command line would contact and what each acquires,
    /// without starting a run.
    Plan(Box<AcquireArgs>),
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
    Review {
        #[command(subcommand)]
        command: ReviewCommand,
    },
    ImportBoxFront {
        #[arg(long)]
        game_id: Option<i64>,
        #[arg(long)]
        game: String,
        #[arg(long)]
        platform: String,
        #[arg(long)]
        region: String,
        #[arg(long)]
        edition: String,
        #[arg(long)]
        file: PathBuf,
    },
    ImportNoIntro {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        max_games: usize,
    },
    /// Records the releases of a Redump datafile as reference data.
    ImportRedump {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        max_games: usize,
    },
    /// Renders the thumbnail of every retained original that has none yet.
    DeriveThumbnails {
        /// Longest edge of the thumbnails, in pixels.
        #[arg(long, default_value_t = 256)]
        max_edge: u32,
    },
    /// Compares the catalog with the stored bytes and reports every disagreement, repairing
    /// nothing.
    Verify,
    /// Applies the named repairs to what verification finds, then verifies again. Run it only
    /// while no other process uses the vault: interrupted and running stores look alike.
    Repair(RepairArgs),
    Library,
    /// Searches the Library and prints one page of matching releases.
    Search(SearchArgs),
}

#[derive(Debug, Args)]
struct SearchArgs {
    /// Text the game title contains, ignoring case.
    #[arg(long)]
    text: Option<String>,
    #[arg(long = "platform")]
    platforms: Vec<String>,
    #[arg(long = "region")]
    regions: Vec<String>,
    #[arg(long = "source")]
    sources: Vec<String>,
    #[arg(long = "asset-type", value_parser = parse_asset_type)]
    asset_types: Vec<AssetTypeSelector>,
    /// complete, partial or needs-review.
    #[arg(long = "status", value_parser = parse_library_status)]
    statuses: Vec<LibraryStatus>,
    /// The Release Edition the previous page ended with.
    #[arg(long)]
    after: Option<i64>,
    /// The `as_of` of the first page, so later pages keep its results.
    #[arg(long)]
    as_of: Option<i64>,
    /// Releases per page.
    #[arg(long, default_value_t = DEFAULT_LIBRARY_PAGE_SIZE)]
    limit: usize,
}

impl SearchArgs {
    fn into_query(self) -> LibraryQuery {
        LibraryQuery {
            text: self.text,
            platforms: self.platforms,
            regions: self.regions,
            sources: self.sources,
            asset_types: self.asset_types,
            statuses: self.statuses,
            after: self.after,
            as_of: self.as_of,
            limit: self.limit,
        }
    }
}

#[derive(Debug, Args)]
struct AcquireArgs {
    #[arg(long = "source")]
    sources: Vec<String>,
    #[arg(long, conflicts_with = "sources")]
    auto_source: bool,
    #[arg(long = "platform")]
    platforms: Vec<String>,
    #[arg(long = "game", conflicts_with_all = ["platform_games", "query_results"])]
    games: Vec<String>,
    #[arg(
        long = "platform-game",
        value_name = "PLATFORM=GAME",
        value_parser = parse_platform_bound_game,
        conflicts_with_all = ["games", "query_results"]
    )]
    platform_games: Vec<PlatformBoundGameSelector>,
    #[arg(
        long = "query-result",
        value_name = "PLATFORM=GAME",
        value_parser = parse_platform_bound_game,
        conflicts_with_all = ["games", "platform_games"]
    )]
    query_results: Vec<PlatformBoundGameSelector>,
    #[arg(long = "region")]
    regions: Vec<String>,
    #[arg(long = "language")]
    languages: Vec<String>,
    #[arg(long = "asset-type", value_parser = parse_asset_type)]
    asset_types: Vec<AssetTypeSelector>,
    #[command(flatten)]
    quality: QualityArgs,
    #[arg(long, default_value = "keep-everything", value_parser = parse_retention_policy)]
    retention: RetentionPolicy,
    #[command(flatten)]
    limits: LimitArgs,
}

impl AcquireArgs {
    fn into_input(self) -> AcquisitionRequestInput {
        AcquisitionRequestInput {
            sources: if self.auto_source {
                SourceSelection::Auto
            } else {
                SourceSelection::Explicit(self.sources)
            },
            platforms: self.platforms,
            games: if !self.platform_games.is_empty() {
                GameSelection::PlatformBound(self.platform_games)
            } else if !self.query_results.is_empty() {
                GameSelection::QueryResult(self.query_results)
            } else if !self.games.is_empty() {
                GameSelection::Explicit(self.games)
            } else {
                GameSelection::All
            },
            regions: self.regions,
            languages: self.languages,
            asset_types: self.asset_types,
            quality: self.quality.into_domain(),
            retention: self.retention,
            limits: self.limits.into(),
        }
    }
}

#[derive(Debug, Subcommand)]
enum RunCommand {
    List,
    /// Prints the request of a run as a portable document, without run state or secrets.
    Export {
        id: i64,
    },
    /// Starts a run from a request document, as `acquire` starts one from arguments.
    Start {
        request_file: PathBuf,
    },
    Show {
        id: i64,
    },
    Execute {
        id: i64,
        #[arg(long, default_value_t = 80)]
        match_high_threshold: u8,
        #[arg(long, default_value_t = 50)]
        match_medium_threshold: u8,
    },
    Pause {
        id: i64,
    },
    Resume {
        id: i64,
    },
    Cancel {
        id: i64,
    },
}

#[derive(Debug, Subcommand)]
enum ReviewCommand {
    List,
    Accept {
        id: i64,
        #[arg(long)]
        release_edition_id: i64,
    },
    Reject {
        id: i64,
    },
    Defer {
        id: i64,
    },
}

#[derive(Debug, Args)]
struct QualityArgs {
    #[arg(long)]
    min_width: Option<u32>,
    #[arg(long)]
    min_height: Option<u32>,
    #[arg(long)]
    min_longest_edge: Option<u32>,
    #[arg(long)]
    min_pixel_count: Option<u64>,
    #[arg(long)]
    original_only: bool,
    #[arg(long = "mime-type")]
    accepted_mime_types: Vec<String>,
    #[arg(long)]
    max_compression_ratio: Option<u32>,
    #[arg(long)]
    min_bitrate_kbps: Option<u32>,
    #[arg(long)]
    preferred_scan_type: Option<String>,
    #[arg(long = "preferred-source-priority")]
    preferred_source_priority: Vec<String>,
    #[arg(long)]
    best_available: bool,
}

impl QualityArgs {
    fn into_domain(self) -> Option<QualityRequirements> {
        let has_requirements = self.min_width.is_some()
            || self.min_height.is_some()
            || self.min_longest_edge.is_some()
            || self.min_pixel_count.is_some()
            || self.original_only
            || !self.accepted_mime_types.is_empty()
            || self.max_compression_ratio.is_some()
            || self.min_bitrate_kbps.is_some()
            || self.preferred_scan_type.is_some()
            || !self.preferred_source_priority.is_empty()
            || self.best_available;

        has_requirements.then_some(QualityRequirements {
            min_width: self.min_width,
            min_height: self.min_height,
            min_longest_edge: self.min_longest_edge,
            min_pixel_count: self.min_pixel_count,
            original_only: self.original_only,
            accepted_mime_types: self.accepted_mime_types,
            max_compression_ratio: self.max_compression_ratio,
            min_bitrate_kbps: self.min_bitrate_kbps,
            preferred_scan_type: self.preferred_scan_type,
            preferred_source_priority: self.preferred_source_priority,
            best_available: self.best_available,
        })
    }
}

#[derive(Debug, Args)]
struct LimitArgs {
    #[arg(long)]
    max_games: Option<u32>,
    #[arg(long)]
    max_downloads: Option<u32>,
    #[arg(long)]
    max_concurrent_downloads: Option<u16>,
    #[arg(long)]
    max_bytes: Option<u64>,
}

impl From<LimitArgs> for AcquisitionLimits {
    fn from(value: LimitArgs) -> Self {
        Self {
            max_games: value.max_games,
            max_downloads: value.max_downloads,
            max_concurrent_downloads: value.max_concurrent_downloads,
            max_bytes: value.max_bytes,
        }
    }
}

/// Runs a command line, executing and checking acquisition plans with Libretro Thumbnails.
pub fn run<I, T>(args: I) -> Result<String, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    run_with_connector(args, &LibretroThumbnailsConnector::new())
}

/// Builds and validates the Acquisition Request of an `acquire` command line without starting
/// a run, whichever connector could execute it.
pub fn acquisition_request_from_args<I, T>(args: I) -> Result<AcquisitionRequest, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match Cli::try_parse_from(args)?.command {
        Command::Acquire(acquire) => Ok(build_acquisition_request(acquire.into_input())?),
        _ => Err(CliError::Parse(clap::Error::raw(
            clap::error::ErrorKind::InvalidSubcommand,
            "expected an acquire command line",
        ))),
    }
}

/// Runs a command line with `connector`, the connector that checks the plan of a started run
/// and executes runs.
pub fn run_with_connector<I, T>(args: I, connector: &dyn ConnectorPort) -> Result<String, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;

    match cli.command {
        Command::Acquire(acquire) => {
            let catalog = SqliteCatalog::open(cli.vault.join("catalog.sqlite3"))?;
            // A plan the connector cannot execute would fail every execution, so no run starts.
            let run =
                start_acquisition_run_with_connectors(&catalog, acquire.into_input(), &[connector])
                    .map_err(map_start_run_error)?;
            Ok(serde_json::to_string_pretty(&run)?)
        }
        Command::Plan(acquire) => {
            let request = build_acquisition_request(acquire.into_input())?;
            let plan = plan_acquisition(&request, &[connector]).map_err(map_start_run_error)?;
            Ok(serde_json::to_string_pretty(&plan)?)
        }
        Command::Run { command } => match command {
            RunCommand::Start { request_file } => {
                let draft = draft_from_document(read_request_document(&request_file)?)?;
                let catalog = SqliteCatalog::open(cli.vault.join("catalog.sqlite3"))?;
                let run = start_acquisition_run_with_connectors(&catalog, draft, &[connector])
                    .map_err(map_start_run_error)?;
                Ok(serde_json::to_string_pretty(&run)?)
            }
            RunCommand::Execute {
                id,
                match_high_threshold,
                match_medium_threshold,
            } => Ok(serde_json::to_string_pretty(
                &execute_acquisition_run_in_vault_with_connector(
                    &cli.vault,
                    id,
                    connector,
                    MatchingPolicy {
                        high_confidence_threshold: match_high_threshold,
                        medium_confidence_threshold: match_medium_threshold,
                    },
                )?,
            )?),
            other => {
                let catalog = SqliteCatalog::open_existing(cli.vault.join("catalog.sqlite3"))?;
                match other {
                    RunCommand::List => Ok(serde_json::to_string_pretty(&list_acquisition_runs(
                        &catalog,
                    )?)?),
                    RunCommand::Show { id } => Ok(serde_json::to_string_pretty(
                        &load_acquisition_run(&catalog, id)?,
                    )?),
                    RunCommand::Pause { id } => Ok(serde_json::to_string_pretty(
                        &pause_acquisition_run(&catalog, id)?,
                    )?),
                    RunCommand::Resume { id } => Ok(serde_json::to_string_pretty(
                        &resume_acquisition_run(&catalog, id)?,
                    )?),
                    RunCommand::Cancel { id } => Ok(serde_json::to_string_pretty(
                        &cancel_acquisition_run(&catalog, id)?,
                    )?),
                    RunCommand::Export { id } => Ok(serde_json::to_string_pretty(
                        &export_acquisition_request(&catalog, id)?,
                    )?),
                    RunCommand::Execute { .. } | RunCommand::Start { .. } => unreachable!(),
                }
            }
        },
        Command::Review { command } => {
            let catalog = SqliteCatalog::open_existing(cli.vault.join("catalog.sqlite3"))?;
            match command {
                ReviewCommand::List => {
                    Ok(serde_json::to_string_pretty(&list_review_items(&catalog)?)?)
                }
                ReviewCommand::Accept {
                    id,
                    release_edition_id,
                } => Ok(serde_json::to_string_pretty(&resolve_review_item(
                    &catalog,
                    id,
                    ReviewDecision::Accept { release_edition_id },
                )?)?),
                ReviewCommand::Reject { id } => Ok(serde_json::to_string_pretty(
                    &resolve_review_item(&catalog, id, ReviewDecision::Reject)?,
                )?),
                ReviewCommand::Defer { id } => Ok(serde_json::to_string_pretty(
                    &resolve_review_item(&catalog, id, ReviewDecision::Defer)?,
                )?),
            }
        }
        Command::ImportBoxFront {
            game_id,
            game,
            platform,
            region,
            edition,
            file,
        } => {
            let catalog = SqliteCatalog::open(cli.vault.join("catalog.sqlite3"))?;
            let object_store = ContentAddressedStore::new(&cli.vault);
            let imported = import_local_box_front(
                &catalog,
                &object_store,
                ImportLocalBoxFrontRequest {
                    existing_game_id: game_id,
                    game_title: game,
                    platform,
                    region,
                    edition_name: edition,
                    source_path: file,
                },
            )?;
            Ok(format!(
                "Imported Box Front as asset #{} ({}, {} bytes)",
                imported.asset_id, imported.object_hash, imported.byte_len
            ))
        }
        Command::Verify => {
            let catalog = SqliteCatalog::open_existing(cli.vault.join("catalog.sqlite3"))?;
            let report = verify_vault(&catalog, &ContentAddressedStore::new(&cli.vault))?;
            Ok(serde_json::to_string_pretty(&VerifyOutput {
                healthy: report.is_healthy(),
                report,
            })?)
        }
        Command::Repair(args) => {
            let catalog = SqliteCatalog::open_existing(cli.vault.join("catalog.sqlite3"))?;
            let summary = repair_vault(
                &catalog,
                &ContentAddressedStore::new(&cli.vault),
                RepairActions {
                    remove_interrupted_staging: args.remove_interrupted_staging,
                    remove_orphaned_derived: args.remove_orphaned_derived,
                    reset_damaged_derived: args.reset_damaged_derived,
                    collect_unreferenced_originals: args.collect_unreferenced_originals,
                },
            )?;
            Ok(serde_json::to_string_pretty(&RepairOutput {
                healthy: summary.remaining.is_healthy(),
                summary,
            })?)
        }
        Command::DeriveThumbnails { max_edge } => {
            let catalog = SqliteCatalog::open_existing(cli.vault.join("catalog.sqlite3"))?;
            let store = ContentAddressedStore::new(&cli.vault);
            Ok(serde_json::to_string_pretty(&derive_assets(
                &catalog,
                &store,
                &ImageTransformer::new(),
                &DerivationRecipe::Thumbnail { max_edge },
            )?)?)
        }
        Command::ImportNoIntro { file, max_games } => {
            import_reference_datafile(&cli.vault, &NoIntroReferenceCatalog::new(), file, max_games)
        }
        Command::ImportRedump { file, max_games } => {
            import_reference_datafile(&cli.vault, &RedumpReferenceCatalog::new(), file, max_games)
        }
        Command::Library => {
            let catalog = SqliteCatalog::open_existing(cli.vault.join("catalog.sqlite3"))?;
            Ok(serde_json::to_string_pretty(&list_library(&catalog)?)?)
        }
        Command::Search(search) => {
            let catalog = SqliteCatalog::open_existing(cli.vault.join("catalog.sqlite3"))?;
            Ok(serde_json::to_string_pretty(&search_library(
                &catalog,
                &catalog,
                &search.into_query(),
            )?)?)
        }
    }
}

pub fn execute_acquisition_run_in_vault_with_connector(
    vault_root: &Path,
    run_id: i64,
    connector: &dyn ConnectorPort,
    matching_policy: MatchingPolicy,
) -> Result<AcquisitionRun, CliError> {
    let catalog = SqliteCatalog::open_existing(vault_root.join("catalog.sqlite3"))?;
    let object_store = ContentAddressedStore::new(vault_root);
    acquire_run_with_connectors(
        &catalog,
        &catalog,
        &catalog,
        &object_store,
        &[connector],
        run_id,
        matching_policy,
    )?;
    Ok(load_acquisition_run(&catalog, run_id)?)
}

fn map_start_run_error(error: ApplicationError) -> CliError {
    match error {
        ApplicationError::Validation(error) => CliError::Validation(error),
        other => CliError::Application(other),
    }
}

fn parse_asset_type(value: &str) -> Result<AssetTypeSelector, String> {
    parse_domain_enum(value).map_err(|_| format!("unsupported asset type: {value}"))
}

fn parse_library_status(value: &str) -> Result<LibraryStatus, String> {
    parse_domain_enum(value).map_err(|_| format!("unsupported library status: {value}"))
}

fn parse_retention_policy(value: &str) -> Result<RetentionPolicy, String> {
    parse_domain_enum(value).map_err(|_| format!("unsupported retention policy: {value}"))
}

fn parse_platform_bound_game(value: &str) -> Result<PlatformBoundGameSelector, String> {
    let (platform, game) = value
        .split_once('=')
        .ok_or_else(|| "expected PLATFORM=GAME".to_owned())?;

    Ok(PlatformBoundGameSelector {
        game: game.to_owned(),
        platform: platform.to_owned(),
    })
}

fn parse_domain_enum<T>(value: &str) -> Result<T, serde_json::Error>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::String(value.replace('-', "_")))
}

/// Records up to `max_games` releases of a reference datafile and prints how many it recorded.
fn import_reference_datafile(
    vault: &Path,
    source: &dyn ReferenceCatalogSourcePort,
    file: PathBuf,
    max_games: usize,
) -> Result<String, CliError> {
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3"))?;
    let summary = import_reference_catalog(
        &catalog,
        source,
        ImportReferenceCatalogRequest {
            source_path: file,
            max_games,
        },
    )?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "imported_releases": summary.imported_releases
    }))?)
}

/// Reads a request document, checking its format version before the request it holds, so a
/// document of another version is refused as such rather than as malformed.
fn read_request_document(path: &Path) -> Result<AcquisitionRequestDocument, CliError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        CliError::InvalidDocument(format!("cannot read {}: {error}", path.display()))
    })?;
    #[derive(serde::Deserialize)]
    struct FormatProbe {
        format_version: u32,
    }
    let invalid = |error: serde_json::Error| {
        CliError::InvalidDocument(format!(
            "invalid request document {}: {error}",
            path.display()
        ))
    };
    let probe: FormatProbe = serde_json::from_str(&text).map_err(invalid)?;
    if probe.format_version != ACQUISITION_REQUEST_DOCUMENT_VERSION {
        return Err(ApplicationError::UnsupportedDocumentVersion {
            found: probe.format_version,
            supported: ACQUISITION_REQUEST_DOCUMENT_VERSION,
        }
        .into());
    }
    serde_json::from_str(&text).map_err(invalid)
}

/// A verification report, said healthy when it found nothing.
#[derive(serde::Serialize)]
struct VerifyOutput {
    healthy: bool,
    #[serde(flatten)]
    report: VaultReport,
}

#[derive(Debug, Args)]
struct RepairArgs {
    /// Deletes the staging files interrupted stores left.
    #[arg(long)]
    remove_interrupted_staging: bool,
    /// Forgets Derived Assets of unreferenced originals and deletes derived files no remaining
    /// record lists.
    #[arg(long)]
    remove_orphaned_derived: bool,
    /// Forgets missing and corrupt Derived Assets so they render again.
    #[arg(long)]
    reset_damaged_derived: bool,
    /// Deletes stored originals no retained Asset references.
    #[arg(long)]
    collect_unreferenced_originals: bool,
}

/// A repair summary, said healthy when verification finds nothing left.
#[derive(serde::Serialize)]
struct RepairOutput {
    healthy: bool,
    #[serde(flatten)]
    summary: RepairSummary,
}
