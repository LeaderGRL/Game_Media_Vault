use std::fs;

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
    assert_eq!(pragma(&path, "user_version"), 2);
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
