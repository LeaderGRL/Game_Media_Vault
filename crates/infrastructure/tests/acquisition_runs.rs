use std::fs;

use game_media_vault_application::{
    ApplicationError, CatalogPort, ImportLocalBoxFrontRequest, cancel_acquisition_run,
    complete_acquisition_run, complete_acquisition_work, import_local_box_front,
    load_acquisition_run, next_acquisition_work, pause_acquisition_run, queue_acquisition_work,
    resume_acquisition_run, start_acquisition_run,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRunStatus, AssetTypeSelector, GameSelection, RetentionPolicy,
    SourceSelection,
};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use rusqlite::Connection;
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
    let vault = temp.path().join("vault");
    let path = vault.join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let run = start_acquisition_run(&catalog, request()).unwrap();
    let source = temp.path().join("front.png");
    let source_bytes = b"accepted acquisition asset";
    fs::write(&source, source_bytes).unwrap();
    let accepted = import_local_box_front(
        &catalog,
        &store,
        ImportLocalBoxFrontRequest {
            existing_game_id: None,
            game_title: "Accepted Game".to_owned(),
            platform: "Windows".to_owned(),
            region: "Worldwide".to_owned(),
            edition_name: "Standard".to_owned(),
            source_path: source,
        },
    )
    .unwrap();

    let cancelled = cancel_acquisition_run(&catalog, run.id).unwrap();
    drop(catalog);
    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let library = reopened.list_library().unwrap();

    assert_eq!(cancelled.status, AcquisitionRunStatus::Cancelled);
    assert!(next_acquisition_work(&reopened, run.id).unwrap().is_none());
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].asset_id, accepted.asset_id);
    assert_eq!(library[0].object_hash, accepted.object_hash);
    assert_eq!(
        fs::read(store.object_path(&accepted.object_hash)).unwrap(),
        source_bytes
    );
}

#[test]
fn duplicate_work_keys_are_queued_only_once() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();

    queue_acquisition_work(&catalog, run.id, "download:cover".to_owned()).unwrap();
    queue_acquisition_work(&catalog, run.id, "download:cover".to_owned()).unwrap();

    let loaded = load_acquisition_run(&catalog, run.id).unwrap();
    let next = next_acquisition_work(&catalog, run.id).unwrap().unwrap();

    assert_eq!(loaded.queued_work, 1);
    assert_eq!(next.key, "download:cover");
}

#[test]
fn a_run_can_complete_only_after_its_persisted_queue_is_empty() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    queue_acquisition_work(&catalog, run.id, "download:cover".to_owned()).unwrap();

    let error = complete_acquisition_run(&catalog, run.id).unwrap_err();
    assert_eq!(error, ApplicationError::RunHasQueuedWork { queued_work: 1 });

    complete_acquisition_work(&catalog, run.id, "download:cover").unwrap();
    let completed = complete_acquisition_run(&catalog, run.id).unwrap();

    assert_eq!(completed.status, AcquisitionRunStatus::Completed);
    assert_eq!(completed.queued_work, 0);
    assert_eq!(completed.completed_work, 1);
}

#[test]
fn terminal_runs_reject_new_work() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();

    let cancelled_run = start_acquisition_run(&catalog, request()).unwrap();
    cancel_acquisition_run(&catalog, cancelled_run.id).unwrap();
    let cancelled_error =
        queue_acquisition_work(&catalog, cancelled_run.id, "late:cancelled".to_owned())
            .unwrap_err();
    assert_eq!(
        cancelled_error,
        ApplicationError::RunNotAcceptingWork {
            status: AcquisitionRunStatus::Cancelled,
        }
    );

    let completed_run = start_acquisition_run(&catalog, request()).unwrap();
    complete_acquisition_run(&catalog, completed_run.id).unwrap();
    let completed_error =
        queue_acquisition_work(&catalog, completed_run.id, "late:completed".to_owned())
            .unwrap_err();
    assert_eq!(
        completed_error,
        ApplicationError::RunNotAcceptingWork {
            status: AcquisitionRunStatus::Completed,
        }
    );
}

#[test]
fn opening_an_unrelated_sqlite_database_does_not_turn_it_into_a_vault() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("unrelated.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE unrelated_data (id INTEGER PRIMARY KEY);")
        .unwrap();
    drop(connection);

    let opened = SqliteCatalog::open_existing(&path);

    assert!(opened.is_err());
    let connection = Connection::open(&path).unwrap();
    let run_table_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'acquisition_runs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(run_table_count, 0);
}

#[test]
fn opening_a_pre_acquisition_run_catalog_migrates_it_in_place() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    drop(catalog);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP TABLE acquisition_run_work;
             DROP TABLE acquisition_runs;",
        )
        .unwrap();
    drop(connection);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let run = start_acquisition_run(&reopened, request()).unwrap();

    assert_eq!(run.status, AcquisitionRunStatus::Running);
}

#[test]
fn opening_an_unversioned_acquisition_run_catalog_adds_the_request_schema_version() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    drop(catalog);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP TABLE acquisition_run_work;
             DROP TABLE acquisition_runs;
             CREATE TABLE acquisition_runs (
                 id INTEGER PRIMARY KEY,
                 request_json TEXT NOT NULL,
                 status TEXT NOT NULL,
                 queued_work INTEGER NOT NULL CHECK(queued_work >= 0),
                 completed_work INTEGER NOT NULL CHECK(completed_work >= 0)
             );
             CREATE TABLE acquisition_run_work (
                 id INTEGER PRIMARY KEY,
                 run_id INTEGER NOT NULL REFERENCES acquisition_runs(id),
                 work_key TEXT NOT NULL,
                 completed INTEGER NOT NULL DEFAULT 0 CHECK(completed IN (0, 1)),
                 UNIQUE(run_id, work_key)
             );",
        )
        .unwrap();
    drop(connection);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let run = start_acquisition_run(&reopened, request()).unwrap();
    let connection = Connection::open(&path).unwrap();
    let schema_version: i64 = connection
        .query_row(
            "SELECT request_schema_version FROM acquisition_runs WHERE id = ?1",
            [run.id],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(schema_version, 1);
}

#[test]
fn persisted_acquisition_requests_have_an_explicit_schema_version() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();

    let connection = Connection::open(&path).unwrap();
    let schema_version: i64 = connection
        .query_row(
            "SELECT request_schema_version FROM acquisition_runs WHERE id = ?1",
            [run.id],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(schema_version, 1);
}

#[test]
fn unsupported_persisted_request_schema_versions_are_rejected() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE acquisition_runs SET request_schema_version = 2 WHERE id = ?1",
            [run.id],
        )
        .unwrap();
    drop(connection);

    let error = load_acquisition_run(&catalog, run.id).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unsupported acquisition request schema version: 2")
    );
}
