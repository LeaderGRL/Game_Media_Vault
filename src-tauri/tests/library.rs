use std::fs;

use game_media_vault_application::{ImportLocalBoxFrontRequest, import_local_box_front};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use tempfile::tempdir;

#[test]
fn loads_an_asset_imported_into_the_selected_vault() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let source = temp.path().join("cover-front.png");
    fs::write(&source, b"tauri library cover bytes").unwrap();
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);

    import_local_box_front(
        &catalog,
        &store,
        ImportLocalBoxFrontRequest {
            existing_game_id: None,
            game_title: "Metal Gear Solid".to_owned(),
            platform: "PlayStation".to_owned(),
            region: "France".to_owned(),
            edition_name: "Original".to_owned(),
            source_path: source,
        },
    )
    .unwrap();

    let library = game_media_vault_tauri::load_library(&vault).unwrap();

    assert_eq!(library.len(), 1);
    assert_eq!(library[0].game_title, "Metal Gear Solid");
    assert_eq!(library[0].assets.len(), 1);
    assert_eq!(library[0].assets[0].original_filename, "cover-front.png");
}
