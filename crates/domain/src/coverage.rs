use serde::{Deserialize, Serialize};

use crate::{AssetTypeSelector, LibraryAsset};

/// How a Release Edition is packaged, which decides what its Coverage Profiles require.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackagingFamily {
    /// A cartridge in a cardboard box, as on the NES, SNES, Nintendo 64 and Game Boy.
    CardboardBox,
    /// A disc in a CD jewel case, as on the PlayStation and Saturn.
    JewelCase,
    /// A disc in a DVD-style keep case, as on the PlayStation 2, GameCube and Xbox.
    KeepCase,
    /// A cartridge or card in a plastic case, as on the Nintendo DS, 3DS and Switch.
    CartridgeCase,
    /// An arcade board, with no retail packaging.
    ArcadeBoard,
    /// A digital release, with no physical media.
    DigitalOnly,
}

/// What a Release Edition must have to be complete for a purpose; each profile includes the
/// requirements of the previous ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageProfile {
    /// Enough to reconstruct the external package.
    Packaging,
    /// The packaging plus the primary physical media and expected physical inserts.
    Physical,
    /// Every known collectible media category of the edition.
    Archival,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Partial,
    PackagingComplete,
    PhysicalComplete,
    ArchivalComplete,
}

/// The Asset Types one Coverage Profile requires and those the edition still lacks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileCoverage {
    pub profile: CoverageProfile,
    pub required: Vec<AssetTypeSelector>,
    pub missing: Vec<AssetTypeSelector>,
}

/// The coverage of a Release Edition: its packaging family, the last profile it completes and
/// what every profile that applies to the family still misses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseCoverage {
    pub packaging_family: PackagingFamily,
    pub status: CoverageStatus,
    pub profiles: Vec<ProfileCoverage>,
}

/// Platforms whose packaging family is known, by the names No-Intro, Redump and Libretro give
/// them.
const PLATFORM_FAMILIES: &[(PackagingFamily, &[&str])] = &[
    (
        PackagingFamily::CardboardBox,
        &[
            "Atari - 2600",
            "Atari - 5200",
            "Atari - 7800",
            "Nintendo - Game Boy",
            "Nintendo - Game Boy Advance",
            "Nintendo - Game Boy Color",
            "Nintendo - Nintendo 64",
            "Nintendo - Nintendo Entertainment System",
            "Nintendo - Super Nintendo Entertainment System",
            "Nintendo - Virtual Boy",
        ],
    ),
    (
        PackagingFamily::JewelCase,
        &[
            "NEC - PC Engine CD - TurboGrafx-CD",
            "Philips - CD-i",
            "Sega - Dreamcast",
            "Sega - Mega-CD - Sega CD",
            "Sega - Saturn",
            "SNK - Neo Geo CD",
            "Sony - PlayStation",
            "The 3DO Company - 3DO",
        ],
    ),
    (
        PackagingFamily::KeepCase,
        &[
            "Microsoft - Xbox",
            "Microsoft - Xbox 360",
            "Microsoft - Xbox One",
            "Nintendo - GameCube",
            "Nintendo - Wii",
            "Nintendo - Wii U",
            "Sony - PlayStation 2",
            "Sony - PlayStation 3",
            "Sony - PlayStation 4",
            "Sony - PlayStation Portable",
        ],
    ),
    (
        PackagingFamily::CartridgeCase,
        &[
            "Nintendo - Nintendo 3DS",
            "Nintendo - Nintendo DS",
            "Nintendo - Nintendo DSi",
            "Nintendo - Nintendo Switch",
            "Sony - PlayStation Vita",
        ],
    ),
    (
        PackagingFamily::ArcadeBoard,
        &["FBNeo - Arcade Games", "MAME"],
    ),
];

/// Suffixes that mark the digital release catalogs of a platform.
const DIGITAL_SUFFIXES: &[&str] = &["(Digital)", "(PSN)"];

/// Every platform whose packaging family is known, named as No-Intro, Redump and Libretro name
/// it, with that family. Digital catalogs, known by their suffix, are left out.
pub fn packaging_platforms() -> impl Iterator<Item = (&'static str, PackagingFamily)> {
    PLATFORM_FAMILIES
        .iter()
        .flat_map(|(family, platforms)| platforms.iter().map(move |platform| (*platform, *family)))
}

