use game_media_vault_application::{
    CatalogPort, ObjectStorePort, PortError, ReviewProcessingFinalization, RunRepositoryPort,
    StagedOriginal, acquisition_work_key, list_review_items as list_review_items_use_case,
    resolve_review_item,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRunStatus,
    AssetCandidate, AssetType, AssetTypeSelector, GameSelection, MatchEvidence, MatchSignal,
    NewReviewItem, PersistAsset, ReleaseAssertion, ReleaseAssertionField, RetentionPolicy,
    ReviewDecision, ReviewMatchCandidate, ReviewStatus, SourceId, SourceSelection, StoredObject,
};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use rusqlite::Connection;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
};
use tempfile::tempdir;

#[derive(Default)]
struct PublishGateState {
    entered: bool,
    release: bool,
}

struct BlockingPreparedOriginal {
    stored: StoredObject,
    gate: Arc<(Mutex<PublishGateState>, Condvar)>,
}

impl BlockingPreparedOriginal {
    fn wait_for_release(&self) {
        let (mutex, condvar) = &*self.gate;
        let mut state = mutex.lock().unwrap();
        if !state.entered {
            state.entered = true;
            condvar.notify_all();
        }
        while !state.release {
            state = condvar.wait(state).unwrap();
        }
    }
}

impl StagedOriginal for BlockingPreparedOriginal {
    fn stored_object(&self) -> &StoredObject {
        &self.stored
    }

    fn prepare_publish(&mut self) -> Result<(), PortError> {
        self.wait_for_release();
        Ok(())
    }

    fn publish(self: Box<Self>) -> Result<StoredObject, PortError> {
        self.wait_for_release();
        Ok(self.stored.clone())
    }
}

struct JournalCheckingOriginal {
    stored: StoredObject,
    catalog_path: PathBuf,
    journal_seen: Arc<Mutex<bool>>,
}

impl StagedOriginal for JournalCheckingOriginal {
    fn stored_object(&self) -> &StoredObject {
        &self.stored
    }

    fn publish(self: Box<Self>) -> Result<StoredObject, PortError> {
        let pending: i64 = Connection::open(&self.catalog_path)
            .map_err(|error| PortError(error.to_string()))?
            .query_row(
                "SELECT COUNT(*) FROM pending_object_publications WHERE object_hash = ?1",
                [&self.stored.hash],
                |row| row.get(0),
            )
            .map_err(|error| PortError(error.to_string()))?;
        *self.journal_seen.lock().unwrap() = pending > 0;
        Ok(self.stored.clone())
    }
}

struct VisibleThenFailingOriginal {
    stored: StoredObject,
    object_path: PathBuf,
}

impl StagedOriginal for VisibleThenFailingOriginal {
    fn stored_object(&self) -> &StoredObject {
        &self.stored
    }

    fn publish(self: Box<Self>) -> Result<StoredObject, PortError> {
        if let Some(parent) = self.object_path.parent() {
            fs::create_dir_all(parent).map_err(|error| PortError(error.to_string()))?;
        }
        fs::write(&self.object_path, b"visible before failure")
            .map_err(|error| PortError(error.to_string()))?;
        Err(PortError("forced publication failure".to_owned()))
    }
}

