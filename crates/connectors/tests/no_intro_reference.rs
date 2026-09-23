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
