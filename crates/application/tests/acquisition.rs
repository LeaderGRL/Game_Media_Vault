mod support;

use game_media_vault_application::{
    ApplicationError, ErrorKind, ReviewRepositoryPort, RunRepositoryPort,
    acquire_run_with_connector, candidate_identity, resolve_review_item,
    start_acquisition_run_for_connector,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRunStatus,
    AssetCandidate, AssetType, AssetTypeSelector, ImportedAsset, LibraryAsset, LibraryEntry,
    MatchConfidence, MatchingPolicy, MediaInfo, NewReviewItem, Outranked, PreferenceReason,
    QualityRequirements, QualityShortfall, RetentionPolicy, ReviewDecision, ReviewStatus, SourceId,
    SourceSelection,
};
use support::*;

fn execute_with(
    vault: &FakeVault,
    connector: &FakeConnector,
    run_id: i64,
    matching_policy: MatchingPolicy,
) -> Result<Vec<ImportedAsset>, ApplicationError> {
    acquire_run_with_connector(
        vault,
        vault,
        vault,
        &FakeStore::default(),
        connector,
        run_id,
        matching_policy,
    )
}

fn execute(
    vault: &FakeVault,
    connector: &FakeConnector,
    run_id: i64,
) -> Result<Vec<ImportedAsset>, ApplicationError> {
    execute_with(vault, connector, run_id, matching_policy())
}

fn execute_storing(
    vault: &FakeVault,
    connector: &FakeConnector,
    run_id: i64,
    store: &FakeStore,
) -> Result<Vec<ImportedAsset>, ApplicationError> {
    acquire_run_with_connector(
        vault,
        vault,
        vault,
        store,
        connector,
        run_id,
        matching_policy(),
    )
}

/// Runs a new acquisition of `candidate` that parks it on a Review Item, and returns the run.
fn park_in_new_run(vault: &FakeVault, candidate: &AssetCandidate, policy: MatchingPolicy) -> i64 {
    let run_id = vault.start_run();
    execute_with(
        vault,
        &FakeConnector::new(vec![candidate.clone()]),
        run_id,
        policy,
    )
    .unwrap();
    assert_eq!(vault.run(run_id).awaiting_review_work, 1);
    run_id
}

#[test]
fn acquires_a_requested_box_front_through_the_connector_pipeline() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let connector = FakeConnector::new(vec![smb.clone()]);
    let run_id = vault.start_run();

    let imported = execute(&vault, &connector, run_id).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(
        connector.downloads.borrow().as_slice(),
        std::slice::from_ref(&smb.source_url)
    );
    let records = vault.records.borrow();
    assert_eq!(records[0].existing_release_edition_id, Some(73));
    assert_eq!(records[0].source_id, SourceId::from(SOURCE_ID));
    assert_eq!(records[0].source_location, smb.source_url);
    let decision = records[0].match_decision.as_ref().unwrap();
    assert_eq!(decision.confidence, MatchConfidence::High);
    assert_eq!(decision.score, 100);
    assert_eq!(decision.evidence.len(), 4);
    let run = vault.run(run_id);
    assert_eq!(run.status, AcquisitionRunStatus::Completed);
    assert_eq!((run.queued_work, run.completed_work), (0, 1));
}

#[test]
fn low_confidence_candidate_is_left_unattached_without_downloading() {
    let unknown = candidate("Unknown Game");
    let vault = FakeVault::with_library(vec![release_for(&candidate("Other Game"), 9)]);
    let connector = FakeConnector::new(vec![unknown]);
    let run_id = vault.start_run();

    let imported = execute(&vault, &connector, run_id).unwrap();

    assert!(imported.is_empty());
    assert!(connector.downloads.borrow().is_empty());
    assert!(vault.review_items.borrow().is_empty());
    assert_eq!(vault.run(run_id).completed_work, 1);
}

#[test]
fn medium_confidence_candidate_parks_its_work_on_a_review_item_with_competing_evidence() {
    let (ambiguous, library) = ambiguous_candidate_and_releases();
    let vault = FakeVault::with_library(library);
    let connector = FakeConnector::new(vec![ambiguous.clone()]);
    let run_id = vault.start_run();

    let imported = execute(&vault, &connector, run_id).unwrap();

    assert!(imported.is_empty());
    assert!(connector.downloads.borrow().is_empty());
    let item = vault.review_item(0);
    assert_eq!(item.status, ReviewStatus::Pending);
    assert_eq!(item.candidate, ambiguous);
    assert_eq!(
        item.candidate_identity,
        candidate_identity(SOURCE_ID, &ambiguous)
    );
    let competing = item
        .competing_matches
        .iter()
        .map(|candidate_match| (candidate_match.release_edition_id, candidate_match.score))
        .collect::<Vec<_>>();
    assert_eq!(competing, vec![(401, 90), (402, 90)]);
    assert!(
        item.competing_matches
            .iter()
            .all(|candidate_match| candidate_match.evidence.len() == 4)
    );
    let run = vault.run(run_id);
    assert_eq!(run.status, AcquisitionRunStatus::Completed);
    assert_eq!(run.awaiting_review_work, 1);
}

#[test]
fn candidates_carrying_url_credentials_are_rejected_before_persistence() {
    let unsafe_candidate = AssetCandidate {
        source_url: "https://user:secret@example.invalid/review.png".to_owned(),
        ..candidate("Secret Game")
    };
    let vault = FakeVault::with_library(vec![release_for(&unsafe_candidate, 1)]);
    let connector = FakeConnector::new(vec![unsafe_candidate]);
    let run_id = vault.start_run();

    let error = execute(&vault, &connector, run_id).unwrap_err();

    assert!(matches!(
        error,
        ApplicationError::UnsafeCandidateLocator { .. }
    ));
    assert!(!error.to_string().contains("secret"));
    assert!(!vault.has_discovered(run_id, SOURCE_ID).unwrap());
    assert_eq!(vault.run(run_id).queued_work, 0);
    assert!(connector.downloads.borrow().is_empty());
}

