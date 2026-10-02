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
    assert_eq!(library[0].entry.game_title, "Metal Gear Solid");
    assert_eq!(library[0].entry.assets.len(), 1);
    assert_eq!(
        library[0].entry.assets[0].original_filename,
        "cover-front.png"
    );
}

#[test]
fn searches_the_selected_vault_with_the_shared_library_query() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    for (title, file) in [("Metal Gear Solid", "mgs.png"), ("Vagrant Story", "vs.png")] {
        let source = temp.path().join(file);
        fs::write(&source, title.as_bytes()).unwrap();
        import_local_box_front(
            &catalog,
            &store,
            ImportLocalBoxFrontRequest {
                existing_game_id: None,
                game_title: title.to_owned(),
                platform: "Sony - PlayStation".to_owned(),
                region: "France".to_owned(),
                edition_name: "Original".to_owned(),
                source_path: source,
            },
        )
        .unwrap();
    }

    let page = game_media_vault_tauri::search_library_in_vault(
        &vault,
        &game_media_vault_application::LibraryQuery {
            text: Some("story".to_owned()),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(page.total, 1);
    assert_eq!(page.releases[0].entry.game_title, "Vagrant Story");
}

#[test]
fn renders_thumbnails_of_the_selected_vault_off_the_calling_thread() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let source = temp.path().join("cover-front.png");
    image::RgbImage::from_pixel(640, 480, image::Rgb([10, 20, 30]))
        .save(&source)
        .unwrap();
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    import_local_box_front(
        &catalog,
        &ContentAddressedStore::new(&vault),
        ImportLocalBoxFrontRequest {
            existing_game_id: None,
            game_title: "Metal Gear Solid".to_owned(),
            platform: "Sony - PlayStation".to_owned(),
            region: "France".to_owned(),
            edition_name: "Original".to_owned(),
            source_path: source,
        },
    )
    .unwrap();

    let summary = tauri::async_runtime::block_on(
        game_media_vault_tauri::derive_thumbnails_in_vault_async(vault.clone(), 160),
    )
    .unwrap();

    assert_eq!(summary.derived, 1);
    let library = game_media_vault_tauri::load_library(&vault).unwrap();
    let thumbnail = &library[0].entry.assets[0].derived[0];
    assert_eq!(
        (thumbnail.media.width, thumbnail.media.height),
        (Some(160), Some(120))
    );
}
