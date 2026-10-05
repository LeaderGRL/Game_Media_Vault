mod support;

use game_media_vault_application::{
    DownloadLimits, PendingReviewDecision, PendingReviewSummary, acquire_run_with_connectors,
    decide_pending_reviews, review_page,
};
use game_media_vault_domain::{
    AssetCandidate, LibraryEntry, MatchingPolicy, ReviewDecision, ReviewStatus,
};
use support::*;

/// A candidate tying two editions of different Games, which no rule tells apart.
fn tied(title: &str, first_id: i64) -> (AssetCandidate, Vec<LibraryEntry>) {
    let candidate = AssetCandidate {
        edition_name: "Collector".to_owned(),
        ..candidate(title)
    };
    let editions = ["Standard", "Deluxe"]
        .iter()
        .enumerate()
        .map(|(index, edition)| LibraryEntry {
            edition_name: (*edition).to_owned(),
            ..release_for(&candidate, first_id + index as i64)
        })
        .collect();
    (candidate, editions)
}

/// Executes a run discovering `candidates` under `policy`, which parks the uncertain ones.
fn park(vault: &FakeVault, candidates: Vec<AssetCandidate>, policy: MatchingPolicy) -> i64 {
    let run_id = vault.start_run();
    acquire_run_with_connectors(
        vault,
        vault,
        vault,
        &FakeStore::default(),
        &[&FakeConnector::new(candidates)],
        run_id,
        policy,
        DownloadLimits::default(),
    )
    .unwrap();
    run_id
}

#[test]
fn reviews_come_a_page_at_a_time_with_how_many_await_a_decision() {
    let (first, mut releases) = tied("First Game", 401);
    let (second, more) = tied("Second Game", 411);
    let (third, others) = tied("Third Game", 421);
    releases.extend(more);
    releases.extend(others);
    let vault = FakeVault::with_library(releases);
    park(&vault, vec![first, second, third], matching_policy());
    let decided = vault.review_item(0).id;
    resolve(&vault, decided, ReviewDecision::Reject);

    let page = review_page(&vault, 1, 1).unwrap();

    // Decided items leave the list; the second undecided item is on the second page.
    assert_eq!(page.undecided, 2);
    assert_eq!(page.offset, 1);
    assert_eq!(
        page.items
            .iter()
            .map(|item| item.candidate.game_title.as_str())
            .collect::<Vec<_>>(),
        ["Third Game"]
    );
}

#[test]
fn accepting_the_best_matches_decides_every_review_but_ties_of_several_games() {
    let (tie, mut releases) = tied("Tied Game", 401);
    let (threshold, release) = threshold_candidate_and_release();
    releases.push(release.clone());
    let vault = FakeVault::with_library(releases);
    // Parked under a stricter matcher, the threshold candidate's best match is its release.
    let run = park(&vault, vec![tie, threshold], stricter_matching_policy());

    let summary = decide_pending_reviews(
        &vault,
        &vault,
        PendingReviewDecision::AcceptBestMatches,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(
        summary,
        PendingReviewSummary {
            decided: 1,
            left: 1
        }
    );
    let statuses: Vec<(String, ReviewStatus, Option<ReviewDecision>)> = vault
        .review_items
        .borrow()
        .iter()
        .map(|item| {
            (
                item.candidate.game_title.clone(),
                item.status,
                item.decision.clone(),
            )
        })
        .collect();
    assert!(statuses.contains(&(
        "Threshold Review Game".to_owned(),
        ReviewStatus::Accepted,
        Some(ReviewDecision::Accept {
            release_edition_id: release.release_edition_id
        })
    )));
    assert!(statuses.contains(&("Tied Game".to_owned(), ReviewStatus::Pending, None)));
    // The accepted candidate's work waits to be downloaded again.
    assert_eq!(vault.run(run).queued_work, 1);
}

#[test]
fn rejecting_every_review_settles_all_their_work() {
    let (tie, releases) = tied("Tied Game", 401);
    let vault = FakeVault::with_library(releases);
    let run = park(&vault, vec![tie], matching_policy());

    let summary = decide_pending_reviews(
        &vault,
        &vault,
        PendingReviewDecision::RejectAll,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(
        summary,
        PendingReviewSummary {
            decided: 1,
            left: 0
        }
    );
    assert_eq!(vault.review_item(0).status, ReviewStatus::Rejected);
    assert_eq!(vault.run(run).dismissed_work, 1);
}

#[test]
fn reviews_closed_meanwhile_are_not_left_awaiting_a_decision() {
    let (first, mut releases) = tied("First Game", 401);
    let (second, more) = tied("Second Game", 411);
    releases.extend(more);
    let vault = FakeVault::with_library(releases);
    park(&vault, vec![first, second], matching_policy());
    // Someone rejects the first item after the listing, before the decisions are recorded.
    *vault.rejection_before_next_decisions.borrow_mut() = true;

    let summary = decide_pending_reviews(
        &vault,
        &vault,
        PendingReviewDecision::RejectAll,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(
        summary,
        PendingReviewSummary {
            decided: 1,
            left: 0
        }
    );
}

#[test]
fn a_tie_closed_meanwhile_is_not_left_awaiting_a_decision() {
    let (tie, mut releases) = tied("Tied Game", 401);
    let (threshold, release) = threshold_candidate_and_release();
    releases.push(release);
    let vault = FakeVault::with_library(releases);
    park(&vault, vec![tie, threshold], stricter_matching_policy());
    // Someone rejects the tie, the first item, while the other decisions are recorded.
    *vault.rejection_before_next_decisions.borrow_mut() = true;

    let summary = decide_pending_reviews(
        &vault,
        &vault,
        PendingReviewDecision::AcceptBestMatches,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(
        summary,
        PendingReviewSummary {
            decided: 1,
            left: 0
        }
    );
}

#[test]
fn a_review_whose_matches_changed_since_it_was_read_is_left_undecided() {
    let (threshold, release) = threshold_candidate_and_release();
    let vault = FakeVault::with_library(vec![release]);
    park(&vault, vec![threshold], stricter_matching_policy());
    // An acquisition refreshes the item's competing releases before the decision is recorded,
    // so the best match chosen may no longer be.
    *vault.matches_refreshed_before_next_decisions.borrow_mut() = true;

    let summary = decide_pending_reviews(
        &vault,
        &vault,
        PendingReviewDecision::AcceptBestMatches,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(
        summary,
        PendingReviewSummary {
            decided: 0,
            left: 1
        }
    );
    assert_eq!(vault.review_item(0).status, ReviewStatus::Pending);
}

#[test]
fn rejecting_every_review_needs_no_valid_matching_policy() {
    let (tie, releases) = tied("Tied Game", 401);
    let vault = FakeVault::with_library(releases);
    park(&vault, vec![tie], matching_policy());
    // Rejecting scores nothing, so thresholds a match could not use do not stop it.
    let unusable = MatchingPolicy {
        high_confidence_threshold: 40,
        medium_confidence_threshold: 50,
    };

    let summary =
        decide_pending_reviews(&vault, &vault, PendingReviewDecision::RejectAll, unusable).unwrap();

    assert_eq!(summary.decided, 1);
}

fn resolve(vault: &FakeVault, review_item_id: i64, decision: ReviewDecision) {
    game_media_vault_application::resolve_review_item(vault, review_item_id, decision).unwrap();
}
