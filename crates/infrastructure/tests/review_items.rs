use game_media_vault_application::CatalogPort;
use game_media_vault_domain::{
    AssetCandidate, AssetType, MatchEvidence, MatchSignal, NewReviewItem, ReleaseAssertion,
    ReleaseAssertionField, ReviewDecision, ReviewMatchCandidate, SourceId,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::tempdir;

fn candidate() -> AssetCandidate {
    AssetCandidate {
        game_title: "Target Game".to_owned(),
        platform: "Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Collector".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("fixture-provider"),
        source_asset_label: Some("front".to_owned()),
        source_url: "fixture://candidate/front".to_owned(),
        original_filename: "front.png".to_owned(),
    }
}

fn review_match(release_edition_id: i64, edition_name: &str) -> ReviewMatchCandidate {
    ReviewMatchCandidate {
        game_id: release_edition_id + 100,
        release_edition_id,
        game_title: "Target Game".to_owned(),
        platform: "Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: edition_name.to_owned(),
        score: 90,
        evidence: vec![
            MatchEvidence {
                signal: MatchSignal::Title,
                candidate_value: "Target Game".to_owned(),
                release_value: "Target Game".to_owned(),
                score_delta: 50,
            },
            MatchEvidence {
                signal: MatchSignal::Edition,
                candidate_value: "Collector".to_owned(),
                release_value: edition_name.to_owned(),
                score_delta: -5,
            },
        ],
        assertions: vec![ReleaseAssertion {
            source_id: SourceId::from("reference-catalog"),
            source_location: "fixture://reference/target-game".to_owned(),
            field: ReleaseAssertionField::Identifier,
            qualifier: Some("source_record".to_owned()),
            value: format!("release-{release_edition_id}"),
        }],
    }
}

#[test]
fn review_item_round_trips_candidate_competitors_and_evidence() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let new_item = NewReviewItem {
        run_id: 7,
        candidate_identity: "connector:fixture-review".to_owned(),
        candidate: candidate(),
        competing_matches: vec![review_match(201, "Standard"), review_match(202, "Deluxe")],
    };

    catalog.persist_review_item(new_item.clone()).unwrap();
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let review_items = reopened.list_review_items().unwrap();

    assert_eq!(review_items.len(), 1);
    assert_eq!(review_items[0].run_id, new_item.run_id);
    assert_eq!(
        review_items[0].candidate_identity,
        new_item.candidate_identity
    );
    assert_eq!(review_items[0].candidate, new_item.candidate);
    assert_eq!(
        review_items[0].competing_matches,
        new_item.competing_matches
    );
}

#[test]
fn review_decision_round_trips_and_can_be_found_by_candidate_identity() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: 7,
            candidate_identity: "connector:fixture-review".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog
        .find_review_item_by_candidate_identity("connector:fixture-review")
        .unwrap()
        .unwrap();

    let resolved = catalog
        .set_review_decision(
            item.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap()
        .unwrap();

    assert_eq!(
        resolved.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201
        })
    );
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let persisted = reopened.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(persisted, resolved);
}
