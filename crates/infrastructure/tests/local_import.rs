use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
};

use game_media_vault_application::{
    CatalogPort, ImportLocalAssetRequest, ObjectStorePort, import_local_box_front, list_library,
};
use game_media_vault_domain::{AssetType, MediaInfo, PersistAsset, SourceId};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
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

    let first = store
        .store_original(&mut fs::File::open(&source).unwrap())
        .unwrap();
    let second = store
        .store_original(&mut fs::File::open(&source).unwrap())
        .unwrap();

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

    let stored = store
        .store_original(&mut fs::File::open(&source).unwrap())
        .unwrap();
    let object_path = store.object_path(&stored.hash);
    fs::write(&object_path, b"corrupted bytes").unwrap();

    let error = store
        .store_original(&mut fs::File::open(&source).unwrap())
        .unwrap_err();

    assert!(error.message().contains("integrity"));
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
    let request = || ImportLocalAssetRequest {
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
    assert_eq!(library[0].entry.game_title, "Metal Gear Solid");
    assert_eq!(library[0].entry.platform, "PlayStation");
    assert_eq!(library[0].entry.region, "France");
    assert_eq!(library[0].entry.assets.len(), 1);
    assert_eq!(
        library[0].entry.assets[0].original_filename,
        "mgs-front.png"
    );
    assert_eq!(library[0].entry.assets[0].provenance.len(), 1);
    assert_eq!(
        fs::canonicalize(std::path::Path::new(
            &library[0].entry.assets[0].provenance[0].source_location
        ))
        .unwrap(),
        fs::canonicalize(&canonical_source).unwrap()
    );
    assert_eq!(library[0].entry.assets[0].object_hash, first.object_hash);
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
        match_decision: None,
        game_title: "Alias Game".to_owned(),
        platform: "Windows".to_owned(),
        region: "Worldwide".to_owned(),
        edition_name: "Standard".to_owned(),
        asset_type: AssetType::BoxFront,
        object_hash: "shared-object-hash".to_owned(),
        byte_len: 18,
        media: MediaInfo::unknown(),
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
                    ImportLocalAssetRequest {
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
            ImportLocalAssetRequest {
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
        ImportLocalAssetRequest {
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
        ImportLocalAssetRequest {
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
fn imported_originals_list_their_media_type_and_pixel_size() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let source = temp.path().join("front.png");
    let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    png.extend_from_slice(&1200_u32.to_be_bytes());
    png.extend_from_slice(&1600_u32.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    fs::write(&source, png).unwrap();

    import_local_box_front(
        &catalog,
        &ContentAddressedStore::new(&vault),
        ImportLocalAssetRequest {
            existing_game_id: None,
            game_title: "Metal Gear Solid".to_owned(),
            platform: "PlayStation".to_owned(),
            region: "France".to_owned(),
            edition_name: "Original".to_owned(),
            source_path: source,
        },
    )
    .unwrap();

    assert_eq!(
        catalog.list_library().unwrap()[0].assets[0].media,
        MediaInfo {
            media_type: "image/png".to_owned(),
            width: Some(1200),
            height: Some(1600),
            document: None,
        }
    );
}
