//! The Library a search pages through: its summaries and the entries of a page agree with the
//! whole projection, which the application derives them from by default.

use game_media_vault_application::{
    CatalogPort, DerivativeRepositoryPort, ReferenceCatalogRepositoryPort,
};
use game_media_vault_domain::{
    AssetType, DerivationRecipe, ImportedReleaseEdition, LibrarySummary, MediaInfo, PersistAsset,
    ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField, SourceId, StoredObject,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::{TempDir, tempdir};

const GAME_BOY: &str = "Nintendo - Game Boy";

fn png() -> MediaInfo {
    MediaInfo {
        media_type: "image/png".to_owned(),
        width: Some(640),
        height: Some(480),
        document: None,
    }
}

fn reference(source: &str, title: &str, region: &str) -> ReferenceReleaseRecord {
    let assertion = |field, qualifier: Option<&str>, value: &str| ReleaseAssertion {
        source_id: SourceId::from(source),
        source_location: format!("C:/catalogs/{source}.dat"),
        field,
        qualifier: qualifier.map(str::to_owned),
        value: value.to_owned(),
    };
    ReferenceReleaseRecord {
        game_title: title.to_owned(),
        platform: GAME_BOY.to_owned(),
        region: region.to_owned(),
        revision: None,
        edition_name: "Standard".to_owned(),
        assertions: vec![
            assertion(ReleaseAssertionField::Title, None, title),
            assertion(ReleaseAssertionField::Region, None, region),
            assertion(
                ReleaseAssertionField::Identifier,
                Some("source_record"),
                &format!("{title}|{region}"),
            ),
        ],
    }
}

fn asset_of(
    release: &ImportedReleaseEdition,
    title: &str,
    asset_type: AssetType,
    source: &str,
    object_hash: &str,
) -> PersistAsset {
    PersistAsset {
        existing_game_id: Some(release.game_id),
        existing_release_edition_id: Some(release.release_edition_id),
        match_decision: None,
        game_title: title.to_owned(),
        platform: GAME_BOY.to_owned(),
        region: "USA".to_owned(),
        edition_name: "Standard".to_owned(),
        asset_type,
        object_hash: object_hash.to_owned(),
        byte_len: 2_048,
        media: png(),
        original_filename: format!("{object_hash}.png"),
        source_id: SourceId::from(source),
        source_asset_label: None,
        source_location: format!("https://example.invalid/{object_hash}.png"),
    }
}

/// A catalog of three releases: one with Assets of two Sources, a derivative and the titles of
/// two catalogs; one with an Asset only; one with reference data only.
fn library() -> (TempDir, SqliteCatalog, [i64; 3]) {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let tetris = catalog
        .persist_reference_release(reference("no-intro", "Tetris", "USA"))
        .unwrap();
    // A second catalog titles the same release otherwise.
    let mut retitled = reference("libretro-database", "Tetris", "USA");
    retitled.assertions[0].value = "Tetris (World)".to_owned();
    catalog.persist_reference_release(retitled).unwrap();
    let wario = catalog
        .persist_reference_release(reference("no-intro", "Wario Land", "Europe"))
        .unwrap();
    let alleyway = catalog
        .persist_reference_release(reference("no-intro", "Alleyway", "Japan"))
        .unwrap();

    for asset in [
        asset_of(
            &tetris,
            "Tetris",
            AssetType::Screenshot,
            "libretro-thumbnails",
            "aaa",
        ),
        asset_of(&tetris, "Tetris", AssetType::BoxFront, "launchbox", "bbb"),
        asset_of(&tetris, "Tetris", AssetType::Screenshot, "launchbox", "ccc"),
        asset_of(&wario, "Wario Land", AssetType::BoxFront, "vgmaps", "ddd"),
    ] {
        catalog.persist_asset(asset).unwrap();
    }
    catalog
        .record_derivative(
            "bbb",
            &DerivationRecipe::Thumbnail { max_edge: 256 },
            &StoredObject {
                hash: "thumb-bbb".to_owned(),
                byte_len: 256,
                media: png(),
            },
        )
        .unwrap();

    let ids = [
        tetris.release_edition_id,
        wario.release_edition_id,
        alleyway.release_edition_id,
    ];
    (temp, catalog, ids)
}

#[test]
fn summaries_agree_with_the_whole_library() {
    let (_temp, catalog, _) = library();

    let whole: Vec<LibrarySummary> = catalog
        .list_library()
        .unwrap()
        .iter()
        .map(LibrarySummary::of)
        .collect();

    assert_eq!(catalog.list_library_summaries().unwrap(), whole);
    // The library is not empty, so the agreement covers Assets, Sources and titles.
    let tetris = whole
        .iter()
        .find(|summary| summary.game_title == "Tetris")
        .unwrap();
    assert_eq!(
        tetris.asset_types,
        [AssetType::Screenshot, AssetType::BoxFront]
    );
    assert_eq!(tetris.title_assertions.len(), 2);
}

#[test]
fn the_entries_of_a_page_agree_with_the_whole_library() {
    let (_temp, catalog, [tetris, _, alleyway]) = library();
    let mut expected = catalog.list_library().unwrap();
    expected.retain(|entry| [alleyway, tetris].contains(&entry.release_edition_id));

    let entries = catalog.library_entries_of(&[alleyway, tetris]).unwrap();

    assert_eq!(entries, expected);
    assert!(
        entries
            .iter()
            .flat_map(|entry| &entry.assets)
            .any(|asset| !asset.derived.is_empty())
    );
    assert!(catalog.library_entries_of(&[]).unwrap().is_empty());
}
