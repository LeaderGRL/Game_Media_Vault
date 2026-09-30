use game_media_vault_application::{
    AcquisitionRequestInput, ParkedReview, ReviewRepositoryPort, RunRepositoryPort,
    cancel_acquisition_run, load_acquisition_run, start_acquisition_run,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate, AssetType,
    AssetTypeSelector, GameSelection, MatchEvidence, MatchSignal, NewReviewItem, ReleaseAssertion,
    ReleaseAssertionField, RetentionPolicy, ReviewDecision, ReviewItem, ReviewMatchCandidate,
    ReviewStatus, SourceId, SourceSelection,
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

/// Discovers the review candidate in `run_id` and parks its work on the Review Item.
fn park(catalog: &SqliteCatalog, run_id: i64) -> ParkedReview {
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
        .unwrap();
    assert_eq!(deferred.status, ReviewStatus::Deferred);
    assert_eq!(counts(&catalog, run_id), (0, 1, 0));

    let rejected = catalog
        .decide_review_item(item.id, ReviewDecision::Reject)
        .unwrap()
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
        .unwrap();

    assert_eq!(
        catalog
            .decide_review_item(
                item.id,
                ReviewDecision::Accept {
                    release_edition_id: 201
                }
            )
            .unwrap(),
        None
    );
    assert!(
        !catalog
            .close_review_item(item.id, ReviewStatus::AutoResolved)
            .unwrap()
    );
    assert_eq!(
        catalog.get_review_item(item.id).unwrap().unwrap().status,
        ReviewStatus::Rejected
    );
}

#[test]
fn closing_automatically_completes_parked_work_of_an_undecided_item() {
    let (_temp, catalog) = open_catalog();
    let first_run = start_run(&catalog);
    let item = parked_item(&catalog, first_run);
    let second_run = start_run(&catalog);
    parked_item(&catalog, second_run);

    assert!(
        catalog
            .close_review_item(item.id, ReviewStatus::AutoResolved)
            .unwrap()
    );

    let closed = catalog.get_review_item(item.id).unwrap().unwrap();
    assert_eq!(closed.status, ReviewStatus::AutoResolved);
    assert_eq!(closed.decision, None);
    assert_eq!(counts(&catalog, first_run), (0, 0, 1));
    assert_eq!(counts(&catalog, second_run), (0, 0, 1));
    assert_eq!(
        catalog
            .decide_review_item(item.id, ReviewDecision::Reject)
            .unwrap(),
        None
    );
}

#[test]
fn only_automatic_outcomes_can_close_an_item_automatically() {
    let (_temp, catalog) = open_catalog();
    let run_id = start_run(&catalog);
    let item = parked_item(&catalog, run_id);

    let error = catalog
        .close_review_item(item.id, ReviewStatus::Accepted)
        .unwrap_err();

    assert!(error.to_string().contains("cannot be closed automatically"));
    assert_eq!(counts(&catalog, run_id), (0, 1, 0));
}

#[test]
fn parking_reopens_an_automatically_closed_item() {
    let (_temp, catalog) = open_catalog();
    let first_run = start_run(&catalog);
    let item = parked_item(&catalog, first_run);
    catalog
        .close_review_item(item.id, ReviewStatus::Superseded)
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
