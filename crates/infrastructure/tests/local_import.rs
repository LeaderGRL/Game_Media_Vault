use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
};

use game_media_vault_application::{
    CatalogPort, ImportLocalBoxFrontRequest, ObjectStorePort, import_local_box_front, list_library,
};
use game_media_vault_domain::{AssetType, PersistAsset, SourceId};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use rusqlite::{Connection, params};
use tempfile::{tempdir, tempdir_in};

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
fn an_existing_catalog_is_not_recreated_if_it_disappears_after_opening() {
    let temp = tempdir().unwrap();
    let catalog_path = temp.path().join("catalog.sqlite3");
    SqliteCatalog::open(&catalog_path).unwrap();
    let catalog = SqliteCatalog::open_existing(&catalog_path).unwrap();
    fs::remove_file(&catalog_path).unwrap();

    assert!(list_library(&catalog).is_err());
    assert!(!catalog_path.exists());
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
    let canonical_source = temp.path().join("mgs-front.png");
    fs::write(&canonical_source, b"metal gear solid front cover").unwrap();
    let alias_dir = temp.path().join("alias");
    fs::create_dir(&alias_dir).unwrap();
    let source = alias_dir.join("..").join("mgs-front.png");
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
    assert_eq!(library[0].assets.len(), 1);
    assert_eq!(library[0].assets[0].original_filename, "mgs-front.png");
    assert_eq!(library[0].assets[0].provenance.len(), 1);
    assert_eq!(
        fs::canonicalize(std::path::Path::new(
            &library[0].assets[0].provenance[0].source_location
        ))
        .unwrap(),
        fs::canonicalize(&canonical_source).unwrap()
    );
    assert_eq!(library[0].assets[0].object_hash, first.object_hash);
}

#[test]
fn canonical_provenance_identity_ignores_caller_filename_alias() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("actual-front.png");
    fs::write(&source, b"shared cover bytes").unwrap();
    let source_location = fs::canonicalize(&source)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let first_record = PersistAsset {
        existing_game_id: None,
        existing_release_edition_id: None,
        game_title: "Alias Game".to_owned(),
        platform: "Windows".to_owned(),
        region: "Worldwide".to_owned(),
        edition_name: "Standard".to_owned(),
        asset_type: AssetType::BoxFront,
        object_hash: "shared-object-hash".to_owned(),
        byte_len: 18,
        original_filename: "alias-front.png".to_owned(),
        source_id: SourceId::from("local_import"),
        source_asset_label: None,
        source_location: source_location.clone(),
    };

    let first = catalog.persist_asset(first_record.clone()).unwrap();
    let second = catalog
        .persist_asset(PersistAsset {
            original_filename: "actual-front.png".to_owned(),
            ..first_record
        })
        .unwrap();

    assert_eq!(second.asset_id, first.asset_id);
    assert_eq!(second.game_id, first.game_id);
    assert_eq!(list_library(&catalog).unwrap().len(), 1);
}