#[test]
fn relative_candidate_locators_are_rejected_before_persistence() {
    let relative = AssetCandidate {
        source_url: "/download/401.png".to_owned(),
        ..candidate("Relative Game")
    };
    let vault = FakeVault::default();
    let run_id = vault.start_run();

    let error = execute(&vault, &FakeConnector::new(vec![relative]), run_id).unwrap_err();

    assert!(matches!(
        error,
        ApplicationError::UnsafeCandidateLocator { .. }
    ));
    assert_eq!(vault.run(run_id).queued_work, 0);
}

#[test]
fn locators_with_query_or_fragment_data_are_rejected_before_persistence() {
    for source_url in [
        "https://example.invalid/media.png?api_key=secret",
        "https://example.invalid/media.png#token=secret",
    ] {
        let signed = AssetCandidate {
            source_url: source_url.to_owned(),
            ..candidate("Signed Game")
        };
        let vault = FakeVault::with_library(vec![release_for(&signed, 1)]);
        let run_id = vault.start_run();

        let error = execute(&vault, &FakeConnector::new(vec![signed]), run_id).unwrap_err();

        assert!(matches!(
            error,
            ApplicationError::UnsafeCandidateLocator { .. }
        ));
        assert!(!error.to_string().contains("secret"));
        assert_eq!(vault.run(run_id).queued_work, 0);
    }
}

#[test]
fn rejects_candidates_whose_source_does_not_match_the_connector() {
    let foreign = AssetCandidate {
        source_id: SourceId::from("another-source"),
        ..candidate("Foreign Game")
    };
    let vault = FakeVault::default();
    let run_id = vault.start_run();

    let error = execute(&vault, &FakeConnector::new(vec![foreign]), run_id).unwrap_err();

    assert!(matches!(
        error,
        ApplicationError::ConnectorCandidateSourceMismatch { .. }
    ));
}

#[test]
fn a_source_is_discovered_once_and_resumes_from_persisted_work() {
    let first = candidate("First Game");
    let second = candidate("Second Game");
    let vault = FakeVault::with_library(vec![release_for(&first, 11), release_for(&second, 12)]);
    let mut connector = FakeConnector::new(vec![first.clone(), second.clone()]);
    connector
        .failing_downloads
        .insert(second.source_url.clone());
    let run_id = vault.start_run();

    execute(&vault, &connector, run_id).unwrap_err();
    assert_eq!(vault.run(run_id).queued_work, 1);

    let resumed = FakeConnector {
        discovery_fails: true,
        ..FakeConnector::new(Vec::new())
    };
    let imported = execute(&vault, &resumed, run_id).unwrap();

    assert_eq!(*connector.discover_calls.borrow(), 1);
    assert_eq!(*resumed.discover_calls.borrow(), 0);
    assert_eq!(imported.len(), 1);
    assert_eq!(resumed.downloads.borrow().as_slice(), &[second.source_url]);
    assert_eq!(vault.run(run_id).status, AcquisitionRunStatus::Completed);
}

#[test]
fn discovery_failure_leaves_the_run_resumable() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let failing = FakeConnector {
        discovery_fails: true,
        ..FakeConnector::new(Vec::new())
    };
    let run_id = vault.start_run();

    execute(&vault, &failing, run_id).unwrap_err();
    assert_eq!(vault.run(run_id).status, AcquisitionRunStatus::Running);

    let imported = execute(&vault, &FakeConnector::new(vec![smb]), run_id).unwrap();
    assert_eq!(imported.len(), 1);
}

#[test]
fn accepting_a_review_requeues_its_work_and_the_next_execution_imports_the_confirmed_match() {
    let (ambiguous, library) = ambiguous_candidate_and_releases();
    let vault = FakeVault::with_library(library);
    let connector = FakeConnector::new(vec![ambiguous]);
    let run_id = vault.start_run();
    execute(&vault, &connector, run_id).unwrap();
    let review_item_id = vault.review_item(0).id;

    resolve_review_item(
        &vault,
        review_item_id,
        ReviewDecision::Accept {
            release_edition_id: 402,
        },
    )
    .unwrap();
    assert_eq!(vault.run(run_id).status, AcquisitionRunStatus::Running);
    assert_eq!(vault.run(run_id).queued_work, 1);
    let imported = execute(&vault, &connector, run_id).unwrap();

    assert_eq!(*connector.discover_calls.borrow(), 1);
    assert_eq!(imported.len(), 1);
    let records = vault.records.borrow();
    assert_eq!(records[0].existing_release_edition_id, Some(402));
    let decision = records[0].match_decision.as_ref().unwrap();
    assert_eq!(decision.confidence, MatchConfidence::Confirmed);
    assert_eq!(decision.release_edition_id, Some(402));
    assert_eq!(vault.review_item(0).status, ReviewStatus::Accepted);
    assert_eq!(vault.run(run_id).status, AcquisitionRunStatus::Completed);
}

#[test]
fn work_requeued_while_the_run_finishes_is_executed_before_completing() {
    let (ambiguous, library) = ambiguous_candidate_and_releases();
    let vault = FakeVault::with_library(library);
    *vault.decision_before_completion.borrow_mut() = Some(ReviewDecision::Accept {
        release_edition_id: 402,
    });
    let run_id = vault.start_run();

    let imported = execute(&vault, &FakeConnector::new(vec![ambiguous]), run_id).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].release_edition_id, 402);
    assert_eq!(vault.run(run_id).status, AcquisitionRunStatus::Completed);
}

