use game_media_vault_application::{CatalogPort, DerivativeRepositoryPort, OriginalObject};
use game_media_vault_domain::{
    AssetType, DerivationRecipe, DerivedAsset, MediaInfo, PersistAsset, SourceId, StoredObject,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::tempdir;

const THUMBNAIL: DerivationRecipe = DerivationRecipe::Thumbnail { max_edge: 256 };

fn png(width: u32, height: u32) -> MediaInfo {
    MediaInfo {
        media_type: "image/png".to_owned(),
        width: Some(width),
        height: Some(height),
        document: None,
    }
}

fn box_front(game_title: &str, object_hash: &str) -> PersistAsset {
    PersistAsset {
        existing_game_id: None,
        existing_release_edition_id: None,
        match_decision: None,
        game_title: game_title.to_owned(),
        platform: "Sony - PlayStation".to_owned(),
        region: "France".to_owned(),
        edition_name: "Original".to_owned(),
        asset_type: AssetType::BoxFront,
        object_hash: object_hash.to_owned(),
        byte_len: 4_096,
        media: png(1200, 1600),
        original_filename: "front.png".to_owned(),
        source_id: SourceId::from("local_import"),
        source_asset_label: None,
        source_location: format!("C:/covers/{game_title}.png"),
    }
}

#[test]
fn originals_lacking_a_recipe_are_listed_once_until_their_output_is_recorded() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    catalog
        .persist_asset(box_front("Metal Gear Solid", "aaa"))
        .unwrap();
    // Identical bytes retained for another release share one original.
    catalog
        .persist_asset(box_front("Metal Gear Solid Integral", "aaa"))
        .unwrap();
    catalog
        .persist_asset(box_front("Vagrant Story", "bbb"))
        .unwrap();

    assert_eq!(
        catalog.originals_without(&THUMBNAIL).unwrap(),
        vec![
            OriginalObject {
                hash: "aaa".to_owned(),
                media_types: vec!["image/png".to_owned()],
            },
            OriginalObject {
                hash: "bbb".to_owned(),
                media_types: vec!["image/png".to_owned()],
            },
        ]
    );

    let thumbnail = StoredObject {
        hash: "thumb-aaa".to_owned(),
        byte_len: 512,
        media: png(192, 256),
    };
    catalog
        .record_derivative("aaa", &THUMBNAIL, &thumbnail)
        .unwrap();
    // Recording the same output again changes nothing.
    catalog
        .record_derivative("aaa", &THUMBNAIL, &thumbnail)
        .unwrap();

    assert_eq!(
        catalog
            .originals_without(&THUMBNAIL)
            .unwrap()
            .into_iter()
            .map(|original| original.hash)
            .collect::<Vec<_>>(),
        ["bbb"]
    );
    let derived = DerivedAsset {
        recipe: THUMBNAIL,
        object_hash: "thumb-aaa".to_owned(),
        byte_len: 512,
        media: png(192, 256),
    };
    let library = catalog.list_library().unwrap();
    let derived_by_title: Vec<(String, Vec<DerivedAsset>)> = library
        .into_iter()
        .map(|entry| (entry.game_title, entry.assets[0].derived.clone()))
        .collect();
    assert_eq!(
        derived_by_title,
        vec![
            ("Metal Gear Solid".to_owned(), vec![derived.clone()]),
            ("Metal Gear Solid Integral".to_owned(), vec![derived]),
            ("Vagrant Story".to_owned(), Vec::new()),
        ]
    );
}

#[test]
fn an_original_lists_every_media_type_recorded_for_its_bytes() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let mut unidentified = box_front("Metal Gear Solid", "aaa");
    unidentified.media = MediaInfo::unknown();
    catalog.persist_asset(unidentified).unwrap();
    catalog
        .persist_asset(box_front("Metal Gear Solid Integral", "aaa"))
        .unwrap();

    assert_eq!(
        catalog.originals_without(&THUMBNAIL).unwrap(),
        vec![OriginalObject {
            hash: "aaa".to_owned(),
            media_types: vec![
                "application/octet-stream".to_owned(),
                "image/png".to_owned()
            ],
        }]
    );
}

#[test]
fn the_assets_retained_last_are_read_alone_with_their_releases_and_derivatives() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    catalog
        .persist_asset(box_front("Metal Gear Solid", "aaa"))
        .unwrap();
    catalog.persist_asset(box_front("Wipeout", "ccc")).unwrap();
    catalog
        .persist_asset(box_front("Vagrant Story", "bbb"))
        .unwrap();
    let thumbnail = StoredObject {
        hash: "thumb-ccc".to_owned(),
        byte_len: 512,
        media: png(192, 256),
    };
    catalog
        .record_derivative("ccc", &THUMBNAIL, &thumbnail)
        .unwrap();

    let latest = catalog.list_latest_assets(2).unwrap();

    // The two Assets retained last, each with its release and derivatives, and no other.
    assert_eq!(
        latest
            .iter()
            .map(|entry| (entry.game_title.as_str(), entry.assets.len()))
            .collect::<Vec<_>>(),
        [("Vagrant Story", 1), ("Wipeout", 1)]
    );
    assert_eq!(
        latest[1].assets[0]
            .derived
            .iter()
            .map(|derived| derived.object_hash.as_str())
            .collect::<Vec<_>>(),
        ["thumb-ccc"]
    );
    assert!(latest[0].assets[0].derived.is_empty());
}
