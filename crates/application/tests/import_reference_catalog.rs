use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use game_media_vault_application::{
    ImportReferenceCatalogRequest, PortError, ReferenceCatalogRead, ReferenceCatalogRepositoryPort,
    ReferenceCatalogSourcePort, import_reference_catalog,
};
use game_media_vault_domain::{
    ImportedReleaseEdition, ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField,
    SourceId,
};

struct SyntheticSource;

impl ReferenceCatalogSourcePort for SyntheticSource {
    fn read_releases(
        &self,
        _source_path: &Path,
        max_games: usize,
    ) -> Result<ReferenceCatalogRead, PortError> {
        // A file whose malformed records the reader skipped.
        Ok(ReferenceCatalogRead {
            releases: (0..max_games).map(reference_release).collect(),
            skipped_records: 3,
        })
    }
}

#[derive(Default)]
struct BatchRecordingCatalog {
    batch_sizes: RefCell<Vec<usize>>,
    assertion_locations: RefCell<Vec<String>>,
}

impl ReferenceCatalogRepositoryPort for BatchRecordingCatalog {
    fn persist_reference_release(
        &self,
        _record: ReferenceReleaseRecord,
    ) -> Result<ImportedReleaseEdition, PortError> {
        panic!("reference import should use the batch persistence boundary");
    }

    fn persist_reference_releases(
        &self,
        records: Vec<ReferenceReleaseRecord>,
    ) -> Result<Vec<ImportedReleaseEdition>, PortError> {
        self.batch_sizes.borrow_mut().push(records.len());
        self.assertion_locations.borrow_mut().extend(
            records
                .iter()
                .flat_map(|record| &record.assertions)
                .map(|assertion| assertion.source_location.clone()),
        );
        Ok(records
            .into_iter()
            .enumerate()
            .map(|(index, _)| ImportedReleaseEdition {
                game_id: index as i64 + 1,
                release_edition_id: index as i64 + 1,
            })
            .collect())
    }
}

#[test]
fn reference_import_persists_large_inputs_in_bounded_batches() {
    let catalog = BatchRecordingCatalog::default();

    let summary = import_reference_catalog(
        &catalog,
        &SyntheticSource,
        ImportReferenceCatalogRequest {
            source_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
            max_games: 513,
        },
    )
    .unwrap();

    assert_eq!(summary.imported_releases, 513);
    assert_eq!(summary.skipped_records, 3);
    assert_eq!(*catalog.batch_sizes.borrow(), vec![256, 256, 1]);
}

fn reference_release(index: usize) -> ReferenceReleaseRecord {
    let title = format!("Game {index}");
    ReferenceReleaseRecord {
        game_title: title.clone(),
        platform: "Synthetic Platform".to_owned(),
        region: "World".to_owned(),
        revision: None,
        edition_name: "Standard".to_owned(),
        assertions: vec![
            ReleaseAssertion {
                source_id: SourceId::from("synthetic"),
                source_location: "synthetic.dat".to_owned(),
                field: ReleaseAssertionField::Title,
                qualifier: None,
                value: title,
            },
            ReleaseAssertion {
                source_id: SourceId::from("synthetic"),
                source_location: "synthetic.dat".to_owned(),
                field: ReleaseAssertionField::Identifier,
                qualifier: Some("source_record".to_owned()),
                value: format!("synthetic:{index}"),
            },
        ],
    }
}

#[derive(Default)]
struct PathRecordingSource {
    source_paths: RefCell<Vec<PathBuf>>,
}

impl ReferenceCatalogSourcePort for PathRecordingSource {
    fn read_releases(
        &self,
        source_path: &Path,
        _max_games: usize,
    ) -> Result<ReferenceCatalogRead, PortError> {
        self.source_paths
            .borrow_mut()
            .push(source_path.to_path_buf());
        Ok(ReferenceCatalogRead {
            releases: vec![reference_release(0)],
            skipped_records: 0,
        })
    }
}

#[test]
fn reference_sources_read_the_canonical_path_and_assertions_record_a_plain_location() {
    let source = PathRecordingSource::default();
    let catalog = BatchRecordingCatalog::default();
    let requested = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("..")
        .join("Cargo.toml");

    import_reference_catalog(
        &catalog,
        &source,
        ImportReferenceCatalogRequest {
            source_path: requested.clone(),
            max_games: 1,
        },
    )
    .unwrap();

    // Reading keeps the canonical path, verbatim on Windows, so every valid file stays readable.
    assert_eq!(
        source.source_paths.borrow()[0],
        std::fs::canonicalize(&requested).unwrap()
    );
    let locations = catalog.assertion_locations.borrow();
    assert_eq!(locations.len(), 2);
    for location in locations.iter() {
        assert!(Path::new(location).is_absolute(), "{location}");
        assert!(!location.starts_with(r"\\?\"), "{location}");
        assert!(!location.contains(".."), "{location}");
        assert!(location.ends_with("Cargo.toml"), "{location}");
    }
}