#[test]
fn accepted_decision_is_reused_by_a_later_run_without_a_new_review() {
    let (ambiguous, library) = ambiguous_candidate_and_releases();
    let vault = FakeVault::with_library(library);
    park_in_new_run(&vault, &ambiguous, matching_policy());
    resolve_review_item(
        &vault,
        vault.review_item(0).id,
        ReviewDecision::Accept {
            release_edition_id: 401,
        },
    )
    .unwrap();

    let later_run = vault.start_run();
    let connector = FakeConnector::new(vec![ambiguous]);
    let imported = execute(&vault, &connector, later_run).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].release_edition_id, 401);
    assert_eq!(vault.review_items.borrow().len(), 1);
    assert_eq!(vault.run(later_run).awaiting_review_work, 0);
}

#[test]
fn an_acceptance_of_an_edition_imported_after_the_run_started_is_honoured() {
    let (ambiguous, library) = ambiguous_candidate_and_releases();
    let vault = FakeVault::with_library(library);
    park_in_new_run(&vault, &ambiguous, matching_policy());
    resolve_review_item(
        &vault,
        vault.review_item(0).id,
        ReviewDecision::Accept {
            release_edition_id: 401,
        },
    )
    .unwrap();
    // The accepted edition reaches the catalog only after the later run read the library.
    let accepted = vault
        .library
        .borrow()
        .iter()
        .find(|release| release.release_edition_id == 401)
        .cloned()
        .unwrap();
    vault
        .library
        .borrow_mut()
        .retain(|release| release.release_edition_id != 401);
    vault
        .library_added_after_next_listing
        .borrow_mut()
        .push(accepted);

    let later_run = vault.start_run();
    let imported = execute(&vault, &FakeConnector::new(vec![ambiguous]), later_run).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].release_edition_id, 401);
}

#[test]
fn accepted_decision_follows_the_provider_identity_when_metadata_changes() {
    let (ambiguous, library) = ambiguous_candidate_and_releases();
    let ambiguous = AssetCandidate {
        provider_candidate_id: Some("provider-review-1".to_owned()),
        ..ambiguous
    };
    let vault = FakeVault::with_library(library);
    park_in_new_run(&vault, &ambiguous, matching_policy());
    resolve_review_item(
        &vault,
        vault.review_item(0).id,
        ReviewDecision::Accept {
            release_edition_id: 402,
        },
    )
    .unwrap();

    let renamed = AssetCandidate {
        game_title: "Review Game (Renamed by provider)".to_owned(),
        source_url: "https://example.invalid/moved.png".to_owned(),
        ..ambiguous
    };
    let later_run = vault.start_run();
    let imported = execute(&vault, &FakeConnector::new(vec![renamed]), later_run).unwrap();

    assert_eq!(imported[0].release_edition_id, 402);
}

#[test]
fn rejected_decision_skips_the_candidate_in_a_later_run() {
    let (ambiguous, library) = ambiguous_candidate_and_releases();
    let vault = FakeVault::with_library(library);
    let first_run = park_in_new_run(&vault, &ambiguous, matching_policy());
    resolve_review_item(&vault, vault.review_item(0).id, ReviewDecision::Reject).unwrap();
    assert_eq!(vault.run(first_run).completed_work, 1);

    let later_run = vault.start_run();
    let connector = FakeConnector::new(vec![ambiguous]);
    let imported = execute(&vault, &connector, later_run).unwrap();

    assert!(imported.is_empty());
    assert!(connector.downloads.borrow().is_empty());
    assert_eq!(vault.run(later_run).completed_work, 1);
    assert_eq!(vault.review_item(0).status, ReviewStatus::Rejected);
}

#[test]
fn deferred_review_keeps_later_run_work_parked_on_the_same_item() {
    let (ambiguous, library) = ambiguous_candidate_and_releases();
    let vault = FakeVault::with_library(library);
    park_in_new_run(&vault, &ambiguous, matching_policy());
    resolve_review_item(&vault, vault.review_item(0).id, ReviewDecision::Defer).unwrap();

    let later_run = park_in_new_run(&vault, &ambiguous, matching_policy());

    assert_eq!(vault.review_items.borrow().len(), 1);
    assert_eq!(vault.review_item(0).status, ReviewStatus::Deferred);
    assert_eq!(vault.run(later_run).status, AcquisitionRunStatus::Completed);
}

#[test]
fn later_run_auto_resolves_a_pending_review_that_became_high_confidence() {
    let (threshold, release) = threshold_candidate_and_release();
    let vault = FakeVault::with_library(vec![release]);
    let first_run = park_in_new_run(&vault, &threshold, stricter_matching_policy());

    let later_run = vault.start_run();
    let imported = execute(&vault, &FakeConnector::new(vec![threshold]), later_run).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(vault.review_item(0).status, ReviewStatus::AutoResolved);
    assert_eq!(vault.work_states(first_run), vec![WorkState::Done]);
}

#[test]
fn later_run_supersedes_a_pending_review_that_became_low_confidence() {
    let (threshold, release) = threshold_candidate_and_release();
    let vault = FakeVault::with_library(vec![release]);
    let first_run = park_in_new_run(&vault, &threshold, stricter_matching_policy());

    let later_run = vault.start_run();
    let connector = FakeConnector::new(vec![threshold]);
    let imported =
        execute_with(&vault, &connector, later_run, dismissive_matching_policy()).unwrap();

    assert!(imported.is_empty());
    assert!(connector.downloads.borrow().is_empty());
    assert_eq!(vault.review_item(0).status, ReviewStatus::Superseded);
    assert_eq!(vault.work_states(first_run), vec![WorkState::Done]);
}

#[test]
fn later_run_auto_resolves_a_review_superseded_earlier() {
    let (threshold, release) = threshold_candidate_and_release();
    let vault = FakeVault::with_library(vec![release]);
    park_in_new_run(&vault, &threshold, stricter_matching_policy());
    let connector = FakeConnector::new(vec![threshold]);
    let dismissing_run = vault.start_run();
    execute_with(
        &vault,
        &connector,
        dismissing_run,
        dismissive_matching_policy(),
    )
    .unwrap();
    assert_eq!(vault.review_item(0).status, ReviewStatus::Superseded);

    let linking_run = vault.start_run();
    let imported = execute(&vault, &connector, linking_run).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(vault.review_item(0).status, ReviewStatus::AutoResolved);
}

