use game_media_vault_application::{CatalogPort, ReferenceCatalogRepositoryPort};
use game_media_vault_domain::{
    ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField, SourceId,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::tempdir;

fn assertion(
    field: ReleaseAssertionField,
    qualifier: Option<&str>,
    value: &str,
) -> ReleaseAssertion {
    ReleaseAssertion {
        source_id: SourceId::from("no-intro"),
        source_location: "C:/fixtures/Nintendo - Game Boy.dat".to_owned(),
        field,
        qualifier: qualifier.map(str::to_owned),
        value: value.to_owned(),
    }
}

fn tetris_release(revision: &str) -> ReferenceReleaseRecord {
    ReferenceReleaseRecord {
        game_title: "Tetris".to_owned(),
        platform: "Nintendo - Game Boy".to_owned(),
        region: "World".to_owned(),
        revision: Some(revision.to_owned()),
        edition_name: revision.to_owned(),
        assertions: vec![
            assertion(ReleaseAssertionField::Title, None, "Tetris"),
            assertion(ReleaseAssertionField::Region, None, "World"),
            assertion(ReleaseAssertionField::Revision, None, revision),
            assertion(
                ReleaseAssertionField::Identifier,
                Some("source_record"),
                &format!("Tetris (World) ({revision})"),
            ),
            assertion(
                ReleaseAssertionField::Identifier,
                Some("sha1"),
                if revision == "Rev 1" {
                    "1111111111111111111111111111111111111111"
                } else {
                    "2222222222222222222222222222222222222222"
                },
            ),
        ],
    }
}

#[test]
fn reference_import_is_idempotent_and_lists_assetless_release_editions() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();

    let first = catalog
        .persist_reference_release(tetris_release("Rev 1"))
        .unwrap();
    let second = catalog
        .persist_reference_release(tetris_release("Rev 2"))
        .unwrap();
    let repeated = catalog
        .persist_reference_release(tetris_release("Rev 1"))
        .unwrap();

    assert_eq!(first, repeated);
    assert_eq!(first.game_id, second.game_id);
    assert_ne!(first.release_edition_id, second.release_edition_id);

    let library = catalog.list_library().unwrap();
    assert_eq!(library.len(), 2);
    assert!(library.iter().all(|entry| entry.game_title == "Tetris"));
    assert!(library.iter().all(|entry| entry.assets.is_empty()));
    assert!(library.iter().all(|entry| {
        entry
            .assertions
            .iter()
            .any(|assertion| assertion.field == ReleaseAssertionField::Title)
    }));
    assert!(library.iter().any(|entry| {
        entry.assertions.iter().any(|assertion| {
            assertion.field == ReleaseAssertionField::Revision && assertion.value == "Rev 1"
        })
    }));
    assert!(library.iter().any(|entry| {
        entry.assertions.iter().any(|assertion| {
            assertion.field == ReleaseAssertionField::Revision && assertion.value == "Rev 2"
        })
    }));
}

#[test]
fn reference_batch_rolls_back_all_releases_when_one_record_is_invalid() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let mut invalid = tetris_release("Rev 2");
    invalid.assertions.retain(|assertion| {
        assertion.field != ReleaseAssertionField::Identifier
            || assertion.qualifier.as_deref() != Some("source_record")
    });

    let error = catalog
        .persist_reference_releases(vec![tetris_release("Rev 1"), invalid])
        .unwrap_err();

    assert!(error.message().contains("source_record"));
    assert!(catalog.list_library().unwrap().is_empty());
}

#[test]
fn a_changed_claim_is_recorded_after_the_claim_it_replaces() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    catalog
        .persist_reference_release(tetris_release("Rev 1"))
        .unwrap();
    let mut updated = tetris_release("Rev 1");
    updated.assertions[0].value = "Tetris DX".to_owned();

    catalog.persist_reference_release(updated).unwrap();

    let titles: Vec<_> = catalog.list_library().unwrap()[0]
        .assertions
        .iter()
        .filter(|assertion| assertion.field == ReleaseAssertionField::Title)
        .map(|assertion| assertion.value.clone())
        .collect();
    assert_eq!(titles, vec!["Tetris", "Tetris DX"]);
}

#[test]
fn a_claim_observed_again_becomes_the_latest_one() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let mut corrected = tetris_release("Rev 1");
    corrected.assertions[0].value = "Tetris DX".to_owned();
    catalog
        .persist_reference_release(tetris_release("Rev 1"))
        .unwrap();
    catalog.persist_reference_release(corrected).unwrap();

    // The source reverts its correction.
    catalog
        .persist_reference_release(tetris_release("Rev 1"))
        .unwrap();

    let titles: Vec<_> = catalog.list_library().unwrap()[0]
        .assertions
        .iter()
        .filter(|assertion| assertion.field == ReleaseAssertionField::Title)
        .map(|assertion| assertion.value.clone())
        .collect();
    assert_eq!(titles, vec!["Tetris DX", "Tetris"]);
}

#[test]
fn a_catalog_imported_again_from_another_path_moves_its_claims() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    catalog
        .persist_reference_release(tetris_release("Rev 1"))
        .unwrap();
    let mut moved = tetris_release("Rev 1");
    for assertion in &mut moved.assertions {
        assertion.source_location = "D:/archive/Nintendo - Game Boy.dat".to_owned();
    }

    let reimported = catalog.persist_reference_release(moved).unwrap();

    let library = catalog.list_library().unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].release_edition_id, reimported.release_edition_id);
    assert_eq!(library[0].assertions.len(), 5);
    assert!(
        library[0]
            .assertions
            .iter()
            .all(|assertion| assertion.source_location == "D:/archive/Nintendo - Game Boy.dat")
    );
}