fn candidate() -> AssetCandidate {
    AssetCandidate {
        provider_candidate_id: None,
        source_url_requires_rediscovery: false,
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
fn accepting_a_review_from_cancelled_run_persists_without_requeueing_work() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let run = catalog.create_run(request()).unwrap();
    let identity = "connector:cancelled-review-acceptance";
    let work_key = "connector:cancelled-review-work";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog
        .stage_review_item_and_complete_work(
            NewReviewItem {
                run_id: run.id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(201, "Standard")],
            },
            work_key,
        )
        .unwrap();
    assert!(
        catalog
            .compare_and_set_run_status(
                run.id,
                AcquisitionRunStatus::Running,
                AcquisitionRunStatus::Cancelled,
            )
            .unwrap()
    );
    let item = catalog
        .find_review_item_for_run_by_candidate_identity(run.id, identity)
        .unwrap()
        .unwrap();

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
    let run = catalog.get_run(run.id).unwrap().unwrap();
    assert_eq!(run.status, AcquisitionRunStatus::Cancelled);
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
    assert!(catalog.next_queued_work(run.id).unwrap().is_none());
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
fn accepting_a_review_requeues_compatible_occurrences_of_the_same_candidate() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let first_run = catalog.create_run(request()).unwrap();
    let second_run = catalog.create_run(request()).unwrap();
    let identity = "connector:shared-compatible-acceptance";
    let staged_candidate = candidate();
    let work_key = acquisition_work_key(staged_candidate.source_id.as_str(), &staged_candidate);

    for run_id in [first_run.id, second_run.id] {
        catalog.queue_work(run_id, work_key.clone()).unwrap();
        catalog
            .stage_review_item_and_complete_work(
                NewReviewItem {
                    run_id,
                    candidate_identity: identity.to_owned(),
                    candidate: staged_candidate.clone(),
                    competing_matches: vec![review_match(201, "Standard")],
                },
                &work_key,
            )
            .unwrap();
    }

    let first_item = catalog
        .find_review_item_for_run_by_candidate_identity(first_run.id, identity)
        .unwrap()
        .unwrap();
    catalog
        .accept_review_item_and_requeue(first_item.id, 201, &work_key)
        .unwrap()
        .unwrap();

    for run_id in [first_run.id, second_run.id] {
        let item = catalog
            .find_review_item_for_run_by_candidate_identity(run_id, identity)
            .unwrap()
            .unwrap();
        assert_eq!(item.status, ReviewStatus::Accepted);
        assert_eq!(
            item.decision,
            Some(ReviewDecision::Accept {
                release_edition_id: 201,
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
fn staging_a_review_keeps_incompatible_historical_acceptance_reviewable() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let historical_run = catalog.create_run(request()).unwrap();
    let current_run = catalog.create_run(request()).unwrap();
    let identity = "connector:stale-accepted-release";
    let current_work_key = "connector:current-review-work";
    catalog
        .queue_work(current_run.id, current_work_key.to_owned())
        .unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: historical_run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let historical = catalog
        .find_review_item_for_run_by_candidate_identity(historical_run.id, identity)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(
            historical.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();
    catalog
        .set_review_status(historical.id, ReviewStatus::Applied)
        .unwrap();

    catalog
        .stage_review_item_and_complete_work(
            NewReviewItem {
                run_id: current_run.id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(202, "Deluxe")],
            },
            current_work_key,
        )
        .unwrap();

    let current = catalog
        .find_review_item_for_run_by_candidate_identity(current_run.id, identity)
        .unwrap()
        .unwrap();
    assert_eq!(current.status, ReviewStatus::Pending);
    assert_eq!(current.decision, None);
    assert_eq!(current.competing_matches, vec![review_match(202, "Deluxe")]);
    let run = catalog.get_run(current_run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
}

#[test]
fn refreshing_a_review_keeps_incompatible_historical_acceptance_reviewable() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let historical_run = catalog.create_run(request()).unwrap();
    let current_run = catalog.create_run(request()).unwrap();
    let identity = "connector:stale-accepted-refresh";
    let current_work_key = "connector:stale-accepted-refresh-work";
    catalog
        .queue_work(current_run.id, current_work_key.to_owned())
        .unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: historical_run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let historical = catalog
        .find_review_item_for_run_by_candidate_identity(historical_run.id, identity)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(
            historical.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();
    catalog
        .set_review_status(historical.id, ReviewStatus::Applied)
        .unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: current_run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(202, "Deluxe")],
        })
        .unwrap();
    let current = catalog
        .find_review_item_for_run_by_candidate_identity(current_run.id, identity)
        .unwrap()
        .unwrap();
    let claim = catalog
        .claim_review_item_for_processing(current.id)
        .unwrap()
        .unwrap();

    let refreshed = catalog
        .refresh_review_processing_and_complete_work(
            current.id,
            &claim.lease_token,
            NewReviewItem {
                run_id: current_run.id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(202, "Deluxe")],
            },
            current_work_key,
            ReviewStatus::Pending,
        )
        .unwrap();

    assert_eq!(refreshed.status, ReviewStatus::Pending);
    assert_eq!(refreshed.decision, None);
    assert_eq!(
        refreshed.competing_matches,
        vec![review_match(202, "Deluxe")]
    );
    let run = catalog.get_run(current_run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
}

#[test]
fn staging_a_review_reuses_a_terminal_rejection_for_the_same_candidate() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let first_run = catalog.create_run(request()).unwrap();
    let current_run = catalog.create_run(request()).unwrap();
    let identity = "connector:stage-terminal-rejection";
    let work_key = "connector:stage-terminal-rejection-work";
    catalog
        .queue_work(current_run.id, work_key.to_owned())
        .unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: first_run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let terminal = catalog
        .find_review_item_for_run_by_candidate_identity(first_run.id, identity)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(terminal.id, ReviewDecision::Reject)
        .unwrap();

    catalog
        .stage_review_item_and_complete_work(
            NewReviewItem {
                run_id: current_run.id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(201, "Standard")],
            },
            work_key,
        )
        .unwrap();

    let staged = catalog
        .find_review_item_for_run_by_candidate_identity(current_run.id, identity)
        .unwrap()
        .unwrap();
    assert_eq!(staged.run_id, current_run.id);
    assert_eq!(staged.status, ReviewStatus::Rejected);
    assert_eq!(staged.decision, Some(ReviewDecision::Reject));
    let run = catalog.get_run(current_run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
}

#[test]
fn staging_a_review_reuses_a_compatible_terminal_acceptance_without_completing_work() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let first_run = catalog.create_run(request()).unwrap();
    let current_run = catalog.create_run(request()).unwrap();
    let identity = "connector:stage-terminal-acceptance";
    let work_key = "connector:stage-terminal-acceptance-work";
    catalog
        .queue_work(current_run.id, work_key.to_owned())
        .unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: first_run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let terminal = catalog
        .find_review_item_for_run_by_candidate_identity(first_run.id, identity)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(
            terminal.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();

    catalog
        .stage_review_item_and_complete_work(
            NewReviewItem {
                run_id: current_run.id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(201, "Standard")],
            },
            work_key,
        )
        .unwrap();

    let staged = catalog
        .find_review_item_for_run_by_candidate_identity(current_run.id, identity)
        .unwrap()
        .unwrap();
    assert_eq!(staged.run_id, current_run.id);
    assert_eq!(staged.status, ReviewStatus::Accepted);
    assert_eq!(
        staged.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201
        })
    );
    let run = catalog.get_run(current_run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 1);
    assert_eq!(run.completed_work, 0);
    assert_eq!(
        catalog
            .next_queued_work(current_run.id)
            .unwrap()
            .unwrap()
            .key,
        work_key
    );
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
fn latest_human_decision_wins_over_newer_review_occurrence_id() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let older_run = catalog.create_run(request()).unwrap();
    let newer_run = catalog.create_run(request()).unwrap();
    let future_run = catalog.create_run(request()).unwrap();
    let identity = "connector:decision-order-race";
    let older_work_key = "connector:decision-order-older";
    let newer_work_key = "connector:decision-order-newer";
    let future_work_key = "connector:decision-order-future";
    for (run_id, work_key) in [
        (older_run.id, older_work_key),
        (newer_run.id, newer_work_key),
        (future_run.id, future_work_key),
    ] {
        catalog.queue_work(run_id, work_key.to_owned()).unwrap();
    }
    catalog
        .persist_review_item(NewReviewItem {
            run_id: older_run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: newer_run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(202, "Deluxe")],
        })
        .unwrap();
    let older = catalog
        .find_review_item_for_run_by_candidate_identity(older_run.id, identity)
        .unwrap()
        .unwrap();
    let newer = catalog
        .find_review_item_for_run_by_candidate_identity(newer_run.id, identity)
        .unwrap()
        .unwrap();
    let older_claim = catalog
        .claim_review_item_for_processing(older.id)
        .unwrap()
        .unwrap();

    catalog
        .accept_review_item_and_requeue(newer.id, 202, newer_work_key)
        .unwrap();
    let restored = catalog
        .restore_review_item_processing(older.id, &older_claim.lease_token, ReviewStatus::Pending)
        .unwrap()
        .unwrap();
    assert_eq!(restored.status, ReviewStatus::Pending);
    catalog
        .set_review_decision(older.id, ReviewDecision::Reject)
        .unwrap();

    catalog
        .stage_review_item_and_complete_work(
            NewReviewItem {
                run_id: future_run.id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(202, "Deluxe")],
            },
            future_work_key,
        )
        .unwrap();

    let future = catalog
        .find_review_item_for_run_by_candidate_identity(future_run.id, identity)
        .unwrap()
        .unwrap();
    assert_eq!(future.status, ReviewStatus::Rejected);
    assert_eq!(future.decision, Some(ReviewDecision::Reject));
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
    let store = ContentAddressedStore::new(temp.path());
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

    let staged = store.stage_original_bytes(b"late rejected bytes").unwrap();
    let stored = staged.stored_object().clone();
    let object_path = store.object_path(&stored.hash);
    assert!(!object_path.exists());

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
                object_hash: stored.hash.clone(),
                byte_len: stored.byte_len,
                original_filename: "front.png".to_owned(),
                source_id: SourceId::from("fixture-provider"),
                source_asset_label: Some("front".to_owned()),
                source_location: "fixture://candidate/front".to_owned(),
            },
            Some(staged),
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
    assert!(!object_path.exists());
}

