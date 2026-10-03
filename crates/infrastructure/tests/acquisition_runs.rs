use std::fs;

use game_media_vault_application::{
    ApplicationError, CatalogPort, ImportLocalAssetRequest, ReviewRepositoryPort,
    RunRepositoryPort, cancel_acquisition_run, complete_acquisition_run, import_local_box_front,
    load_acquisition_run, pause_acquisition_run, resume_acquisition_run, start_acquisition_run,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate, AssetType,
    AssetTypeSelector, GameSelection, QualityShortfall, RetentionPolicy, SourceFailureStage,
    SourceId, SourceSelection,
};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use rusqlite::Connection;
use tempfile::tempdir;

const SOURCE_ID: &str = "fixture-provider";

fn work(key: &str) -> AcquisitionWorkItem {
    AcquisitionWorkItem {
        key: key.to_owned(),
        candidate: AssetCandidate {
            provider_candidate_id: Some(key.to_owned()),
            game_title: format!("Game for {key}"),
            platform: "Windows".to_owned(),
            region: "Worldwide".to_owned(),
            edition_name: "Standard".to_owned(),
            asset_type: AssetType::BoxFront,
            source_id: SourceId::from(SOURCE_ID),
            source_asset_label: None,
            source_url: format!("https://example.invalid/{key}.png"),
            original_filename: format!("{key}.png"),
        },
    }
}

fn queue(catalog: &SqliteCatalog, run_id: i64, key: &str) {
    catalog
        .record_discovery(run_id, SOURCE_ID, &[work(key)])
        .unwrap();
}

fn request() -> game_media_vault_application::AcquisitionRequestInput {
    game_media_vault_application::AcquisitionRequestInput {
        sources: SourceSelection::Explicit(vec![SOURCE_ID.to_owned()]),
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
    assert_eq!(started.planned_sources, [SOURCE_ID]);
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

    catalog
        .record_discovery(
            run.id,
            SOURCE_ID,
            &[work("discover:first"), work("discover:second")],
        )
        .unwrap();
    let first = catalog.next_queued_work(run.id, &[]).unwrap().unwrap();
    assert_eq!(first.key, "discover:first");
    catalog.complete_work(run.id, &first.key).unwrap();
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let loaded = load_acquisition_run(&reopened, run.id).unwrap();
    let next = reopened.next_queued_work(run.id, &[]).unwrap().unwrap();

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
    queue(&catalog, run.id, "download:cover");

    let paused = pause_acquisition_run(&catalog, run.id).unwrap();
    assert_eq!(paused.status, AcquisitionRunStatus::Paused);
    assert!(catalog.next_queued_work(run.id, &[]).unwrap().is_none());
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let resumed = resume_acquisition_run(&reopened, run.id).unwrap();
    let next = reopened.next_queued_work(run.id, &[]).unwrap().unwrap();

    assert_eq!(resumed.status, AcquisitionRunStatus::Running);
    assert_eq!(resumed.queued_work, 1);
    assert_eq!(next.key, "download:cover");
}

#[test]
fn repository_does_not_return_work_for_a_non_running_run() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();

    let paused_run = start_acquisition_run(&catalog, request()).unwrap();
    queue(&catalog, paused_run.id, "download:paused");
    pause_acquisition_run(&catalog, paused_run.id).unwrap();

    let cancelled_run = start_acquisition_run(&catalog, request()).unwrap();
    queue(&catalog, cancelled_run.id, "download:cancelled");
    cancel_acquisition_run(&catalog, cancelled_run.id).unwrap();

    assert!(
        catalog
            .next_queued_work(paused_run.id, &[])
            .unwrap()
            .is_none()
    );
    assert!(
        catalog
            .next_queued_work(cancelled_run.id, &[])
            .unwrap()
            .is_none()
    );
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
        ImportLocalAssetRequest {
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
    assert!(reopened.next_queued_work(run.id, &[]).unwrap().is_none());
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].assets.len(), 1);
    assert_eq!(library[0].assets[0].asset_id, accepted.asset_id);
    assert_eq!(library[0].assets[0].object_hash, accepted.object_hash);
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

    catalog
        .record_discovery(
            run.id,
            SOURCE_ID,
            &[work("download:cover"), work("download:cover")],
        )
        .unwrap();

    let loaded = load_acquisition_run(&catalog, run.id).unwrap();
    let next = catalog.next_queued_work(run.id, &[]).unwrap().unwrap();

    assert_eq!(loaded.queued_work, 1);
    assert_eq!(next.key, "download:cover");
}

