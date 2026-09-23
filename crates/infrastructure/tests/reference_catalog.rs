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
