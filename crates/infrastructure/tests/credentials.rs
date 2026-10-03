use game_media_vault_application::{ApiKey, CredentialStorePort};
use game_media_vault_infrastructure::{KeyringCredentialStore, NoCredentials};

/// Writes one entry to this machine's real credential store, under a service of its own, and
/// removes it: run it with `cargo test -- --ignored` on a machine whose store is available.
#[test]
#[ignore = "writes to the OS credential store"]
fn an_api_key_is_kept_by_the_os_credential_store_until_cleared() {
    let store =
        KeyringCredentialStore::named(format!("game-media-vault-test-{}", std::process::id()));
    let key = ApiKey::new("test-key-not-a-secret").unwrap();

    store.set_api_key("test-source", &key).unwrap();
    let kept = store.api_key("test-source").unwrap();
    store.clear_api_key("test-source").unwrap();

    assert_eq!(kept, Some(key));
    assert_eq!(store.api_key("test-source").unwrap(), None);
    // Clearing a key that is not stored is no failure.
    store.clear_api_key("test-source").unwrap();
    // A value no key can be, as another tool may write, reads as no key.
    let service = format!("game-media-vault-test-{}", std::process::id());
    let raw = keyring::v1::Entry::new(&service, "test-source").unwrap();
    raw.set_password("not a key").unwrap();
    let read = store.api_key("test-source");
    raw.delete_credential().unwrap();
    assert_eq!(read.unwrap(), None);
}

#[test]
fn a_machine_keeping_no_credentials_has_no_key_and_stores_none() {
    let key = ApiKey::new("zq-test-secret").unwrap();

    assert_eq!(NoCredentials.api_key("test-source").unwrap(), None);
    let error = NoCredentials.set_api_key("test-source", &key).unwrap_err();
    assert!(!error.message().contains("zq-test-secret"), "{error}");
    assert!(NoCredentials.clear_api_key("test-source").is_ok());
}