#[test]
fn later_run_supersedes_an_auto_resolved_review_and_drops_its_link() {
    let (threshold, release) = threshold_candidate_and_release();
    let vault = FakeVault::with_library(vec![release]);
    park_in_new_run(&vault, &threshold, stricter_matching_policy());
    let connector = FakeConnector::new(vec![threshold]);
    let linking_run = vault.start_run();
    execute(&vault, &connector, linking_run).unwrap();
    assert_eq!(vault.review_item(0).status, ReviewStatus::AutoResolved);

    let dismissing_run = vault.start_run();
    execute_with(
        &vault,
        &connector,
        dismissing_run,
        dismissive_matching_policy(),
    )
    .unwrap();

    assert_eq!(vault.review_item(0).status, ReviewStatus::Superseded);
    assert!(vault.candidate_links.borrow().is_empty());
}

#[test]
fn automatically_closed_review_reopens_when_the_candidate_is_uncertain_again() {
    let (threshold, release) = threshold_candidate_and_release();
    let vault = FakeVault::with_library(vec![release]);
    park_in_new_run(&vault, &threshold, stricter_matching_policy());
    let review_item_id = vault.review_item(0).id;
    let resolving_run = vault.start_run();
    execute(
        &vault,
        &FakeConnector::new(vec![threshold.clone()]),
        resolving_run,
    )
    .unwrap();
    assert_eq!(vault.review_item(0).status, ReviewStatus::AutoResolved);

    park_in_new_run(&vault, &threshold, stricter_matching_policy());

    assert_eq!(vault.review_items.borrow().len(), 1);
    assert_eq!(vault.review_item(0).id, review_item_id);
    assert_eq!(vault.review_item(0).status, ReviewStatus::Pending);
}

#[test]
fn later_run_refreshes_competing_matches_of_a_pending_review() {
    let (ambiguous, library) = ambiguous_candidate_and_releases();
    let vault = FakeVault::with_library(library);
    park_in_new_run(&vault, &ambiguous, matching_policy());
    vault.library.borrow_mut().push(LibraryEntry {
        edition_name: "Limited".to_owned(),
        ..release_for(&ambiguous, 403)
    });

    park_in_new_run(&vault, &ambiguous, matching_policy());

    let competing = vault
        .review_item(0)
        .competing_matches
        .iter()
        .map(|candidate_match| candidate_match.release_edition_id)
        .collect::<Vec<_>>();
    assert_eq!(competing, vec![401, 402, 403]);
}

#[test]
fn human_rejection_committed_before_the_auto_link_wins() {
    let (threshold, release) = threshold_candidate_and_release();
    let vault = FakeVault::with_library(vec![release]);
    park_in_new_run(&vault, &threshold, stricter_matching_policy());
    *vault.human_decision_before_next_write.borrow_mut() = Some(ReviewDecision::Reject);

    let later_run = vault.start_run();
    let imported = execute(&vault, &FakeConnector::new(vec![threshold]), later_run).unwrap();

    assert!(imported.is_empty());
    assert!(vault.records.borrow().is_empty());
    assert_eq!(vault.review_item(0).status, ReviewStatus::Rejected);
    assert_eq!(vault.run(later_run).status, AcquisitionRunStatus::Completed);
}

#[test]
fn rejecting_a_reopened_review_detaches_the_earlier_automatic_link() {
    let (threshold, release) = threshold_candidate_and_release();
    let identity = candidate_identity(SOURCE_ID, &threshold);
    let vault = FakeVault::with_library(vec![release]);
    let linking_run = vault.start_run();
    execute(
        &vault,
        &FakeConnector::new(vec![threshold.clone()]),
        linking_run,
    )
    .unwrap();
    assert_eq!(vault.candidate_links.borrow().get(&identity), Some(&501));

    park_in_new_run(&vault, &threshold, stricter_matching_policy());
    resolve_review_item(&vault, vault.review_item(0).id, ReviewDecision::Reject).unwrap();

    assert_eq!(vault.candidate_links.borrow().get(&identity), None);
}

#[test]
fn review_opened_by_another_run_during_an_auto_link_is_closed_with_it() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    *vault.review_opened_before_next_write.borrow_mut() = Some(NewReviewItem {
        candidate_identity: candidate_identity(SOURCE_ID, &smb),
        candidate: smb.clone(),
        competing_matches: Vec::new(),
    });
    let run_id = vault.start_run();

    let imported = execute(&vault, &FakeConnector::new(vec![smb]), run_id).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(vault.review_item(0).status, ReviewStatus::AutoResolved);
}

#[test]
fn review_opened_by_another_run_before_a_supersession_is_superseded_with_it() {
    let (threshold, release) = threshold_candidate_and_release();
    let vault = FakeVault::with_library(vec![release]);
    *vault.review_opened_before_next_write.borrow_mut() = Some(NewReviewItem {
        candidate_identity: candidate_identity(SOURCE_ID, &threshold),
        candidate: threshold.clone(),
        competing_matches: Vec::new(),
    });
    let run_id = vault.start_run();

    execute_with(
        &vault,
        &FakeConnector::new(vec![threshold]),
        run_id,
        dismissive_matching_policy(),
    )
    .unwrap();

    assert_eq!(vault.review_item(0).status, ReviewStatus::Superseded);
}

