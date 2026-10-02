use std::fs;

use game_media_vault_application::{
    CatalogPort, CorruptObject, DerivativeRepositoryPort, DerivedStorePort, ObjectStorePort,
    verify_vault,
};
use game_media_vault_domain::{AssetType, DerivationRecipe, PersistAsset, SourceId};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use tempfile::tempdir;

const THUMBNAIL: DerivationRecipe = DerivationRecipe::Thumbnail { max_edge: 256 };

fn box_front(game_title: &str, object_hash: &str, byte_len: u64) -> PersistAsset {
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
        byte_len,
        media: game_media_vault_domain::MediaInfo::unknown(),
        original_filename: "front.png".to_owned(),
        source_id: SourceId::from("local_import"),
        source_asset_label: None,
        source_location: format!("C:/covers/{game_title}.png"),
    }
}

#[test]
fn verification_compares_the_catalog_with_the_stored_bytes() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let intact = store.store_original(&mut &b"intact cover"[..]).unwrap();
    let corrupt = store.store_original(&mut &b"cover to corrupt"[..]).unwrap();
    let missing = store.store_original(&mut &b"cover to lose"[..]).unwrap();
    let loose = store
        .store_original(&mut &b"below quality cover"[..])
        .unwrap();
    for (title, object) in [
        ("Intact", &intact),
        ("Corrupt", &corrupt),
        ("Missing", &missing),
    ] {
        catalog
            .persist_asset(box_front(title, &object.hash, object.byte_len))
            .unwrap();
    }
    let thumbnail = store.store_derived(&mut &b"intact thumbnail"[..]).unwrap();
    catalog
        .record_derivative(&intact.hash, &THUMBNAIL, &thumbnail)
        .unwrap();
    let stray = store.store_derived(&mut &b"stray thumbnail"[..]).unwrap();
    fs::write(store.object_path(&corrupt.hash), b"bit rot").unwrap();
    fs::remove_file(store.object_path(&missing.hash)).unwrap();
    fs::write(vault.join("staging").join("4242-0.tmp"), b"interrupted").unwrap();

    let report = verify_vault(&catalog, &store).unwrap();

    assert_eq!(report.missing_originals, [missing.hash]);
    assert_eq!(
        report.corrupt_originals,
        [CorruptObject {
            hash: corrupt.hash.clone(),
            actual_hash: blake3::hash(b"bit rot").to_hex().to_string(),
        }]
    );
    assert_eq!(report.unreferenced_originals, [loose.hash]);
    assert!(report.missing_derived.is_empty());
    assert_eq!(report.orphaned_derived, [stray.hash]);
    assert_eq!(report.interrupted_staging, ["4242-0.tmp"]);
    // Verifying repairs nothing.
    assert_eq!(
        fs::read(store.object_path(&corrupt.hash)).unwrap(),
        b"bit rot"
    );
    assert_eq!(catalog.list_library().unwrap().len(), 3);
}

#[test]
fn an_empty_vault_is_healthy() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);

    assert!(verify_vault(&catalog, &store).unwrap().is_healthy());
}
