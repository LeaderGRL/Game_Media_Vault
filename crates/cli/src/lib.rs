use std::{ffi::OsString, path::PathBuf};

use clap::{Parser, Subcommand};
use game_media_vault_application::{
    AcquisitionRequestInput, AcquisitionRequestValidationError, ApplicationError,
    ImportLocalBoxFrontRequest, PortError, build_acquisition_request, import_local_box_front,
    list_library,
};
use game_media_vault_domain::{
    AcquisitionLimits, AssetTypeSelector, GameSelection, RetentionPolicy, SourceSelection,
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
        #[arg(long = "asset-type", value_parser = parse_asset_type)]
        asset_types: Vec<AssetTypeSelector>,
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
            asset_types,
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
                regions: Vec::new(),
                languages: Vec::new(),
                asset_types,
                quality: None,
                retention: RetentionPolicy::KeepEverything,
                limits: AcquisitionLimits::default(),
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
    match value {
        "box-front" | "box_front" => Ok(AssetTypeSelector::BoxFront),
        other => Err(format!("unsupported asset type: {other}")),
    }
}
