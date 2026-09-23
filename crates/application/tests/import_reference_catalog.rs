use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use game_media_vault_application::{
    ImportReferenceCatalogRequest, PortError, ReferenceCatalogRepositoryPort,
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
    ) -> Result<Vec<ReferenceReleaseRecord>, PortError> {
        Ok((0..max_games).map(reference_release).collect())
    }
}

#[derive(Default)]
struct BatchRecordingCatalog {
    batch_sizes: RefCell<Vec<usize>>,
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
