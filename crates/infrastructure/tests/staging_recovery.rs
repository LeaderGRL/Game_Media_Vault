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
    fs::write(
        staging.join(format!("{}-0.tmp", std::process::id())),
        b"interrupted import",
    )
    .unwrap();

    let source = temp.path().join("cover.png");
    fs::write(&source, b"fresh cover bytes").unwrap();
    let store = ContentAddressedStore::new(&vault);

    let stored = store.store_original(&source).unwrap();

    assert_eq!(
        fs::read(store.object_path(&stored.hash)).unwrap(),
        b"fresh cover bytes"
    );
}
