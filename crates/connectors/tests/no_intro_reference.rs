use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use game_media_vault_application::{
    ApplicationError, ImportReferenceCatalogRequest, PortError, ReferenceCatalogRead,
    ReferenceCatalogRepositoryPort, ReferenceCatalogSourcePort, import_reference_catalog,
};
use game_media_vault_connectors::NoIntroReferenceCatalog;
use game_media_vault_domain::{
    ImportedReleaseEdition, ReferenceReleaseRecord, ReleaseAssertionField,
};

#[derive(Default)]
struct RecordingReferenceCatalog {
    records: RefCell<Vec<ReferenceReleaseRecord>>,
}

impl ReferenceCatalogRepositoryPort for RecordingReferenceCatalog {
    fn persist_reference_release(
        &self,
        record: ReferenceReleaseRecord,
    ) -> Result<ImportedReleaseEdition, PortError> {
        self.records.borrow_mut().push(record);
        let index = self.records.borrow().len() as i64;
        Ok(ImportedReleaseEdition {
            game_id: index,
            release_edition_id: index,
        })
    }
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("no_intro_sample.dat")
}

fn escaped_platform_fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("no_intro_escaped_platform.dat")
}

fn unlisted_region_fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("no_intro_unlisted_region.dat")
}

fn metadata_tags_fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("no_intro_metadata_tags.dat")
}

#[test]
fn imports_a_bounded_no_intro_fixture_as_release_assertions() {
    let catalog = RecordingReferenceCatalog::default();
    let source = NoIntroReferenceCatalog::new();

    let summary = import_reference_catalog(
        &catalog,
        &source,
        ImportReferenceCatalogRequest {
            source_path: fixture_path(),
            max_games: 2,
        },
    )
    .unwrap();

    assert_eq!(summary.imported_releases, 2);
    let records = catalog.records.borrow();
    assert_eq!(records.len(), 2);

    let tetris = &records[0];
    assert_eq!(tetris.game_title, "Tetris");
    assert_eq!(tetris.platform, "Nintendo - Game Boy");
    assert_eq!(tetris.region, "World");
    assert_eq!(tetris.revision.as_deref(), Some("Rev 1"));
    assert_eq!(tetris.edition_name, "Rev 1");
    assert!(tetris.assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Title && assertion.value == "Tetris"
    }));
    assert!(tetris.assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Region && assertion.value == "World"
    }));
    assert!(tetris.assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Revision && assertion.value == "Rev 1"
    }));
    assert!(tetris.assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Identifier
            && assertion.qualifier.as_deref() == Some("sha1")
            && assertion.value == "74591CC9504F3BDEBDAE9D9F8F9D7D68A6B4873B"
    }));

    let mario = &records[1];
    assert_eq!(mario.game_title, "Super Mario Land");
    assert_eq!(mario.region, "USA, Europe");
    assert_eq!(mario.revision, None);

    assert!(
        records
            .iter()
            .all(|record| record.game_title != "Kirby's Dream Land")
    );
}

#[test]
fn decodes_xml_entities_and_preserves_no_intro_regions() {
    let catalog = RecordingReferenceCatalog::default();
    let source = NoIntroReferenceCatalog::new();

    let summary = import_reference_catalog(
        &catalog,
        &source,
        ImportReferenceCatalogRequest {
            source_path: fixture_path(),
            max_games: 6,
        },
    )
    .unwrap();

    assert_eq!(summary.imported_releases, 6);
    let records = catalog.records.borrow();

    let tom_and_jerry = records
        .iter()
        .find(|record| record.game_title == "Tom & Jerry")
        .expect("escaped No-Intro title should be decoded");
    assert_eq!(tom_and_jerry.region, "Portugal");
    assert!(tom_and_jerry.assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Identifier
            && assertion.qualifier.as_deref() == Some("rom_name")
            && assertion.value == "Tom & Jerry (Portugal).gb"
    }));
    assert!(tom_and_jerry.assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Identifier
            && assertion.qualifier.as_deref() == Some("source_record")
            && assertion.value.ends_with("Tom & Jerry (Portugal)")
    }));

    let poland = records
        .iter()
        .find(|record| record.game_title == "Region Test Poland")
        .unwrap();
    assert_eq!(poland.region, "Poland");
    assert!(poland.assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Region && assertion.value == "Poland"
    }));

    let denmark = records
        .iter()
        .find(|record| record.game_title == "Region Test Denmark")
        .unwrap();
    assert_eq!(denmark.region, "Denmark");
    assert!(denmark.assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Region && assertion.value == "Denmark"
    }));
}

