use game_media_vault_application::CatalogPort;
use game_media_vault_domain::{AssetType, MediaInfo, PersistAsset, SourceId};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::tempdir;

#[test]
fn every_stored_asset_type_survives_the_catalog() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    for (index, asset_type) in AssetType::ALL.into_iter().enumerate() {
        catalog
            .persist_asset(PersistAsset {
                existing_game_id: None,
                existing_release_edition_id: None,
                match_decision: None,
                game_title: "Super Mario Bros.".to_owned(),
                platform: "Nintendo - Nintendo Entertainment System".to_owned(),
                region: "USA".to_owned(),
                edition_name: "Standard".to_owned(),
                asset_type,
                object_hash: format!("{index:064x}"),
                byte_len: 1,
                media: MediaInfo::unknown(),
                original_filename: format!("{}.png", asset_type.as_str()),
                source_id: SourceId::from("local_import"),
                source_asset_label: None,
                source_location: format!("C:/media/{}.png", asset_type.as_str()),
            })
            .unwrap();
    }

    let stored: Vec<AssetType> = catalog
        .list_library()
        .unwrap()
        .iter()
        .flat_map(|entry| entry.assets.iter().map(|asset| asset.asset_type))
        .collect();

    assert_eq!(stored.len(), AssetType::ALL.len());
    for asset_type in AssetType::ALL {
        assert!(stored.contains(&asset_type), "{asset_type:?}");
    }
}