#[test]
fn terminal_acceptance_reuses_processing_bytes_without_requeueing() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "INSERT INTO games (id, title, normalized_title)
             VALUES (301, 'Target Game', 'target game');
             INSERT INTO release_editions (
                 id, game_id, platform, normalized_platform, region, normalized_region,
                 edition_name, normalized_edition_name
             ) VALUES (
                 201, 301, 'Nintendo Entertainment System', 'nintendo entertainment system',
                 'USA', 'usa', 'Standard', 'standard'
             );",
        )
        .unwrap();
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
                object_hash: "late-accepted-hash".to_owned(),
                byte_len: 42,
                original_filename: "front.png".to_owned(),
                source_id: SourceId::from("fixture-provider"),
                source_asset_label: Some("front".to_owned()),
                source_location: "fixture://candidate/front".to_owned(),
            },
            None,
        )
        .unwrap();

    let ReviewProcessingFinalization::Imported(imported) = outcome else {
        panic!("expected the already stored bytes to be imported");
    };
    assert_eq!(imported.release_edition_id, 201);
    assert_eq!(imported.object_hash, "late-accepted-hash");
    let processing = catalog.get_review_item(processing.id).unwrap().unwrap();
    assert_eq!(processing.status, ReviewStatus::Applied);
    assert_eq!(
        processing.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201,
        })
    );
    let run = catalog.get_run(processing_run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
    let library = catalog.list_library().unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].release_edition_id, 201);
    assert_eq!(library[0].assets.len(), 1);
}

#[test]
fn processing_asset_finalization_persists_asset_work_and_review_together() {
    let temp = tempdir().unwrap();
    let catalog_path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&catalog_path).unwrap();
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
    let journal_seen = Arc::new(Mutex::new(false));

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
            Some(Box::new(JournalCheckingOriginal {
                stored: StoredObject {
                    hash: "atomic-finalization-hash".to_owned(),
                    byte_len: 42,
                },
                catalog_path: catalog_path.clone(),
                journal_seen: Arc::clone(&journal_seen),
            })),
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
    assert!(*journal_seen.lock().unwrap());
    let pending: i64 = Connection::open(&catalog_path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM pending_object_publications",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
}

