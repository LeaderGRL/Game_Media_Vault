use std::path::{Path, PathBuf};

use game_media_vault_infrastructure::SqliteCatalog;
use game_media_vault_tauri::{ReferenceCatalogKind, ReferenceImportInput};
use tempfile::tempdir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("crates")
        .join("connectors")
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn input(kind: ReferenceCatalogKind, file: &str) -> ReferenceImportInput {
    ReferenceImportInput {
        kind,
        file: fixture(file).to_string_lossy().into_owned(),
        max_games: 10,
        mame_version: None,
    }
}

#[test]
fn the_desktop_imports_each_kind_of_reference_catalog_into_the_opened_vault() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();

    let no_intro = game_media_vault_tauri::import_reference_catalog_in_vault(
        &vault,
        input(ReferenceCatalogKind::NoIntro, "no_intro_sample.dat"),
    )
    .unwrap();
    let redump = game_media_vault_tauri::import_reference_catalog_in_vault(
        &vault,
        input(ReferenceCatalogKind::Redump, "redump_sample.dat"),
    )
    .unwrap();
    let mame = game_media_vault_tauri::import_reference_catalog_in_vault(
        &vault,
        ReferenceImportInput {
            mame_version: Some("0.268".to_owned()),
            ..input(
                ReferenceCatalogKind::MameSoftwareList,
                "mame_nes_sample.xml",
            )
        },
    )
    .unwrap();

    assert!(no_intro.imported_releases > 0);
    assert!(redump.imported_releases > 0);
    assert_eq!((mame.imported_releases, mame.skipped_records), (3, 2));
    let library = game_media_vault_tauri::load_library(&vault).unwrap();
    let mame_versions: Vec<&str> = library
        .iter()
        .flat_map(|release| &release.entry.assertions)
        .filter(|assertion| assertion.qualifier.as_deref() == Some("mame_version"))
        .map(|assertion| assertion.value.as_str())
        .collect();
    assert_eq!(mame_versions, ["0.268"; 3]);
}

#[test]
fn an_invalid_reference_catalog_changes_nothing() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let broken = temp.path().join("broken.dat");
    std::fs::write(&broken, "<datafile><header>").unwrap();

    let error = game_media_vault_tauri::import_reference_catalog_in_vault(
        &vault,
        ReferenceImportInput {
            kind: ReferenceCatalogKind::NoIntro,
            file: broken.to_string_lossy().into_owned(),
            max_games: 10,
            mame_version: None,
        },
    )
    .unwrap_err();

    assert_eq!(error.kind, "source_failure");
    assert!(
        game_media_vault_tauri::load_library(&vault)
            .unwrap()
            .is_empty()
    );
}