#[test]
fn concurrent_reimports_persist_one_logical_asset() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog_path = vault.join("catalog.sqlite3");
    SqliteCatalog::open(&catalog_path).unwrap();
    let source = temp.path().join("shared-front.png");
    fs::write(&source, b"shared cover bytes").unwrap();
    let workers = 2;
    let barrier = Arc::new(Barrier::new(workers + 1));
    let handles: Vec<_> = (0..workers)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let vault = vault.clone();
            let catalog_path = catalog_path.clone();
            let source = source.clone();
            thread::spawn(move || {
                let catalog = SqliteCatalog::open_existing(catalog_path).unwrap();
                let store = ContentAddressedStore::new(vault);
                barrier.wait();
                import_local_box_front(
                    &catalog,
                    &store,
                    ImportLocalBoxFrontRequest {
                        existing_game_id: None,
                        game_title: "Concurrent Game".to_owned(),
                        platform: "Windows".to_owned(),
                        region: "Worldwide".to_owned(),
                        edition_name: "Standard".to_owned(),
                        source_path: source,
                    },
                )
                .unwrap()
            })
        })
        .collect();

    barrier.wait();

    let imported: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    let first_asset_id = imported[0].asset_id;
    assert!(
        imported
            .iter()
            .all(|asset| asset.asset_id == first_asset_id)
    );

    let catalog = SqliteCatalog::open_existing(catalog_path).unwrap();
    assert_eq!(list_library(&catalog).unwrap().len(), 1);
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
            game_title: "Localized Same Game".to_owned(),
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
fn legacy_relative_provenance_is_not_rebased_to_the_current_working_directory() {
    let current_dir = std::env::current_dir().unwrap();
    let temp = tempdir_in(&current_dir).unwrap();
    let source_dir = temp.path().join("previous-session");
    fs::create_dir_all(&source_dir).unwrap();
    let source = source_dir.join("legacy-front.png");
    fs::write(&source, b"legacy cover bytes").unwrap();
    let relative_source = source.strip_prefix(&current_dir).unwrap().to_path_buf();
    let vault = temp.path().join("vault");
    let catalog_path = vault.join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&catalog_path).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let stored = store.store_original(&source).unwrap();

    let connection = Connection::open(&catalog_path).unwrap();
    connection
        .execute(
            "INSERT INTO games (id, title, normalized_title) VALUES (1, 'Legacy Game', 'legacy game')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO release_editions (
                id, game_id, platform, normalized_platform, region, normalized_region,
                edition_name, normalized_edition_name
             ) VALUES (1, 1, 'Windows', 'windows', 'Worldwide', 'worldwide', 'Standard', 'standard')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO assets (
                id, release_edition_id, asset_type, object_hash, byte_len, original_filename
             ) VALUES (1, 1, 'box_front', ?1, ?2, 'legacy-front.png')",
            params![stored.hash, i64::try_from(stored.byte_len).unwrap()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO asset_provenance (id, asset_id, source_kind, source_location)
             VALUES (1, 1, 'local_import', ?1)",
            params![relative_source.to_string_lossy()],
        )
        .unwrap();
    drop(connection);

    let imported = import_local_box_front(
        &catalog,
        &store,
        ImportLocalBoxFrontRequest {
            existing_game_id: None,
            game_title: "Legacy Game".to_owned(),
            platform: "Windows".to_owned(),
            region: "Worldwide".to_owned(),
            edition_name: "Standard".to_owned(),
            source_path: source.clone(),
        },
    )
    .unwrap();

    assert_ne!(imported.game_id, 1);
    assert_ne!(imported.asset_id, 1);

    let repeated = import_local_box_front(
        &catalog,
        &store,
        ImportLocalBoxFrontRequest {
            existing_game_id: None,
            game_title: "Legacy Game".to_owned(),
            platform: "Windows".to_owned(),
            region: "Worldwide".to_owned(),
            edition_name: "Standard".to_owned(),
            source_path: source.clone(),
        },
    )
    .unwrap();
    assert_eq!(repeated.asset_id, imported.asset_id);

    let library = list_library(&catalog).unwrap();
    assert_eq!(library.len(), 2);
    let legacy = library
        .iter()
        .flat_map(|entry| entry.assets.iter())
        .find(|asset| asset.asset_id == 1)
        .unwrap();
    assert_eq!(legacy.provenance.len(), 1);
    assert_eq!(
        legacy.provenance[0].source_location,
        relative_source.to_string_lossy()
    );
    let current = library
        .iter()
        .flat_map(|entry| entry.assets.iter())
        .find(|asset| asset.asset_id == imported.asset_id)
        .unwrap();
    assert_eq!(current.provenance.len(), 1);
    let current_location = std::path::Path::new(&current.provenance[0].source_location);
    assert!(current_location.is_absolute());
    assert_eq!(
        fs::canonicalize(current_location).unwrap(),
        fs::canonicalize(&source).unwrap()
    );
}

#[test]
fn unresolved_legacy_relative_provenance_does_not_merge_same_named_distinct_games() {
    let temp = tempdir().unwrap();
    let source_dir = temp.path().join("second-copy");
    fs::create_dir_all(&source_dir).unwrap();
    let source = source_dir.join("front.png");
    fs::write(&source, b"shared legacy bytes").unwrap();
    let vault = temp.path().join("vault");
    let catalog_path = vault.join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&catalog_path).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let stored = store.store_original(&source).unwrap();

    let connection = Connection::open(&catalog_path).unwrap();
    for (game_id, relative_source) in [
        (1_i64, "first-copy/front.png"),
        (2_i64, "second-copy/front.png"),
    ] {
        connection
            .execute(
                "INSERT INTO games (id, title, normalized_title)
                 VALUES (?1, 'Same Legacy Title', 'same legacy title')",
                params![game_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO release_editions (
                    id, game_id, platform, normalized_platform, region, normalized_region,
                    edition_name, normalized_edition_name
                 ) VALUES (?1, ?1, 'Windows', 'windows', 'Worldwide', 'worldwide', 'Standard', 'standard')",
                params![game_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO assets (
                    id, release_edition_id, asset_type, object_hash, byte_len, original_filename
                 ) VALUES (?1, ?1, 'box_front', ?2, ?3, 'front.png')",
                params![
                    game_id,
                    stored.hash,
                    i64::try_from(stored.byte_len).unwrap()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO asset_provenance (id, asset_id, source_kind, source_location)
                 VALUES (?1, ?1, 'local_import', ?2)",
                params![game_id, relative_source],
            )
            .unwrap();
    }
    drop(connection);

    let imported = import_local_box_front(
        &catalog,
        &store,
        ImportLocalBoxFrontRequest {
            existing_game_id: None,
            game_title: "Same Legacy Title".to_owned(),
            platform: "Windows".to_owned(),
            region: "Worldwide".to_owned(),
            edition_name: "Standard".to_owned(),
            source_path: source,
        },
    )
    .unwrap();

    assert_ne!(imported.game_id, 1);
    assert_ne!(imported.game_id, 2);
    assert_ne!(imported.asset_id, 1);
    assert_ne!(imported.asset_id, 2);
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