#[test]
fn processing_asset_prepares_staged_original_before_taking_sqlite_write_lock() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = catalog.create_run(request()).unwrap();
    let work_key = "connector:prepare-before-write-lock";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: "connector:prepare-before-write-lock".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    let claim = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();
    let stored = StoredObject {
        hash: "prepared-before-write-lock-hash".to_owned(),
        byte_len: 42,
    };
    let gate = Arc::new((Mutex::new(PublishGateState::default()), Condvar::new()));
    let writer_gate = Arc::clone(&gate);
    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
        let (mutex, condvar) = &*writer_gate;
        let mut state = mutex.lock().unwrap();
        while !state.entered {
            state = condvar.wait(state).unwrap();
        }
        drop(state);

        let acquired = Connection::open(writer_path)
            .unwrap()
            .execute_batch("PRAGMA busy_timeout = 100; BEGIN IMMEDIATE; COMMIT;")
            .is_ok();

        let mut state = mutex.lock().unwrap();
        state.release = true;
        condvar.notify_all();
        acquired
    });
    let staged = Box::new(BlockingPreparedOriginal {
        stored: stored.clone(),
        gate,
    });

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
                object_hash: stored.hash,
                byte_len: stored.byte_len,
                original_filename: "front.png".to_owned(),
                source_id: SourceId::from("fixture-provider"),
                source_asset_label: Some("front".to_owned()),
                source_location: "fixture://candidate/front".to_owned(),
            },
            Some(staged),
        )
        .unwrap();

    assert!(matches!(outcome, ReviewProcessingFinalization::Imported(_)));
    assert!(
        writer.join().unwrap(),
        "staged object preparation ran while the SQLite write lock was already held"
    );
}

#[test]
fn processing_asset_finalization_rolls_back_if_review_transition_fails() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let store = ContentAddressedStore::new(temp.path());
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

    let staged = store
        .stage_original_bytes(b"processing rollback bytes")
        .unwrap();
    let stored = staged.stored_object().clone();
    let object_path = store.object_path(&stored.hash);

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
            object_hash: stored.hash,
            byte_len: stored.byte_len,
            original_filename: "front.png".to_owned(),
            source_id: SourceId::from("fixture-provider"),
            source_asset_label: Some("front".to_owned()),
            source_location: "fixture://candidate/front".to_owned(),
        },
        Some(staged),
    );

    assert!(result.is_err());
    let review = catalog.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(review.status, ReviewStatus::Processing);
    let run = catalog.get_run(run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 1);
    assert_eq!(run.completed_work, 0);
    assert!(catalog.list_library().unwrap().is_empty());
    assert!(!object_path.exists());
    let pending: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM pending_object_publications",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
}

#[test]
fn publication_failure_after_bytes_become_visible_keeps_recovery_journal() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog_path = vault.join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&catalog_path).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let run = catalog.create_run(request()).unwrap();
    let work_key = "connector:publication-failure-recovery";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: "connector:publication-failure-recovery".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    let claim = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();
    let stored = StoredObject {
        hash: "ab".repeat(32),
        byte_len: 22,
    };
    let object_path = store.object_path(&stored.hash);

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
            object_hash: stored.hash.clone(),
            byte_len: stored.byte_len,
            original_filename: "front.png".to_owned(),
            source_id: SourceId::from("fixture-provider"),
            source_asset_label: Some("front".to_owned()),
            source_location: "fixture://candidate/front".to_owned(),
        },
        Some(Box::new(VisibleThenFailingOriginal {
            stored: stored.clone(),
            object_path: object_path.clone(),
        })),
    );

    assert!(result.is_err());
    assert!(object_path.is_file());
    assert!(catalog.list_library().unwrap().is_empty());
    let connection = Connection::open(&catalog_path).unwrap();
    let pending: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pending_object_publications WHERE object_hash = ?1",
            [&stored.hash],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1);
    connection
        .execute(
            "UPDATE pending_object_publications SET created_at_unix = 0 WHERE object_hash = ?1",
            [&stored.hash],
        )
        .unwrap();
    drop(connection);

    SqliteCatalog::open_existing(&catalog_path).unwrap();

    assert!(!object_path.exists());
    let pending: i64 = Connection::open(&catalog_path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM pending_object_publications WHERE object_hash = ?1",
            [&stored.hash],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
}

#[test]
fn processing_review_refresh_updates_snapshot_work_and_status_together() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let run = catalog.create_run(request()).unwrap();
    let work_key = "connector:processing-refresh";
    let identity = "connector:processing-refresh-candidate";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    let claim = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();
    let mut refreshed_candidate = candidate();
    refreshed_candidate.original_filename = "refreshed-front.png".to_owned();

    let refreshed = catalog
        .refresh_review_processing_and_complete_work(
            item.id,
            &claim.lease_token,
            NewReviewItem {
                run_id: run.id,
                candidate_identity: identity.to_owned(),
                candidate: refreshed_candidate.clone(),
                competing_matches: vec![review_match(202, "Deluxe")],
            },
            work_key,
            ReviewStatus::Pending,
        )
        .unwrap();

    assert_eq!(refreshed.status, ReviewStatus::Pending);
    assert_eq!(refreshed.candidate, refreshed_candidate);
    assert_eq!(
        refreshed.competing_matches,
        vec![review_match(202, "Deluxe")]
    );
    let run = catalog.get_run(run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
}

