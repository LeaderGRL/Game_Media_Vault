use game_media_vault_application::{
    complete_acquisition_work, load_acquisition_run, next_acquisition_work, queue_acquisition_work,
    start_acquisition_run,
};
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

#[test]
fn completed_work_is_not_returned_after_restart() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();

    queue_acquisition_work(&catalog, run.id, "discover:first".to_owned()).unwrap();
    queue_acquisition_work(&catalog, run.id, "discover:second".to_owned()).unwrap();
    let first = next_acquisition_work(&catalog, run.id).unwrap().unwrap();
    assert_eq!(first.key, "discover:first");
    complete_acquisition_work(&catalog, run.id, &first.key).unwrap();
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let loaded = load_acquisition_run(&reopened, run.id).unwrap();
    let next = next_acquisition_work(&reopened, run.id).unwrap().unwrap();

    assert_eq!(loaded.queued_work, 1);
    assert_eq!(loaded.completed_work, 1);
    assert_eq!(next.key, "discover:second");
}
