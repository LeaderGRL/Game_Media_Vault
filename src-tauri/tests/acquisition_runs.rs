use std::io::{Cursor, Read};

use game_media_vault_application::{
    AcquisitionRequestInput, ConnectorPort, PortError,
    load_acquisition_run as load_acquisition_run_use_case,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRunStatus, AssetCandidate, AssetType,
    AssetTypeSelector, ConnectorCapabilities, GameSelection, RetentionPolicy, SourceId,
    SourceSelection,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::tempdir;

fn request_input() -> AcquisitionRequestInput {
    AcquisitionRequestInput {
        sources: SourceSelection::Explicit(vec!["screenscraper".to_owned()]),
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::All,
        regions: Vec::new(),
        languages: vec!["en".to_owned()],
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    }
}

struct FixtureConnector;

impl ConnectorPort for FixtureConnector {
    fn source_id(&self) -> &'static str {
        "libretro-thumbnails"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::BoxFront],
            direct_media_download: true,
        }
    }

    fn discover(&self, _request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        Ok(vec![AssetCandidate {
            game_title: "Super Mario Bros. (World)".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "World".to_owned(),
            edition_name: "Unspecified".to_owned(),
            asset_type: AssetType::BoxFront,
            source_id: SourceId::from("libretro-thumbnails"),
            source_url: "https://example.invalid/smb-box-front.png".to_owned(),
            original_filename: "Super Mario Bros. (World).png".to_owned(),
        }])
    }

    fn download(&self, _candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        Ok(Box::new(Cursor::new(b"tauri connector fixture".to_vec())))
    }
}

#[test]
fn tauri_exposes_the_shared_persisted_acquisition_run_state() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");

    let started =
        game_media_vault_tauri::start_acquisition_run_in_vault(&vault, request_input()).unwrap();
    assert_eq!(started.status, AcquisitionRunStatus::Running);

    let catalog = SqliteCatalog::open_existing(vault.join("catalog.sqlite3")).unwrap();
    let application_view = load_acquisition_run_use_case(&catalog, started.id).unwrap();
    assert_eq!(
        serde_json::to_value(&started).unwrap(),
        serde_json::to_value(&application_view).unwrap()
    );

    let loaded =
        game_media_vault_tauri::load_acquisition_run_from_vault(&vault, started.id).unwrap();
    assert_eq!(loaded, application_view);

    let listed = game_media_vault_tauri::list_acquisition_runs_from_vault(&vault).unwrap();
    assert_eq!(listed, vec![application_view.clone()]);

    let paused =
        game_media_vault_tauri::pause_acquisition_run_in_vault(&vault, started.id).unwrap();
    assert_eq!(paused.status, AcquisitionRunStatus::Paused);

    let resumed =
        game_media_vault_tauri::resume_acquisition_run_in_vault(&vault, started.id).unwrap();
    assert_eq!(resumed.status, AcquisitionRunStatus::Running);

    let cancelled =
        game_media_vault_tauri::cancel_acquisition_run_in_vault(&vault, started.id).unwrap();
    assert_eq!(cancelled.status, AcquisitionRunStatus::Cancelled);

    let reloaded =
        game_media_vault_tauri::load_acquisition_run_from_vault(&vault, started.id).unwrap();
    assert_eq!(reloaded, cancelled);
}

#[test]
fn tauri_adapter_can_execute_a_persisted_run_through_a_connector() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let request = AcquisitionRequestInput {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };
    let started = game_media_vault_tauri::start_acquisition_run_in_vault(&vault, request).unwrap();

    let completed = game_media_vault_tauri::execute_acquisition_run_in_vault_with_connector(
        &vault,
        started.id,
        &FixtureConnector,
    )
    .unwrap();

    assert_eq!(completed.status, AcquisitionRunStatus::Completed);
    let library = game_media_vault_tauri::load_library(&vault).unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(
        library[0].provenance[0].source_location,
        "https://example.invalid/smb-box-front.png"
    );
}