#[test]
fn processing_review_refresh_rolls_back_if_status_restore_fails() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = catalog.create_run(request()).unwrap();
    let work_key = "connector:processing-refresh-rollback";
    let identity = "connector:processing-refresh-rollback-candidate";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: identity.to_owned(),
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
            "CREATE TRIGGER fail_processing_refresh_restore
             BEFORE UPDATE OF status ON review_items
             WHEN OLD.status = 'processing' AND NEW.status = 'pending'
             BEGIN
                 SELECT RAISE(ABORT, 'forced processing refresh restore failure');
             END;",
        )
        .unwrap();
    let mut refreshed_candidate = candidate();
    refreshed_candidate.original_filename = "should-rollback.png".to_owned();

    let result = catalog.refresh_review_processing_and_complete_work(
        item.id,
        &claim.lease_token,
        NewReviewItem {
            run_id: run.id,
            candidate_identity: identity.to_owned(),
            candidate: refreshed_candidate,
            competing_matches: vec![review_match(202, "Deluxe")],
        },
        work_key,
        ReviewStatus::Pending,
    );

    assert!(result.is_err());
    let persisted = catalog.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(persisted.status, ReviewStatus::Processing);
    assert_eq!(persisted.candidate, candidate());
    assert_eq!(
        persisted.competing_matches,
        vec![review_match(201, "Standard")]
    );
    let run = catalog.get_run(run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 1);
    assert_eq!(run.completed_work, 0);
    assert!(
        catalog
            .renew_review_item_processing(item.id, &claim.lease_token)
            .unwrap()
    );
}

#[test]
fn accepted_review_finalization_persists_asset_work_and_status_together() {
    let temp = tempdir().unwrap();
    let catalog_path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&catalog_path).unwrap();
    let run = catalog.create_run(request()).unwrap();
    let work_key = "connector:accepted-atomic-finalization";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: "connector:accepted-atomic-finalization".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    catalog
        .set_review_decision(
            item.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();
    let claim = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();
    let journal_seen = Arc::new(Mutex::new(false));

    let outcome = catalog
        .finalize_accepted_review_asset(
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
                object_hash: "accepted-atomic-hash".to_owned(),
                byte_len: 42,
                original_filename: "front.png".to_owned(),
                source_id: SourceId::from("fixture-provider"),
                source_asset_label: Some("front".to_owned()),
                source_location: "fixture://candidate/front".to_owned(),
            },
            Some(Box::new(JournalCheckingOriginal {
                stored: StoredObject {
                    hash: "accepted-atomic-hash".to_owned(),
                    byte_len: 42,
                },
                catalog_path: catalog_path.clone(),
                journal_seen: Arc::clone(&journal_seen),
            })),
        )
        .unwrap();

    let ReviewProcessingFinalization::Imported(imported) = outcome else {
        panic!("expected the accepted review asset to be imported");
    };

    assert_eq!(imported.object_hash, "accepted-atomic-hash");
    assert_eq!(
        catalog.get_review_item(item.id).unwrap().unwrap().status,
        ReviewStatus::Applied
    );
    let run = catalog.get_run(run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
    assert_eq!(catalog.list_library().unwrap()[0].assets.len(), 1);
    assert!(*journal_seen.lock().unwrap());
    let pending: i64 = Connection::open(&catalog_path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM pending_object_publications",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
}

#[test]
fn accepted_review_prepares_staged_original_before_taking_sqlite_write_lock() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = catalog.create_run(request()).unwrap();
    let work_key = "connector:accepted-prepare-before-write-lock";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: "connector:accepted-prepare-before-write-lock".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    catalog
        .set_review_decision(
            item.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();
    let claim = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();
    let stored = StoredObject {
        hash: "accepted-prepared-before-write-lock-hash".to_owned(),
        byte_len: 42,
    };
    let gate = Arc::new((Mutex::new(PublishGateState::default()), Condvar::new()));
    let writer_gate = Arc::clone(&gate);
    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
        let (mutex, condvar) = &*writer_gate;
        let mut state = mutex.lock().unwrap();
        while !state.entered {
            state = condvar.wait(state).unwrap();
        }
        drop(state);

        let acquired = Connection::open(writer_path)
            .unwrap()
            .execute_batch("PRAGMA busy_timeout = 100; BEGIN IMMEDIATE; COMMIT;")
            .is_ok();

        let mut state = mutex.lock().unwrap();
        state.release = true;
        condvar.notify_all();
        acquired
    });
    let staged = Box::new(BlockingPreparedOriginal {
        stored: stored.clone(),
        gate,
    });

    let outcome = catalog
        .finalize_accepted_review_asset(
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
                object_hash: stored.hash,
                byte_len: stored.byte_len,
                original_filename: "front.png".to_owned(),
                source_id: SourceId::from("fixture-provider"),
                source_asset_label: Some("front".to_owned()),
                source_location: "fixture://candidate/front".to_owned(),
            },
            Some(staged),
        )
        .unwrap();

    assert!(matches!(outcome, ReviewProcessingFinalization::Imported(_)));
    assert!(
        writer.join().unwrap(),
        "accepted staged object preparation ran while the SQLite write lock was already held"
    );
}

