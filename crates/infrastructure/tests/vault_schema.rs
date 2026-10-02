use std::fs;

use game_media_vault_application::CatalogPort;
use game_media_vault_domain::{AssetType, MediaInfo, PersistAsset, SourceId};
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
    assert_eq!(pragma(&path, "user_version"), 5);
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

    assert!(error.0.contains("newer"), "{error}");
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
        error.0.contains("not a Game Media Vault catalog"),
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
        error.0.contains("not a Game Media Vault catalog"),
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

    assert!(error.0.contains("unsupported schema version 1"), "{error}");
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
            },
            original_filename: "front.png".to_owned(),
            source_id: SourceId::from("local_import"),
            source_asset_label: None,
            source_location: "C:/covers/front.png".to_owned(),
        })
        .unwrap();
    // Rebuild the version 2 layout, which had no media, quality shortfall or outranked columns.
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "ALTER TABLE assets DROP COLUMN media_type;
             ALTER TABLE assets DROP COLUMN width;
             ALTER TABLE assets DROP COLUMN height;
             ALTER TABLE acquisition_run_work DROP COLUMN quality_shortfalls_json;
             ALTER TABLE acquisition_run_work DROP COLUMN outranked_json;
             PRAGMA user_version = 2;",
        )
        .unwrap();

    let catalog = SqliteCatalog::open_existing(&path).unwrap();

    assert_eq!(pragma(&path, "user_version"), 5);
    let library = catalog.list_library().unwrap();
    assert_eq!(library[0].assets[0].media, MediaInfo::unknown());
}
