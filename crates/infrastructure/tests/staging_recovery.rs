use std::fs;

use game_media_vault_application::{CatalogPort, ObjectStorePort};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use tempfile::tempdir;

#[test]
fn stale_staging_file_does_not_block_a_new_process_import() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let staging = vault.join("staging");
    fs::create_dir_all(&staging).unwrap();
    let stale_path = staging.join(format!("{}-0.tmp", std::process::id()));
    fs::write(&stale_path, b"interrupted import").unwrap();

    let source = temp.path().join("cover.png");
    fs::write(&source, b"fresh cover bytes").unwrap();
    let store = ContentAddressedStore::new(&vault);

    let stored = store
        .store_original(&mut fs::File::open(&source).unwrap())
        .unwrap();

    assert_eq!(
        fs::read(store.object_path(&stored.hash)).unwrap(),
        b"fresh cover bytes"
    );
    assert_eq!(fs::read(stale_path).unwrap(), b"interrupted import");
}

#[test]
fn failed_publish_does_not_leave_a_staging_file() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("objects"), b"blocks object directories").unwrap();
    let source = temp.path().join("cover.png");
    fs::write(&source, b"cover bytes").unwrap();
    let store = ContentAddressedStore::new(&vault);

    assert!(
        store
            .store_original(&mut fs::File::open(&source).unwrap())
            .is_err()
    );

    let staging = vault.join("staging");
    assert_eq!(fs::read_dir(staging).unwrap().count(), 0);
}

#[test]
fn reopening_a_vault_keeps_unreferenced_objects_for_verification() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog_path = vault.join("catalog.sqlite3");
    SqliteCatalog::open(&catalog_path).unwrap();
    let store = ContentAddressedStore::new(&vault);
    // An import interrupted after publication leaves an object no catalog row references.
    let stored = store
        .store_original(&mut &b"interrupted import bytes"[..])
        .unwrap();

    let reopened = SqliteCatalog::open_existing(&catalog_path).unwrap();

    assert!(reopened.list_library().unwrap().is_empty());
    assert_eq!(
        fs::read(store.object_path(&stored.hash)).unwrap(),
        b"interrupted import bytes"
    );
}
