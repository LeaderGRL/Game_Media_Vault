mod support;

use game_media_vault_application::list_library;
use game_media_vault_domain::{
    AssetPreference, AssetType, LibraryAsset, LibraryEntry, MediaInfo, PreferenceReason,
    PreferredAsset,
};
use support::FakeVault;

fn original(asset_id: i64, width: Option<u32>, height: Option<u32>, byte_len: u64) -> LibraryAsset {
    LibraryAsset {
        asset_id,
        asset_type: AssetType::BoxFront,
        object_hash: format!("object-{asset_id}"),
        byte_len,
        media: MediaInfo {
            media_type: "image/png".to_owned(),
            width,
            height,
            document: None,
        },
        original_filename: format!("front-{asset_id}.png"),
        provenance: Vec::new(),
        derived: Vec::new(),
    }
}

fn preferred_assets(assets: Vec<LibraryAsset>) -> Vec<PreferredAsset> {
    let vault = FakeVault::with_library(vec![LibraryEntry {
        game_id: 1,
        game_title: "Tetris".to_owned(),
        release_edition_id: 2,
        platform: "Nintendo - Game Boy".to_owned(),
        region: "World".to_owned(),
        edition_name: "Rev 1".to_owned(),
        assertions: Vec::new(),
        assets,
    }]);
    list_library(&vault).unwrap().remove(0).preferred_assets
}

#[test]
fn the_original_with_the_most_pixels_is_preferred_and_says_why() {
    let preferred = preferred_assets(vec![
        original(3, Some(640), Some(900), 90_000),
        original(4, Some(1200), Some(1600), 400_000),
        original(5, None, None, 800_000),
    ]);

    assert_eq!(
        preferred,
        vec![PreferredAsset {
            asset_type: AssetType::BoxFront,
            asset_id: 4,
            outranks: vec![
                AssetPreference {
                    asset_id: 3,
                    reason: PreferenceReason::MorePixels {
                        preferred: 1_920_000,
                        other: Some(576_000),
                    },
                },
                AssetPreference {
                    asset_id: 5,
                    reason: PreferenceReason::MorePixels {
                        preferred: 1_920_000,
                        other: None,
                    },
                },
            ],
        }]
    );
}

#[test]
fn ties_go_to_more_bytes_then_to_the_original_acquired_first() {
    let preferred = preferred_assets(vec![
        original(7, Some(640), Some(480), 50_000),
        original(8, Some(640), Some(480), 120_000),
        original(9, Some(640), Some(480), 50_000),
    ]);

    assert_eq!(preferred[0].asset_id, 8);
    assert_eq!(
        preferred[0].outranks,
        vec![
            AssetPreference {
                asset_id: 7,
                reason: PreferenceReason::MoreBytes {
                    preferred: 120_000,
                    other: 50_000,
                },
            },
            AssetPreference {
                asset_id: 9,
                reason: PreferenceReason::MoreBytes {
                    preferred: 120_000,
                    other: 50_000,
                },
            },
        ]
    );
    let equal = preferred_assets(vec![
        original(9, Some(640), Some(480), 50_000),
        original(7, Some(640), Some(480), 50_000),
    ]);
    assert_eq!(equal[0].asset_id, 7);
    assert_eq!(equal[0].outranks[0].reason, PreferenceReason::AcquiredFirst);
}

#[test]
fn a_lone_original_is_preferred_and_a_release_without_originals_has_none() {
    assert_eq!(
        preferred_assets(vec![original(10, None, None, 1_000)]),
        vec![PreferredAsset {
            asset_type: AssetType::BoxFront,
            asset_id: 10,
            outranks: Vec::new(),
        }]
    );
    assert!(preferred_assets(Vec::new()).is_empty());
}
