use std::fs;

use game_media_vault_application::{CatalogPort, RunRepositoryPort};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetType, AssetTypeSelector,
    GameSelection, MediaInfo, PersistAsset, RetentionPolicy, SourceFailureStage, SourceId,
    SourceSelection,
};
use game_media_vault_infrastructure::SqliteCatalog;
use rusqlite::Connection;
use tempfile::tempdir;

const VAULT_APPLICATION_ID: i32 = 0x474D_5641;

fn pragma(path: &std::path::Path, name: &str) -> i32 {
    Connection::open(path)
        .unwrap()
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .unwrap()
}

#[test]
fn new_vault_records_its_application_id_and_schema_version() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");

    SqliteCatalog::open(&path).unwrap();

    assert_eq!(pragma(&path, "application_id"), VAULT_APPLICATION_ID);
    assert_eq!(pragma(&path, "user_version"), 14);
}

#[test]
fn catalog_from_a_newer_schema_version_is_refused_unchanged() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    SqliteCatalog::open(&path).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA user_version = 99;")
        .unwrap();
    let before = fs::read(&path).unwrap();

    let error = SqliteCatalog::open_existing(&path).err().unwrap();

    assert!(error.message().contains("newer"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn vault_shaped_database_without_the_application_id_is_refused_unchanged() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    SqliteCatalog::open(&path).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA application_id = 0;")
        .unwrap();
    let before = fs::read(&path).unwrap();

    let error = SqliteCatalog::open_existing(&path).err().unwrap();

    assert!(
        error.message().contains("not a Game Media Vault catalog"),
        "{error}"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn creating_a_vault_over_an_unrelated_database_is_refused_unchanged() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE unrelated_data (id INTEGER PRIMARY KEY);")
        .unwrap();
    let before = fs::read(&path).unwrap();

    let error = SqliteCatalog::open(&path).err().unwrap();

    assert!(
        error.message().contains("not a Game Media Vault catalog"),
        "{error}"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn catalog_from_an_unsupported_older_schema_version_is_refused_unchanged() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    SqliteCatalog::open(&path).unwrap();
    // Version 1 was a pre-release layout that is not migrated.
    Connection::open(&path)
        .unwrap()
        .execute_batch("PRAGMA user_version = 1;")
        .unwrap();
    let before = fs::read(&path).unwrap();

    let error = SqliteCatalog::open_existing(&path).err().unwrap();

    assert!(
        error.message().contains("unsupported schema version 1"),
        "{error}"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn version_2_catalogs_are_upgraded_to_the_current_layout() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    SqliteCatalog::open(&path)
        .unwrap()
        .persist_asset(PersistAsset {
            existing_game_id: None,
            existing_release_edition_id: None,
            match_decision: None,
            game_title: "Metal Gear Solid".to_owned(),
            platform: "PlayStation".to_owned(),
            region: "France".to_owned(),
            edition_name: "Original".to_owned(),
            asset_type: AssetType::BoxFront,
            object_hash: "abc123".to_owned(),
            byte_len: 4096,
            media: MediaInfo {
                media_type: "image/png".to_owned(),
                width: Some(1200),
                height: Some(1600),
                document: None,
            },
            original_filename: "front.png".to_owned(),
            source_id: SourceId::from("local_import"),
            source_asset_label: None,
            source_location: "C:/covers/front.png".to_owned(),
        })
        .unwrap();
    // Rebuild the version 2 layout, which had no media, quality shortfall, outranked, planned Source or unavailable columns nor Derived Assets, Source failures, dump sets the run work Source index or reference review items.
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "ALTER TABLE assets DROP COLUMN media_type;
             ALTER TABLE assets DROP COLUMN width;
             ALTER TABLE assets DROP COLUMN height;
             ALTER TABLE acquisition_run_work DROP COLUMN quality_shortfalls_json;
             ALTER TABLE acquisition_run_work DROP COLUMN outranked_json;
             DROP TABLE derived_objects;
             ALTER TABLE acquisition_runs DROP COLUMN planned_sources_json;
             ALTER TABLE acquisition_run_work DROP COLUMN unavailable_reason;
             DROP INDEX idx_release_assertion_value;
             DROP TABLE acquisition_source_failures;
             DROP TABLE reference_dump_sets;
             DROP INDEX idx_run_work_source;
             DROP TABLE reference_review_items;
             ALTER TABLE assets DROP COLUMN document_json;
             PRAGMA user_version = 2;",
        )
        .unwrap();

    let catalog = SqliteCatalog::open_existing(&path).unwrap();

    assert_eq!(pragma(&path, "user_version"), 14);
    let library = catalog.list_library().unwrap();
    assert_eq!(library[0].assets[0].media, MediaInfo::unknown());
}

#[test]
fn runs_from_version_6_plan_the_sources_their_request_selects() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Game Boy".to_owned()],
        games: GameSelection::Explicit(vec!["Tetris (World) (Rev 1)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();
    let run = SqliteCatalog::open(&path)
        .unwrap()
        .create_run(request, vec!["libretro-thumbnails".to_owned()])
        .unwrap();
    // Version 6 runs recorded no plan, nor unavailable work.
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "ALTER TABLE acquisition_runs DROP COLUMN planned_sources_json;
             ALTER TABLE acquisition_run_work DROP COLUMN unavailable_reason;
             DROP INDEX idx_release_assertion_value;
             DROP TABLE acquisition_source_failures;
             DROP TABLE reference_dump_sets;
             DROP INDEX idx_run_work_source;
             DROP TABLE reference_review_items;
             ALTER TABLE assets DROP COLUMN document_json;
             PRAGMA user_version = 6;",
        )
        .unwrap();

    let catalog = SqliteCatalog::open_existing(&path).unwrap();

    assert_eq!(pragma(&path, "user_version"), 14);
    assert_eq!(
        catalog.get_run(run.id).unwrap().unwrap().planned_sources,
        ["libretro-thumbnails"]
    );
}

fn has_index(path: &std::path::Path, name: &str) -> bool {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
            [name],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

#[test]
fn assertions_are_indexed_by_the_values_reference_imports_look_up() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");

    SqliteCatalog::open(&path).unwrap();

    assert!(has_index(&path, "idx_release_assertion_value"));
}

#[test]
fn catalogs_from_version_8_gain_the_assertion_value_index() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    SqliteCatalog::open(&path).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "DROP INDEX idx_release_assertion_value;
             DROP TABLE acquisition_source_failures;
             DROP TABLE reference_dump_sets;
             DROP INDEX idx_run_work_source;
             DROP TABLE reference_review_items;
             ALTER TABLE assets DROP COLUMN document_json;
             PRAGMA user_version = 8;",
        )
        .unwrap();

    SqliteCatalog::open_existing(&path).unwrap();

    assert_eq!(pragma(&path, "user_version"), 14);
    assert!(has_index(&path, "idx_release_assertion_value"));
}

#[test]
fn catalogs_from_version_9_gain_the_source_failure_log() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let run = SqliteCatalog::open(&path)
        .unwrap()
        .create_run(
            AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
                sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
                platforms: vec!["Nintendo - Game Boy".to_owned()],
                games: GameSelection::Explicit(vec!["Tetris (World) (Rev 1)".to_owned()]),
                regions: Vec::new(),
                languages: Vec::new(),
                asset_types: vec![AssetTypeSelector::BoxFront],
                quality: None,
                retention: RetentionPolicy::KeepEverything,
                limits: AcquisitionLimits::default(),
            })
            .unwrap(),
            vec!["libretro-thumbnails".to_owned()],
        )
        .unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "DROP TABLE acquisition_source_failures;
             DROP TABLE reference_dump_sets;
             DROP INDEX idx_run_work_source;
             DROP TABLE reference_review_items;
             ALTER TABLE assets DROP COLUMN document_json;
             PRAGMA user_version = 9;",
        )
        .unwrap();

    let catalog = SqliteCatalog::open_existing(&path).unwrap();

    assert_eq!(pragma(&path, "user_version"), 14);
    catalog
        .record_source_failure(
            run.id,
            "libretro-thumbnails",
            SourceFailureStage::Download,
            "connection reset",
        )
        .unwrap();
    assert_eq!(catalog.source_failures().unwrap().len(), 1);
}