/// The packaging family of releases on `platform`, when it is known.
pub fn packaging_family(platform: &str) -> Option<PackagingFamily> {
    let platform = platform.trim();
    if DIGITAL_SUFFIXES
        .iter()
        .any(|suffix| platform.ends_with(suffix))
    {
        return Some(PackagingFamily::DigitalOnly);
    }
    PLATFORM_FAMILIES
        .iter()
        .find(|(_, platforms)| {
            platforms
                .iter()
                .any(|known| known.eq_ignore_ascii_case(platform))
        })
        .map(|(family, _)| *family)
}

/// What each profile that applies to `family` adds to the previous ones.
fn profile_additions(
    family: PackagingFamily,
) -> &'static [(CoverageProfile, &'static [AssetTypeSelector])] {
    use AssetTypeSelector::*;
    use CoverageProfile as Profile;
    match family {
        PackagingFamily::CardboardBox => &[
            (Profile::Packaging, &[BoxFront, BoxBack, Spine]),
            (Profile::Physical, &[Cartridge, Manual]),
            (Profile::Archival, &[Insert]),
        ],
        PackagingFamily::JewelCase => &[
            (Profile::Packaging, &[BoxFront, BoxBack, Spine]),
            (Profile::Physical, &[Disc, Manual]),
            (Profile::Archival, &[Insert]),
        ],
        PackagingFamily::KeepCase => &[
            (Profile::Packaging, &[BoxFront, BoxBack, Spine]),
            (Profile::Physical, &[Disc]),
            (Profile::Archival, &[Manual, Insert]),
        ],
        PackagingFamily::CartridgeCase => &[
            (Profile::Packaging, &[BoxFront, BoxBack, Spine]),
            (Profile::Physical, &[Cartridge]),
            (Profile::Archival, &[Manual, Insert]),
        ],
        PackagingFamily::ArcadeBoard => &[
            (Profile::Packaging, &[Marquee]),
            (Profile::Physical, &[Pcb]),
            (Profile::Archival, &[Flyer, ControlPanel, Bezel]),
        ],
        // A digital release has no physical media, so no Physical profile applies.
        PackagingFamily::DigitalOnly => &[
            (Profile::Packaging, &[BoxFront]),
            (Profile::Archival, &[Logo, Icon]),
        ],
    }
}

/// Evaluates the Coverage Profiles of a release on `platform` against its retained Assets;
/// `None` when the platform's packaging family is unknown, since no universal checklist fits.
pub fn release_coverage(platform: &str, assets: &[LibraryAsset]) -> Option<ReleaseCoverage> {
    let packaging_family = packaging_family(platform)?;
    // The front of a cartridge shows the cartridge itself, which the checklists require.
    let present: Vec<AssetTypeSelector> = assets
        .iter()
        .flat_map(|asset| {
            let selector = asset.asset_type.selector();
            let shown = (selector == AssetTypeSelector::CartridgeFront)
                .then_some(AssetTypeSelector::Cartridge);
            std::iter::once(selector).chain(shown)
        })
        .collect();
    let mut required: Vec<AssetTypeSelector> = Vec::new();
    let mut profiles = Vec::new();
    let mut status = CoverageStatus::Partial;
    let mut complete_so_far = true;
    for (profile, additions) in profile_additions(packaging_family) {
        required.extend_from_slice(additions);
        let missing: Vec<AssetTypeSelector> = required
            .iter()
            .copied()
            .filter(|asset_type| !present.contains(asset_type))
            .collect();
        complete_so_far &= missing.is_empty();
        if complete_so_far {
            status = completed_status(*profile);
        }
        profiles.push(ProfileCoverage {
            profile: *profile,
            required: required.clone(),
            missing,
        });
    }
    Some(ReleaseCoverage {
        packaging_family,
        status,
        profiles,
    })
}

fn completed_status(profile: CoverageProfile) -> CoverageStatus {
    match profile {
        CoverageProfile::Packaging => CoverageStatus::PackagingComplete,
        CoverageProfile::Physical => CoverageStatus::PhysicalComplete,
        CoverageProfile::Archival => CoverageStatus::ArchivalComplete,
    }
}
