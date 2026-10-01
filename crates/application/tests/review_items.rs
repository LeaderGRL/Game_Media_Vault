mod support;

use std::cell::RefCell;

use game_media_vault_application::{
    ApplicationError, ParkedReview, PortError, ReviewDecisionOutcome, ReviewRepositoryPort,
    list_review_items, load_review_preview, resolve_review_item,
};
use game_media_vault_domain::{
    AssetCandidate, ImportedAsset, MatchEvidence, MatchSignal, NewReviewItem, PersistAsset,
    ReviewDecision, ReviewItem, ReviewMatchCandidate, ReviewStatus, SourceId,
};
use support::*;

fn review_item() -> ReviewItem {
    let candidate = AssetCandidate {
        edition_name: "Collector".to_owned(),
        ..candidate("Target Game")
    };
    ReviewItem {
        id: 17,
        candidate_identity: "candidate:fixture-review".to_owned(),
        candidate,
        competing_matches: vec![ReviewMatchCandidate {
            game_id: 301,
            release_edition_id: 201,
            game_title: "Target Game".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "USA".to_owned(),
            edition_name: "Standard".to_owned(),
            score: 90,
            evidence: vec![MatchEvidence {
                signal: MatchSignal::Title,
                candidate_value: "Target Game".to_owned(),
                release_value: "Target Game".to_owned(),
                score_delta: 50,
            }],
            assertions: Vec::new(),
        }],
        decision: None,
        status: ReviewStatus::Pending,
    }
}

fn vault_with(item: ReviewItem) -> FakeVault {
    let vault = FakeVault::default();
    vault.review_items.borrow_mut().push(item);
    vault
}

#[test]
fn resolves_and_lists_a_review_decision_through_the_application_seam() {
    let vault = vault_with(review_item());

    let resolved = resolve_review_item(
        &vault,
        17,
        ReviewDecision::Accept {
            release_edition_id: 201,
        },
    )
    .unwrap();

    assert_eq!(resolved.status, ReviewStatus::Accepted);
    assert_eq!(
        resolved.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201
        })
    );
    assert_eq!(list_review_items(&vault).unwrap(), vec![resolved]);
}

#[test]
fn rejects_an_accept_decision_for_an_edition_not_offered_by_the_review_item() {
    let vault = vault_with(review_item());

    let error = resolve_review_item(
        &vault,
        17,
        ReviewDecision::Accept {
            release_edition_id: 999,
        },
    )
    .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::ReviewAcceptanceNotCompeting {
            review_item_id: 17,
            release_edition_id: 999,
        }
    );
    assert_eq!(vault.review_item(0).status, ReviewStatus::Pending);
}

#[test]
fn decided_and_automatically_closed_items_cannot_be_resolved() {
    for status in [
        ReviewStatus::Accepted,
        ReviewStatus::Rejected,
        ReviewStatus::AutoResolved,
        ReviewStatus::Superseded,
    ] {
        let vault = vault_with(ReviewItem {
            status,
            ..review_item()
        });

        let error = resolve_review_item(&vault, 17, ReviewDecision::Defer).unwrap_err();

        assert_eq!(
            error,
            ApplicationError::ReviewItemNotActionable {
                review_item_id: 17,
                status,
            }
        );
    }
}

#[test]
fn deferred_items_can_still_be_decided() {
    let vault = vault_with(ReviewItem {
        status: ReviewStatus::Deferred,
        decision: Some(ReviewDecision::Defer),
        ..review_item()
    });

    let resolved = resolve_review_item(&vault, 17, ReviewDecision::Reject).unwrap();

    assert_eq!(resolved.status, ReviewStatus::Rejected);
}

#[test]
fn resolving_an_unknown_item_reports_it_missing() {
    let error = resolve_review_item(&FakeVault::default(), 99, ReviewDecision::Reject).unwrap_err();

    assert_eq!(error, ApplicationError::ReviewItemNotFound(99));
}

/// Review repository where acquisition changes the item between the read and the decision.
struct ChangedDuringDecision {
    outcome: ReviewDecisionOutcome,
    item: RefCell<ReviewItem>,
}

impl ReviewRepositoryPort for ChangedDuringDecision {
    fn list_review_items(&self) -> Result<Vec<ReviewItem>, PortError> {
        Ok(vec![self.item.borrow().clone()])
    }

    fn get_review_item(&self, _review_item_id: i64) -> Result<Option<ReviewItem>, PortError> {
        Ok(Some(self.item.borrow().clone()))
    }

    fn find_review_item(&self, _identity: &str) -> Result<Option<ReviewItem>, PortError> {
        unreachable!()
    }

    fn park_work_for_review(
        &self,
        _run_id: i64,
        _work_key: &str,
        _item: NewReviewItem,
    ) -> Result<ParkedReview, PortError> {
        unreachable!()
    }

    fn supersede_candidate_review(
        &self,
        _run_id: i64,
        _candidate_identity: &str,
    ) -> Result<bool, PortError> {
        unreachable!()
    }

    fn decide_review_item(
        &self,
        _review_item_id: i64,
        _decision: ReviewDecision,
    ) -> Result<ReviewDecisionOutcome, PortError> {
        Ok(self.outcome.clone())
    }

    fn persist_candidate_asset(
        &self,
        _run_id: i64,
        _candidate_identity: &str,
        _record: PersistAsset,
    ) -> Result<Option<ImportedAsset>, PortError> {
        unreachable!()
    }
}

#[test]
fn decision_losing_a_race_with_acquisition_reports_the_new_status() {
    let reviews = ChangedDuringDecision {
        item: RefCell::new(review_item()),
        outcome: ReviewDecisionOutcome::NotUndecided(ReviewStatus::AutoResolved),
    };

    let error = resolve_review_item(&reviews, 17, ReviewDecision::Reject).unwrap_err();

    assert_eq!(
        error,
        ApplicationError::ReviewItemNotActionable {
            review_item_id: 17,
            status: ReviewStatus::AutoResolved,
        }
    );
}

#[test]
fn acceptance_of_an_edition_dropped_by_a_concurrent_refresh_is_refused() {
    let reviews = ChangedDuringDecision {
        item: RefCell::new(review_item()),
        outcome: ReviewDecisionOutcome::NotCompeting,
    };

    let error = resolve_review_item(
        &reviews,
        17,
        ReviewDecision::Accept {
            release_edition_id: 201,
        },
    )
    .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::ReviewAcceptanceNotCompeting {
            review_item_id: 17,
            release_edition_id: 201,
        }
    );
}

#[test]
fn review_preview_downloads_the_persisted_candidate_through_the_connector() {
    let vault = vault_with(review_item());
    let connector = FakeConnector::new(Vec::new());

    let preview = load_review_preview(&vault, &connector, 17).unwrap();

    assert_eq!(preview.original_filename, "Target Game.png");
    assert_eq!(preview.bytes, b"bytes of Target Game.png");
    assert_eq!(
        connector.downloads.borrow().as_slice(),
        &[review_item().candidate.source_url]
    );
}

#[test]
fn review_preview_requires_the_connector_of_the_candidate_source() {
    let vault = vault_with(ReviewItem {
        candidate: AssetCandidate {
            source_id: SourceId::from("another-source"),
            ..review_item().candidate
        },
        ..review_item()
    });
    let connector = FakeConnector::new(Vec::new());

    let error = load_review_preview(&vault, &connector, 17).unwrap_err();

    assert!(matches!(
        error,
        ApplicationError::ConnectorCandidateSourceMismatch { .. }
    ));
    assert!(connector.downloads.borrow().is_empty());
}
