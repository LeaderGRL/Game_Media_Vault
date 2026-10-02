use std::io::Read;

use game_media_vault_application::{DerivedStorePort, ObjectStorePort};
use game_media_vault_infrastructure::ContentAddressedStore;
use tempfile::tempdir;

#[test]
fn derived_outputs_are_stored_apart_from_originals() {
    let temp = tempdir().unwrap();
    let store = ContentAddressedStore::new(temp.path());
    let original = store.store_original(&mut &b"original bytes"[..]).unwrap();

    let mut read_back = Vec::new();
    store
        .open_original(&original.hash)
        .unwrap()
        .read_to_end(&mut read_back)
        .unwrap();
    let derived = store.store_derived(&mut &b"thumbnail bytes"[..]).unwrap();

    assert_eq!(read_back, b"original bytes");
    assert!(store.derived_path(&derived.hash).is_file());
    assert!(!store.object_path(&derived.hash).exists());
    assert_eq!(derived.byte_len, 15);
}

#[test]
fn only_content_addresses_can_be_opened() {
    let temp = tempdir().unwrap();
    let store = ContentAddressedStore::new(temp.path());

    assert!(store.open_original("../catalog.sqlite3").is_err());
}