#[test]
fn accepted_review_finalization_reconciles_a_late_terminal_sibling() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(temp.path());
    let processing_run = catalog.create_run(request()).unwrap();
    let competing_run = catalog.create_run(request()).unwrap();
    let identity = "connector:accepted-late-terminal-sibling";
    let work_key = "connector:accepted-late-terminal-sibling-work";
    catalog
        .queue_work(processing_run.id, work_key.to_owned())
        .unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: processing_run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let processing = catalog
        .find_review_item_for_run_by_candidate_identity(processing_run.id, identity)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(
            processing.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();
    let claim = catalog
        .claim_review_item_for_processing(processing.id)
        .unwrap()
        .unwrap();

    catalog
        .persist_review_item(NewReviewItem {
            run_id: competing_run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let competing = catalog
        .find_review_item_for_run_by_candidate_identity(competing_run.id, identity)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(competing.id, ReviewDecision::Reject)
        .unwrap();

    let staged = store
        .stage_original_bytes(b"late rejected accepted-review bytes")
        .unwrap();
    let stored = staged.stored_object().clone();
    let object_path = store.object_path(&stored.hash);

    catalog
        .finalize_accepted_review_asset(
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
                object_hash: stored.hash,
                byte_len: stored.byte_len,
                original_filename: "front.png".to_owned(),
                source_id: SourceId::from("fixture-provider"),
                source_asset_label: Some("front".to_owned()),
                source_location: "fixture://candidate/front".to_owned(),
            },
            Some(staged),
        )
        .unwrap();

    let processing = catalog.get_review_item(processing.id).unwrap().unwrap();
    assert_eq!(processing.status, ReviewStatus::Rejected);
    assert_eq!(processing.decision, Some(ReviewDecision::Reject));
    let run = catalog.get_run(processing_run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
    assert!(catalog.list_library().unwrap().is_empty());
    assert!(!object_path.exists());
}

#[test]
fn accepted_review_finalization_reuses_bytes_for_changed_compatible_acceptance() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "INSERT INTO games (id, title, normalized_title)
             VALUES
                 (301, 'Target Game Standard', 'target game standard'),
                 (302, 'Target Game Deluxe', 'target game deluxe');
             INSERT INTO release_editions (
                 id, game_id, platform, normalized_platform, region, normalized_region,
                 edition_name, normalized_edition_name
             ) VALUES
                 (201, 301, 'Nintendo Entertainment System', 'nintendo entertainment system',
                  'USA', 'usa', 'Standard', 'standard'),
                 (202, 302, 'Nintendo Entertainment System', 'nintendo entertainment system',
                  'USA', 'usa', 'Deluxe', 'deluxe');",
        )
        .unwrap();
    let processing_run = catalog.create_run(request()).unwrap();
    let competing_run = catalog.create_run(request()).unwrap();
    let identity = "connector:accepted-changed-compatible-sibling";
    let work_key = "connector:accepted-changed-compatible-work";
    catalog
        .queue_work(processing_run.id, work_key.to_owned())
        .unwrap();

    for run_id in [processing_run.id, competing_run.id] {
        catalog
            .persist_review_item(NewReviewItem {
                run_id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(201, "Standard"), review_match(202, "Deluxe")],
            })
            .unwrap();
    }

    let processing = catalog
        .find_review_item_for_run_by_candidate_identity(processing_run.id, identity)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(
            processing.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();
    let claim = catalog
        .claim_review_item_for_processing(processing.id)
        .unwrap()
        .unwrap();

    let competing = catalog
        .find_review_item_for_run_by_candidate_identity(competing_run.id, identity)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(
            competing.id,
            ReviewDecision::Accept {
                release_edition_id: 202,
            },
        )
        .unwrap();

    let outcome = catalog
        .finalize_accepted_review_asset(
            processing.id,
            &claim.lease_token,
            processing_run.id,
            work_key,
            PersistAsset {
                existing_game_id: None,
                existing_release_edition_id: Some(201),
                match_decision: None,
                game_title: "Target Game".to_owned(),
                platform: "Nintendo Entertainment System".to_owned(),
                region: "USA".to_owned(),
                edition_name: "Collector".to_owned(),
                asset_type: AssetType::BoxFront,
                object_hash: "changed-compatible-accepted-hash".to_owned(),
                byte_len: 42,
                original_filename: "front.png".to_owned(),
                source_id: SourceId::from("fixture-provider"),
                source_asset_label: Some("front".to_owned()),
                source_location: "fixture://candidate/front".to_owned(),
            },
            None,
        )
        .unwrap();

    let ReviewProcessingFinalization::Imported(imported) = outcome else {
        panic!("expected the already stored bytes to follow the changed acceptance");
    };
    assert_eq!(imported.release_edition_id, 202);
    assert_eq!(imported.object_hash, "changed-compatible-accepted-hash");
    let processing = catalog.get_review_item(processing.id).unwrap().unwrap();
    assert_eq!(processing.status, ReviewStatus::Applied);
    assert_eq!(
        processing.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 202,
        })
    );
    let run = catalog.get_run(processing_run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
    assert_eq!(catalog.list_library().unwrap()[0].release_edition_id, 202);
}