#[test]
fn a_run_can_complete_only_after_its_persisted_queue_is_empty() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    queue(&catalog, run.id, "download:cover");

    let error = complete_acquisition_run(&catalog, run.id).unwrap_err();
    assert_eq!(error, ApplicationError::RunHasQueuedWork { queued_work: 1 });

    catalog.complete_work(run.id, "download:cover").unwrap();
    let completed = complete_acquisition_run(&catalog, run.id).unwrap();

    assert_eq!(completed.status, AcquisitionRunStatus::Completed);
    assert_eq!(completed.queued_work, 0);
    assert_eq!(completed.completed_work, 1);
}

#[test]
fn repository_cannot_complete_a_run_with_queued_work() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    queue(&catalog, run.id, "download:cover");

    let updated = catalog
        .compare_and_set_run_status(
            run.id,
            AcquisitionRunStatus::Running,
            AcquisitionRunStatus::Completed,
        )
        .unwrap();

    assert!(!updated);
    let loaded = load_acquisition_run(&catalog, run.id).unwrap();
    assert_eq!(loaded.status, AcquisitionRunStatus::Running);
    assert_eq!(loaded.queued_work, 1);
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
fn opening_or_creating_an_unrelated_sqlite_database_does_not_turn_it_into_a_vault() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("unrelated-create.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE unrelated_data (id INTEGER PRIMARY KEY);")
        .unwrap();
    drop(connection);

    let opened = SqliteCatalog::open(&path);

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
fn opening_tables_that_only_look_like_a_vault_does_not_mutate_them() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("lookalike.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE games (id INTEGER PRIMARY KEY, payload TEXT);
             CREATE TABLE release_editions (id INTEGER PRIMARY KEY, payload TEXT);
             CREATE TABLE assets (id INTEGER PRIMARY KEY, payload TEXT);
             CREATE TABLE asset_provenance (id INTEGER PRIMARY KEY, payload TEXT);",
        )
        .unwrap();
    drop(connection);

    let opened = SqliteCatalog::open_existing(&path);

    assert!(opened.is_err());
    let connection = Connection::open(&path).unwrap();
    let acquisition_tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name IN ('acquisition_runs', 'acquisition_run_work')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(acquisition_tables, 0);
}

#[test]
fn opening_a_malformed_legacy_catalog_does_not_fill_in_missing_tables() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("malformed-legacy.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE games (
                 id INTEGER PRIMARY KEY,
                 title TEXT NOT NULL,
                 normalized_title TEXT NOT NULL,
                 unrelated TEXT
             );
             CREATE TABLE release_editions (
                 id INTEGER PRIMARY KEY,
                 game_id INTEGER NOT NULL REFERENCES games(id),
                 platform TEXT NOT NULL,
                 normalized_platform TEXT NOT NULL,
                 region TEXT NOT NULL,
                 normalized_region TEXT NOT NULL,
                 edition_name TEXT NOT NULL,
                 normalized_edition_name TEXT NOT NULL,
                 UNIQUE(game_id, normalized_platform, normalized_region, normalized_edition_name)
             );",
        )
        .unwrap();
    drop(connection);

    let opened = SqliteCatalog::open_existing(&path);

    assert!(opened.is_err());
    let connection = Connection::open(&path).unwrap();
    let added_tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table'
               AND name IN ('assets', 'asset_provenance', 'acquisition_runs', 'acquisition_run_work')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(added_tables, 0);
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

#[test]
fn recorded_discovery_persists_work_candidates_and_the_discovered_source() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    assert!(!catalog.has_discovered(run.id, SOURCE_ID).unwrap());

    catalog
        .record_discovery(run.id, SOURCE_ID, &[work("first"), work("second")])
        .unwrap();
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    assert!(reopened.has_discovered(run.id, SOURCE_ID).unwrap());
    assert!(!reopened.has_discovered(run.id, "another-source").unwrap());
    assert_eq!(
        reopened.next_queued_work(run.id, &[]).unwrap(),
        Some(work("first"))
    );
    assert_eq!(
        load_acquisition_run(&reopened, run.id).unwrap().queued_work,
        2
    );
}

