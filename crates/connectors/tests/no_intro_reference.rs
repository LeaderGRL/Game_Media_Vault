use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use game_media_vault_application::{
    ApplicationError, ImportReferenceCatalogRequest, PortError, ReferenceCatalogRepositoryPort,
    ReferenceCatalogSourcePort, import_reference_catalog,
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
        .unwrap();

    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0].platform, "Nintendo - Game & Watch");
    assert!(releases[0].assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Identifier
            && assertion.qualifier.as_deref() == Some("source_record")
            && assertion.value.starts_with("23:Nintendo - Game & Watch")
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
        ) -> Result<Vec<ReferenceReleaseRecord>, PortError> {
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

    let releases = source.read_releases(&fixture_path(), 0).unwrap();

    assert!(releases.is_empty());
}
