use std::cell::RefCell;

use game_media_vault_application::{
    CatalogPort, PortError, list_review_items, resolve_review_item,
};
use game_media_vault_domain::{
    AssetCandidate, AssetType, ImportedAsset, LibraryEntry, MatchEvidence, MatchSignal,
    NewReviewItem, PersistAsset, ReviewDecision, ReviewItem, ReviewMatchCandidate, ReviewStatus,
    SourceId,
};

struct FakeCatalog {
    items: RefCell<Vec<ReviewItem>>,
    accepted_work: RefCell<Vec<(i64, i64, String)>>,
    fail_atomic_accept: bool,
}

#[test]
fn failed_atomic_accept_does_not_persist_an_accept_decision() {
    let catalog = FakeCatalog {
        items: RefCell::new(vec![review_item()]),
        accepted_work: RefCell::new(Vec::new()),
        fail_atomic_accept: true,
    };

    let error = resolve_review_item(
        &catalog,
        17,
        ReviewDecision::Accept {
            release_edition_id: 201,
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("atomic accept failed"));
    assert_eq!(catalog.items.borrow()[0].decision, None);
}

impl CatalogPort for FakeCatalog {
    fn persist_asset(&self, _record: PersistAsset) -> Result<ImportedAsset, PortError> {
        unreachable!()
    }

    fn persist_review_item(&self, _item: NewReviewItem) -> Result<(), PortError> {
        unreachable!()
    }

    fn list_review_items(&self) -> Result<Vec<ReviewItem>, PortError> {
        Ok(self.items.borrow().clone())
    }

    fn set_review_decision(
        &self,
        review_item_id: i64,
        decision: ReviewDecision,
    ) -> Result<Option<ReviewItem>, PortError> {
        let mut items = self.items.borrow_mut();
        let Some(item) = items.iter_mut().find(|item| item.id == review_item_id) else {
            return Ok(None);
        };
        item.decision = Some(decision);
        Ok(Some(item.clone()))
    }

    fn accept_review_item_and_requeue(
        &self,
        review_item_id: i64,
        release_edition_id: i64,
        work_key: &str,
    ) -> Result<Option<ReviewItem>, PortError> {
        if self.fail_atomic_accept {
            return Err(PortError("atomic accept failed".to_owned()));
        }
        let mut items = self.items.borrow_mut();
        let Some(item) = items.iter_mut().find(|item| item.id == review_item_id) else {
            return Ok(None);
        };
        item.decision = Some(ReviewDecision::Accept { release_edition_id });
        item.status = ReviewStatus::Accepted;
        self.accepted_work.borrow_mut().push((
            item.run_id,
            release_edition_id,
            work_key.to_owned(),
        ));
        Ok(Some(item.clone()))
    }

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError> {
        Ok(Vec::new())
    }
}

fn review_item() -> ReviewItem {
    ReviewItem {
        id: 17,
        run_id: 7,
        candidate_identity: "connector:fixture-review".to_owned(),
        candidate: AssetCandidate {
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
        },
        competing_matches: vec![ReviewMatchCandidate {
            game_id: 301,
            release_edition_id: 201,
            game_title: "Target Game".to_owned(),
            platform: "Nintendo Entertainment System".to_owned(),
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

#[test]
fn resolves_and_lists_a_review_decision_through_the_application_seam() {
    let catalog = FakeCatalog {
        items: RefCell::new(vec![review_item()]),
        accepted_work: RefCell::new(Vec::new()),
        fail_atomic_accept: false,
    };

    let resolved = resolve_review_item(
        &catalog,
        17,
        ReviewDecision::Accept {
            release_edition_id: 201,
        },
    )
    .unwrap();

    assert_eq!(
        resolved.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201
        })
    );
    assert_eq!(list_review_items(&catalog).unwrap(), vec![resolved]);
    let accepted_work = catalog.accepted_work.borrow();
    assert_eq!(accepted_work.len(), 1);
    assert_eq!(accepted_work[0].0, 7);
    assert_eq!(accepted_work[0].1, 201);
    assert!(accepted_work[0].2.contains("fixture://candidate/front"));
}

#[test]
fn rejects_an_accept_decision_for_an_edition_not_offered_by_the_review_item() {
    let catalog = FakeCatalog {
        items: RefCell::new(vec![review_item()]),
        accepted_work: RefCell::new(Vec::new()),
        fail_atomic_accept: false,
    };

    let error = resolve_review_item(
        &catalog,
        17,
        ReviewDecision::Accept {
            release_edition_id: 999,
        },
    )
    .unwrap_err();

    assert!(error.to_string().contains("not a competing release"));
    assert_eq!(catalog.items.borrow()[0].decision, None);
    assert!(catalog.accepted_work.borrow().is_empty());
}
