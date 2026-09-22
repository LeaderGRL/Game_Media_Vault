use game_media_vault_application::{load_acquisition_run, start_acquisition_run};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRunStatus, AssetTypeSelector, GameSelection, RetentionPolicy,
    SourceSelection,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::tempdir;

fn request() -> game_media_vault_application::AcquisitionRequestInput {
    game_media_vault_application::AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::All,
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    }
}

#[test]
fn persists_an_acquisition_run_across_catalog_reopen() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();

    let started = start_acquisition_run(&catalog, request()).unwrap();
    assert_eq!(started.status, AcquisitionRunStatus::Running);
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let loaded = load_acquisition_run(&reopened, started.id).unwrap();

    assert_eq!(loaded, started);
}
