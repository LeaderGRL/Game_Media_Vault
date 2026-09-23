use game_media_vault_application::{CatalogPort, ReferenceCatalogRepositoryPort};
use game_media_vault_domain::{
    ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField, SourceId,
};
use game_media_vault_infrastructure::SqliteCatalog;
use rusqlite::Connection;
use tempfile::tempdir;

fn assertion(
    field: ReleaseAssertionField,
    qualifier: Option<&str>,
    value: &str,
) -> ReleaseAssertion {
    ReleaseAssertion {
        source_id: SourceId::from("no-intro"),
        source_location: "C:/fixtures/Nintendo - Game Boy.dat".to_owned(),
        field,
        qualifier: qualifier.map(str::to_owned),
        value: value.to_owned(),
    }
}

fn tetris_release(revision: &str) -> ReferenceReleaseRecord {
    ReferenceReleaseRecord {
        game_title: "Tetris".to_owned(),
        platform: "Nintendo - Game Boy".to_owned(),
        region: "World".to_owned(),
        revision: Some(revision.to_owned()),
        edition_name: revision.to_owned(),
        assertions: vec![
            assertion(ReleaseAssertionField::Title, None, "Tetris"),
            assertion(ReleaseAssertionField::Region, None, "World"),
            assertion(ReleaseAssertionField::Revision, None, revision),
            assertion(
                ReleaseAssertionField::Identifier,
                Some("source_record"),
                &format!("Tetris (World) ({revision})"),
            ),
            assertion(
                ReleaseAssertionField::Identifier,
                Some("sha1"),
                if revision == "Rev 1" {
                    "1111111111111111111111111111111111111111"
                } else {
                    "2222222222222222222222222222222222222222"
                },
            ),
        ],
    }
}

#[test]
fn reference_import_is_idempotent_and_lists_assetless_release_editions() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();

    let first = catalog
        .persist_reference_release(tetris_release("Rev 1"))
        .unwrap();
    let second = catalog
        .persist_reference_release(tetris_release("Rev 2"))
        .unwrap();
    let repeated = catalog
        .persist_reference_release(tetris_release("Rev 1"))
        .unwrap();

    assert_eq!(first, repeated);
    assert_eq!(first.game_id, second.game_id);
    assert_ne!(first.release_edition_id, second.release_edition_id);

    let library = catalog.list_library().unwrap();
    assert_eq!(library.len(), 2);
    assert!(library.iter().all(|entry| entry.game_title == "Tetris"));
    assert!(library.iter().all(|entry| entry.assets.is_empty()));
    assert!(library.iter().all(|entry| {
        entry
            .assertions
            .iter()
            .any(|assertion| assertion.field == ReleaseAssertionField::Title)
    }));
    assert!(library.iter().any(|entry| {
        entry.assertions.iter().any(|assertion| {
            assertion.field == ReleaseAssertionField::Revision && assertion.value == "Rev 1"
        })
    }));
    assert!(library.iter().any(|entry| {
        entry.assertions.iter().any(|assertion| {
            assertion.field == ReleaseAssertionField::Revision && assertion.value == "Rev 2"
        })
    }));
}

#[test]
fn reference_batch_rolls_back_all_releases_when_one_record_is_invalid() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let mut invalid = tetris_release("Rev 2");
    invalid.assertions.retain(|assertion| {
        assertion.field != ReleaseAssertionField::Identifier
            || assertion.qualifier.as_deref() != Some("source_record")
    });

    let error = catalog
        .persist_reference_releases(vec![tetris_release("Rev 1"), invalid])
        .unwrap_err();

    assert!(error.0.contains("source_record"));
    assert!(catalog.list_library().unwrap().is_empty());
}

