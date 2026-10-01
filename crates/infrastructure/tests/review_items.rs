use game_media_vault_application::{
    AcquisitionRequestInput, CatalogPort, ParkedReview, ReviewDecisionOutcome,
    ReviewRepositoryPort, RunRepositoryPort, cancel_acquisition_run, load_acquisition_run,
    start_acquisition_run,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate, AssetType,
    AssetTypeSelector, GameSelection, MatchEvidence, MatchSignal, NewReviewItem, PersistAsset,
    ReleaseAssertion, ReleaseAssertionField, RetentionPolicy, ReviewDecision, ReviewItem,
    ReviewMatchCandidate, ReviewStatus, SourceId, SourceSelection,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::{TempDir, tempdir};

const SOURCE_ID: &str = "fixture-provider";
const IDENTITY: &str = "candidate:fixture-provider:review-game";

fn open_catalog() -> (TempDir, SqliteCatalog) {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    (temp, catalog)
}

fn start_run(catalog: &SqliteCatalog) -> i64 {
    start_acquisition_run(
        catalog,
        AcquisitionRequestInput {
            sources: SourceSelection::Explicit(vec![SOURCE_ID.to_owned()]),
            platforms: vec!["Nintendo Entertainment System".to_owned()],
            games: GameSelection::All,
            regions: Vec::new(),
            languages: Vec::new(),
            asset_types: vec![AssetTypeSelector::BoxFront],
            quality: None,
            retention: RetentionPolicy::KeepEverything,
            limits: AcquisitionLimits::default(),
        },
    )
    .unwrap()
    .id
}

fn candidate() -> AssetCandidate {
    AssetCandidate {
        provider_candidate_id: Some("review-game".to_owned()),
        game_title: "Review Game".to_owned(),
        platform: "Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Collector".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from(SOURCE_ID),
        source_asset_label: Some("front".to_owned()),
        source_url: "https://example.invalid/review-game/front.png".to_owned(),
        original_filename: "front.png".to_owned(),
    }
}

fn competing_match(release_edition_id: i64, edition_name: &str) -> ReviewMatchCandidate {
    ReviewMatchCandidate {
        game_id: 301,
        release_edition_id,
        game_title: "Review Game".to_owned(),
        platform: "Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: edition_name.to_owned(),
        score: 90,
        evidence: vec![MatchEvidence {
            signal: MatchSignal::Edition,
            candidate_value: "Collector".to_owned(),
            release_value: edition_name.to_owned(),
            score_delta: -5,
        }],
        assertions: vec![ReleaseAssertion {
            source_id: SourceId::from("reference-catalog"),
            source_location: "fixture://reference".to_owned(),
            field: ReleaseAssertionField::Identifier,
            qualifier: Some("source_record".to_owned()),
            value: format!("review-game-{release_edition_id}"),
        }],
    }
}

fn new_item(competing_matches: Vec<ReviewMatchCandidate>) -> NewReviewItem {
    NewReviewItem {
        candidate_identity: IDENTITY.to_owned(),
        candidate: candidate(),
        competing_matches,
    }
}

/// Discovers the review candidate as queued work of `run_id`.
fn discover(catalog: &SqliteCatalog, run_id: i64) {
    catalog
        .record_discovery(
            run_id,
            SOURCE_ID,
            &[AcquisitionWorkItem {
                key: IDENTITY.to_owned(),
                candidate: candidate(),
            }],
        )
        .unwrap();
}

/// Discovers the review candidate in `run_id` and parks its work on the Review Item.
fn park(catalog: &SqliteCatalog, run_id: i64) -> ParkedReview {
    discover(catalog, run_id);
    catalog
        .park_work_for_review(
            run_id,
            IDENTITY,
            new_item(vec![
                competing_match(201, "Standard"),
                competing_match(202, "Deluxe"),
            ]),
        )
        .unwrap()
}

fn parked_item(catalog: &SqliteCatalog, run_id: i64) -> ReviewItem {
    match park(catalog, run_id) {
        ParkedReview::Parked(item) => item,
        other => panic!("expected parked work, got {other:?}"),
    }
}

fn complete_run(catalog: &SqliteCatalog, run_id: i64) {
    assert!(
        catalog
            .compare_and_set_run_status(
                run_id,
                AcquisitionRunStatus::Running,
                AcquisitionRunStatus::Completed,
            )
            .unwrap()
    );
}

fn counts(catalog: &SqliteCatalog, run_id: i64) -> (u64, u64, u64) {
    let run = load_acquisition_run(catalog, run_id).unwrap();
    (
        run.queued_work,
        run.awaiting_review_work,
        run.completed_work,
    )
}

#[test]
fn parked_review_item_round_trips_candidate_competitors_and_evidence() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite3");
    let catalog = SqliteCatalog::open(&path).unwrap();
    let run_id = start_run(&catalog);

    let parked = parked_item(&catalog, run_id);
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&path).unwrap();
    let loaded = reopened.find_review_item(IDENTITY).unwrap().unwrap();
    assert_eq!(loaded, parked);
    assert_eq!(loaded.status, ReviewStatus::Pending);
    assert_eq!(loaded.decision, None);
    assert_eq!(loaded.candidate, candidate());
    assert_eq!(
        loaded.competing_matches,
        vec![
            competing_match(201, "Standard"),
            competing_match(202, "Deluxe")
        ]
    );
    assert_eq!(
        reopened.get_review_item(loaded.id).unwrap(),
        Some(loaded.clone())
    );
    assert_eq!(reopened.list_review_items().unwrap(), vec![loaded]);
    assert_eq!(counts(&reopened, run_id), (0, 1, 0));
}

