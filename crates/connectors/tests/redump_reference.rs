use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use game_media_vault_application::{
    ImportReferenceCatalogRequest, PortError, ReferenceCatalogRepositoryPort,
    import_reference_catalog,
};
use game_media_vault_connectors::{REDUMP_SOURCE_ID, RedumpReferenceCatalog};
use game_media_vault_domain::{
    ImportedReleaseEdition, ReferenceReleaseRecord, ReleaseAssertionField, SourceId,
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
        .join("redump_sample.dat")
}

fn import(max_games: usize) -> Vec<ReferenceReleaseRecord> {
    let catalog = RecordingReferenceCatalog::default();
    import_reference_catalog(
        &catalog,
        &RedumpReferenceCatalog::new(),
        ImportReferenceCatalogRequest {
            source_path: fixture_path(),
            max_games,
        },
    )
    .unwrap();
    catalog.records.into_inner()
}

fn values(record: &ReferenceReleaseRecord, qualifier: &str) -> Vec<String> {
    record
        .assertions
        .iter()
        .filter(|assertion| assertion.qualifier.as_deref() == Some(qualifier))
        .map(|assertion| assertion.value.clone())
        .collect()
}

#[test]
fn imports_redump_discs_as_releases_with_their_track_identifiers() {
    let records = import(10);

    assert_eq!(records.len(), 2);
    let ff7 = &records[0];
    assert_eq!(ff7.game_title, "Final Fantasy VII");
    assert_eq!(ff7.platform, "Sony - PlayStation");
    assert_eq!(ff7.region, "USA");
    assert_eq!(ff7.edition_name, "Disc 1");
    assert!(
        ff7.assertions
            .iter()
            .all(|assertion| assertion.source_id == SourceId::from(REDUMP_SOURCE_ID))
    );
    assert_eq!(
        values(ff7, "rom_name"),
        [
            "Final Fantasy VII (USA) (Disc 1).cue",
            "Final Fantasy VII (USA) (Disc 1).bin"
        ]
    );
    assert_eq!(
        values(ff7, "sha1"),
        [
            "1111111111111111111111111111111111111111",
            "2222222222222222222222222222222222222222"
        ]
    );
    assert!(ff7.assertions.iter().any(|assertion| {
        assertion.field == ReleaseAssertionField::Region && assertion.value == "USA"
    }));
}

#[test]
fn records_the_datafile_version_each_release_came_from() {
    let records = import(10);

    assert_eq!(values(&records[1], "dat_version"), ["2026-09-30 21-14-52"]);
}

#[test]
fn reimporting_the_same_datafile_gives_the_same_records() {
    assert_eq!(import(1), import(1));
}
