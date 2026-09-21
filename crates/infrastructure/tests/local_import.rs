use std::fs;

use game_media_vault_application::{
    ImportLocalBoxFrontRequest, ObjectStorePort, import_local_box_front, list_library,
};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn opening_a_missing_catalog_for_reading_does_not_create_a_vault() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("missing-vault");
    let catalog_path = vault.join("catalog.sqlite3");

    let result = SqliteCatalog::open_existing(&catalog_path);

    assert!(result.is_err());
    assert!(!vault.exists());
}

#[test]
fn stores_identical_original_bytes_only_once() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("cover.png");
    fs::write(&source, b"same original bytes").unwrap();
    let store = ContentAddressedStore::new(temp.path().join("vault"));

    let first = store.store_original(&source).unwrap();
    let second = store.store_original(&source).unwrap();

    assert_eq!(first, second);
    assert_eq!(
        fs::read(store.object_path(&first.hash)).unwrap(),
        b"same original bytes"
    );
}

#[test]
fn reimport_rejects_a_corrupted_existing_object() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("cover.png");
    fs::write(&source, b"trusted original bytes").unwrap();
    let store = ContentAddressedStore::new(temp.path().join("vault"));

    let stored = store.store_original(&source).unwrap();
    let object_path = store.object_path(&stored.hash);
    fs::write(&object_path, b"corrupted bytes").unwrap();

    let error = store.store_original(&source).unwrap_err();

    assert!(error.0.contains("integrity"));
    assert_eq!(fs::read(object_path).unwrap(), b"corrupted bytes");
}

#[test]
fn persists_and_lists_one_logical_asset_for_repeated_imports() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("mgs-front.png");
    fs::write(&source, b"metal gear solid front cover").unwrap();
    let store = ContentAddressedStore::new(temp.path().join("vault"));
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let request = || ImportLocalBoxFrontRequest {
        existing_game_id: None,
        game_title: "Metal Gear Solid".to_owned(),
        platform: "PlayStation".to_owned(),
        region: "France".to_owned(),
        edition_name: "Original".to_owned(),
        source_path: source.clone(),
    };

    let first = import_local_box_front(&catalog, &store, request()).unwrap();
    let second = import_local_box_front(&catalog, &store, request()).unwrap();
    let library = list_library(&catalog).unwrap();

    assert_eq!(first.asset_id, second.asset_id);
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].game_title, "Metal Gear Solid");
    assert_eq!(library[0].platform, "PlayStation");
    assert_eq!(library[0].region, "France");
    assert_eq!(library[0].original_filename, "mgs-front.png");
    assert_eq!(library[0].provenance.len(), 1);
    assert_eq!(
        library[0].provenance[0].source_location,
        source.to_string_lossy()
    );
    assert_eq!(library[0].object_hash, first.object_hash);
}

#[test]
fn same_title_imports_do_not_merge_distinct_games_without_matching_evidence() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let first_source = temp.path().join("first-front.png");
    let second_source = temp.path().join("second-front.png");
    fs::write(&first_source, b"first game cover").unwrap();
    fs::write(&second_source, b"second game cover").unwrap();

    let import = |source| {
        import_local_box_front(
            &catalog,
            &store,
            ImportLocalBoxFrontRequest {
                existing_game_id: None,
                game_title: "Same Name".to_owned(),
                platform: "Windows".to_owned(),
                region: "Worldwide".to_owned(),
                edition_name: "Standard".to_owned(),
                source_path: source,
            },
        )
        .unwrap()
    };

    let first = import(first_source);
    let second = import(second_source);

    assert_ne!(first.game_id, second.game_id);
}

#[test]
fn explicit_game_id_attaches_a_new_asset_to_the_existing_game() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let first_source = temp.path().join("first-front.png");
    let second_source = temp.path().join("alternate-front.png");
    fs::write(&first_source, b"first cover").unwrap();
    fs::write(&second_source, b"alternate cover").unwrap();

    let first = import_local_box_front(
        &catalog,
        &store,
        ImportLocalBoxFrontRequest {
            existing_game_id: None,
            game_title: "Same Game".to_owned(),
            platform: "Windows".to_owned(),
            region: "Worldwide".to_owned(),
            edition_name: "Standard".to_owned(),
            source_path: first_source,
        },
    )
    .unwrap();

    let second = import_local_box_front(
        &catalog,
        &store,
        ImportLocalBoxFrontRequest {
            existing_game_id: Some(first.game_id),
            game_title: "Same Game".to_owned(),
            platform: "Windows".to_owned(),
            region: "Worldwide".to_owned(),
            edition_name: "Standard".to_owned(),
            source_path: second_source,
        },
    )
    .unwrap();

    assert_eq!(second.game_id, first.game_id);
    assert_eq!(second.release_edition_id, first.release_edition_id);
    assert_ne!(second.asset_id, first.asset_id);
}

#[test]
fn opening_a_legacy_catalog_removes_title_only_game_identity() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    let catalog_path = vault.join("catalog.sqlite3");
    let connection = Connection::open(&catalog_path).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE games (
                 id INTEGER PRIMARY KEY,
                 title TEXT NOT NULL,
                 normalized_title TEXT NOT NULL UNIQUE
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
             INSERT INTO games (id, title, normalized_title)
             VALUES (1, 'Same Name', 'same name');
             INSERT INTO release_editions (
                 id, game_id, platform, normalized_platform, region, normalized_region,
                 edition_name, normalized_edition_name
             ) VALUES (1, 1, 'Windows', 'windows', 'Worldwide', 'worldwide', 'Standard', 'standard');",
        )
        .unwrap();
    drop(connection);

    let catalog = SqliteCatalog::open(&catalog_path).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let source = temp.path().join("new-front.png");
    fs::write(&source, b"different same-title game").unwrap();

    let imported = import_local_box_front(
        &catalog,
        &store,
        ImportLocalBoxFrontRequest {
            existing_game_id: None,
            game_title: "Same Name".to_owned(),
            platform: "Windows".to_owned(),
            region: "Worldwide".to_owned(),
            edition_name: "Standard".to_owned(),
            source_path: source,
        },
    )
    .unwrap();

    assert_ne!(imported.game_id, 1);
    let migrated = Connection::open(&catalog_path).unwrap();
    let foreign_key_errors: i64 = migrated
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(foreign_key_errors, 0);
}
