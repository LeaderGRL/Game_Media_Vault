mod support;

use game_media_vault_application::{LibraryQuery, LibraryStatus, search_library};
use game_media_vault_domain::{
    AssetProvenance, AssetType, AssetTypeSelector, LibraryAsset, LibraryEntry, MediaInfo,
    ReviewItem, ReviewStatus, SourceId,
};
use support::{FakeVault, ambiguous_candidate_and_releases};

fn release(id: i64, title: &str, platform: &str, region: &str) -> LibraryEntry {
    LibraryEntry {
        game_id: id + 1_000,
        game_title: title.to_owned(),
        release_edition_id: id,
        platform: platform.to_owned(),
        region: region.to_owned(),
        edition_name: "Standard".to_owned(),
        assertions: Vec::new(),
        assets: Vec::new(),
    }
}

fn with_asset(mut entry: LibraryEntry, asset_type: AssetType, source_id: &str) -> LibraryEntry {
    entry.assets.push(LibraryAsset {
        asset_id: entry.release_edition_id * 10 + entry.assets.len() as i64,
        asset_type,
        object_hash: format!("object-{}-{}", entry.release_edition_id, entry.assets.len()),
        byte_len: 1_000,
        media: MediaInfo::unknown(),
        original_filename: "original.png".to_owned(),
        provenance: vec![AssetProvenance {
            source_id: SourceId::from(source_id),
            source_asset_label: None,
            source_location: "https://example.invalid/original.png".to_owned(),
            match_decision: None,
        }],
    });
    entry
}

const SNES: &str = "Nintendo - Super Nintendo Entertainment System";
const DS_DIGITAL: &str = "Nintendo - Nintendo DSi (Digital)";

fn library() -> Vec<LibraryEntry> {
    vec![
        with_asset(
            release(1, "Super Mario World", SNES, "USA"),
            AssetType::BoxFront,
            "libretro-thumbnails",
        ),
        release(2, "Super Mario World", SNES, "Europe"),
        with_asset(
            release(3, "Flipnote Studio", DS_DIGITAL, "USA"),
            AssetType::BoxFront,
            "local_import",
        ),
        release(4, "Homebrew Quest", "Homebrew Console", "World"),
    ]
}

fn ids(vault: &FakeVault, query: LibraryQuery) -> Vec<i64> {
    search_library(vault, vault, &query)
        .unwrap()
        .releases
        .iter()
        .map(|release| release.entry.release_edition_id)
        .collect()
}

#[test]
fn filters_narrow_across_kinds_and_widen_within_one() {
    let vault = FakeVault::with_library(library());

    assert_eq!(
        ids(
            &vault,
            LibraryQuery {
                text: Some("mario".to_owned()),
                regions: vec!["usa".to_owned(), "Europe".to_owned()],
                ..LibraryQuery::default()
            }
        ),
        vec![2, 1]
    );
    assert_eq!(
        ids(
            &vault,
            LibraryQuery {
                platforms: vec![SNES.to_owned()],
                asset_types: vec![AssetTypeSelector::BoxFront],
                ..LibraryQuery::default()
            }
        ),
        vec![1]
    );
    assert_eq!(
        ids(
            &vault,
            LibraryQuery {
                sources: vec!["local_import".to_owned()],
                ..LibraryQuery::default()
            }
        ),
        vec![3]
    );
}

#[test]
fn complete_and_partial_follow_coverage() {
    let vault = FakeVault::with_library(library());

    // The digital release's cover art completes its packaging; the unknown platform is
    // neither complete nor partial.
    assert_eq!(
        ids(
            &vault,
            LibraryQuery {
                statuses: vec![LibraryStatus::Complete],
                ..LibraryQuery::default()
            }
        ),
        vec![3]
    );
    assert_eq!(
        ids(
            &vault,
            LibraryQuery {
                statuses: vec![LibraryStatus::Partial],
                ..LibraryQuery::default()
            }
        ),
        vec![2, 1]
    );
}

#[test]
fn needs_review_finds_the_releases_an_undecided_item_offers() {
    let (candidate, releases) = ambiguous_candidate_and_releases();
    let vault = FakeVault::with_library(releases);
    let competing_matches = game_media_vault_domain::review_matches_for_asset_candidate(
        &candidate,
        &vault.library.borrow(),
        support::matching_policy().validate().unwrap(),
    );
    vault.review_items.borrow_mut().push(ReviewItem {
        id: 1,
        candidate_identity: "candidate:review".to_owned(),
        candidate,
        competing_matches,
        decision: None,
        status: ReviewStatus::Pending,
    });

    assert_eq!(
        ids(
            &vault,
            LibraryQuery {
                statuses: vec![LibraryStatus::NeedsReview],
                ..LibraryQuery::default()
            }
        ),
        vec![402, 401]
    );
}

#[test]
fn pages_follow_a_stable_order_and_resume_after_their_cursor() {
    let vault = FakeVault::with_library(library());
    let first = search_library(
        &vault,
        &vault,
        &LibraryQuery {
            limit: 2,
            ..LibraryQuery::default()
        },
    )
    .unwrap();
    assert_eq!(first.total, 4);
    assert_eq!(
        first
            .releases
            .iter()
            .map(|release| release.entry.release_edition_id)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );

    // A release imported between two pages sorts before the cursor and is not repeated.
    vault
        .library
        .borrow_mut()
        .push(release(5, "Aardvark Adventure", SNES, "USA"));
    let second = search_library(
        &vault,
        &vault,
        &LibraryQuery {
            limit: 2,
            after: first.next_after,
            ..LibraryQuery::default()
        },
    )
    .unwrap();

    assert_eq!(
        second
            .releases
            .iter()
            .map(|release| release.entry.release_edition_id)
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert_eq!(second.next_after, None);
}
