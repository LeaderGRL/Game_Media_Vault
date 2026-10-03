use std::path::{Path, PathBuf};

use game_media_vault_infrastructure::SqliteCatalog;
use game_media_vault_tauri::{ReferenceCatalogKind, ReferenceImportInput};
use tempfile::{TempDir, tempdir};

/// A vault where two releases of one catalog share their dumps with a release of another
/// catalog, which awaits review.
fn vault_awaiting_review() -> (TempDir, PathBuf) {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let sha1 = "1111111111111111111111111111111111111111";
    let datafile = |name: &str, games: &[&str]| {
        let path = temp.path().join(name);
        let entries: String = games
            .iter()
            .map(|game| {
                format!(
                    r#"<game name="{game} (World)"><rom name="{game}.gb" size="1" sha1="{sha1}"/></game>"#
                )
            })
            .collect();
        std::fs::write(
            &path,
            format!(
                "<datafile><header><name>Nintendo - Game Boy</name></header>{entries}</datafile>"
            ),
        )
        .unwrap();
        path
    };
    let import = |kind, file: &Path| {
        game_media_vault_tauri::import_reference_catalog_in_vault(
            &vault,
            ReferenceImportInput {
                kind,
                file: file.to_string_lossy().into_owned(),
                max_games: 10,
                mame_version: None,
            },
        )
        .unwrap();
    };
    import(
        ReferenceCatalogKind::NoIntro,
        &datafile("no-intro.dat", &["Game A", "Game B"]),
    );
    import(
        ReferenceCatalogKind::Redump,
        &datafile("redump.dat", &["Game C"]),
    );
    (temp, vault)
}

#[test]
fn the_desktop_lists_reference_review_items_with_the_editions_they_name() {
    let (_temp, vault) = vault_awaiting_review();

    let items = game_media_vault_tauri::load_reference_review_items(&vault).unwrap();

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].item.source_id.as_str(), "redump");
    let titles: Vec<&str> = items[0]
        .editions
        .iter()
        .map(|edition| edition.game_title.as_str())
        .collect();
    assert_eq!(titles, ["Game C", "Game A", "Game B"]);
}

#[test]
fn the_desktop_links_a_reference_review_item_to_one_of_its_candidates() {
    let (_temp, vault) = vault_awaiting_review();
    let item = game_media_vault_tauri::load_reference_review_items(&vault).unwrap()[0]
        .item
        .clone();

    let remaining = game_media_vault_tauri::link_reference_review_item_in_vault(
        &vault,
        item.id,
        item.candidates[0],
    )
    .unwrap();

    assert!(remaining.is_empty());
    assert_eq!(
        game_media_vault_tauri::load_library(&vault).unwrap().len(),
        2
    );
}

#[test]
fn the_desktop_keeps_a_reference_review_item_apart() {
    let (_temp, vault) = vault_awaiting_review();
    let item = game_media_vault_tauri::load_reference_review_items(&vault).unwrap()[0]
        .item
        .clone();

    let remaining =
        game_media_vault_tauri::keep_reference_review_item_apart_in_vault(&vault, item.id).unwrap();
    let error = game_media_vault_tauri::keep_reference_review_item_apart_in_vault(&vault, item.id)
        .unwrap_err();

    assert!(remaining.is_empty());
    assert_eq!(
        game_media_vault_tauri::load_library(&vault).unwrap().len(),
        3
    );
    assert_eq!(error.kind, "not_found");
}