#[test]
fn human_acceptance_committed_before_parking_wins() {
    let (ambiguous, library) = ambiguous_candidate_and_releases();
    let vault = FakeVault::with_library(library);
    park_in_new_run(&vault, &ambiguous, matching_policy());
    *vault.human_decision_before_next_write.borrow_mut() = Some(ReviewDecision::Accept {
        release_edition_id: 401,
    });

    let later_run = vault.start_run();
    let imported = execute(&vault, &FakeConnector::new(vec![ambiguous]), later_run).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].release_edition_id, 401);
    assert_eq!(vault.run(later_run).awaiting_review_work, 0);
}

#[test]
fn a_family_is_refused_while_the_connector_supports_only_some_of_its_types() {
    // Packaging also selects box backs, spines, inserts…, which this connector cannot acquire.
    let error = plan_error(request_with(|draft| {
        draft.asset_types = vec![AssetTypeSelector::Packaging];
    }));

    assert!(matches!(
        error,
        ApplicationError::UnsupportedConnectorPlan { .. }
    ));
}

#[test]
fn pause_or_cancel_during_the_last_download_preserves_the_requested_run_status() {
    for status in [
        AcquisitionRunStatus::Paused,
        AcquisitionRunStatus::Cancelled,
    ] {
        let smb = candidate("Super Mario Bros.");
        let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
        *vault.status_after_next_completion.borrow_mut() = Some(status);
        let run_id = vault.start_run();

        let imported = execute(&vault, &FakeConnector::new(vec![smb]), run_id).unwrap();

        assert_eq!(imported.len(), 1);
        assert_eq!(vault.run(run_id).status, status);
    }
}

#[test]
fn executing_a_completed_run_does_nothing() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let connector = FakeConnector::new(vec![smb]);
    let run_id = vault.start_run();
    execute(&vault, &connector, run_id).unwrap();

    let imported = execute(&vault, &connector, run_id).unwrap();

    assert!(imported.is_empty());
    assert_eq!(connector.downloads.borrow().len(), 1);
}

#[test]
fn paused_and_cancelled_runs_are_not_executable() {
    for status in [
        AcquisitionRunStatus::Paused,
        AcquisitionRunStatus::Cancelled,
    ] {
        let vault = FakeVault::default();
        let run_id = vault.start_run();
        vault
            .compare_and_set_run_status(run_id, AcquisitionRunStatus::Running, status)
            .unwrap();

        let error = execute(&vault, &FakeConnector::new(Vec::new()), run_id).unwrap_err();

        assert_eq!(error, ApplicationError::RunNotExecutable { status });
    }
}

fn request_with(change: impl FnOnce(&mut AcquisitionRequestDraft)) -> AcquisitionRequest {
    let mut draft = request_draft();
    change(&mut draft);
    AcquisitionRequest::try_from_draft(draft).unwrap()
}

fn plan_error(request: AcquisitionRequest) -> ApplicationError {
    let vault = FakeVault::default();
    let run_id = vault.create_run(request).unwrap().id;
    let connector = FakeConnector::new(Vec::new());
    let error = execute(&vault, &connector, run_id).unwrap_err();
    assert_eq!(*connector.discover_calls.borrow(), 0);
    error
}

#[test]
fn rejects_multi_source_execution_until_a_multi_source_plan_exists() {
    let error = plan_error(request_with(|draft| {
        draft.sources = SourceSelection::Explicit(vec![SOURCE_ID.to_owned(), "other".to_owned()]);
    }));
    assert!(matches!(
        error,
        ApplicationError::UnsupportedConnectorPlan { .. }
    ));

    let error = plan_error(request_with(|draft| draft.sources = SourceSelection::Auto));
    assert!(matches!(
        error,
        ApplicationError::UnsupportedConnectorPlan { .. }
    ));
}

#[test]
fn rejects_quality_requirements_the_engine_cannot_measure_yet() {
    for quality in [
        QualityRequirements {
            min_bitrate_kbps: Some(800),
            ..QualityRequirements::default()
        },
        QualityRequirements {
            preferred_source_priority: vec![SOURCE_ID.to_owned()],
            ..QualityRequirements::default()
        },
        QualityRequirements {
            best_available: true,
            ..QualityRequirements::default()
        },
    ] {
        let error = plan_error(request_with(|draft| draft.quality = Some(quality)));
        assert!(matches!(
            error,
            ApplicationError::UnsupportedConnectorPlan { .. }
        ));
    }
}

fn keep_best_run(vault: &FakeVault) -> i64 {
    vault
        .create_run(request_with(|draft| {
            draft.retention = RetentionPolicy::KeepBestPerType;
        }))
        .unwrap()
        .id
}

fn with_retained_box_front(release: LibraryEntry, asset_id: i64, media: MediaInfo) -> LibraryEntry {
    LibraryEntry {
        assets: vec![LibraryAsset {
            asset_id,
            asset_type: AssetType::BoxFront,
            object_hash: format!("retained-{asset_id}"),
            byte_len: 90_000,
            media,
            original_filename: "front.png".to_owned(),
            provenance: Vec::new(),
            derived: Vec::new(),
        }],
        ..release
    }
}

#[test]
fn keep_best_per_type_links_an_original_that_becomes_the_preferred_asset() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![with_retained_box_front(
        release_for(&smb, 73),
        40,
        png(640, 900),
    )]);
    let run_id = keep_best_run(&vault);

    let imported = execute_storing(
        &vault,
        &FakeConnector::new(vec![smb]),
        run_id,
        &FakeStore::storing(png(1200, 1600)),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(vault.run(run_id).outranked_work, 0);
}

#[test]
fn keep_best_per_type_retains_no_original_a_retained_one_outranks() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![with_retained_box_front(
        release_for(&smb, 73),
        40,
        png(1200, 1600),
    )]);
    let run_id = keep_best_run(&vault);

    let imported = execute_storing(
        &vault,
        &FakeConnector::new(vec![smb]),
        run_id,
        &FakeStore::storing(png(640, 900)),
    )
    .unwrap();

    assert!(imported.is_empty());
    assert!(vault.records.borrow().is_empty());
    let run = vault.run(run_id);
    assert_eq!((run.completed_work, run.outranked_work), (1, 1));
    assert_eq!(
        vault.outranked(run_id),
        vec![Outranked {
            preferred_asset_id: 40,
            reason: PreferenceReason::MorePixels {
                preferred: 1_920_000,
                other: Some(576_000),
            },
        }]
    );
}

