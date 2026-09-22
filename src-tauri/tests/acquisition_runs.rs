use game_media_vault_application::{
    AcquisitionRequestInput, load_acquisition_run as load_acquisition_run_use_case,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRunStatus, AssetTypeSelector, GameSelection, RetentionPolicy,
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
