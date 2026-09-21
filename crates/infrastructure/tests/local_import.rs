use std::fs;

use game_media_vault_application::{
    ImportLocalBoxFrontRequest, ObjectStorePort, import_local_box_front, list_library,
};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use tempfile::tempdir;

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
fn persists_and_lists_one_logical_asset_for_repeated_imports() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("mgs-front.png");
    fs::write(&source, b"metal gear solid front cover").unwrap();
    let store = ContentAddressedStore::new(temp.path().join("vault"));
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let request = || ImportLocalBoxFrontRequest {
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