#[test]
fn keep_everything_links_an_original_a_retained_one_outranks() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![with_retained_box_front(
        release_for(&smb, 73),
        40,
        png(1200, 1600),
    )]);
    let run_id = vault.start_run();

    let imported = execute_storing(
        &vault,
        &FakeConnector::new(vec![smb]),
        run_id,
        &FakeStore::storing(png(640, 900)),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
}

#[test]
fn an_outranked_original_still_settles_the_review_of_a_now_certain_match() {
    let (threshold, release) = threshold_candidate_and_release();
    let vault =
        FakeVault::with_library(vec![with_retained_box_front(release, 40, png(1200, 1600))]);
    let first_run = park_in_new_run(&vault, &threshold, stricter_matching_policy());
    let keep_best = keep_best_run(&vault);

    execute_storing(
        &vault,
        &FakeConnector::new(vec![threshold]),
        keep_best,
        &FakeStore::storing(png(640, 900)),
    )
    .unwrap();

    assert_eq!(vault.run(keep_best).outranked_work, 1);
    assert_eq!(vault.review_item(0).status, ReviewStatus::AutoResolved);
    // Nothing was linked, so the parked run applies its own retention to the candidate.
    assert_eq!(vault.work_states(first_run), vec![WorkState::Queued]);
}

fn quality_run(vault: &FakeVault, quality: QualityRequirements) -> i64 {
    vault
        .create_run(request_with(|draft| draft.quality = Some(quality)))
        .unwrap()
        .id
}

fn png(width: u32, height: u32) -> MediaInfo {
    MediaInfo {
        media_type: "image/png".to_owned(),
        width: Some(width),
        height: Some(height),
    }
}

#[test]
fn an_original_below_the_quality_requirements_is_not_linked() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let run_id = quality_run(
        &vault,
        QualityRequirements {
            min_width: Some(1000),
            min_longest_edge: Some(1200),
            ..QualityRequirements::default()
        },
    );
    let store = FakeStore::storing(png(640, 900));

    let imported = execute_storing(&vault, &FakeConnector::new(vec![smb]), run_id, &store).unwrap();

    assert!(imported.is_empty());
    assert!(vault.records.borrow().is_empty());
    let run = vault.run(run_id);
    assert_eq!((run.completed_work, run.below_quality_work), (1, 1));
    assert_eq!(
        vault.quality_shortfalls(run_id),
        vec![
            QualityShortfall::MinWidth {
                minimum: 1000,
                actual: Some(640),
            },
            QualityShortfall::MinLongestEdge {
                minimum: 1200,
                actual: Some(900),
            },
        ]
    );
}

#[test]
fn an_original_meeting_the_quality_requirements_is_linked() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let run_id = quality_run(
        &vault,
        QualityRequirements {
            min_width: Some(1000),
            min_pixel_count: Some(1_000_000),
            accepted_mime_types: vec!["image/png".to_owned()],
            original_only: true,
            ..QualityRequirements::default()
        },
    );
    let store = FakeStore::storing(png(1200, 1600));

    let imported = execute_storing(&vault, &FakeConnector::new(vec![smb]), run_id, &store).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(vault.run(run_id).below_quality_work, 0);
}

#[test]
fn an_original_of_an_unaccepted_media_type_is_not_linked() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let run_id = quality_run(
        &vault,
        QualityRequirements {
            accepted_mime_types: vec!["image/png".to_owned()],
            ..QualityRequirements::default()
        },
    );
    let store = FakeStore::storing(MediaInfo {
        media_type: "image/jpeg".to_owned(),
        ..png(1200, 1600)
    });

    let imported = execute_storing(&vault, &FakeConnector::new(vec![smb]), run_id, &store).unwrap();

    assert!(imported.is_empty());
    assert_eq!(
        vault.quality_shortfalls(run_id),
        vec![QualityShortfall::AcceptedMediaTypes {
            accepted: vec!["image/png".to_owned()],
            actual: "image/jpeg".to_owned(),
        }]
    );
}

#[test]
fn a_below_quality_original_still_settles_the_review_of_a_now_certain_match() {
    let (threshold, release) = threshold_candidate_and_release();
    let vault = FakeVault::with_library(vec![release]);
    let first_run = park_in_new_run(&vault, &threshold, stricter_matching_policy());
    let strict_run = quality_run(
        &vault,
        QualityRequirements {
            min_width: Some(1000),
            ..QualityRequirements::default()
        },
    );

    let imported = execute_storing(
        &vault,
        &FakeConnector::new(vec![threshold]),
        strict_run,
        &FakeStore::storing(png(640, 900)),
    )
    .unwrap();

    assert!(imported.is_empty());
    assert_eq!(vault.run(strict_run).below_quality_work, 1);
    assert_eq!(vault.review_item(0).status, ReviewStatus::AutoResolved);
    // Nothing was linked, so the parked run applies its own requirements to the candidate.
    assert_eq!(vault.work_states(first_run), vec![WorkState::Queued]);
    assert_eq!(vault.run(first_run).status, AcquisitionRunStatus::Running);
}

