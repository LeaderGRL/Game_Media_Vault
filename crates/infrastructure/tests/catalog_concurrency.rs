//! Readers and writers of one catalog, as an executing run and the desktop app reading the
//! Library are, never wait for each other.

use game_media_vault_application::CatalogPort;
use game_media_vault_domain::{AssetType, MediaInfo, PersistAsset, SourceId};
use game_media_vault_infrastructure::SqliteCatalog;
use rusqlite::Connection;
use tempfile::tempdir;

fn front_of(title: &str) -> PersistAsset {
    PersistAsset {
        existing_game_id: None,
        existing_release_edition_id: None,
        match_decision: None,
        game_title: title.to_owned(),
        platform: "Nintendo - Game Boy Advance".to_owned(),
        region: "Europe".to_owned(),
        edition_name: "Standard".to_owned(),
        asset_type: AssetType::BoxFront,
        object_hash: format!("{title}-front-hash"),
        byte_len: 42,
        media: MediaInfo::unknown(),
        original_filename: "front.png".to_owned(),
        source_id: SourceId::from("local_import"),
        source_asset_label: None,
        source_location: format!("C:/collection/{title}/front.png"),
    }
}

fn asset_count(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn a_write_commits_while_another_connection_reads_the_catalog() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    catalog.persist_asset(front_of("First Game")).unwrap();
    // A long read, as the whole Library projected in one snapshot.
    let reader = Connection::open(&path).unwrap();
    reader.execute_batch("BEGIN;").unwrap();
    assert_eq!(asset_count(&reader), 1);

    catalog.persist_asset(front_of("Second Game")).unwrap();

    // The reader keeps the snapshot it started with, then sees the write.
    assert_eq!(asset_count(&reader), 1);
    reader.execute_batch("COMMIT;").unwrap();
    assert_eq!(asset_count(&reader), 2);
}

#[test]
fn a_read_succeeds_while_another_connection_writes_the_catalog() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    catalog.persist_asset(front_of("First Game")).unwrap();
    // A write holding the catalog, as one committing or spilling a large transaction does.
    let writer = Connection::open(&path).unwrap();
    writer.execute_batch("BEGIN EXCLUSIVE;").unwrap();
    writer
        .execute("UPDATE games SET title = 'Renamed Game'", [])
        .unwrap();

    let library = catalog.list_library().unwrap();

    assert_eq!(library.len(), 1);
    assert_eq!(library[0].game_title, "First Game");
    writer.execute_batch("COMMIT;").unwrap();
}

#[test]
fn an_upgraded_catalog_keeps_its_readers_and_writers_apart() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    drop(SqliteCatalog::open(&path).unwrap());
    // A catalog an earlier version left in the rollback journal mode.
    Connection::open(&path)
        .unwrap()
        .query_row("PRAGMA journal_mode = DELETE", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap();

    let catalog = SqliteCatalog::open_existing(&path).unwrap();
    let reader = Connection::open(&path).unwrap();
    reader.execute_batch("BEGIN;").unwrap();
    assert_eq!(asset_count(&reader), 0);

    catalog.persist_asset(front_of("First Game")).unwrap();

    reader.execute_batch("COMMIT;").unwrap();
    assert_eq!(asset_count(&reader), 1);
}