#[test]
fn a_later_discovery_of_an_already_discovered_source_is_ignored() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    catalog
        .record_discovery(run.id, SOURCE_ID, &[work("first")])
        .unwrap();

    // A concurrent execution that also discovered the source must not add its snapshot.
    catalog
        .record_discovery(run.id, SOURCE_ID, &[work("second")])
        .unwrap();

    assert_eq!(
        load_acquisition_run(&catalog, run.id).unwrap().queued_work,
        1
    );
    assert_eq!(
        catalog.next_queued_work(run.id, &[]).unwrap(),
        Some(work("first"))
    );
}

#[test]
fn terminal_runs_reject_discovered_work() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let cancelled_run = start_acquisition_run(&catalog, request()).unwrap();
    cancel_acquisition_run(&catalog, cancelled_run.id).unwrap();
    let completed_run = start_acquisition_run(&catalog, request()).unwrap();
    complete_acquisition_run(&catalog, completed_run.id).unwrap();

    for run_id in [cancelled_run.id, completed_run.id] {
        let recorded = catalog
            .record_discovery(run_id, SOURCE_ID, &[work("late")])
            .unwrap();

        assert!(!recorded);
        assert!(!catalog.has_discovered(run_id, SOURCE_ID).unwrap());
        assert_eq!(
            load_acquisition_run(&catalog, run_id).unwrap().queued_work,
            0
        );
    }
}

#[test]
fn completing_unknown_work_is_an_error() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();

    let error = catalog.complete_work(run.id, "missing").unwrap_err();

    assert!(error.to_string().contains("has no work"), "{error}");
}

#[test]
fn work_below_quality_is_completed_and_counted_apart() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    catalog
        .record_discovery(run.id, SOURCE_ID, &[work("small"), work("large")])
        .unwrap();

    catalog
        .complete_candidate_below_quality(
            run.id,
            "small",
            73,
            &[QualityShortfall::MinWidth {
                minimum: 1000,
                actual: Some(640),
            }],
        )
        .unwrap();
    catalog.complete_work(run.id, "large").unwrap();
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let loaded = load_acquisition_run(&reopened, run.id).unwrap();
    assert_eq!((loaded.completed_work, loaded.below_quality_work), (2, 1));
}

#[test]
fn queued_work_of_skipped_sources_waits_for_a_later_execution() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    let mut other = work("other-work");
    other.candidate.source_id = SourceId::from("other-source");
    catalog
        .record_discovery(run.id, SOURCE_ID, &[work("first-work")])
        .unwrap();
    catalog
        .record_discovery(run.id, "other-source", &[other])
        .unwrap();

    let skipping = catalog
        .next_queued_work(run.id, &[SOURCE_ID.to_owned()])
        .unwrap()
        .unwrap();

    assert_eq!(skipping.key, "other-work");
    assert_eq!(
        catalog.next_queued_work(run.id, &[]).unwrap().unwrap().key,
        "first-work"
    );
}

#[test]
fn unavailable_work_is_completed_and_counted_across_reopen() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    catalog
        .record_discovery(run.id, SOURCE_ID, &[work("gone"), work("kept")])
        .unwrap();

    catalog
        .complete_unavailable_work(run.id, "gone", "download returned HTTP 404")
        .unwrap();
    drop(catalog);
    let reopened = SqliteCatalog::open_existing(&path).unwrap();

    let run = load_acquisition_run(&reopened, run.id).unwrap();
    assert_eq!(
        (run.queued_work, run.completed_work, run.unavailable_work),
        (1, 1, 1)
    );
    assert_eq!(
        reopened.next_queued_work(run.id, &[]).unwrap().unwrap().key,
        "kept"
    );
}

