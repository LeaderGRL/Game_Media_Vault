use game_media_vault_application::{
    CatalogPort, ReviewProcessingFinalization, RunRepositoryPort,
    list_review_items as list_review_items_use_case, resolve_review_item,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRunStatus,
    AssetCandidate, AssetType, AssetTypeSelector, GameSelection, MatchEvidence, MatchSignal,
    NewReviewItem, PersistAsset, ReleaseAssertion, ReleaseAssertionField, RetentionPolicy,
    ReviewDecision, ReviewMatchCandidate, ReviewStatus, SourceId, SourceSelection,
};
use game_media_vault_infrastructure::SqliteCatalog;
use rusqlite::Connection;
use tempfile::tempdir;

fn candidate() -> AssetCandidate {
    AssetCandidate {
        provider_candidate_id: None,
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

fn request() -> AcquisitionRequest {
    AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["fixture-provider".to_owned()]),
        platforms: vec!["Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Target Game".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap()
}

fn completed_review_work(catalog: &SqliteCatalog, work_key: &str) -> i64 {
    let run = catalog.create_run(request()).unwrap();
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog.complete_work(run.id, work_key).unwrap();
    assert!(
        catalog
            .compare_and_set_run_status(
                run.id,
                AcquisitionRunStatus::Running,
                AcquisitionRunStatus::Completed,
            )
            .unwrap()
    );
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: format!("connector:fixture-review:{}", run.id),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    run.id
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
    assert_eq!(review_items[0].status, ReviewStatus::Pending);
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
    assert_eq!(resolved.status, ReviewStatus::Accepted);
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let persisted = reopened.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(persisted, resolved);
}

#[test]
fn accepting_a_review_atomically_requeues_its_completed_work() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let work_key = "connector:fixture-review-work";
    let run_id = completed_review_work(&catalog, work_key);
    let item = catalog.list_review_items().unwrap().remove(0);

    let accepted = catalog
        .accept_review_item_and_requeue(item.id, 201, work_key)
        .unwrap()
        .unwrap();

    assert_eq!(accepted.status, ReviewStatus::Accepted);
    assert_eq!(
        accepted.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201
        })
    );
    let run = catalog.get_run(run_id).unwrap().unwrap();
    assert_eq!(run.status, AcquisitionRunStatus::Running);
    assert_eq!(run.queued_work, 1);
    assert_eq!(run.completed_work, 0);
    assert_eq!(
        catalog.next_queued_work(run_id).unwrap().unwrap().key,
        work_key
    );
}

#[test]
fn accepting_a_review_supersedes_incompatible_occurrences_of_the_same_candidate() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let first_run = catalog.create_run(request()).unwrap();
    let second_run = catalog.create_run(request()).unwrap();
    let identity = "connector:shared-acceptance";
    let first_work_key = "connector:first-shared-review";

    catalog
        .queue_work(first_run.id, first_work_key.to_owned())
        .unwrap();
    catalog.complete_work(first_run.id, first_work_key).unwrap();
    assert!(
        catalog
            .compare_and_set_run_status(
                first_run.id,
                AcquisitionRunStatus::Running,
                AcquisitionRunStatus::Completed,
            )
            .unwrap()
    );

    catalog
        .persist_review_item(NewReviewItem {
            run_id: first_run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: second_run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(202, "Deluxe")],
        })
        .unwrap();

    let first_item = catalog
        .list_review_items()
        .unwrap()
        .into_iter()
        .find(|item| item.run_id == first_run.id)
        .unwrap();
    catalog
        .accept_review_item_and_requeue(first_item.id, 201, first_work_key)
        .unwrap()
        .unwrap();

    let items = catalog.list_review_items().unwrap();
    let accepted = items
        .iter()
        .find(|item| item.run_id == first_run.id)
        .unwrap();
    let incompatible = items
        .iter()
        .find(|item| item.run_id == second_run.id)
        .unwrap();
    assert_eq!(accepted.status, ReviewStatus::Accepted);
    assert_eq!(incompatible.status, ReviewStatus::Superseded);
    assert_eq!(
        catalog
            .find_review_item_for_run_by_candidate_identity(second_run.id, identity)
            .unwrap()
            .unwrap()
            .decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201
        })
    );
}