#[test]
fn a_human_decision_landing_before_a_below_quality_completion_wins() {
    let (threshold, release) = threshold_candidate_and_release();
    let vault = FakeVault::with_library(vec![release]);
    park_in_new_run(&vault, &threshold, stricter_matching_policy());
    let strict_run = quality_run(
        &vault,
        QualityRequirements {
            min_width: Some(1000),
            ..QualityRequirements::default()
        },
    );
    *vault.human_decision_before_next_write.borrow_mut() = Some(ReviewDecision::Reject);

    execute_storing(
        &vault,
        &FakeConnector::new(vec![threshold]),
        strict_run,
        &FakeStore::storing(png(640, 900)),
    )
    .unwrap();

    assert_eq!(vault.review_item(0).status, ReviewStatus::Rejected);
    assert_eq!(vault.work_states(strict_run), vec![WorkState::Done]);
    assert!(vault.quality_shortfalls(strict_run).is_empty());
}

#[test]
fn rejects_acquisition_limits_until_the_scheduler_slice_can_enforce_them() {
    let error = plan_error(request_with(|draft| {
        draft.limits = AcquisitionLimits {
            max_downloads: Some(1),
            ..AcquisitionLimits::default()
        };
    }));
    assert!(matches!(
        error,
        ApplicationError::UnsupportedConnectorPlan { .. }
    ));
}

#[test]
fn rejects_asset_types_not_declared_by_the_connector() {
    let error = plan_error(request_with(|draft| {
        draft.asset_types = vec![AssetTypeSelector::BoxFront, AssetTypeSelector::Manual];
    }));
    assert!(matches!(
        error,
        ApplicationError::UnsupportedConnectorPlan { .. }
    ));
}

/// Acquires two candidates, each matching one release (a single release when they share
/// descriptive metadata, so matching stays unambiguous).
fn acquire_both(first: AssetCandidate, second: AssetCandidate) -> (FakeVault, FakeConnector) {
    let mut library = vec![release_for(&first, 81)];
    if second.game_title != first.game_title {
        library.push(release_for(&second, 82));
    }
    let vault = FakeVault::with_library(library);
    let connector = FakeConnector::new(vec![first, second]);
    let run_id = vault.start_run();
    execute(&vault, &connector, run_id).unwrap();
    (vault, connector)
}

#[test]
fn distinct_candidates_that_share_a_source_url_keep_distinct_work_items() {
    let first = candidate("A:B");
    let second = AssetCandidate {
        game_title: "A?B".to_owned(),
        ..first.clone()
    };

    let (vault, connector) = acquire_both(first, second);

    assert_eq!(connector.downloads.borrow().len(), 2);
    assert_eq!(vault.records.borrow().len(), 2);
}

#[test]
fn fallback_candidates_with_distinct_labels_or_filenames_keep_distinct_work_items() {
    let labelled = candidate("Shared Game");
    let other_label = AssetCandidate {
        source_asset_label: Some("Alternate_Boxarts".to_owned()),
        ..labelled.clone()
    };
    let (vault, _) = acquire_both(labelled.clone(), other_label);
    assert_eq!(vault.records.borrow().len(), 2);

    let unlabelled = AssetCandidate {
        source_asset_label: None,
        ..labelled
    };
    let other_filename = AssetCandidate {
        original_filename: "shared-alternate.png".to_owned(),
        ..unlabelled.clone()
    };
    let (vault, _) = acquire_both(unlabelled, other_filename);
    assert_eq!(vault.records.borrow().len(), 2);
}

#[test]
fn distinct_provider_candidates_with_identical_metadata_keep_distinct_work_items() {
    let first = AssetCandidate {
        provider_candidate_id: Some("provider-release-1".to_owned()),
        ..candidate("Shared Game")
    };
    let second = AssetCandidate {
        provider_candidate_id: Some("provider-release-2".to_owned()),
        ..first.clone()
    };

    let (vault, _) = acquire_both(first, second);

    assert_eq!(vault.records.borrow().len(), 2);
}

#[test]
fn blank_provider_candidate_ids_fall_back_to_the_descriptive_identity() {
    let first = AssetCandidate {
        provider_candidate_id: Some(String::new()),
        ..candidate("First Game")
    };
    let second = AssetCandidate {
        provider_candidate_id: Some(String::new()),
        ..candidate("Second Game")
    };

    let (vault, _) = acquire_both(first, second);

    assert_eq!(vault.records.borrow().len(), 2);
}

#[test]
fn fallback_identity_ignores_case_and_surrounding_whitespace() {
    let original = candidate("Shared Game");
    let rediscovered = AssetCandidate {
        game_title: " shared GAME ".to_owned(),
        platform: " NINTENDO - NINTENDO ENTERTAINMENT SYSTEM ".to_owned(),
        region: " usa ".to_owned(),
        edition_name: " STANDARD ".to_owned(),
        source_asset_label: original
            .source_asset_label
            .as_deref()
            .map(|label| format!(" {} ", label.to_uppercase())),
        ..original.clone()
    };
    let unlabelled = AssetCandidate {
        source_asset_label: None,
        ..original.clone()
    };
    let unlabelled_rediscovered = AssetCandidate {
        original_filename: format!(" {} ", unlabelled.original_filename.to_uppercase()),
        ..unlabelled.clone()
    };

    assert_eq!(
        candidate_identity(SOURCE_ID, &original),
        candidate_identity(SOURCE_ID, &rediscovered)
    );
    assert_eq!(
        candidate_identity(SOURCE_ID, &unlabelled),
        candidate_identity(SOURCE_ID, &unlabelled_rediscovered)
    );
}

#[test]
fn candidate_identity_fields_cannot_collide_through_delimiters() {
    let first = AssetCandidate {
        game_title: "C".to_owned(),
        platform: "A:B".to_owned(),
        ..candidate("unused")
    };
    let second = AssetCandidate {
        game_title: "B:C".to_owned(),
        platform: "A".to_owned(),
        ..first.clone()
    };

    assert_ne!(
        candidate_identity(SOURCE_ID, &first),
        candidate_identity(SOURCE_ID, &second)
    );
}

