use game_media_vault_application::{CatalogPort, ObjectStorePort};
use game_media_vault_domain::{AssetType, PersistAsset, SourceKind};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use tempfile::tempdir;

#[test]
fn stores_connector_bytes_and_persists_libretro_provenance() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let bytes = b"libretro box front fixture";

    let stored = store.store_original_bytes(bytes).unwrap();
    let imported = catalog
        .persist_asset(PersistAsset {
            existing_game_id: None,
            game_title: "Super Mario Bros. (World)".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "Unknown".to_owned(),
            edition_name: "Unspecified".to_owned(),
            asset_type: AssetType::BoxFront,
            object_hash: stored.hash.clone(),
            byte_len: stored.byte_len,
            original_filename: "Super Mario Bros. (World).png".to_owned(),
            source_kind: SourceKind::LibretroThumbnails,
            source_location: "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System/master/Named_Boxarts/Super%20Mario%20Bros.%20(World).png".to_owned(),
        })
        .unwrap();

    assert_eq!(
        std::fs::read(store.object_path(&stored.hash)).unwrap(),
        bytes
    );

    let library = catalog.list_library().unwrap();
    let entry = library
        .iter()
        .find(|entry| entry.asset_id == imported.asset_id)
        .unwrap();
    assert_eq!(entry.provenance.len(), 1);
    assert_eq!(
        entry.provenance[0].source_kind,
        SourceKind::LibretroThumbnails
    );
    assert_eq!(
        entry.provenance[0].source_location,
        "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System/master/Named_Boxarts/Super%20Mario%20Bros.%20(World).png"
    );
}