#[test]
fn staging_a_review_atomically_completes_its_work() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let run = catalog.create_run(request()).unwrap();
    let work_key = "connector:atomic-stage";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();

    assert!(
        catalog
            .stage_review_item_and_complete_work(
                NewReviewItem {
                    run_id: run.id,
                    candidate_identity: "connector:atomic-stage".to_owned(),
                    candidate: candidate(),
                    competing_matches: vec![review_match(201, "Standard")],
                },
                work_key,
            )
            .unwrap()
    );

    let run = catalog.get_run(run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
    assert_eq!(catalog.list_review_items().unwrap().len(), 1);
}

#[test]
fn failed_review_acceptance_rolls_back_the_work_requeue() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let work_key = "connector:fixture-review-work";
    let run_id = completed_review_work(&catalog, work_key);
    let item = catalog.list_review_items().unwrap().remove(0);
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_review_accept
             BEFORE UPDATE OF decision_json ON review_items
             BEGIN
                 SELECT RAISE(FAIL, 'forced review write failure');
             END;",
        )
        .unwrap();

    assert!(
        catalog
            .accept_review_item_and_requeue(item.id, 201, work_key)
            .is_err()
    );

    let run = catalog.get_run(run_id).unwrap().unwrap();
    assert_eq!(run.status, AcquisitionRunStatus::Completed);
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
    assert!(catalog.next_queued_work(run_id).unwrap().is_none());
    let persisted = catalog.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(persisted.status, ReviewStatus::Pending);
    assert_eq!(persisted.decision, None);
}

#[test]
fn terminal_review_decisions_cannot_be_overwritten_or_reopened() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: 7,
            candidate_identity: "connector:terminal-review".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    catalog
        .set_review_decision(item.id, ReviewDecision::Reject)
        .unwrap();

    assert!(
        catalog
            .set_review_decision(item.id, ReviewDecision::Defer)
            .is_err()
    );
    catalog
        .persist_review_item(NewReviewItem {
            run_id: 8,
            candidate_identity: "connector:terminal-review".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(202, "Deluxe")],
        })
        .unwrap();

    let persisted = catalog.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(persisted.run_id, 7);
    assert_eq!(persisted.status, ReviewStatus::Rejected);
    assert_eq!(persisted.decision, Some(ReviewDecision::Reject));
    assert_eq!(
        persisted.competing_matches,
        vec![review_match(201, "Standard")]
    );
}

#[test]
fn rejecting_a_processing_occurrence_does_not_reject_pending_siblings() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let identity = "connector:reject-race";
    for run_id in [7, 8] {
        catalog
            .persist_review_item(NewReviewItem {
                run_id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(201, "Standard")],
            })
            .unwrap();
    }
    let items = catalog.list_review_items().unwrap();
    let processing = items.iter().find(|item| item.run_id == 7).unwrap();
    let sibling = items.iter().find(|item| item.run_id == 8).unwrap();
    catalog
        .claim_review_item_for_processing(processing.id)
        .unwrap()
        .unwrap();

    assert!(
        catalog
            .set_review_decision(processing.id, ReviewDecision::Reject)
            .is_err()
    );

    assert_eq!(
        catalog
            .get_review_item(processing.id)
            .unwrap()
            .unwrap()
            .status,
        ReviewStatus::Processing
    );
    let sibling = catalog.get_review_item(sibling.id).unwrap().unwrap();
    assert_eq!(sibling.status, ReviewStatus::Pending);
    assert_eq!(sibling.decision, None);
}

#[test]
fn current_run_review_occurrence_is_preferred_after_a_processing_race() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let identity = "connector:current-run-race";
    for run_id in [7, 8] {
        catalog
            .persist_review_item(NewReviewItem {
                run_id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(201, "Standard")],
            })
            .unwrap();
    }
    let items = catalog.list_review_items().unwrap();
    let first = items.iter().find(|item| item.run_id == 7).unwrap();
    let current = items.iter().find(|item| item.run_id == 8).unwrap();
    let claim = catalog
        .claim_review_item_for_processing(current.id)
        .unwrap()
        .unwrap();

    catalog
        .set_review_decision(first.id, ReviewDecision::Reject)
        .unwrap();
    catalog
        .restore_review_item_processing(current.id, &claim.lease_token, ReviewStatus::Pending)
        .unwrap();

    let selected = catalog
        .find_review_item_for_run_by_candidate_identity(8, identity)
        .unwrap()
        .unwrap();
    assert_eq!(selected.run_id, 8);
    assert_eq!(selected.status, ReviewStatus::Rejected);
    assert_eq!(selected.decision, Some(ReviewDecision::Reject));
}