#[test]
fn reviews_with_a_colliding_source_url_keep_independent_decisions() {
    let (first, mut library) = ambiguous_candidate_and_releases();
    let first = AssetCandidate {
        provider_candidate_id: Some("provider:A:B".to_owned()),
        ..first
    };
    let second = AssetCandidate {
        provider_candidate_id: Some("provider:A?B".to_owned()),
        game_title: "Other Review Game".to_owned(),
        ..first.clone()
    };
    library.extend([
        LibraryEntry {
            edition_name: "Standard".to_owned(),
            ..release_for(&second, 403)
        },
        LibraryEntry {
            edition_name: "Deluxe".to_owned(),
            ..release_for(&second, 404)
        },
    ]);
    let vault = FakeVault::with_library(library);
    let run_id = vault.start_run();
    execute(&vault, &FakeConnector::new(vec![first, second]), run_id).unwrap();
    assert_eq!(vault.list_review_items().unwrap().len(), 2);

    resolve_review_item(&vault, vault.review_item(0).id, ReviewDecision::Reject).unwrap();

    assert_eq!(vault.review_item(1).status, ReviewStatus::Pending);
    assert_eq!(vault.run(run_id).awaiting_review_work, 1);
}

#[test]
fn an_acceptance_reopening_a_run_read_as_completed_is_executed() {
    let (ambiguous, library) = ambiguous_candidate_and_releases();
    let vault = FakeVault::with_library(library);
    let connector = FakeConnector::new(vec![ambiguous]);
    let run_id = vault.start_run();
    execute(&vault, &connector, run_id).unwrap();
    assert_eq!(vault.run(run_id).status, AcquisitionRunStatus::Completed);
    *vault.decision_after_next_run_read.borrow_mut() = Some(ReviewDecision::Accept {
        release_edition_id: 402,
    });

    let imported = execute(&vault, &connector, run_id).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(vault.run(run_id).status, AcquisitionRunStatus::Completed);
}

#[test]
fn a_run_is_started_for_a_connector_only_when_it_can_execute_it() {
    let vault = FakeVault::default();
    let connector = FakeConnector::new(Vec::new());

    let error = start_acquisition_run_for_connector(
        &vault,
        AcquisitionRequestDraft {
            sources: SourceSelection::Auto,
            ..request_draft()
        },
        &connector,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ApplicationError::UnsupportedConnectorPlan { .. }
    ));
    assert!(vault.runs.borrow().is_empty());
    let started = start_acquisition_run_for_connector(&vault, request_draft(), &connector).unwrap();
    assert_eq!(vault.run(started.id).status, AcquisitionRunStatus::Running);
}

#[test]
fn connector_specific_limits_are_checked_before_a_run_starts() {
    let vault = FakeVault::default();
    let connector = FakeConnector {
        unsupported_reason: Some("this source needs an explicit game selection".to_owned()),
        ..FakeConnector::new(Vec::new())
    };

    let error =
        start_acquisition_run_for_connector(&vault, request_draft(), &connector).unwrap_err();

    assert_eq!(
        error,
        ApplicationError::UnsupportedConnectorPlan {
            source_id: SOURCE_ID.to_owned(),
            reason: "this source needs an explicit game selection".to_owned(),
        }
    );
    assert!(vault.runs.borrow().is_empty());
}

#[test]
fn a_discovered_run_resumes_without_consulting_the_source_for_its_plan() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let run_id = vault.start_run();
    let failing = FakeConnector {
        failing_downloads: [smb.source_url.clone()].into(),
        ..FakeConnector::new(vec![smb.clone()])
    };
    execute(&vault, &failing, run_id).unwrap_err();
    assert_eq!(vault.run(run_id).queued_work, 1);

    // The Source cannot be reached for a plan check, but the persisted queue still executes.
    let unreachable = FakeConnector {
        plan_check_fails: true,
        ..FakeConnector::new(vec![smb])
    };
    let imported = execute(&vault, &unreachable, run_id).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(*unreachable.discover_calls.borrow(), 0);
}

#[test]
fn a_plan_the_connector_cannot_check_starts_no_run() {
    let vault = FakeVault::default();
    let connector = FakeConnector {
        plan_check_fails: true,
        ..FakeConnector::new(Vec::new())
    };

    let error =
        start_acquisition_run_for_connector(&vault, request_draft(), &connector).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::External);
    assert!(vault.runs.borrow().is_empty());
}

#[test]
fn a_cancellation_during_discovery_stops_the_execution_quietly() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    *vault.status_before_next_discovery.borrow_mut() = Some(AcquisitionRunStatus::Cancelled);
    let run_id = vault.start_run();

    let imported = execute(&vault, &FakeConnector::new(vec![smb]), run_id).unwrap();

    assert!(imported.is_empty());
    let run = vault.run(run_id);
    assert_eq!(run.status, AcquisitionRunStatus::Cancelled);
    assert_eq!(run.queued_work, 0);
}

#[test]
fn a_run_acquires_only_the_asset_types_it_selects() {
    let smb = candidate("Super Mario Bros.");
    let screenshot = AssetCandidate {
        asset_type: AssetType::Screenshot,
        source_asset_label: Some("Named_Snaps".to_owned()),
        source_url: "https://example.invalid/Named_Snaps/Super Mario Bros..png".to_owned(),
        ..smb.clone()
    };
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let run_id = vault
        .create_run(request_with(|draft| {
            draft.asset_types = vec![AssetTypeSelector::Screenshot];
        }))
        .unwrap()
        .id;
    let connector = FakeConnector {
        asset_types: vec![AssetType::BoxFront, AssetType::Screenshot],
        ..FakeConnector::new(vec![smb, screenshot.clone()])
    };

    let imported = execute(&vault, &connector, run_id).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(vault.records.borrow()[0].asset_type, AssetType::Screenshot);
    // The sibling Box Front is never downloaded.
    assert_eq!(*connector.downloads.borrow(), vec![screenshot.source_url]);
}
