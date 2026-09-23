use game_media_vault_application::{CatalogPort, ObjectStorePort};
use game_media_vault_domain::{AssetType, PersistAsset, SourceId};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use tempfile::tempdir;

#[test]
fn stores_connector_bytes_and_round_trips_a_data_driven_source_id() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let bytes = b"libretro box front fixture";

    let stored = store.store_original_bytes(bytes).unwrap();
    let imported = catalog
        .persist_asset(PersistAsset {
            existing_game_id: None,
            existing_release_edition_id: None,
            game_title: "Super Mario Bros. (World)".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "Unknown".to_owned(),
            edition_name: "Unspecified".to_owned(),
            asset_type: AssetType::BoxFront,
            object_hash: stored.hash.clone(),
            byte_len: stored.byte_len,
            original_filename: "Super Mario Bros. (World).png".to_owned(),
            source_id: SourceId::from("provider-added-without-domain-change"),
            source_asset_label: Some("Named_Boxarts".to_owned()),
            source_location: "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System/master/Named_Boxarts/Super%20Mario%20Bros.%20(World).png".to_owned(),
        })
        .unwrap();

    assert_eq!(
        std::fs::read(store.object_path(&stored.hash)).unwrap(),
        bytes
    );

    let library = catalog.list_library().unwrap();
    let asset = library
        .iter()
        .flat_map(|entry| entry.assets.iter())
        .find(|asset| asset.asset_id == imported.asset_id)
        .unwrap();
    assert_eq!(asset.provenance.len(), 1);
    assert_eq!(
        asset.provenance[0].source_id,
        SourceId::from("provider-added-without-domain-change")
    );
    assert_eq!(
        asset.provenance[0].source_asset_label.as_deref(),
        Some("Named_Boxarts")
    );
    assert_eq!(
        asset.provenance[0].source_location,
        "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System/master/Named_Boxarts/Super%20Mario%20Bros.%20(World).png"
    );
}

#[test]
fn explicit_release_target_attaches_asset_to_that_release() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();

    let existing = catalog
        .persist_asset(PersistAsset {
            existing_game_id: None,
            existing_release_edition_id: None,
            game_title: "Target Game".to_owned(),
            platform: "Nintendo Entertainment System".to_owned(),
            region: "USA".to_owned(),
            edition_name: "Standard".to_owned(),
            asset_type: AssetType::BoxFront,
            object_hash: "existing-hash".to_owned(),
            byte_len: 10,
            original_filename: "existing.png".to_owned(),
            source_id: SourceId::from("fixture"),
            source_asset_label: None,
            source_location: "fixture://existing".to_owned(),
        })
        .unwrap();

    let imported = catalog
        .persist_asset(PersistAsset {
            existing_game_id: Some(existing.game_id),
            existing_release_edition_id: Some(existing.release_edition_id),
            game_title: "Target Game".to_owned(),
            platform: "Nintendo Entertainment System".to_owned(),
            region: "Europe".to_owned(),
            edition_name: "Collector".to_owned(),
            asset_type: AssetType::BoxFront,
            object_hash: "matched-hash".to_owned(),
            byte_len: 11,
            original_filename: "matched.png".to_owned(),
            source_id: SourceId::from("fixture"),
            source_asset_label: None,
            source_location: "fixture://matched".to_owned(),
        })
        .unwrap();

    assert_eq!(imported.game_id, existing.game_id);
    assert_eq!(imported.release_edition_id, existing.release_edition_id);
    assert_eq!(catalog.list_library().unwrap().len(), 1);
}

#[test]
fn explicit_release_target_is_validated_before_duplicate_lookup() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let existing_record = PersistAsset {
        existing_game_id: None,
        existing_release_edition_id: None,
        game_title: "Target Game".to_owned(),
        platform: "Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Standard".to_owned(),
        asset_type: AssetType::BoxFront,
        object_hash: "shared-hash".to_owned(),
        byte_len: 10,
        original_filename: "front.png".to_owned(),
        source_id: SourceId::from("fixture"),
        source_asset_label: None,
        source_location: "fixture://shared".to_owned(),
    };
    let existing = catalog.persist_asset(existing_record.clone()).unwrap();

    let error = catalog
        .persist_asset(PersistAsset {
            existing_game_id: Some(existing.game_id),
            existing_release_edition_id: Some(999_999),
            ..existing_record
        })
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("release edition #999999 does not exist")
    );
}

#[test]
fn explicit_release_target_scopes_duplicate_lookup_to_that_release() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let existing_record = PersistAsset {
        existing_game_id: None,
        existing_release_edition_id: None,
        game_title: "Target Game".to_owned(),
        platform: "Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Standard".to_owned(),
        asset_type: AssetType::BoxFront,
        object_hash: "shared-hash".to_owned(),
        byte_len: 10,
        original_filename: "front.png".to_owned(),
        source_id: SourceId::from("fixture"),
        source_asset_label: None,
        source_location: "fixture://shared".to_owned(),
    };
    let existing = catalog.persist_asset(existing_record.clone()).unwrap();
    let target = catalog
        .persist_asset(PersistAsset {
            existing_game_id: Some(existing.game_id),
            existing_release_edition_id: None,
            region: "Europe".to_owned(),
            edition_name: "Collector".to_owned(),
            object_hash: "target-seed-hash".to_owned(),
            source_location: "fixture://target-seed".to_owned(),
            ..existing_record.clone()
        })
        .unwrap();

    let imported = catalog
        .persist_asset(PersistAsset {
            existing_game_id: Some(existing.game_id),
            existing_release_edition_id: Some(target.release_edition_id),
            ..existing_record
        })
        .unwrap();

    assert_eq!(imported.release_edition_id, target.release_edition_id);
    assert_ne!(imported.asset_id, existing.asset_id);
}

