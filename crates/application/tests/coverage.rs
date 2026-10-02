mod support;

use game_media_vault_application::list_library;
use game_media_vault_domain::{
    AssetType, AssetTypeSelector::*, CoverageProfile, CoverageStatus, LibraryAsset, LibraryEntry,
    MediaInfo, PackagingFamily, ProfileCoverage, ReleaseCoverage,
};
use support::FakeVault;

fn box_front(asset_id: i64) -> LibraryAsset {
    asset(asset_id, AssetType::BoxFront)
}

fn asset(asset_id: i64, asset_type: AssetType) -> LibraryAsset {
    LibraryAsset {
        asset_id,
        asset_type,
        object_hash: format!("object-{asset_id}"),
        byte_len: 1_000,
        media: MediaInfo::unknown(),
        original_filename: "front.png".to_owned(),
        provenance: Vec::new(),
        derived: Vec::new(),
    }
}

fn coverage_of(platform: &str, assets: Vec<LibraryAsset>) -> Option<ReleaseCoverage> {
    let vault = FakeVault::with_library(vec![LibraryEntry {
        game_id: 1,
        game_title: "Super Mario World".to_owned(),
        release_edition_id: 2,
        platform: platform.to_owned(),
        region: "USA".to_owned(),
        edition_name: "Standard".to_owned(),
        assertions: Vec::new(),
        assets,
    }]);
    list_library(&vault).unwrap().remove(0).coverage
}

#[test]
fn a_cartridge_in_a_cardboard_box_lists_what_each_profile_misses() {
    let coverage = coverage_of(
        "Nintendo - Super Nintendo Entertainment System",
        vec![box_front(3)],
    );

    assert_eq!(
        coverage,
        Some(ReleaseCoverage {
            packaging_family: PackagingFamily::CardboardBox,
            status: CoverageStatus::Partial,
            profiles: vec![
                ProfileCoverage {
                    profile: CoverageProfile::Packaging,
                    required: vec![BoxFront, BoxBack, Spine],
                    missing: vec![BoxBack, Spine],
                },
                ProfileCoverage {
                    profile: CoverageProfile::Physical,
                    required: vec![BoxFront, BoxBack, Spine, Cartridge, Manual],
                    missing: vec![BoxBack, Spine, Cartridge, Manual],
                },
                ProfileCoverage {
                    profile: CoverageProfile::Archival,
                    required: vec![BoxFront, BoxBack, Spine, Cartridge, Manual, Insert],
                    missing: vec![BoxBack, Spine, Cartridge, Manual, Insert],
                },
            ],
        })
    );
}

#[test]
fn packaging_families_require_different_media() {
    let physical = |platform: &str| {
        let coverage = coverage_of(platform, Vec::new()).unwrap();
        let required = coverage
            .profiles
            .into_iter()
            .find(|profile| profile.profile == CoverageProfile::Physical)
            .unwrap()
            .required;
        (coverage.packaging_family, required)
    };

    assert_eq!(
        physical("Sony - PlayStation"),
        (
            PackagingFamily::JewelCase,
            vec![BoxFront, BoxBack, Spine, Disc, Manual]
        )
    );
    assert_eq!(
        physical("Nintendo - Nintendo DS"),
        (
            PackagingFamily::CartridgeCase,
            vec![BoxFront, BoxBack, Spine, Cartridge]
        )
    );
}

#[test]
fn coverage_rises_to_the_last_profile_whose_requirements_are_met() {
    // A digital release has no physical media: its cover art completes its packaging.
    let coverage = coverage_of("Nintendo - Nintendo 3DS (Digital)", vec![box_front(3)]).unwrap();

    assert_eq!(coverage.packaging_family, PackagingFamily::DigitalOnly);
    assert_eq!(coverage.status, CoverageStatus::PackagingComplete);
    assert_eq!(
        coverage
            .profiles
            .iter()
            .map(|profile| profile.profile)
            .collect::<Vec<_>>(),
        vec![CoverageProfile::Packaging, CoverageProfile::Archival]
    );
}

#[test]
fn coverage_of_an_unrecognized_platform_is_not_evaluated() {
    assert_eq!(coverage_of("Homebrew Console", vec![box_front(3)]), None);
}

#[test]
fn a_retained_cartridge_front_meets_the_cartridge_requirement() {
    let coverage = coverage_of(
        "Nintendo - Super Nintendo Entertainment System",
        vec![box_front(3), asset(4, AssetType::CartridgeFront)],
    )
    .unwrap();

    let physical = &coverage.profiles[1];
    assert_eq!(physical.profile, CoverageProfile::Physical);
    assert_eq!(physical.missing, vec![BoxBack, Spine, Manual]);
}

#[test]
fn a_cartridge_back_alone_does_not_meet_the_cartridge_requirement() {
    let coverage = coverage_of(
        "Nintendo - Nintendo DS",
        vec![asset(4, AssetType::CartridgeBack)],
    )
    .unwrap();

    assert!(coverage.profiles[1].missing.contains(&Cartridge));
}

#[test]
fn a_box_front_back_and_spine_complete_the_packaging_profile() {
    let coverage = coverage_of(
        "Nintendo - Super Nintendo Entertainment System",
        vec![
            box_front(3),
            asset(4, AssetType::BoxBack),
            asset(5, AssetType::Spine),
        ],
    )
    .unwrap();

    assert_eq!(coverage.status, CoverageStatus::PackagingComplete);
    assert!(coverage.profiles[0].missing.is_empty());
}

#[test]
fn an_arcade_board_with_its_marquee_and_circuit_board_is_physically_complete() {
    let coverage = coverage_of(
        "MAME",
        vec![
            asset(3, AssetType::Marquee),
            asset(4, AssetType::Pcb),
            asset(5, AssetType::Flyer),
            asset(6, AssetType::ControlPanel),
            // A cabinet is no requirement of any profile.
            asset(7, AssetType::ArcadeCabinet),
        ],
    )
    .unwrap();

    assert_eq!(coverage.status, CoverageStatus::PhysicalComplete);
    // A bezel is still missing for the Archival profile.
    assert_eq!(coverage.profiles[2].missing, [Bezel]);
}
