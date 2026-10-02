use game_media_vault_application::ObjectStorePort;
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use game_media_vault_tauri::{VaultSession, object_response};
use tempfile::{TempDir, tempdir};

const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\nfixture box front";

fn open_vault_with_object() -> (TempDir, VaultSession, String) {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let stored = ContentAddressedStore::new(&vault)
        .store_original(&mut &PNG_BYTES[..])
        .unwrap();
    let session = VaultSession::default();
    session.open(&vault, false).unwrap();
    (temp, session, stored.hash)
}

#[test]
fn serves_an_original_object_of_the_open_vault() {
    let (_temp, session, hash) = open_vault_with_object();

    let response = object_response(&session, &format!("/{hash}"));

    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(
        response.headers()["cache-control"],
        "private, max-age=31536000, immutable"
    );
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.body().as_slice(), PNG_BYTES);
}

#[test]
fn labels_objects_with_the_media_type_of_their_signature() {
    let (temp, session, _hash) = open_vault_with_object();
    let store = ContentAddressedStore::new(temp.path().join("vault"));

    // Smallest headers of each format, as image files start.
    let jpeg =
        b"\xff\xd8\xff\xc0\x00\x11\x08\x01\xe0\x02\x80\x03\x01\x22\x00\x02\x11\x01\x03\x11\x01";
    let gif = b"GIF89a\x80\x02\xe0\x01\x00\x00\x00";
    let webp =
        b"RIFF\x16\x00\x00\x00WEBPVP8X\x0a\x00\x00\x00\x00\x00\x00\x00\x7f\x02\x00\xdf\x01\x00";
    for (bytes, media_type) in [
        (&jpeg[..], "image/jpeg"),
        (&gif[..], "image/gif"),
        (&webp[..], "image/webp"),
        (b"%PDF-1.7", "application/pdf"),
        (b"<html><script>", "application/octet-stream"),
    ] {
        let stored = store.store_original(&mut &bytes[..]).unwrap();

        let response = object_response(&session, &format!("/{}", stored.hash));

        assert_eq!(response.headers()["content-type"], media_type);
    }
}

#[test]
fn refuses_paths_that_are_not_object_hashes() {
    let (_temp, session, _hash) = open_vault_with_object();

    for path in ["/../catalog.sqlite3", "/ABC", "/", "/objects/aa/bb/x"] {
        assert_eq!(object_response(&session, path).status(), 400, "{path}");
    }
}

#[test]
fn reports_missing_objects_and_closed_vaults() {
    let (_temp, session, _hash) = open_vault_with_object();
    let unknown = format!("/{}", "0".repeat(64));

    assert_eq!(object_response(&session, &unknown).status(), 404);
    assert_eq!(
        object_response(&VaultSession::default(), &unknown).status(),
        409
    );
}

#[test]
fn serves_derived_assets_by_their_hash_too() {
    let (temp, session, _hash) = open_vault_with_object();
    let thumbnail = b"\x89PNG\r\n\x1a\nfixture thumbnail";
    let derived = game_media_vault_application::DerivedStorePort::store_derived(
        &ContentAddressedStore::new(temp.path().join("vault")),
        &mut &thumbnail[..],
    )
    .unwrap();

    let response = object_response(&session, &format!("/{}", derived.hash));

    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(response.body().as_slice(), thumbnail);
}
