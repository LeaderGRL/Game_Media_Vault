use std::path::{Path, PathBuf};

use game_media_vault_application::ReferenceCatalogSourcePort;
use game_media_vault_connectors::{MAME_SOFTWARE_LISTS_SOURCE_ID, MameSoftwareListCatalog};
use game_media_vault_domain::{ReferenceReleaseRecord, ReleaseAssertionField};

const NES: &str = "Nintendo - Nintendo Entertainment System";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("mame_nes_sample.xml")
}

/// The values of the identifiers `release` asserts under `qualifier`, in file order.
fn identifier<'a>(release: &'a ReferenceReleaseRecord, qualifier: &str) -> Vec<&'a str> {
    release
        .assertions
        .iter()
        .filter(|assertion| {
            assertion.field == ReleaseAssertionField::Identifier
                && assertion.qualifier.as_deref() == Some(qualifier)
        })
        .map(|assertion| assertion.value.as_str())
        .collect()
}

#[test]
fn reads_each_software_as_a_release_on_the_platform_of_its_list() {
    let read = MameSoftwareListCatalog::new()
        .read_releases(&fixture(), 10)
        .unwrap();

    let releases: Vec<(&str, &str, &str, Option<&str>, &str)> = read
        .releases
        .iter()
        .map(|release| {
            (
                release.game_title.as_str(),
                release.platform.as_str(),
                release.region.as_str(),
                release.revision.as_deref(),
                release.edition_name.as_str(),
            )
        })
        .collect();
    assert_eq!(
        releases,
        [
            ("Super Mario Bros.", NES, "World", None, "Standard"),
            (
                "Zelda no Densetsu - The Hyrule Fantasy",
                NES,
                "Japan",
                Some("Rev A"),
                "Rev A"
            ),
            ("Metroid", NES, "Europe", None, "Standard"),
        ]
    );
    // Entries without a short name or a description identify no release.
    assert_eq!(read.skipped_records, 2);
    assert!(read.releases.iter().all(|release| {
        release
            .assertions
            .iter()
            .all(|assertion| assertion.source_id.as_str() == MAME_SOFTWARE_LISTS_SOURCE_ID)
    }));
}

#[test]
fn keeps_package_identifiers_release_metadata_and_dump_checksums() {
    let read = MameSoftwareListCatalog::new()
        .read_releases(&fixture(), 10)
        .unwrap();
    let smb = &read.releases[0];

    assert_eq!(identifier(smb, "serial"), ["NES-SM-USA"]);
    assert_eq!(identifier(smb, "barcode"), ["0 45496 63010 5"]);
    assert_eq!(identifier(smb, "release"), ["19851018"]);
    assert_eq!(identifier(smb, "year"), ["1985"]);
    assert_eq!(identifier(smb, "publisher"), ["Nintendo"]);
    assert_eq!(identifier(smb, "alt_title"), ["Super Mario Brothers"]);
    assert_eq!(identifier(smb, "rom_name"), ["smb.prg", "smb.chr"]);
    assert_eq!(identifier(smb, "crc"), ["5cf548d3", "867b51ad"]);
    assert_eq!(
        identifier(smb, "sha1"),
        [
            "1111111111111111111111111111111111111111",
            "2222222222222222222222222222222222222222"
        ]
    );
    assert_eq!(identifier(smb, "software_list"), ["nes"]);
    assert_eq!(identifier(smb, "software"), ["smb"]);
    // One stable record identifier per software keeps re-imports idempotent.
    assert_eq!(identifier(smb, "source_record"), ["3:nessmb"]);
    assert_eq!(identifier(&read.releases[1], "clone_of"), ["zelda"]);
    assert_eq!(
        identifier(&read.releases[2], "publisher"),
        ["Nintendo & Intelligent Systems"]
    );
}

#[test]
fn records_the_mame_version_its_lists_came_with_when_given() {
    let versioned = MameSoftwareListCatalog::with_mame_version("0.268")
        .read_releases(&fixture(), 10)
        .unwrap();
    let unversioned = MameSoftwareListCatalog::new()
        .read_releases(&fixture(), 10)
        .unwrap();

    assert!(
        versioned
            .releases
            .iter()
            .all(|release| identifier(release, "mame_version") == ["0.268"])
    );
    assert!(identifier(&unversioned.releases[0], "mame_version").is_empty());
}

#[test]
fn reading_is_bounded_and_deterministic() {
    let catalog = MameSoftwareListCatalog::new();

    let first = catalog.read_releases(&fixture(), 2).unwrap();
    let again = catalog.read_releases(&fixture(), 2).unwrap();

    assert_eq!(first.releases.len(), 2);
    assert_eq!(first, again);
}

#[test]
fn a_list_this_importer_does_not_name_keeps_its_own_description_as_platform() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("homebrew.xml");
    std::fs::write(
        &path,
        r#"<softwarelist name="homebrew" description="Homebrew Console cartridges">
  <software name="demo"><description>Demo (World)</description></software>
</softwarelist>"#,
    )
    .unwrap();

    let read = MameSoftwareListCatalog::new()
        .read_releases(&path, 10)
        .unwrap();

    assert_eq!(read.releases[0].platform, "Homebrew Console cartridges");
}

#[test]
fn a_list_that_is_not_well_formed_is_invalid_source_data() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("broken.xml");
    std::fs::write(
        &path,
        r#"<softwarelist name="nes" description="NES"><software name="smb"><description>"#,
    )
    .unwrap();

    let error = MameSoftwareListCatalog::new()
        .read_releases(&path, 10)
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{error}");
}

#[test]
fn a_list_without_a_description_is_named_by_its_list_name() {
    let temp = tempfile::tempdir().unwrap();
    let named = temp.path().join("named.xml");
    std::fs::write(
        &named,
        r#"<softwarelist name="foo_cart">
  <software name="demo"><description>Demo (World)</description></software>
</softwarelist>"#,
    )
    .unwrap();
    let anonymous = temp.path().join("anonymous.xml");
    std::fs::write(
        &anonymous,
        r#"<softwarelist>
  <software name="demo"><description>Demo (World)</description></software>
</softwarelist>"#,
    )
    .unwrap();
    let catalog = MameSoftwareListCatalog::new();

    let named = catalog.read_releases(&named, 10).unwrap();
    let anonymous = catalog.read_releases(&anonymous, 10).unwrap();

    assert_eq!(named.releases[0].platform, "foo_cart");
    // A list naming neither its system nor itself places its software on no platform.
    assert!(anonymous.releases.is_empty());
    assert_eq!(anonymous.skipped_records, 1);
}
