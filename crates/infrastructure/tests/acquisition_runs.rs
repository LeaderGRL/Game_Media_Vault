use game_media_vault_application::{
    CatalogPort, cancel_acquisition_run, complete_acquisition_work, load_acquisition_run,
    next_acquisition_work, pause_acquisition_run, queue_acquisition_work, resume_acquisition_run,
    start_acquisition_run,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRunStatus, AssetType, AssetTypeSelector, GameSelection,
    PersistAsset, RetentionPolicy, SourceKind, SourceSelection,
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

#[test]
fn pause_and_resume_preserve_queued_work_across_restart() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    queue_acquisition_work(&catalog, run.id, "download:cover".to_owned()).unwrap();

    let paused = pause_acquisition_run(&catalog, run.id).unwrap();
    assert_eq!(paused.status, AcquisitionRunStatus::Paused);
    assert!(next_acquisition_work(&catalog, run.id).unwrap().is_none());
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let resumed = resume_acquisition_run(&reopened, run.id).unwrap();
    let next = next_acquisition_work(&reopened, run.id).unwrap().unwrap();

    assert_eq!(resumed.status, AcquisitionRunStatus::Running);
    assert_eq!(resumed.queued_work, 1);
    assert_eq!(next.key, "download:cover");
}

#[test]
fn cancellation_preserves_assets_accepted_before_the_run_was_cancelled() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    let accepted = catalog
        .persist_asset(PersistAsset {
            existing_game_id: None,
            game_title: "Accepted Game".to_owned(),
            platform: "Windows".to_owned(),
            region: "Worldwide".to_owned(),
            edition_name: "Standard".to_owned(),
            asset_type: AssetType::BoxFront,
            object_hash: "accepted-object".to_owned(),
            byte_len: 42,
            original_filename: "front.png".to_owned(),
            source_kind: SourceKind::LocalImport,
            source_location: "accepted/front.png".to_owned(),
        })
        .unwrap();

    let cancelled = cancel_acquisition_run(&catalog, run.id).unwrap();
    let library = catalog.list_library().unwrap();

    assert_eq!(cancelled.status, AcquisitionRunStatus::Cancelled);
    assert!(next_acquisition_work(&catalog, run.id).unwrap().is_none());
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].asset_id, accepted.asset_id);
}
