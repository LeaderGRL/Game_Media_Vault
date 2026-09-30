use std::fs;

use game_media_vault_application::{CatalogPort, ObjectStorePort};
use game_media_vault_domain::{AssetType, PersistAsset, SourceId};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use rusqlite::{Connection, params};
use tempfile::tempdir;

#[test]
fn reopening_vault_removes_a_stale_unreferenced_pending_publication() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog_path = vault.join("catalog.sqlite3");
    let _catalog = SqliteCatalog::open(&catalog_path).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let stored = store
        .store_original_bytes(b"orphaned review publication")
        .unwrap();
    let object_path = store.object_path(&stored.hash);
    assert!(object_path.is_file());

    Connection::open(&catalog_path)
        .unwrap()
        .execute(
            "INSERT INTO pending_object_publications (object_hash, byte_len, created_at_unix)
             VALUES (?1, ?2, 0)",
            params![stored.hash, stored.byte_len as i64],
        )
        .unwrap();

    SqliteCatalog::open_existing(&catalog_path).unwrap();

    assert!(!object_path.exists());
    let pending: i64 = Connection::open(&catalog_path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM pending_object_publications",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
}

#[test]
fn reopening_vault_keeps_a_stale_pending_publication_object_when_asset_references_it() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog_path = vault.join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&catalog_path).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let stored = store
        .store_original_bytes(b"referenced review publication")
        .unwrap();
    let object_path = store.object_path(&stored.hash);
    catalog
        .persist_asset(PersistAsset {
            existing_game_id: None,
            existing_release_edition_id: None,
            match_decision: None,
            game_title: "Referenced Game".to_owned(),
            platform: "Fixture Platform".to_owned(),
            region: "World".to_owned(),
            edition_name: "Standard".to_owned(),
            asset_type: AssetType::BoxFront,
            object_hash: stored.hash.clone(),
            byte_len: stored.byte_len,
            original_filename: "front.png".to_owned(),
            source_id: SourceId::from("fixture-provider"),
            source_asset_label: Some("front".to_owned()),
            source_location: "fixture://referenced/front".to_owned(),
        })
        .unwrap();

    Connection::open(&catalog_path)
        .unwrap()
        .execute(
            "INSERT INTO pending_object_publications (object_hash, byte_len, created_at_unix)
             VALUES (?1, ?2, 0)",
            params![stored.hash, stored.byte_len as i64],
        )
        .unwrap();

    SqliteCatalog::open_existing(&catalog_path).unwrap();

    assert!(object_path.is_file());
    let pending: i64 = Connection::open(&catalog_path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM pending_object_publications",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
}

#[test]
fn reopening_vault_leaves_a_fresh_pending_publication_untouched() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog_path = vault.join("catalog.sqlite3");
    let _catalog = SqliteCatalog::open(&catalog_path).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let stored = store.store_original_bytes(b"fresh publication").unwrap();
    let object_path = store.object_path(&stored.hash);

    Connection::open(&catalog_path)
        .unwrap()
        .execute(
            "INSERT INTO pending_object_publications (object_hash, byte_len)
             VALUES (?1, ?2)",
            params![stored.hash, stored.byte_len as i64],
        )
        .unwrap();

    SqliteCatalog::open_existing(&catalog_path).unwrap();

    assert!(object_path.is_file());
    let pending: i64 = Connection::open(&catalog_path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM pending_object_publications",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1);
}

#[test]
fn reopening_vault_keeps_object_for_fresh_sibling_and_cleans_only_stale_publication() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog_path = vault.join("catalog.sqlite3");
    let _catalog = SqliteCatalog::open(&catalog_path).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let stored = store.store_original_bytes(b"shared publication").unwrap();
    let object_path = store.object_path(&stored.hash);
    let connection = Connection::open(&catalog_path).unwrap();
    connection
        .execute(
            "INSERT INTO pending_object_publications (object_hash, byte_len, created_at_unix)
             VALUES (?1, ?2, 0)",
            params![stored.hash, stored.byte_len as i64],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO pending_object_publications (object_hash, byte_len)
             VALUES (?1, ?2)",
            params![stored.hash, stored.byte_len as i64],
        )
        .unwrap();
    drop(connection);

    SqliteCatalog::open_existing(&catalog_path).unwrap();

    assert!(object_path.is_file());
    let pending: i64 = Connection::open(&catalog_path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM pending_object_publications",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1);
}

#[test]
fn reopening_vault_does_not_turn_a_malformed_hash_into_a_filesystem_path() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog_path = vault.join("catalog.sqlite3");
    let _catalog = SqliteCatalog::open(&catalog_path).unwrap();
    let sentinel = vault.join("sentinel.txt");
    fs::write(&sentinel, b"must survive recovery").unwrap();

    Connection::open(&catalog_path)
        .unwrap()
        .execute(
            "INSERT INTO pending_object_publications (object_hash, byte_len, created_at_unix)
             VALUES (?1, 1, 0)",
            ["../../sentinel.txt"],
        )
        .unwrap();

    SqliteCatalog::open_existing(&catalog_path).unwrap();

    assert_eq!(fs::read(&sentinel).unwrap(), b"must survive recovery");
    let pending: i64 = Connection::open(&catalog_path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM pending_object_publications",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
}

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