#[test]
fn source_failures_are_kept_in_recording_order_across_reopen() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();

    catalog
        .record_source_failure(
            run.id,
            SOURCE_ID,
            SourceFailureStage::Discovery,
            "timed out",
        )
        .unwrap();
    catalog
        .record_source_failure(
            run.id,
            "other-source",
            SourceFailureStage::Download,
            "HTTP 503",
        )
        .unwrap();
    drop(catalog);

    let failures = SqliteCatalog::open_existing(&path)
        .unwrap()
        .source_failures()
        .unwrap();
    let described: Vec<(&str, i64, SourceFailureStage, &str)> = failures
        .iter()
        .map(|failure| {
            (
                failure.source_id.as_str(),
                failure.run_id,
                failure.stage,
                failure.message.as_str(),
            )
        })
        .collect();
    assert_eq!(
        described,
        [
            (
                SOURCE_ID,
                run.id,
                SourceFailureStage::Discovery,
                "timed out"
            ),
            (
                "other-source",
                run.id,
                SourceFailureStage::Download,
                "HTTP 503"
            ),
        ]
    );
    assert!(failures[0].sequence < failures[1].sequence);
    // Recorded at the time of recording, in seconds since the Unix epoch.
    assert!(
        failures
            .iter()
            .all(|failure| failure.recorded_at > 1_600_000_000)
    );
}

#[test]
fn queued_work_lists_the_oldest_items_of_each_source_while_the_run_runs() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    catalog
        .record_discovery(run.id, SOURCE_ID, &[work("a1"), work("a2"), work("a3")])
        .unwrap();
    let other = |key: &str| {
        let mut item = work(key);
        item.candidate.source_id = SourceId::from("other-source");
        item
    };
    catalog
        .record_discovery(run.id, "other-source", &[other("b1"), other("b2")])
        .unwrap();
    let keys = |skipped: &[String], per_source: usize| -> Vec<String> {
        catalog
            .queued_work(run.id, skipped, per_source)
            .unwrap()
            .into_iter()
            .map(|item| item.key)
            .collect()
    };

    // Every Source's oldest item comes before any second one.
    assert_eq!(keys(&[], 2), ["a1", "b1", "a2", "b2"]);
    assert_eq!(keys(&[SOURCE_ID.to_owned()], 2), ["b1", "b2"]);
    catalog.complete_work(run.id, "a1").unwrap();
    assert_eq!(keys(&[], 1), ["a2", "b1"]);
    pause_acquisition_run(&catalog, run.id).unwrap();
    assert!(keys(&[], 2).is_empty());
}

#[test]
fn the_next_queued_work_is_the_oldest_of_the_sources_not_skipped() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let run = start_acquisition_run(&catalog, request()).unwrap();
    catalog
        .record_discovery(run.id, SOURCE_ID, &[work("a1"), work("a2")])
        .unwrap();
    let mut b1 = work("b1");
    b1.candidate.source_id = SourceId::from("other-source");
    catalog
        .record_discovery(run.id, "other-source", &[b1])
        .unwrap();
    catalog.complete_work(run.id, "a1").unwrap();
    let next = |skipped: &[String]| {
        catalog
            .next_queued_work(run.id, skipped)
            .unwrap()
            .map(|work| work.key)
    };

    assert_eq!(next(&[]).as_deref(), Some("a2"));
    assert_eq!(next(&[SOURCE_ID.to_owned()]).as_deref(), Some("b1"));
    assert_eq!(
        next(&[SOURCE_ID.to_owned(), "other-source".to_owned()]),
        None
    );
}

#[test]
fn a_run_is_claimed_by_one_execution_at_a_time_across_processes() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let first = SqliteCatalog::open(&path).unwrap();
    let run = start_acquisition_run(&first, request()).unwrap();
    let other_run = start_acquisition_run(&first, request()).unwrap();
    // Another process opens the same vault.
    let second = SqliteCatalog::open_existing(&path).unwrap();

    assert!(first.claim_execution(run.id).unwrap());
    assert!(!second.claim_execution(run.id).unwrap());
    assert!(!first.claim_execution(run.id).unwrap());
    assert!(second.claim_execution(other_run.id).unwrap());

    first.release_execution(run.id).unwrap();
    assert!(second.claim_execution(run.id).unwrap());
}
