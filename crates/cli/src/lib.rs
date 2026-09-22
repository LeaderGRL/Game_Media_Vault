use std::{ffi::OsString, path::PathBuf};

use clap::{Args, Parser, Subcommand};
use game_media_vault_application::{
    AcquisitionRequestInput, AcquisitionRequestValidationError, ApplicationError,
    ImportLocalBoxFrontRequest, PortError, build_acquisition_request, import_local_box_front,
    list_library,
};
use game_media_vault_domain::{
    AcquisitionLimits, AssetTypeSelector, GameSelection, QualityRequirements, RetentionPolicy,
    SourceSelection,
};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
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
    Acquire {
        #[arg(long = "source")]
        sources: Vec<String>,
        #[arg(long, conflicts_with = "sources")]
        auto_source: bool,
        #[arg(long = "platform")]
        platforms: Vec<String>,
        #[arg(long = "game")]
        games: Vec<String>,
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
    Library,
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
    min_bitrate_kbps: Option<u32>,
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
            || self.min_bitrate_kbps.is_some()
            || self.best_available;

        has_requirements.then_some(QualityRequirements {
            min_width: self.min_width,
            min_height: self.min_height,
            min_longest_edge: self.min_longest_edge,
            min_pixel_count: self.min_pixel_count,
            original_only: self.original_only,
            accepted_mime_types: self.accepted_mime_types,
            min_bitrate_kbps: self.min_bitrate_kbps,
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

pub fn run<I, T>(args: I) -> Result<String, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;

    match cli.command {
        Command::Acquire {
            sources,
            auto_source,
            platforms,
            games,
            regions,
            languages,
            asset_types,
            quality,
            retention,
            limits,
        } => {
            let request = build_acquisition_request(AcquisitionRequestInput {
                sources: if auto_source {
                    SourceSelection::Auto
                } else {
                    SourceSelection::Explicit(sources)
                },
                platforms,
                games: if games.is_empty() {
                    GameSelection::All
                } else {
                    GameSelection::Explicit(games)
                },
                regions,
                languages,
                asset_types,
                quality: quality.into_domain(),
                retention,
                limits: limits.into(),
            })?;
            Ok(serde_json::to_string_pretty(&request)?)
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
        Command::Library => {
            let catalog = SqliteCatalog::open_existing(cli.vault.join("catalog.sqlite3"))?;
            Ok(serde_json::to_string_pretty(&list_library(&catalog)?)?)
        }
    }
}

fn parse_asset_type(value: &str) -> Result<AssetTypeSelector, String> {
    parse_domain_enum(value).map_err(|_| format!("unsupported asset type: {value}"))
}

fn parse_retention_policy(value: &str) -> Result<RetentionPolicy, String> {
    parse_domain_enum(value).map_err(|_| format!("unsupported retention policy: {value}"))
}

fn parse_domain_enum<T>(value: &str) -> Result<T, serde_json::Error>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::String(value.replace('-', "_")))
}