#[test]
fn opening_a_pre_reference_catalog_adds_assertions_without_losing_assets() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("pre-reference.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE games (
                 id INTEGER PRIMARY KEY,
                 title TEXT NOT NULL,
                 normalized_title TEXT NOT NULL
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
             );
             CREATE TABLE assets (
                 id INTEGER PRIMARY KEY,
                 release_edition_id INTEGER NOT NULL REFERENCES release_editions(id),
                 asset_type TEXT NOT NULL,
                 object_hash TEXT NOT NULL,
                 byte_len INTEGER NOT NULL CHECK(byte_len >= 0),
                 original_filename TEXT NOT NULL,
                 UNIQUE(release_edition_id, asset_type, object_hash)
             );
             CREATE TABLE asset_provenance (
                 id INTEGER PRIMARY KEY,
                 asset_id INTEGER NOT NULL REFERENCES assets(id),
                 source_kind TEXT NOT NULL,
                 source_location TEXT NOT NULL,
                 source_asset_label TEXT,
                 UNIQUE(asset_id, source_kind, source_location)
             );
             INSERT INTO games (id, title, normalized_title)
             VALUES (1, 'Tetris', 'tetris');
             INSERT INTO release_editions (
                 id, game_id, platform, normalized_platform, region, normalized_region,
                 edition_name, normalized_edition_name
             ) VALUES (2, 1, 'Nintendo - Game Boy', 'nintendo - game boy', 'World', 'world', 'Standard', 'standard');
             INSERT INTO assets (
                 id, release_edition_id, asset_type, object_hash, byte_len, original_filename
             ) VALUES (3, 2, 'box_front', 'abc123', 4096, 'tetris-front.png');
             INSERT INTO asset_provenance (
                 asset_id, source_kind, source_location, source_asset_label
             ) VALUES (3, 'local_import', 'C:/covers/tetris-front.png', NULL);",
        )
        .unwrap();
    drop(connection);

    let catalog = SqliteCatalog::open_existing(&path).unwrap();
    let library = catalog.list_library().unwrap();

    assert_eq!(library.len(), 1);
    assert_eq!(library[0].game_title, "Tetris");
    assert_eq!(library[0].assets.len(), 1);
    assert!(library[0].assertions.is_empty());
    drop(catalog);

    let connection = Connection::open(&path).unwrap();
    let assertions_table: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'release_assertions'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let assertion_indexes: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name IN (
                 'idx_release_assertion_release',
                 'idx_release_assertion_title',
                 'idx_release_assertion_source_record'
             )",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(assertions_table, 1);
    assert_eq!(assertion_indexes, 3);
}

#[test]
fn opening_catalog_rejects_release_assertions_without_the_required_unique_constraint() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("invalid-reference-assertions.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE games (
                 id INTEGER PRIMARY KEY,
                 title TEXT NOT NULL,
                 normalized_title TEXT NOT NULL
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
             );
             CREATE TABLE release_assertions (
                 id INTEGER PRIMARY KEY,
                 release_edition_id INTEGER NOT NULL REFERENCES release_editions(id),
                 source_id TEXT NOT NULL,
                 source_location TEXT NOT NULL,
                 field TEXT NOT NULL,
                 qualifier TEXT NOT NULL,
                 value TEXT NOT NULL,
                 normalized_value TEXT NOT NULL
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
fn opening_catalog_rejects_release_assertions_without_uniqueness_constraint() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("missing-assertion-uniqueness.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    drop(catalog);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys = OFF;
             DROP TABLE release_assertions;
             CREATE TABLE release_assertions (
                 id INTEGER PRIMARY KEY,
                 release_edition_id INTEGER NOT NULL REFERENCES release_editions(id),
                 source_id TEXT NOT NULL,
                 source_location TEXT NOT NULL,
                 field TEXT NOT NULL,
                 qualifier TEXT NOT NULL,
                 value TEXT NOT NULL,
                 normalized_value TEXT NOT NULL
             );
             PRAGMA foreign_keys = ON;",
        )
        .unwrap();
    drop(connection);

    assert!(SqliteCatalog::open_existing(&path).is_err());
}
