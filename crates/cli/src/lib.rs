use std::{ffi::OsString, path::PathBuf};

use clap::{Parser, Subcommand};
use game_media_vault_application::{
    ApplicationError, ImportLocalBoxFrontRequest, PortError, import_local_box_front, list_library,
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
    ImportBoxFront {
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
    let catalog = SqliteCatalog::open(cli.vault.join("catalog.sqlite3"))?;
    let object_store = ContentAddressedStore::new(&cli.vault);

    match cli.command {
        Command::ImportBoxFront {
            game,
            platform,
            region,
            edition,
            file,
        } => {
            let imported = import_local_box_front(
                &catalog,
                &object_store,
                ImportLocalBoxFrontRequest {
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
        Command::Library => Ok(serde_json::to_string_pretty(&list_library(&catalog)?)?),
    }
}