#[test]
fn accepted_review_finalization_rolls_back_if_status_transition_fails() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let store = ContentAddressedStore::new(temp.path());
    let run = catalog.create_run(request()).unwrap();
    let work_key = "connector:accepted-atomic-rollback";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: "connector:accepted-atomic-rollback".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    catalog
        .set_review_decision(
            item.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();
    let claim = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_applied
             BEFORE UPDATE OF status ON review_items
             WHEN NEW.status = 'applied'
             BEGIN
                 SELECT RAISE(ABORT, 'forced applied transition failure');
             END;",
        )
        .unwrap();

    let staged = store
        .stage_original_bytes(b"accepted rollback bytes")
        .unwrap();
    let stored = staged.stored_object().clone();
    let object_path = store.object_path(&stored.hash);

    let result = catalog.finalize_accepted_review_asset(
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
            object_hash: stored.hash,
            byte_len: stored.byte_len,
            original_filename: "front.png".to_owned(),
            source_id: SourceId::from("fixture-provider"),
            source_asset_label: Some("front".to_owned()),
            source_location: "fixture://candidate/front".to_owned(),
        },
        Some(staged),
    );

    assert!(result.is_err());
    assert_eq!(
        catalog.get_review_item(item.id).unwrap().unwrap().status,
        ReviewStatus::Processing
    );
    let run = catalog.get_run(run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 1);
    assert_eq!(run.completed_work, 0);
    assert!(catalog.list_library().unwrap().is_empty());
    assert!(!object_path.exists());
    let pending: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM pending_object_publications",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
    assert!(
        catalog
            .renew_review_item_processing(item.id, &claim.lease_token)
            .unwrap()
    );
}

#[test]
fn accepted_review_processing_remains_visible_to_later_runs() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let first_run = catalog.create_run(request()).unwrap();
    let later_run = catalog.create_run(request()).unwrap();
    let identity = "connector:accepted-processing-global-visibility";

    catalog
        .persist_review_item(NewReviewItem {
            run_id: first_run.id,
            candidate_identity: identity.to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog
        .find_review_item_for_run_by_candidate_identity(first_run.id, identity)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(
            item.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();
    catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();

    let visible = catalog
        .find_review_item_for_run_by_candidate_identity(later_run.id, identity)
        .unwrap()
        .unwrap();

    assert_eq!(visible.id, item.id);
    assert_eq!(visible.status, ReviewStatus::Processing);
    assert_eq!(
        visible.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201,
        })
    );
}

#[test]
fn accepted_review_claim_is_exclusive_and_recovers_to_accepted_after_expiry() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = catalog.create_run(request()).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: "connector:accepted-exclusive-claim".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    catalog
        .set_review_decision(
            item.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();

    let first = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();
    assert_eq!(first.item.status, ReviewStatus::Processing);
    assert!(
        catalog
            .claim_review_item_for_processing(item.id)
            .unwrap()
            .is_none()
    );

    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE review_processing_leases SET acquired_at = unixepoch() - 3601 WHERE review_item_id = ?1",
            [item.id],
        )
        .unwrap();
    catalog.recover_expired_review_processing(run.id).unwrap();

    let recovered = catalog.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(recovered.status, ReviewStatus::Accepted);
    assert_eq!(
        recovered.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201
        })
    );
}

#[test]
fn processing_review_supersession_completes_work_and_status_together() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let run = catalog.create_run(request()).unwrap();
    let work_key = "connector:processing-supersede";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: "connector:processing-supersede".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    let item = catalog.list_review_items().unwrap().remove(0);
    let claim = catalog
        .claim_review_item_for_processing(item.id)
        .unwrap()
        .unwrap();

    let finalized = catalog
        .supersede_review_processing_and_complete_work(
            item.id,
            &claim.lease_token,
            run.id,
            work_key,
        )
        .unwrap()
        .unwrap();

    assert_eq!(finalized.status, ReviewStatus::Superseded);
    let run = catalog.get_run(run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
}

#[test]
fn terminal_rejection_wins_over_processing_supersession() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let terminal_run = catalog.create_run(request()).unwrap();
    let processing_run = catalog.create_run(request()).unwrap();
    let identity = "connector:terminal-rejection-supersede-race";
    let work_key = "connector:terminal-rejection-supersede-work";
    catalog
        .queue_work(processing_run.id, work_key.to_owned())
        .unwrap();
    for run_id in [terminal_run.id, processing_run.id] {
        catalog
            .persist_review_item(NewReviewItem {
                run_id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(201, "Standard")],
            })
            .unwrap();
    }
    let terminal = catalog
        .find_review_item_for_run_by_candidate_identity(terminal_run.id, identity)
        .unwrap()
        .unwrap();
    let processing = catalog
        .find_review_item_for_run_by_candidate_identity(processing_run.id, identity)
        .unwrap()
        .unwrap();
    let claim = catalog
        .claim_review_item_for_processing(processing.id)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(terminal.id, ReviewDecision::Reject)
        .unwrap();

    let finalized = catalog
        .supersede_review_processing_and_complete_work(
            processing.id,
            &claim.lease_token,
            processing_run.id,
            work_key,
        )
        .unwrap()
        .unwrap();

    assert_eq!(finalized.status, ReviewStatus::Rejected);
    assert_eq!(finalized.decision, Some(ReviewDecision::Reject));
    let run = catalog.get_run(processing_run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 1);
}

