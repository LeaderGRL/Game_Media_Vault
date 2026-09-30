use std::fs;

use game_media_vault_application::ObjectStorePort;
use game_media_vault_infrastructure::ContentAddressedStore;
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

    let stored = store.store_original(&source).unwrap();

    assert_eq!(
        fs::read(store.object_path(&stored.hash)).unwrap(),
        b"fresh cover bytes"
    );
    assert_eq!(fs::read(stale_path).unwrap(), b"interrupted import");
}

#[test]
fn missing_source_does_not_leave_a_staging_file() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let source = temp.path().join("missing.png");
    let store = ContentAddressedStore::new(&vault);

    assert!(store.store_original(&source).is_err());

    let staging = vault.join("staging");
    if staging.exists() {
        assert_eq!(fs::read_dir(staging).unwrap().count(), 0);
    }
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

    assert!(store.store_original(&source).is_err());

    let staging = vault.join("staging");
    assert_eq!(fs::read_dir(staging).unwrap().count(), 0);
}

#[test]
fn preparing_a_staged_original_verifies_an_existing_cas_object() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let store = ContentAddressedStore::new(&vault);
    let bytes = b"trusted original bytes";
    let stored = store.store_original_bytes(bytes).unwrap();
    let object_path = store.object_path(&stored.hash);
    let mut corrupted = bytes.to_vec();
    corrupted[0] ^= 0xff;
    fs::write(&object_path, &corrupted).unwrap();
    let mut staged = store.stage_original_bytes(bytes).unwrap();

    let error = staged.prepare_publish().unwrap_err();

    assert!(error.0.contains("integrity"));
    assert_eq!(fs::read(object_path).unwrap(), corrupted);
}