#[test]
fn runs_meeting_the_same_candidate_share_one_review_item() {
    let (_temp, catalog) = open_catalog();
    let first_run = start_run(&catalog);
    let second_run = start_run(&catalog);

    let first = parked_item(&catalog, first_run);
    let second = parked_item(&catalog, second_run);

    assert_eq!(first.id, second.id);
    assert_eq!(catalog.list_review_items().unwrap().len(), 1);
    assert_eq!(counts(&catalog, first_run), (0, 1, 0));
    assert_eq!(counts(&catalog, second_run), (0, 1, 0));
}

#[test]
fn parking_refreshes_the_evidence_of_an_undecided_item() {
    let (_temp, catalog) = open_catalog();
    let first_run = start_run(&catalog);
    parked_item(&catalog, first_run);
    let second_run = start_run(&catalog);
    catalog
        .record_discovery(
            second_run,
            SOURCE_ID,
            &[AcquisitionWorkItem {
                key: IDENTITY.to_owned(),
                candidate: candidate(),
            }],
        )
        .unwrap();

    catalog
        .park_work_for_review(
            second_run,
            IDENTITY,
            new_item(vec![competing_match(203, "Limited")]),
        )
        .unwrap();

    let item = catalog.find_review_item(IDENTITY).unwrap().unwrap();
    assert_eq!(
        item.competing_matches,
        vec![competing_match(203, "Limited")]
    );
}

#[test]
fn accepting_requeues_parked_work_and_reopens_completed_runs() {
    let (_temp, catalog) = open_catalog();
    let completed_run = start_run(&catalog);
    let item = parked_item(&catalog, completed_run);
    complete_run(&catalog, completed_run);
    let running_run = start_run(&catalog);
    parked_item(&catalog, running_run);

    let accepted = catalog
        .decide_review_item(
            item.id,
            ReviewDecision::Accept {
                release_edition_id: 202,
            },
        )
        .unwrap()
        .recorded()
        .unwrap();

    assert_eq!(accepted.status, ReviewStatus::Accepted);
    assert_eq!(
        accepted.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 202
        })
    );
    for run_id in [completed_run, running_run] {
        let run = load_acquisition_run(&catalog, run_id).unwrap();
        assert_eq!(run.status, AcquisitionRunStatus::Running);
        assert_eq!(counts(&catalog, run_id), (1, 0, 0));
        assert_eq!(
            catalog.next_queued_work(run_id).unwrap().unwrap().key,
            IDENTITY
        );
    }
}