#[test]
fn compatible_terminal_acceptance_wins_over_processing_supersession() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    let terminal_run = catalog.create_run(request()).unwrap();
    let processing_run = catalog.create_run(request()).unwrap();
    let identity = "connector:terminal-acceptance-supersede-race";
    let work_key = "connector:terminal-acceptance-supersede-work";
    catalog
        .queue_work(processing_run.id, work_key.to_owned())
        .unwrap();
    for run_id in [terminal_run.id, processing_run.id] {
        catalog
            .persist_review_item(NewReviewItem {
                run_id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(201, "Standard")],
            })
            .unwrap();
    }
    let terminal = catalog
        .find_review_item_for_run_by_candidate_identity(terminal_run.id, identity)
        .unwrap()
        .unwrap();
    let processing = catalog
        .find_review_item_for_run_by_candidate_identity(processing_run.id, identity)
        .unwrap()
        .unwrap();
    let claim = catalog
        .claim_review_item_for_processing(processing.id)
        .unwrap()
        .unwrap();
    catalog
        .set_review_decision(
            terminal.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();
    catalog
        .set_review_status(terminal.id, ReviewStatus::Applied)
        .unwrap();

    let finalized = catalog
        .supersede_review_processing_and_complete_work(
            processing.id,
            &claim.lease_token,
            processing_run.id,
            work_key,
        )
        .unwrap()
        .unwrap();

    assert_eq!(finalized.status, ReviewStatus::Accepted);
    assert_eq!(
        finalized.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201
        })
    );
    let run = catalog.get_run(processing_run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 1);
    assert_eq!(run.completed_work, 0);
}

#[test]
fn processing_review_supersession_rolls_back_if_status_transition_fails() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run = catalog.create_run(request()).unwrap();
    let work_key = "connector:processing-supersede-rollback";
    catalog.queue_work(run.id, work_key.to_owned()).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: "connector:processing-supersede-rollback".to_owned(),
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
            "CREATE TRIGGER fail_superseded
             BEFORE UPDATE OF status ON review_items
             WHEN NEW.status = 'superseded'
             BEGIN
                 SELECT RAISE(ABORT, 'forced superseded transition failure');
             END;",
        )
        .unwrap();

    let result = catalog.supersede_review_processing_and_complete_work(
        item.id,
        &claim.lease_token,
        run.id,
        work_key,
    );

    assert!(result.is_err());
    assert_eq!(
        catalog.get_review_item(item.id).unwrap().unwrap().status,
        ReviewStatus::Processing
    );
    let run = catalog.get_run(run.id).unwrap().unwrap();
    assert_eq!(run.queued_work, 1);
    assert_eq!(run.completed_work, 0);
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
fn migrating_unsequenced_conflicting_decisions_keeps_future_occurrences_reviewable() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let identity = "connector:legacy-ambiguous-decision-order";
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
    let lower_id = items.iter().find(|item| item.run_id == 7).unwrap();
    let higher_id = items.iter().find(|item| item.run_id == 8).unwrap();
    let lower_id = lower_id.id;
    let higher_id = higher_id.id;
    drop(catalog);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE review_items
             SET decision_json = '{\"decision\":\"reject\"}', status = 'rejected'
             WHERE id = ?1",
            [lower_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE review_items
             SET decision_json = '{\"decision\":\"accept\",\"release_edition_id\":201}',
                 status = 'accepted'
             WHERE id = ?1",
            [higher_id],
        )
        .unwrap();
    connection
        .execute_batch("ALTER TABLE review_items DROP COLUMN decision_seq;")
        .unwrap();
    drop(connection);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let legacy_sequences: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM review_items WHERE decision_seq IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy_sequences, 0);

    let run = reopened.create_run(request()).unwrap();
    let work_key = "connector:legacy-ambiguous-decision-order-work";
    reopened.queue_work(run.id, work_key.to_owned()).unwrap();
    reopened
        .stage_review_item_and_complete_work(
            NewReviewItem {
                run_id: run.id,
                candidate_identity: identity.to_owned(),
                candidate: candidate(),
                competing_matches: vec![review_match(201, "Standard")],
            },
            work_key,
        )
        .unwrap();

    let staged = reopened
        .find_review_item_for_run_by_candidate_identity(run.id, identity)
        .unwrap()
        .unwrap();
    assert_eq!(staged.status, ReviewStatus::Pending);
    assert_eq!(staged.decision, None);
}

#[test]
fn migrating_legacy_review_identity_preserves_processing_acceptance() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: 7,
            candidate_identity: "connector:legacy-processing-accept".to_owned(),
            candidate: candidate(),
            competing_matches: vec![review_match(201, "Standard")],
        })
        .unwrap();
    drop(catalog);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP INDEX IF EXISTS idx_review_items_run;
             DROP INDEX IF EXISTS idx_review_items_status;
             DROP INDEX IF EXISTS idx_review_items_run_status;
             DROP INDEX IF EXISTS idx_review_items_candidate_identity;
             DROP TABLE review_processing_leases;
             ALTER TABLE review_items RENAME TO review_items_current;
             CREATE TABLE review_items (
                 id INTEGER PRIMARY KEY,
                 run_id INTEGER NOT NULL,
                 candidate_identity TEXT NOT NULL UNIQUE,
                 candidate_json TEXT NOT NULL,
                 competing_matches_json TEXT NOT NULL,
                 decision_json TEXT,
                 status TEXT NOT NULL DEFAULT 'pending'
             );
             INSERT INTO review_items (
                 id, run_id, candidate_identity, candidate_json,
                 competing_matches_json, decision_json, status
             )
             SELECT id, run_id, candidate_identity, candidate_json,
                    competing_matches_json,
                    '{\"decision\":\"accept\",\"release_edition_id\":201}',
                    'processing'
             FROM review_items_current;
             DROP TABLE review_items_current;",
        )
        .unwrap();
    drop(connection);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let migrated = reopened.list_review_items().unwrap().remove(0);

    assert_eq!(migrated.status, ReviewStatus::Accepted);
    assert_eq!(
        migrated.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201
        })
    );
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