#[test]
fn catalogs_from_version_10_gain_the_reference_dump_sets() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    SqliteCatalog::open(&path).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "DROP TABLE reference_dump_sets;
             DROP INDEX idx_run_work_source;
             DROP TABLE reference_review_items;
             ALTER TABLE assets DROP COLUMN document_json;
             PRAGMA user_version = 10;",
        )
        .unwrap();

    SqliteCatalog::open_existing(&path).unwrap();

    assert_eq!(pragma(&path, "user_version"), 14);
    assert!(has_index(&path, "idx_reference_dump_set"));
    assert!(has_index(&path, "idx_run_work_source"));
}

#[test]
fn run_work_is_indexed_by_the_source_of_its_candidate() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");

    SqliteCatalog::open(&path).unwrap();

    assert!(has_index(&path, "idx_run_work_source"));
}

#[test]
fn catalogs_from_version_11_gain_the_run_work_source_index() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    SqliteCatalog::open(&path).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "DROP INDEX idx_run_work_source;
             DROP TABLE reference_review_items;
             ALTER TABLE assets DROP COLUMN document_json;
             PRAGMA user_version = 11;",
        )
        .unwrap();

    SqliteCatalog::open_existing(&path).unwrap();

    assert_eq!(pragma(&path, "user_version"), 14);
    assert!(has_index(&path, "idx_run_work_source"));
}

fn has_table(path: &std::path::Path, name: &str) -> bool {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

#[test]
fn catalogs_from_version_12_gain_the_reference_review_items() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    SqliteCatalog::open(&path).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "DROP TABLE reference_review_items;
             ALTER TABLE assets DROP COLUMN document_json;
             PRAGMA user_version = 12;",
        )
        .unwrap();

    SqliteCatalog::open_existing(&path).unwrap();

    assert_eq!(pragma(&path, "user_version"), 14);
    assert!(has_table(&path, "reference_review_items"));
}

#[test]
fn catalogs_from_version_13_gain_the_document_metadata_of_assets() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    SqliteCatalog::open(&path).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "ALTER TABLE assets DROP COLUMN document_json;
             PRAGMA user_version = 13;",
        )
        .unwrap();

    SqliteCatalog::open_existing(&path).unwrap();

    assert_eq!(pragma(&path, "user_version"), 14);
    let columns: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('assets') WHERE name = 'document_json'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(columns, 1);
}