#[test]
fn expired_processing_occurrence_reconciles_a_terminal_sibling_decision() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let identity = "connector:expired-terminal-race";
    for run_id in [7, 8] {
        catalog
            .persist_review_item(NewReviewItem {
                run_id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(201, "Standard")],
            })
            .unwrap();
    }
    let items = catalog.list_review_items().unwrap();
    let first = items.iter().find(|item| item.run_id == 7).unwrap();
    let current = items.iter().find(|item| item.run_id == 8).unwrap();
    catalog
        .claim_review_item_for_processing(current.id)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(first.id, ReviewDecision::Reject)
        .unwrap();
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE review_processing_leases SET acquired_at = unixepoch() - 3601 WHERE review_item_id = ?1",
            [current.id],
        )
        .unwrap();

    catalog.recover_expired_review_processing(8).unwrap();

    let recovered = catalog.get_review_item(current.id).unwrap().unwrap();
    assert_eq!(recovered.status, ReviewStatus::Rejected);
    assert_eq!(recovered.decision, Some(ReviewDecision::Reject));
}

#[test]
fn terminal_rejection_wins_before_processing_asset_is_persisted() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let identity = "connector:terminal-rejection-processing-race";
    let first_run = catalog.create_run(request()).unwrap();
    let processing_run = catalog.create_run(request()).unwrap();
    let work_key = "connector:processing-race";
    catalog
        .queue_work(processing_run.id, work_key.to_owned())
        .unwrap();
    for run_id in [first_run.id, processing_run.id] {
        catalog
            .persist_review_item(NewReviewItem {
                run_id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(201, "Standard")],
            })
            .unwrap();
    }
    let items = catalog.list_review_items().unwrap();
    let first = items
        .iter()
        .find(|item| item.run_id == first_run.id)
        .unwrap();
    let processing = items
        .iter()
        .find(|item| item.run_id == processing_run.id)
        .unwrap();
    let claim = catalog
        .claim_review_item_for_processing(processing.id)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(first.id, ReviewDecision::Reject)
        .unwrap();

    let outcome = catalog
        .finalize_review_processing_asset(
            processing.id,
            &claim.lease_token,
            processing_run.id,
            work_key,
            PersistAsset {
                existing_game_id: None,
                existing_release_edition_id: None,
                match_decision: None,
                game_title: "Target Game".to_owned(),
                platform: "Nintendo Entertainment System".to_owned(),
                region: "USA".to_owned(),
                edition_name: "Collector".to_owned(),
                asset_type: AssetType::BoxFront,
                object_hash: "should-not-be-persisted".to_owned(),
                byte_len: 42,
                original_filename: "front.png".to_owned(),
                source_id: SourceId::from("fixture-provider"),
                source_asset_label: Some("front".to_owned()),
                source_location: "fixture://candidate/front".to_owned(),
            },
        )
        .unwrap();

    assert_eq!(outcome, ReviewProcessingFinalization::Discarded);
    let processing = catalog.get_review_item(processing.id).unwrap().unwrap();
    assert_eq!(processing.status, ReviewStatus::Rejected);
    assert_eq!(processing.decision, Some(ReviewDecision::Reject));
    let run = catalog.get_run(processing_run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
    assert!(catalog.list_library().unwrap().is_empty());
}

#[test]
fn terminal_acceptance_requeues_processing_occurrence_before_persisting() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let identity = "connector:terminal-acceptance-processing-race";
    let first_run = catalog.create_run(request()).unwrap();
    let processing_run = catalog.create_run(request()).unwrap();
    let work_key = "connector:processing-acceptance-race";
    catalog
        .queue_work(processing_run.id, work_key.to_owned())
        .unwrap();
    for run_id in [first_run.id, processing_run.id] {
        catalog
            .persist_review_item(NewReviewItem {
                run_id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(201, "Standard")],
            })
            .unwrap();
    }
    let items = catalog.list_review_items().unwrap();
    let first = items
        .iter()
        .find(|item| item.run_id == first_run.id)
        .unwrap();
    let processing = items
        .iter()
        .find(|item| item.run_id == processing_run.id)
        .unwrap();
    let claim = catalog
        .claim_review_item_for_processing(processing.id)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(
            first.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();

    let outcome = catalog
        .finalize_review_processing_asset(
            processing.id,
            &claim.lease_token,
            processing_run.id,
            work_key,
            PersistAsset {
                existing_game_id: None,
                existing_release_edition_id: None,
                match_decision: None,
                game_title: "Target Game".to_owned(),
                platform: "Nintendo Entertainment System".to_owned(),
                region: "USA".to_owned(),
                edition_name: "Collector".to_owned(),
                asset_type: AssetType::BoxFront,
                object_hash: "should-not-be-persisted".to_owned(),
                byte_len: 42,
                original_filename: "front.png".to_owned(),
                source_id: SourceId::from("fixture-provider"),
                source_asset_label: Some("front".to_owned()),
                source_location: "fixture://candidate/front".to_owned(),
            },
        )
        .unwrap();

    assert_eq!(outcome, ReviewProcessingFinalization::Requeued);
    let processing = catalog.get_review_item(processing.id).unwrap().unwrap();
    assert_eq!(processing.status, ReviewStatus::Accepted);
    assert_eq!(
        processing.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201,
        })
    );
    let run = catalog.get_run(processing_run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 1);
    assert_eq!(run.completed_work, 0);
    assert!(catalog.list_library().unwrap().is_empty());
}