#[test]
fn accepting_leaves_parked_work_of_cancelled_runs_untouched() {
    let (_temp, catalog) = open_catalog();
    let run_id = start_run(&catalog);
    let item = parked_item(&catalog, run_id);
    cancel_acquisition_run(&catalog, run_id).unwrap();

    catalog
        .decide_review_item(
            item.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap()
        .recorded()
        .unwrap();

    let run = load_acquisition_run(&catalog, run_id).unwrap();
    assert_eq!(run.status, AcquisitionRunStatus::Cancelled);
    assert_eq!(counts(&catalog, run_id), (0, 1, 0));
}

#[test]
fn rejecting_completes_parked_work_in_every_run() {
    let (_temp, catalog) = open_catalog();
    let completed_run = start_run(&catalog);
    let item = parked_item(&catalog, completed_run);
    complete_run(&catalog, completed_run);
    let running_run = start_run(&catalog);
    parked_item(&catalog, running_run);

    let rejected = catalog
        .decide_review_item(item.id, ReviewDecision::Reject)
        .unwrap()
        .recorded()
        .unwrap();

    assert_eq!(rejected.status, ReviewStatus::Rejected);
    assert_eq!(counts(&catalog, completed_run), (0, 0, 1));
    assert_eq!(counts(&catalog, running_run), (0, 0, 1));
    assert_eq!(
        load_acquisition_run(&catalog, completed_run)
            .unwrap()
            .status,
        AcquisitionRunStatus::Completed
    );
}

#[test]
fn deferring_keeps_work_parked_until_a_final_decision() {
    let (_temp, catalog) = open_catalog();
    let run_id = start_run(&catalog);
    let item = parked_item(&catalog, run_id);

    let deferred = catalog
        .decide_review_item(item.id, ReviewDecision::Defer)
        .unwrap()
        .recorded()
        .unwrap();
    assert_eq!(deferred.status, ReviewStatus::Deferred);
    assert_eq!(counts(&catalog, run_id), (0, 1, 0));

    let rejected = catalog
        .decide_review_item(item.id, ReviewDecision::Reject)
        .unwrap()
        .recorded()
        .unwrap();
    assert_eq!(rejected.status, ReviewStatus::Rejected);
    assert_eq!(counts(&catalog, run_id), (0, 0, 1));
}

#[test]
fn human_decisions_are_final() {
    let (_temp, catalog) = open_catalog();
    let run_id = start_run(&catalog);
    let item = parked_item(&catalog, run_id);
    catalog
        .decide_review_item(item.id, ReviewDecision::Reject)
        .unwrap()
        .recorded()
        .unwrap();

    assert_eq!(
        catalog
            .decide_review_item(
                item.id,
                ReviewDecision::Accept {
                    release_edition_id: 201
                }
            )
            .unwrap()
            .recorded(),
        None
    );
    assert!(
        !catalog
            .supersede_candidate_review(start_run(&catalog), IDENTITY)
            .unwrap()
    );
    assert_eq!(
        catalog.get_review_item(item.id).unwrap().unwrap().status,
        ReviewStatus::Rejected
    );
}

#[test]
fn superseding_completes_parked_work_of_an_undecided_item() {
    let (_temp, catalog) = open_catalog();
    let first_run = start_run(&catalog);
    let item = parked_item(&catalog, first_run);
    let second_run = start_run(&catalog);
    parked_item(&catalog, second_run);

    assert!(
        catalog
            .supersede_candidate_review(start_run(&catalog), IDENTITY)
            .unwrap()
    );

    let closed = catalog.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(closed.status, ReviewStatus::Superseded);
    assert_eq!(closed.decision, None);
    assert_eq!(counts(&catalog, first_run), (0, 0, 1));
    assert_eq!(counts(&catalog, second_run), (0, 0, 1));
    assert_eq!(
        catalog
            .decide_review_item(item.id, ReviewDecision::Reject)
            .unwrap()
            .recorded(),
        None
    );
}

#[test]
fn superseding_a_candidate_without_review_item_records_nothing() {
    let (_temp, catalog) = open_catalog();

    assert!(
        catalog
            .supersede_candidate_review(start_run(&catalog), IDENTITY)
            .unwrap()
    );

    assert!(catalog.list_review_items().unwrap().is_empty());
}

#[test]
fn superseding_an_auto_linked_candidate_detaches_its_asset() {
    let (_temp, catalog) = open_catalog();
    catalog
        .persist_candidate_asset(start_run(&catalog), IDENTITY, auto_linked_record())
        .unwrap()
        .unwrap();

    assert!(
        catalog
            .supersede_candidate_review(start_run(&catalog), IDENTITY)
            .unwrap()
    );

    assert_eq!(library_asset_count(&catalog), 0);
    assert!(catalog.list_review_items().unwrap().is_empty());
}

#[test]
fn superseding_an_auto_resolved_item_marks_it_superseded() {
    let (_temp, catalog) = open_catalog();
    let item = parked_item(&catalog, start_run(&catalog));
    catalog
        .persist_candidate_asset(start_run(&catalog), IDENTITY, auto_linked_record())
        .unwrap()
        .unwrap();

    assert!(
        catalog
            .supersede_candidate_review(start_run(&catalog), IDENTITY)
            .unwrap()
    );

    assert_eq!(
        catalog.get_review_item(item.id).unwrap().unwrap().status,
        ReviewStatus::Superseded
    );
    assert_eq!(library_asset_count(&catalog), 0);
}

#[test]
fn superseding_settles_the_run_work_with_the_dismissal() {
    let (_temp, catalog) = open_catalog();
    let run_id = start_run(&catalog);
    discover(&catalog, run_id);

    assert!(
        catalog
            .supersede_candidate_review(run_id, IDENTITY)
            .unwrap()
    );

    assert_eq!(counts(&catalog, run_id), (0, 0, 1));
}

#[test]
fn auto_linking_settles_the_run_work_with_the_link() {
    let (_temp, catalog) = open_catalog();
    let run_id = start_run(&catalog);
    discover(&catalog, run_id);

    catalog
        .persist_candidate_asset(run_id, IDENTITY, auto_linked_record())
        .unwrap()
        .unwrap();

    assert_eq!(counts(&catalog, run_id), (0, 0, 1));
}

#[test]
fn parking_work_another_execution_settled_opens_no_review_item() {
    let (_temp, catalog) = open_catalog();
    let run_id = start_run(&catalog);
    discover(&catalog, run_id);
    catalog
        .persist_candidate_asset(run_id, IDENTITY, auto_linked_record())
        .unwrap()
        .unwrap();

    let parked = catalog
        .park_work_for_review(
            run_id,
            IDENTITY,
            new_item(vec![competing_match(201, "Standard")]),
        )
        .unwrap();

    assert_eq!(parked, ParkedReview::Settled);
    assert!(catalog.list_review_items().unwrap().is_empty());
    assert_eq!(counts(&catalog, run_id), (0, 0, 1));
}

#[test]
fn parking_reopens_an_automatically_closed_item() {
    let (_temp, catalog) = open_catalog();
    let first_run = start_run(&catalog);
    let item = parked_item(&catalog, first_run);
    catalog
        .supersede_candidate_review(start_run(&catalog), IDENTITY)
        .unwrap();

    let reopened = parked_item(&catalog, start_run(&catalog));

    assert_eq!(reopened.id, item.id);
    assert_eq!(reopened.status, ReviewStatus::Pending);
}

#[test]
fn parking_keeps_a_deferral() {
    let (_temp, catalog) = open_catalog();
    let item = parked_item(&catalog, start_run(&catalog));
    catalog
        .decide_review_item(item.id, ReviewDecision::Defer)
        .unwrap();

    let parked = parked_item(&catalog, start_run(&catalog));

    assert_eq!(parked.status, ReviewStatus::Deferred);
    assert_eq!(parked.decision, Some(ReviewDecision::Defer));
}

#[test]
fn parking_reports_a_human_decision_and_leaves_the_work_queued() {
    let (_temp, catalog) = open_catalog();
    let item = parked_item(&catalog, start_run(&catalog));
    catalog
        .decide_review_item(
            item.id,
            ReviewDecision::Accept {
                release_edition_id: 201,
            },
        )
        .unwrap();
    let later_run = start_run(&catalog);

    let outcome = park(&catalog, later_run);

    let ParkedReview::AlreadyDecided(decided) = outcome else {
        panic!("expected the human decision, got {outcome:?}");
    };
    assert_eq!(decided.status, ReviewStatus::Accepted);
    assert_eq!(counts(&catalog, later_run), (1, 0, 0));
}

#[test]
fn parking_unknown_work_creates_no_review_item() {
    let (_temp, catalog) = open_catalog();
    let run_id = start_run(&catalog);

    let error = catalog
        .park_work_for_review(run_id, "missing", new_item(Vec::new()))
        .unwrap_err();

    assert!(error.to_string().contains("no queued work"), "{error}");
    assert!(catalog.list_review_items().unwrap().is_empty());
}

#[test]
fn parking_work_already_parked_on_the_same_item_is_idempotent() {
    let (_temp, catalog) = open_catalog();
    let run_id = start_run(&catalog);
    let item = parked_item(&catalog, run_id);

    // A concurrent execution of the same run parks the same work again.
    let again = catalog
        .park_work_for_review(
            run_id,
            IDENTITY,
            new_item(vec![competing_match(201, "Standard")]),
        )
        .unwrap();

    let ParkedReview::Parked(parked) = again else {
        panic!("expected the work to stay parked, got {again:?}");
    };
    assert_eq!(parked.id, item.id);
    assert_eq!(counts(&catalog, run_id), (0, 1, 0));
}

fn auto_linked_record() -> PersistAsset {
    PersistAsset {
        existing_game_id: None,
        existing_release_edition_id: None,
        match_decision: None,
        game_title: "Review Game".to_owned(),
        platform: "Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Standard".to_owned(),
        asset_type: AssetType::BoxFront,
        object_hash: "auto-linked-object".to_owned(),
        byte_len: 12,
        original_filename: "front.png".to_owned(),
        source_id: SourceId::from(SOURCE_ID),
        source_asset_label: Some("front".to_owned()),
        source_location: candidate().source_url,
    }
}

fn library_asset_count(catalog: &SqliteCatalog) -> usize {
    catalog
        .list_library()
        .unwrap()
        .iter()
        .map(|entry| entry.assets.len())
        .sum()
}

#[test]
fn auto_linking_persists_the_asset_and_closes_the_undecided_item_together() {
    let (_temp, catalog) = open_catalog();
    let parked_run = start_run(&catalog);
    let item = parked_item(&catalog, parked_run);

    let imported = catalog
        .persist_candidate_asset(start_run(&catalog), IDENTITY, auto_linked_record())
        .unwrap();

    assert!(imported.is_some());
    assert_eq!(library_asset_count(&catalog), 1);
    let closed = catalog.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(closed.status, ReviewStatus::AutoResolved);
    assert_eq!(counts(&catalog, parked_run), (0, 0, 1));
}

#[test]
fn auto_linking_a_superseded_candidate_marks_its_item_auto_resolved() {
    let (_temp, catalog) = open_catalog();
    let item = parked_item(&catalog, start_run(&catalog));
    catalog
        .supersede_candidate_review(start_run(&catalog), IDENTITY)
        .unwrap();

    catalog
        .persist_candidate_asset(start_run(&catalog), IDENTITY, auto_linked_record())
        .unwrap()
        .unwrap();

    assert_eq!(
        catalog.get_review_item(item.id).unwrap().unwrap().status,
        ReviewStatus::AutoResolved
    );
}

#[test]
fn auto_linking_persists_nothing_once_a_human_decided() {
    let (_temp, catalog) = open_catalog();
    let item = parked_item(&catalog, start_run(&catalog));
    catalog
        .decide_review_item(item.id, ReviewDecision::Reject)
        .unwrap();

    let imported = catalog
        .persist_candidate_asset(start_run(&catalog), IDENTITY, auto_linked_record())
        .unwrap();

    assert_eq!(imported, None);
    assert_eq!(library_asset_count(&catalog), 0);
    assert_eq!(
        catalog.get_review_item(item.id).unwrap().unwrap().status,
        ReviewStatus::Rejected
    );
}

#[test]
fn auto_linking_a_candidate_without_review_item_only_persists_the_asset() {
    let (_temp, catalog) = open_catalog();

    let imported = catalog
        .persist_candidate_asset(start_run(&catalog), IDENTITY, auto_linked_record())
        .unwrap();

    assert!(imported.is_some());
    assert_eq!(library_asset_count(&catalog), 1);
    assert!(catalog.list_review_items().unwrap().is_empty());
}

fn record_for_edition(edition_name: &str, object_hash: &str) -> PersistAsset {
    PersistAsset {
        edition_name: edition_name.to_owned(),
        object_hash: object_hash.to_owned(),
        ..auto_linked_record()
    }
}

fn assets_by_edition(catalog: &SqliteCatalog) -> Vec<(String, usize)> {
    catalog
        .list_library()
        .unwrap()
        .into_iter()
        .map(|entry| (entry.edition_name, entry.assets.len()))
        .collect()
}

#[test]
fn a_later_link_moves_the_candidate_to_its_new_edition() {
    let (_temp, catalog) = open_catalog();
    catalog
        .persist_candidate_asset(
            start_run(&catalog),
            IDENTITY,
            record_for_edition("Standard", "first-bytes"),
        )
        .unwrap();

    catalog
        .persist_candidate_asset(
            start_run(&catalog),
            IDENTITY,
            record_for_edition("Deluxe", "second-bytes"),
        )
        .unwrap();

    assert_eq!(
        assets_by_edition(&catalog),
        vec![("Standard".to_owned(), 0), ("Deluxe".to_owned(), 1)]
    );
}

#[test]
fn rejecting_a_reopened_candidate_detaches_its_earlier_automatic_link() {
    let (_temp, catalog) = open_catalog();
    catalog
        .persist_candidate_asset(start_run(&catalog), IDENTITY, auto_linked_record())
        .unwrap();
    let item = parked_item(&catalog, start_run(&catalog));

    catalog
        .decide_review_item(item.id, ReviewDecision::Reject)
        .unwrap();

    assert_eq!(library_asset_count(&catalog), 0);
}

#[test]
fn detaching_a_candidate_keeps_identical_bytes_linked_by_another_candidate() {
    let (_temp, catalog) = open_catalog();
    let first = catalog
        .persist_candidate_asset(start_run(&catalog), IDENTITY, auto_linked_record())
        .unwrap()
        .unwrap();
    catalog
        .persist_candidate_asset(
            start_run(&catalog),
            "candidate:fixture-provider:mirror",
            PersistAsset {
                existing_release_edition_id: Some(first.release_edition_id),
                source_location: "https://example.invalid/mirror/front.png".to_owned(),
                ..auto_linked_record()
            },
        )
        .unwrap();
    let item = parked_item(&catalog, start_run(&catalog));

    catalog
        .decide_review_item(item.id, ReviewDecision::Reject)
        .unwrap();

    let library = catalog.list_library().unwrap();
    assert_eq!(library[0].assets.len(), 1);
    assert_eq!(
        library[0].assets[0].provenance[0].source_location,
        "https://example.invalid/mirror/front.png"
    );
}

#[test]
fn an_accepted_candidate_is_not_linked_to_another_edition() {
    let (_temp, catalog) = open_catalog();
    let standard = catalog
        .persist_candidate_asset(
            start_run(&catalog),
            IDENTITY,
            record_for_edition("Standard", "standard-bytes"),
        )
        .unwrap()
        .unwrap();
    let run_id = start_run(&catalog);
    catalog
        .record_discovery(
            run_id,
            SOURCE_ID,
            &[AcquisitionWorkItem {
                key: IDENTITY.to_owned(),
                candidate: candidate(),
            }],
        )
        .unwrap();
    let ParkedReview::Parked(item) = catalog
        .park_work_for_review(
            run_id,
            IDENTITY,
            new_item(vec![competing_match(
                standard.release_edition_id,
                "Standard",
            )]),
        )
        .unwrap()
    else {
        panic!("expected a pending review item");
    };
    catalog
        .decide_review_item(
            item.id,
            ReviewDecision::Accept {
                release_edition_id: standard.release_edition_id,
            },
        )
        .unwrap()
        .recorded()
        .unwrap();

    let elsewhere = catalog
        .persist_candidate_asset(
            start_run(&catalog),
            IDENTITY,
            record_for_edition("Deluxe", "deluxe-bytes"),
        )
        .unwrap();
    let again = catalog
        .persist_candidate_asset(
            start_run(&catalog),
            IDENTITY,
            PersistAsset {
                existing_release_edition_id: Some(standard.release_edition_id),
                ..record_for_edition("Standard", "standard-bytes")
            },
        )
        .unwrap();

    assert_eq!(elsewhere, None);
    assert!(again.is_some());
}

#[test]
fn accepting_an_edition_the_item_no_longer_offers_is_refused_atomically() {
    let (_temp, catalog) = open_catalog();
    let run_id = start_run(&catalog);
    let item = parked_item(&catalog, run_id);

    let outcome = catalog
        .decide_review_item(
            item.id,
            ReviewDecision::Accept {
                release_edition_id: 999,
            },
        )
        .unwrap();

    assert_eq!(outcome, ReviewDecisionOutcome::NotCompeting);
    assert_eq!(
        catalog.get_review_item(item.id).unwrap().unwrap().status,
        ReviewStatus::Pending
    );
    assert_eq!(counts(&catalog, run_id), (0, 1, 0));
}

const OTHER_IDENTITY: &str = "candidate:fixture-provider:same-locator";

/// Links IDENTITY, then another candidate serving the same locator and bytes for the same
/// edition with its own `label`, and returns the reopened Review Item of the second one.
fn link_two_candidates_sharing_a_locator(catalog: &SqliteCatalog, label: &str) -> ReviewItem {
    let first = catalog
        .persist_candidate_asset(start_run(catalog), IDENTITY, auto_linked_record())
        .unwrap()
        .unwrap();
    catalog
        .persist_candidate_asset(
            start_run(catalog),
            OTHER_IDENTITY,
            PersistAsset {
                existing_release_edition_id: Some(first.release_edition_id),
                source_asset_label: Some(label.to_owned()),
                ..auto_linked_record()
            },
        )
        .unwrap();
    let run_id = start_run(catalog);
    catalog
        .record_discovery(
            run_id,
            SOURCE_ID,
            &[AcquisitionWorkItem {
                key: OTHER_IDENTITY.to_owned(),
                candidate: candidate(),
            }],
        )
        .unwrap();
    let ParkedReview::Parked(other_item) = catalog
        .park_work_for_review(
            run_id,
            OTHER_IDENTITY,
            NewReviewItem {
                candidate_identity: OTHER_IDENTITY.to_owned(),
                ..new_item(vec![competing_match(201, "Standard")])
            },
        )
        .unwrap()
    else {
        panic!("expected a pending review item");
    };
    other_item
}

#[test]
fn candidates_sharing_one_locator_are_detached_independently() {
    let (_temp, catalog) = open_catalog();
    let other_item = link_two_candidates_sharing_a_locator(&catalog, "front");

    catalog
        .decide_review_item(other_item.id, ReviewDecision::Reject)
        .unwrap();

    assert_eq!(
        library_asset_count(&catalog),
        1,
        "the first candidate still supports it"
    );
    let item = parked_item(&catalog, start_run(&catalog));
    catalog
        .decide_review_item(item.id, ReviewDecision::Reject)
        .unwrap();
    assert_eq!(library_asset_count(&catalog), 0);
}

#[test]
fn a_detached_candidate_takes_its_provenance_details_along() {
    let (_temp, catalog) = open_catalog();
    let other_item = link_two_candidates_sharing_a_locator(&catalog, "rejected label");

    catalog
        .decide_review_item(other_item.id, ReviewDecision::Reject)
        .unwrap();

    let library = catalog.list_library().unwrap();
    let labels: Vec<_> = library[0].assets[0]
        .provenance
        .iter()
        .map(|provenance| provenance.source_asset_label.as_deref())
        .collect();
    assert_eq!(labels, vec![Some("front")]);
}