#[test]
fn decodes_xml_entities_in_the_platform_header() {
    let source = NoIntroReferenceCatalog::new();

    let releases = source
        .read_releases(&escaped_platform_fixture_path(), 1)
        .unwrap()
        .releases;

    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0].platform, "Nintendo - Game & Watch");
    assert!(releases[0].assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Identifier
            && assertion.qualifier.as_deref() == Some("source_record")
            && assertion.value.starts_with("23:Nintendo - Game & Watch")
    }));
}

#[test]
fn preserves_unlisted_region_claims() {
    let source = NoIntroReferenceCatalog::new();

    let releases = source
        .read_releases(&unlisted_region_fixture_path(), 1)
        .unwrap()
        .releases;

    assert_eq!(releases.len(), 1);
    let release = &releases[0];
    assert_eq!(release.game_title, "Region Preservation Test");
    assert_eq!(release.region, "Caribbean");
    assert_eq!(release.edition_name, "Rev 1");
    assert!(release.assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Region && assertion.value == "Caribbean"
    }));
}

#[test]
fn preserves_status_tags_as_editions_and_version_tags_as_revisions() {
    let source = NoIntroReferenceCatalog::new();

    let releases = source
        .read_releases(&metadata_tags_fixture_path(), 4)
        .unwrap()
        .releases;

    assert_eq!(releases.len(), 4);

    let beta = &releases[0];
    assert_eq!(beta.game_title, "Preview Test");
    assert_eq!(beta.region, "Unknown");
    assert_eq!(beta.edition_name, "Beta");
    assert!(
        !beta
            .assertions
            .iter()
            .any(|assertion| { assertion.field == ReleaseAssertionField::Region })
    );

    let proto = &releases[1];
    assert_eq!(proto.game_title, "Prototype Test");
    assert_eq!(proto.region, "Unknown");
    assert_eq!(proto.edition_name, "Proto");

    let version = &releases[2];
    assert_eq!(version.region, "USA");
    assert_eq!(version.revision.as_deref(), Some("v1.1"));
    assert_eq!(version.edition_name, "v1.1");
    assert!(version.assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Revision && assertion.value == "v1.1"
    }));

    let long_version = &releases[3];
    assert_eq!(long_version.region, "Europe");
    assert_eq!(long_version.revision.as_deref(), Some("Version 2.0"));
    assert_eq!(long_version.edition_name, "Version 2.0");
    assert!(long_version.assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Revision && assertion.value == "Version 2.0"
    }));
}

#[test]
fn rejects_an_unbounded_reference_import_before_reading_the_source() {
    struct PanicSource;

    impl ReferenceCatalogSourcePort for PanicSource {
        fn read_releases(
            &self,
            _source_path: &Path,
            _max_games: usize,
        ) -> Result<ReferenceCatalogRead, PortError> {
            panic!("source should not be read for an invalid limit");
        }
    }

    let error = import_reference_catalog(
        &RecordingReferenceCatalog::default(),
        &PanicSource,
        ImportReferenceCatalogRequest {
            source_path: fixture_path(),
            max_games: 0,
        },
    )
    .unwrap_err();

    assert_eq!(error, ApplicationError::InvalidReferenceImportLimit);
}

#[test]
fn connector_returns_no_releases_when_the_requested_bound_is_zero() {
    let source = NoIntroReferenceCatalog::new();

    let releases = source.read_releases(&fixture_path(), 0).unwrap().releases;

    assert!(releases.is_empty());
}

#[test]
fn platform_names_drop_dat_variant_qualifiers() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("no_intro_headered_platform.dat");

    let releases = NoIntroReferenceCatalog::new()
        .read_releases(&fixture, 1)
        .unwrap()
        .releases;

    assert_eq!(
        releases[0].platform,
        "Nintendo - Nintendo Entertainment System"
    );
    assert!(releases[0].assertions.iter().any(|assertion| {
        assertion.qualifier.as_deref() == Some("source_record")
            && assertion.value
                == "40:Nintendo - Nintendo Entertainment SystemSuper Mario Bros. (World)"
    }));
}