#[test]
fn processing_asset_finalization_persists_asset_work_and_review_together() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let run = catalog.create_run(request()).unwrap();
    let work_key = "connector:atomic-finalization";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: "connector:atomic-finalization".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    let claim = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();

    let outcome = catalog
        .finalize_review_processing_asset(
            item.id,
            &claim.lease_token,
            run.id,
            work_key,
            PersistAsset {
                existing_game_id: None,
                existing_release_edition_id: None,
                match_decision: None,
                game_title: "Target Game".to_owned(),
                platform: "Nintendo Entertainment System".to_owned(),
                region: "USA".to_owned(),
                edition_name: "Collector".to_owned(),
                asset_type: AssetType::BoxFront,
                object_hash: "atomic-finalization-hash".to_owned(),
                byte_len: 42,
                original_filename: "front.png".to_owned(),
                source_id: SourceId::from("fixture-provider"),
                source_asset_label: Some("front".to_owned()),
                source_location: "fixture://candidate/front".to_owned(),
            },
        )
        .unwrap();

    let ReviewProcessingFinalization::Imported(imported) = outcome else {
        panic!("expected the processing asset to be imported");
    };
    assert_eq!(imported.object_hash, "atomic-finalization-hash");
    let review = catalog.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(review.status, ReviewStatus::AutoResolved);
    let run = catalog.get_run(run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
    let library = catalog.list_library().unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].assets.len(), 1);
    assert_eq!(library[0].assets[0].object_hash, "atomic-finalization-hash");
}

#[test]
fn processing_asset_finalization_rolls_back_if_review_transition_fails() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = catalog.create_run(request()).unwrap();
    let work_key = "connector:atomic-finalization-rollback";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: "connector:atomic-finalization-rollback".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    let claim = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_auto_resolve
             BEFORE UPDATE OF status ON review_items
             WHEN NEW.status = 'auto_resolved'
             BEGIN
                 SELECT RAISE(ABORT, 'forced review transition failure');
             END;",
        )
        .unwrap();

    let result = catalog.finalize_review_processing_asset(
        item.id,
        &claim.lease_token,
        run.id,
        work_key,
        PersistAsset {
            existing_game_id: None,
            existing_release_edition_id: None,
            match_decision: None,
            game_title: "Target Game".to_owned(),
            platform: "Nintendo Entertainment System".to_owned(),
            region: "USA".to_owned(),
            edition_name: "Collector".to_owned(),
            asset_type: AssetType::BoxFront,
            object_hash: "rollback-finalization-hash".to_owned(),
            byte_len: 42,
            original_filename: "front.png".to_owned(),
            source_id: SourceId::from("fixture-provider"),
            source_asset_label: Some("front".to_owned()),
            source_location: "fixture://candidate/front".to_owned(),
        },
    );

    assert!(result.is_err());
    let review = catalog.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(review.status, ReviewStatus::Processing);
    let run = catalog.get_run(run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 1);
    assert_eq!(run.completed_work, 0);
    assert!(catalog.list_library().unwrap().is_empty());
}

#[test]
fn a_second_run_cannot_move_an_existing_pending_review() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let first = NewReviewItem {
        run_id: 7,
        candidate_identity: "connector:shared-review".to_owned(),
        candidate: candidate(),
        competing_matches: vec![review_match(201, "Standard")],
    };
    catalog.persist_review_item(first.clone()).unwrap();

    catalog
        .persist_review_item(NewReviewItem {
            run_id: 8,
            candidate_identity: first.candidate_identity.clone(),
            candidate: AssetCandidate {
                source_url: "fixture://candidate/rotated-front".to_owned(),
                ..candidate()
            },
            competing_matches: vec![review_match(202, "Deluxe")],
        })
        .unwrap();

    let items = catalog.list_review_items().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].run_id, 7);
    assert_eq!(items[0].candidate, first.candidate);
    assert_eq!(items[0].competing_matches, first.competing_matches);
    assert_eq!(items[0].status, ReviewStatus::Pending);
    assert_eq!(items[1].run_id, 8);
    assert_eq!(items[1].status, ReviewStatus::Pending);
}