#[test]
fn a_malformed_dat_is_invalid_source_data() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("broken.dat");
    std::fs::write(
        &path,
        "<datafile><header><name>Nintendo - Game Boy</name></header><game>",
    )
    .unwrap();

    let error = NoIntroReferenceCatalog::new()
        .read_releases(&path, 10)
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{error}");
}

#[test]
fn malformed_game_entries_are_skipped_and_counted_without_losing_the_others() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("partly-broken.dat");
    std::fs::write(
        &path,
        r#"<datafile>
  <header><name>Nintendo - Game Boy</name></header>
  <game name="Tetris (World)"><rom name="Tetris (World).gb" crc="46df91ad"/></game>
  <game><rom name="nameless.gb" crc="00000000"/></game>
  <game name="Garbled (World)"><rom name="garbled.gb" crc="&bogus;"/></game>
  <game name="Dr. Mario (World)"><rom name="Dr. Mario (World).gb" crc="12345678"/></game>
</datafile>"#,
    )
    .unwrap();

    let read = NoIntroReferenceCatalog::new()
        .read_releases(&path, 10)
        .unwrap();

    let titles: Vec<&str> = read
        .releases
        .iter()
        .map(|release| release.game_title.as_str())
        .collect();
    assert_eq!(titles, ["Tetris", "Dr. Mario"]);
    assert_eq!(read.skipped_records, 2);
}

#[test]
fn a_datafile_cut_short_between_entries_is_invalid_source_data() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("truncated.dat");
    // A complete entry, but the datafile never closes.
    std::fs::write(
        &path,
        r#"<datafile><header><name>Nintendo - Game Boy</name></header>
  <game name="Tetris (World)"><rom name="Tetris (World).gb" crc="46df91ad"/></game>"#,
    )
    .unwrap();

    let error = NoIntroReferenceCatalog::new()
        .read_releases(&path, 10)
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{error}");
}

#[test]
fn self_closing_game_entries_are_read_or_skipped_like_the_others() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self-closing.dat");
    std::fs::write(
        &path,
        r#"<datafile><header><name>Nintendo - Game Boy</name></header>
  <game name="Dumpless (World)"/>
  <game/>
</datafile>"#,
    )
    .unwrap();

    let read = NoIntroReferenceCatalog::new()
        .read_releases(&path, 10)
        .unwrap();

    let titles: Vec<&str> = read
        .releases
        .iter()
        .map(|release| release.game_title.as_str())
        .collect();
    assert_eq!(titles, ["Dumpless"]);
    assert_eq!(read.skipped_records, 1);
}

#[test]
fn a_bounded_read_still_refuses_a_datafile_broken_after_the_bound() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("broken-later.dat");
    std::fs::write(
        &path,
        r#"<datafile><header><name>Nintendo - Game Boy</name></header>
  <game name="Tetris (World)"><rom name="Tetris (World).gb" crc="46df91ad"/></game>
  <game name="Dr. Mario (World)"><rom name="Dr. Mario (World).gb" crc="12345678"/></game>"#,
    )
    .unwrap();

    let error = NoIntroReferenceCatalog::new()
        .read_releases(&path, 1)
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{error}");
}

#[test]
fn a_dump_without_a_name_asserts_none_of_its_checksums() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nameless-dump.dat");
    std::fs::write(
        &path,
        r#"<datafile><header><name>Nintendo - Game Boy</name></header>
  <game name="Two Dumps (World)">
    <rom name="named.gb" size="1"/>
    <rom size="1" crc="12345678" sha1="1111111111111111111111111111111111111111"/>
  </game>
</datafile>"#,
    )
    .unwrap();

    let read = NoIntroReferenceCatalog::new()
        .read_releases(&path, 10)
        .unwrap();

    let identifiers: Vec<&str> = read.releases[0]
        .assertions
        .iter()
        .filter(|assertion| assertion.field == ReleaseAssertionField::Identifier)
        .filter_map(|assertion| assertion.qualifier.as_deref())
        .filter(|qualifier| *qualifier != "source_record")
        .collect();
    assert_eq!(identifiers, ["rom_name"]);
}