#[test]
fn processing_claim_blocks_human_resolution_and_only_recovers_after_lease_expiry() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: 7,
            candidate_identity: "connector:processing-review".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);

    let claimed = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.item.status, ReviewStatus::Processing);
    assert!(
        catalog
            .set_review_decision(item.id, ReviewDecision::Reject)
            .is_err()
    );
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    assert_eq!(
        reopened.get_review_item(item.id).unwrap().unwrap().status,
        ReviewStatus::Processing
    );
    drop(reopened);
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE review_processing_leases SET acquired_at = unixepoch() - 3601",
            [],
        )
        .unwrap();
    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    reopened.recover_expired_review_processing(7).unwrap();
    assert_eq!(
        reopened.get_review_item(item.id).unwrap().unwrap().status,
        ReviewStatus::Pending
    );
}

#[test]
fn stale_processing_claim_cannot_finalize_a_reclaimed_lease() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: 7,
            candidate_identity: "connector:reclaimed-review".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    let first_claim = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();

    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE review_processing_leases SET acquired_at = unixepoch() - 3601",
            [],
        )
        .unwrap();
    catalog.recover_expired_review_processing(7).unwrap();
    let second_claim = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();

    assert_ne!(first_claim.lease_token, second_claim.lease_token);
    assert!(
        !catalog
            .renew_review_item_processing(item.id, &first_claim.lease_token)
            .unwrap()
    );
    assert!(
        catalog
            .finish_review_item_processing(
                item.id,
                &first_claim.lease_token,
                ReviewStatus::AutoResolved,
            )
            .is_err()
    );
    assert_eq!(
        catalog.get_review_item(item.id).unwrap().unwrap().status,
        ReviewStatus::Processing
    );

    let completed = catalog
        .finish_review_item_processing(
            item.id,
            &second_claim.lease_token,
            ReviewStatus::AutoResolved,
        )
        .unwrap()
        .unwrap();
    assert_eq!(completed.status, ReviewStatus::AutoResolved);
}

#[test]
fn listing_reviews_recovers_expired_processing_claims() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: 7,
            candidate_identity: "connector:list-recovery".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE review_processing_leases SET acquired_at = unixepoch() - 3601",
            [],
        )
        .unwrap();

    let items = list_review_items_use_case(&catalog).unwrap();

    assert_eq!(items[0].status, ReviewStatus::Pending);
}

#[test]
fn resolving_a_review_recovers_its_expired_processing_claim() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: 7,
            candidate_identity: "connector:resolve-recovery".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE review_processing_leases SET acquired_at = unixepoch() - 3601",
            [],
        )
        .unwrap();

    let resolved = resolve_review_item(&catalog, item.id, ReviewDecision::Reject).unwrap();

    assert_eq!(resolved.status, ReviewStatus::Rejected);
}

#[test]
fn opening_a_review_catalog_without_status_migrates_existing_decisions() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: 7,
            candidate_identity: "connector:legacy-review".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    catalog
        .set_review_decision(item.id, ReviewDecision::Defer)
        .unwrap();
    drop(catalog);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP INDEX IF EXISTS idx_review_items_status;
             DROP INDEX IF EXISTS idx_review_items_run_status;
             ALTER TABLE review_items DROP COLUMN status;",
        )
        .unwrap();
    drop(connection);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let migrated = reopened.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(migrated.status, ReviewStatus::Deferred);
    assert_eq!(migrated.decision, Some(ReviewDecision::Defer));
    drop(reopened);

    let connection = Connection::open(&path).unwrap();
    let status_index_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name = 'idx_review_items_run_status'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status_index_count, 1);
}

#[test]
fn review_candidate_identity_lookups_are_indexed_across_runs() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    drop(catalog);

    let connection = Connection::open(&path).unwrap();
    let leading_identity_columns: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_index_info('idx_review_items_candidate_identity')
             WHERE seqno = 0 AND name = 'candidate_identity'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(leading_identity_columns, 1);
}
