use std::{
    cell::RefCell,
    collections::HashMap,
    io::{Cursor, Read},
};

use game_media_vault_application::{
    CatalogPort, ConnectorPort, ObjectStorePort, PortError, ReviewProcessingClaim,
    ReviewProcessingFinalization, RunRepositoryPort, acquire_run_with_connector,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRun,
    AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate, AssetType, AssetTypeSelector,
    ConnectorCapabilities, GameSelection, ImportedAsset, LibraryEntry, MatchConfidence,
    MatchingPolicy, NewReviewItem, PersistAsset, QualityRequirements, ReleaseAssertion,
    ReleaseAssertionField, RetentionPolicy, ReviewDecision, ReviewItem, ReviewStatus, SourceId,
    SourceSelection, StoredObject,
};

fn matching_policy() -> MatchingPolicy {
    MatchingPolicy {
        high_confidence_threshold: 80,
        medium_confidence_threshold: 50,
    }
}

fn request() -> AcquisitionRequest {
    AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap()
}

struct FakeRuns {
    run: RefCell<AcquisitionRun>,
    work: RefCell<HashMap<String, bool>>,
    status_after_complete: Option<AcquisitionRunStatus>,
    status_before_complete_cas: Option<AcquisitionRunStatus>,
}

impl FakeRuns {
    fn new(run: AcquisitionRun) -> Self {
        Self {
            run: RefCell::new(run),
            work: RefCell::new(HashMap::new()),
            status_after_complete: None,
            status_before_complete_cas: None,
        }
    }

    fn with_status_after_complete(mut self, status: AcquisitionRunStatus) -> Self {
        self.status_after_complete = Some(status);
        self
    }

    fn with_status_before_complete_cas(mut self, status: AcquisitionRunStatus) -> Self {
        self.status_before_complete_cas = Some(status);
        self
    }
}

impl RunRepositoryPort for FakeRuns {
    fn create_run(&self, _request: AcquisitionRequest) -> Result<AcquisitionRun, PortError> {
        unreachable!()
    }

    fn get_run(&self, run_id: i64) -> Result<Option<AcquisitionRun>, PortError> {
        Ok((self.run.borrow().id == run_id).then(|| self.run.borrow().clone()))
    }

    fn list_runs(&self) -> Result<Vec<AcquisitionRun>, PortError> {
        Ok(vec![self.run.borrow().clone()])
    }

    fn queue_work(&self, run_id: i64, work_key: String) -> Result<(), PortError> {
        assert_eq!(run_id, self.run.borrow().id);
        let mut work = self.work.borrow_mut();
        if work.insert(work_key, false).is_none() {
            self.run.borrow_mut().queued_work += 1;
        }
        Ok(())
    }

    fn requeue_completed_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError> {
        assert_eq!(run_id, self.run.borrow().id);
        let mut work = self.work.borrow_mut();
        let Some(completed) = work.get_mut(work_key) else {
            return Err(PortError(format!(
                "acquisition work {work_key:?} does not exist for run #{run_id}"
            )));
        };
        if *completed {
            *completed = false;
            let mut run = self.run.borrow_mut();
            run.queued_work += 1;
            run.completed_work -= 1;
            if run.status == AcquisitionRunStatus::Completed {
                run.status = AcquisitionRunStatus::Running;
            }
        }
        Ok(())
    }

    fn next_queued_work(&self, run_id: i64) -> Result<Option<AcquisitionWorkItem>, PortError> {
        assert_eq!(run_id, self.run.borrow().id);
        Ok(self
            .work
            .borrow()
            .iter()
            .find(|(_, completed)| !**completed)
            .map(|(key, _)| AcquisitionWorkItem { key: key.clone() }))
    }

    fn complete_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError> {
        assert_eq!(run_id, self.run.borrow().id);
        let mut work = self.work.borrow_mut();
        if let Some(completed) = work.get_mut(work_key)
            && !*completed
        {
            *completed = true;
            let mut run = self.run.borrow_mut();
            run.queued_work -= 1;
            run.completed_work += 1;
            if let Some(status) = self.status_after_complete {
                run.status = status;
            }
        }
        Ok(())
    }

    fn compare_and_set_run_status(
        &self,
        run_id: i64,
        expected: AcquisitionRunStatus,
        target: AcquisitionRunStatus,
    ) -> Result<bool, PortError> {
        assert_eq!(run_id, self.run.borrow().id);
        let mut run = self.run.borrow_mut();
        if target == AcquisitionRunStatus::Completed
            && let Some(status) = self.status_before_complete_cas
        {
            run.status = status;
        }
        if run.status != expected
            || (target == AcquisitionRunStatus::Completed && run.queued_work != 0)
        {
            return Ok(false);
        }
        run.status = target;
        Ok(true)
    }
}

struct FakeConnector {
    downloads: RefCell<Vec<String>>,
    candidates: Vec<AssetCandidate>,
}

struct NoDiscoveryConnector {
    downloads: RefCell<Vec<String>>,
}

struct FailingDiscoveryConnector {
    downloads: RefCell<Vec<String>>,
}

impl ConnectorPort for FakeConnector {
    fn source_id(&self) -> &'static str {
        "libretro-thumbnails"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::BoxFront],
            direct_media_download: true,
        }
    }

    fn discover(&self, _request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        if !self.candidates.is_empty() {
            return Ok(self.candidates.clone());
        }
        Ok(vec![AssetCandidate {
            provider_candidate_id: None,
            game_title: "Super Mario Bros. (World)".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "Unknown".to_owned(),
            edition_name: "Unspecified".to_owned(),
            asset_type: AssetType::BoxFront,
            source_id: SourceId::from("libretro-thumbnails"),
            source_asset_label: Some("Named_Boxarts".to_owned()),
            source_url: "https://example.invalid/Named_Boxarts/Super%20Mario%20Bros.%20(World).png"
                .to_owned(),
            original_filename: "Super Mario Bros. (World).png".to_owned(),
        }])
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        self.downloads
            .borrow_mut()
            .push(candidate.source_url.clone());
        Ok(Box::new(Cursor::new(b"fixture box front".to_vec())))
    }
}

impl ConnectorPort for NoDiscoveryConnector {
    fn source_id(&self) -> &'static str {
        "libretro-thumbnails"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::BoxFront],
            direct_media_download: true,
        }
    }

    fn discover(&self, _request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        Ok(Vec::new())
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        self.downloads
            .borrow_mut()
            .push(candidate.source_url.clone());
        Ok(Box::new(Cursor::new(b"fixture box front".to_vec())))
    }
}

impl ConnectorPort for FailingDiscoveryConnector {
    fn source_id(&self) -> &'static str {
        "libretro-thumbnails"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::BoxFront],
            direct_media_download: true,
        }
    }

    fn discover(&self, _request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        Err(PortError("fixture discovery unavailable".to_owned()))
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        self.downloads
            .borrow_mut()
            .push(candidate.source_url.clone());
        Ok(Box::new(Cursor::new(b"fixture box front".to_vec())))
    }
}

#[derive(Default)]
struct FakeStore {
    bytes: RefCell<Vec<Vec<u8>>>,
}

impl ObjectStorePort for FakeStore {
    fn store_original(&self, _source: &std::path::Path) -> Result<StoredObject, PortError> {
        unreachable!()
    }

    fn store_original_reader(&self, reader: &mut dyn Read) -> Result<StoredObject, PortError> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|error| PortError(error.to_string()))?;
        self.bytes.borrow_mut().push(bytes.clone());
        Ok(StoredObject {
            hash: "fixture-hash".to_owned(),
            byte_len: bytes.len() as u64,
        })
    }
}

struct FakeCatalog {
    records: RefCell<Vec<PersistAsset>>,
    review_items: RefCell<Vec<ReviewItem>>,
    finalize_calls: RefCell<u32>,
    library: Vec<LibraryEntry>,
}

impl Default for FakeCatalog {
    fn default() -> Self {
        Self {
            records: RefCell::new(Vec::new()),
            review_items: RefCell::new(Vec::new()),
            finalize_calls: RefCell::new(0),
            library: vec![matching_release()],
        }
    }
}

impl CatalogPort for FakeCatalog {
    fn persist_asset(&self, record: PersistAsset) -> Result<ImportedAsset, PortError> {
        self.records.borrow_mut().push(record.clone());
        Ok(ImportedAsset {
            game_id: 1,
            release_edition_id: 1,
            asset_id: 1,
            object_hash: record.object_hash,
            byte_len: record.byte_len,
        })
    }

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError> {
        Ok(self.library.clone())
    }

    fn persist_review_item(&self, item: NewReviewItem) -> Result<(), PortError> {
        let mut review_items = self.review_items.borrow_mut();
        if let Some(existing) = review_items
            .iter_mut()
            .find(|existing| existing.candidate_identity == item.candidate_identity)
        {
            if existing.run_id == item.run_id
                && matches!(
                    existing.status,
                    ReviewStatus::Pending | ReviewStatus::Deferred | ReviewStatus::Processing
                )
            {
                existing.candidate = item.candidate;
                existing.competing_matches = item.competing_matches;
            }
            return Ok(());
        }
        let id = review_items.len() as i64 + 1;
        review_items.push(ReviewItem {
            id,
            run_id: item.run_id,
            candidate_identity: item.candidate_identity,
            candidate: item.candidate,
            competing_matches: item.competing_matches,
            decision: None,
            status: ReviewStatus::Pending,
        });
        Ok(())
    }

    fn stage_review_item_and_complete_work(
        &self,
        item: NewReviewItem,
        _work_key: &str,
    ) -> Result<bool, PortError> {
        let mut review_items = self.review_items.borrow_mut();
        let inherited = review_items
            .iter()
            .rev()
            .find(|existing| {
                existing.run_id != item.run_id
                    && existing.candidate_identity == item.candidate_identity
                    && matches!(
                        existing.status,
                        ReviewStatus::Accepted | ReviewStatus::Applied | ReviewStatus::Rejected
                    )
            })
            .cloned();
        let Some(inherited) = inherited else {
            return Ok(false);
        };

        let (decision, status) = match inherited.decision {
            Some(ReviewDecision::Accept { release_edition_id }) => {
                let status = if item
                    .competing_matches
                    .iter()
                    .any(|candidate| candidate.release_edition_id == release_edition_id)
                {
                    ReviewStatus::Accepted
                } else {
                    ReviewStatus::Superseded
                };
                (Some(ReviewDecision::Accept { release_edition_id }), status)
            }
            Some(ReviewDecision::Reject) => (Some(ReviewDecision::Reject), ReviewStatus::Rejected),
            _ => return Ok(false),
        };
        let id = review_items
            .iter()
            .map(|review| review.id)
            .max()
            .unwrap_or(0)
            + 1;
        review_items.push(ReviewItem {
            id,
            run_id: item.run_id,
            candidate_identity: item.candidate_identity,
            candidate: item.candidate,
            competing_matches: item.competing_matches,
            decision,
            status,
        });
        Ok(true)
    }

    fn list_review_items(&self) -> Result<Vec<ReviewItem>, PortError> {
        Ok(self.review_items.borrow().clone())
    }

    fn set_review_decision(
        &self,
        review_item_id: i64,
        decision: ReviewDecision,
    ) -> Result<Option<ReviewItem>, PortError> {
        let mut review_items = self.review_items.borrow_mut();
        let Some(review_item) = review_items
            .iter_mut()
            .find(|review_item| review_item.id == review_item_id)
        else {
            return Ok(None);
        };
        review_item.status = match &decision {
            ReviewDecision::Accept { .. } => ReviewStatus::Accepted,
            ReviewDecision::Reject => ReviewStatus::Rejected,
            ReviewDecision::Defer => ReviewStatus::Deferred,
        };
        review_item.decision = Some(decision);
        Ok(Some(review_item.clone()))
    }

    fn set_review_status(
        &self,
        review_item_id: i64,
        status: ReviewStatus,
    ) -> Result<Option<ReviewItem>, PortError> {
        let mut review_items = self.review_items.borrow_mut();
        let Some(review_item) = review_items
            .iter_mut()
            .find(|review_item| review_item.id == review_item_id)
        else {
            return Ok(None);
        };
        review_item.status = status;
        Ok(Some(review_item.clone()))
    }

    fn claim_review_item_for_processing(
        &self,
        review_item_id: i64,
    ) -> Result<Option<ReviewProcessingClaim>, PortError> {
        let mut review_items = self.review_items.borrow_mut();
        let Some(review_item) = review_items
            .iter_mut()
            .find(|review_item| review_item.id == review_item_id)
        else {
            return Ok(None);
        };
        if !matches!(
            review_item.status,
            ReviewStatus::Pending | ReviewStatus::Deferred | ReviewStatus::Accepted
        ) {
            return Ok(None);
        }
        review_item.status = ReviewStatus::Processing;
        Ok(Some(ReviewProcessingClaim {
            item: review_item.clone(),
            lease_token: format!("fake-lease-{review_item_id}"),
        }))
    }

    fn renew_review_item_processing(
        &self,
        review_item_id: i64,
        lease_token: &str,
    ) -> Result<bool, PortError> {
        Ok(lease_token == format!("fake-lease-{review_item_id}"))
    }

    fn finalize_review_processing_asset(
        &self,
        review_item_id: i64,
        _lease_token: &str,
        _run_id: i64,
        _work_key: &str,
        record: PersistAsset,
    ) -> Result<ReviewProcessingFinalization, PortError> {
        *self.finalize_calls.borrow_mut() += 1;
        let imported = self.persist_asset(record)?;
        self.update_processing_status(review_item_id, ReviewStatus::AutoResolved)?;
        Ok(ReviewProcessingFinalization::Imported(imported))
    }

    fn finalize_accepted_review_asset(
        &self,
        review_item_id: i64,
        lease_token: &str,
        _run_id: i64,
        _work_key: &str,
        record: PersistAsset,
    ) -> Result<ReviewProcessingFinalization, PortError> {
        assert_eq!(lease_token, format!("fake-lease-{review_item_id}"));
        *self.finalize_calls.borrow_mut() += 1;
        let imported = self.persist_asset(record)?;
        self.update_processing_status(review_item_id, ReviewStatus::Applied)?;
        Ok(ReviewProcessingFinalization::Imported(imported))
    }

    fn refresh_review_processing_and_complete_work(
        &self,
        review_item_id: i64,
        lease_token: &str,
        item: NewReviewItem,
        _work_key: &str,
        status: ReviewStatus,
    ) -> Result<ReviewItem, PortError> {
        assert_eq!(lease_token, format!("fake-lease-{review_item_id}"));
        assert!(matches!(
            status,
            ReviewStatus::Pending | ReviewStatus::Deferred
        ));
        let mut review_items = self.review_items.borrow_mut();
        let review_item = review_items
            .iter_mut()
            .find(|review_item| review_item.id == review_item_id)
            .ok_or_else(|| PortError(format!("review item #{review_item_id} does not exist")))?;
        assert_eq!(review_item.status, ReviewStatus::Processing);
        assert_eq!(review_item.run_id, item.run_id);
        assert_eq!(review_item.candidate_identity, item.candidate_identity);
        review_item.candidate = item.candidate;
        review_item.competing_matches = item.competing_matches;
        review_item.status = status;
        Ok(review_item.clone())
    }

    fn supersede_review_processing_and_complete_work(
        &self,
        review_item_id: i64,
        _lease_token: &str,
        _run_id: i64,
        _work_key: &str,
    ) -> Result<Option<ReviewItem>, PortError> {
        *self.finalize_calls.borrow_mut() += 1;
        self.update_processing_status(review_item_id, ReviewStatus::Superseded)
    }

    fn finish_review_item_processing(
        &self,
        review_item_id: i64,
        _lease_token: &str,
        status: ReviewStatus,
    ) -> Result<Option<ReviewItem>, PortError> {
        self.update_processing_status(review_item_id, status)
    }

    fn restore_review_item_processing(
        &self,
        review_item_id: i64,
        _lease_token: &str,
        status: ReviewStatus,
    ) -> Result<Option<ReviewItem>, PortError> {
        self.update_processing_status(review_item_id, status)
    }
}

impl FakeCatalog {
    fn update_processing_status(
        &self,
        review_item_id: i64,
        status: ReviewStatus,
    ) -> Result<Option<ReviewItem>, PortError> {
        let mut review_items = self.review_items.borrow_mut();
        let Some(review_item) = review_items
            .iter_mut()
            .find(|review_item| review_item.id == review_item_id)
        else {
            return Ok(None);
        };
        assert_eq!(review_item.status, ReviewStatus::Processing);
        review_item.status = status;
        Ok(Some(review_item.clone()))
    }
}

fn matching_release() -> LibraryEntry {
    LibraryEntry {
        game_id: 41,
        game_title: "Super Mario Bros. (World)".to_owned(),
        release_edition_id: 73,
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        assertions: Vec::new(),
        assets: Vec::new(),
    }
}

fn release_for_candidate(candidate: &AssetCandidate, release_edition_id: i64) -> LibraryEntry {
    LibraryEntry {
        game_id: release_edition_id + 1_000,
        game_title: candidate.game_title.clone(),
        release_edition_id,
        platform: candidate.platform.clone(),
        region: candidate.region.clone(),
        edition_name: candidate.edition_name.clone(),
        assertions: Vec::new(),
        assets: Vec::new(),
    }
}

#[test]
fn acquires_a_requested_box_front_through_the_connector_pipeline() {
    let run = AcquisitionRun {
        id: 7,
        request: request(),
        status: AcquisitionRunStatus::Running,
        queued_work: 0,
        completed_work: 0,
    };
    let runs = FakeRuns::new(run);
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: Vec::new(),
    };
    let store = FakeStore::default();
    let catalog = FakeCatalog::default();

    let imported =
        acquire_run_with_connector(&runs, &catalog, &store, &connector, 7, matching_policy())
            .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(connector.downloads.borrow().len(), 1);
    assert_eq!(
        store.bytes.borrow().as_slice(),
        &[b"fixture box front".to_vec()]
    );

    let records = catalog.records.borrow();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].asset_type, AssetType::BoxFront);
    assert_eq!(records[0].existing_game_id, Some(41));
    assert_eq!(records[0].existing_release_edition_id, Some(73));
    let match_decision = records[0].match_decision.as_ref().unwrap();
    assert_eq!(match_decision.release_edition_id, Some(73));
    assert_eq!(match_decision.score, 80);
    assert_eq!(match_decision.confidence, MatchConfidence::High);
    assert_eq!(match_decision.evidence.len(), 4);
    assert_eq!(records[0].source_id, SourceId::from("libretro-thumbnails"));
    assert_eq!(
        records[0].source_location,
        "https://example.invalid/Named_Boxarts/Super%20Mario%20Bros.%20(World).png"
    );

    let final_run = runs.run.borrow();
    assert_eq!(final_run.status, AcquisitionRunStatus::Completed);
    assert_eq!(final_run.queued_work, 0);
    assert_eq!(final_run.completed_work, 1);
}

#[test]
fn low_confidence_candidate_is_left_unattached_without_downloading() {
    let runs = FakeRuns::new(run_with_request(request()));
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![AssetCandidate {
            provider_candidate_id: None,
            game_title: "Completely Different Game".to_owned(),
            platform: "Different Platform".to_owned(),
            region: "Europe".to_owned(),
            edition_name: "Collector".to_owned(),
            asset_type: AssetType::BoxFront,
            source_id: SourceId::from("libretro-thumbnails"),
            source_asset_label: Some("Named_Boxarts".to_owned()),
            source_url: "https://example.invalid/unmatched.png".to_owned(),
            original_filename: "unmatched.png".to_owned(),
        }],
    };
    let store = FakeStore::default();
    let catalog = FakeCatalog::default();

    let imported =
        acquire_run_with_connector(&runs, &catalog, &store, &connector, 7, matching_policy())
            .unwrap();

    assert!(imported.is_empty());
    assert!(connector.downloads.borrow().is_empty());
    assert!(store.bytes.borrow().is_empty());
    assert!(catalog.records.borrow().is_empty());
    assert_eq!(runs.run.borrow().completed_work, 1);
    assert_eq!(runs.run.borrow().status, AcquisitionRunStatus::Completed);
}

#[test]
fn medium_confidence_candidate_creates_review_item_with_competing_release_evidence() {
    let candidate = AssetCandidate {
        provider_candidate_id: None,
        game_title: "Super Mario Bros. (World)".to_owned(),
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Collector".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("libretro-thumbnails"),
        source_asset_label: Some("Named_Boxarts".to_owned()),
        source_url: "https://example.invalid/review.png".to_owned(),
        original_filename: "review.png".to_owned(),
    };
    let first_release = LibraryEntry {
        game_id: 101,
        game_title: candidate.game_title.clone(),
        release_edition_id: 201,
        platform: candidate.platform.clone(),
        region: candidate.region.clone(),
        edition_name: "Standard".to_owned(),
        assertions: vec![ReleaseAssertion {
            source_id: SourceId::from("reference-catalog"),
            source_location: "fixture://reference/smb-standard".to_owned(),
            field: ReleaseAssertionField::Identifier,
            qualifier: Some("source_record".to_owned()),
            value: "smb-standard".to_owned(),
        }],
        assets: Vec::new(),
    };
    let second_release = LibraryEntry {
        game_id: 102,
        release_edition_id: 202,
        edition_name: "Deluxe".to_owned(),
        ..first_release.clone()
    };
    let runs = FakeRuns::new(run_with_request(request()));
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate.clone()],
    };
    let store = FakeStore::default();
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library: vec![first_release, second_release],
    };

    let imported =
        acquire_run_with_connector(&runs, &catalog, &store, &connector, 7, matching_policy())
            .unwrap();

    assert!(imported.is_empty());
    assert!(connector.downloads.borrow().is_empty());
    assert!(store.bytes.borrow().is_empty());
    assert!(catalog.records.borrow().is_empty());
    let review_items = catalog.review_items.borrow();
    assert_eq!(review_items.len(), 1);
    assert_eq!(review_items[0].run_id, 7);
    assert_eq!(review_items[0].candidate, candidate);
    assert_eq!(
        review_items[0]
            .competing_matches
            .iter()
            .map(|candidate_match| candidate_match.release_edition_id)
            .collect::<Vec<_>>(),
        vec![201, 202]
    );
    assert!(
        review_items[0]
            .competing_matches
            .iter()
            .all(|candidate_match| candidate_match.score == 90)
    );
    assert!(
        review_items[0]
            .competing_matches
            .iter()
            .all(|candidate_match| candidate_match.evidence.len() == 4)
    );
    assert_eq!(
        review_items[0].competing_matches[0].assertions[0].source_id,
        SourceId::from("reference-catalog")
    );
    assert_eq!(runs.run.borrow().completed_work, 1);
    assert_eq!(runs.run.borrow().status, AcquisitionRunStatus::Completed);
}

fn ambiguous_candidate_and_releases() -> (AssetCandidate, Vec<LibraryEntry>) {
    let candidate = AssetCandidate {
        provider_candidate_id: None,
        game_title: "Review Game".to_owned(),
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Collector".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("libretro-thumbnails"),
        source_asset_label: Some("Named_Boxarts".to_owned()),
        source_url: "https://example.invalid/review-reuse.png".to_owned(),
        original_filename: "review-reuse.png".to_owned(),
    };
    let first = LibraryEntry {
        game_id: 301,
        game_title: candidate.game_title.clone(),
        release_edition_id: 401,
        platform: candidate.platform.clone(),
        region: candidate.region.clone(),
        edition_name: "Standard".to_owned(),
        assertions: Vec::new(),
        assets: Vec::new(),
    };
    let second = LibraryEntry {
        game_id: 302,
        release_edition_id: 402,
        edition_name: "Deluxe".to_owned(),
        ..first.clone()
    };
    (candidate, vec![first, second])
}

fn create_review_then_set_decision(
    catalog: &FakeCatalog,
    connector: &FakeConnector,
    decision: ReviewDecision,
) {
    let first_run = FakeRuns::new(run_with_request(request()));
    acquire_run_with_connector(
        &first_run,
        catalog,
        &FakeStore::default(),
        connector,
        7,
        matching_policy(),
    )
    .unwrap();
    catalog.review_items.borrow_mut()[0].decision = Some(decision);
}

#[test]
fn accepted_review_decision_is_reused_for_the_same_candidate_identity() {
    let (candidate, library) = ambiguous_candidate_and_releases();
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    create_review_then_set_decision(
        &catalog,
        &connector,
        ReviewDecision::Accept {
            release_edition_id: 402,
        },
    );
    let second_run = FakeRuns::new(run_with_request(request()));

    let imported = acquire_run_with_connector(
        &second_run,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(connector.downloads.borrow().len(), 1);
    let records = catalog.records.borrow();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].existing_game_id, Some(302));
    assert_eq!(records[0].existing_release_edition_id, Some(402));
    assert_eq!(
        records[0].match_decision.as_ref().unwrap().confidence,
        MatchConfidence::Medium
    );
}

#[test]
fn inherited_acceptance_is_materialized_and_leased_before_download() {
    let (candidate, library) = ambiguous_candidate_and_releases();
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    let mut historical_run = run_with_request(request());
    historical_run.id = 1;
    acquire_run_with_connector(
        &FakeRuns::new(historical_run),
        &catalog,
        &FakeStore::default(),
        &connector,
        1,
        matching_policy(),
    )
    .unwrap();
    let historical_review_id = catalog.review_items.borrow()[0].id;
    catalog
        .set_review_decision(
            historical_review_id,
            ReviewDecision::Accept {
                release_edition_id: 402,
            },
        )
        .unwrap();
    catalog
        .set_review_status(historical_review_id, ReviewStatus::Applied)
        .unwrap();

    let current_runs = FakeRuns::new(run_with_request(request()));
    let imported = acquire_run_with_connector(
        &current_runs,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(connector.downloads.borrow().len(), 1);
    assert_eq!(*catalog.finalize_calls.borrow(), 1);
    let current_review = catalog
        .review_items
        .borrow()
        .iter()
        .find(|item| item.run_id == 7)
        .cloned()
        .expect("current run should materialize the inherited acceptance");
    assert_eq!(current_review.status, ReviewStatus::Applied);
    assert_eq!(
        current_review.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 402,
        })
    );
}

#[test]
fn accepting_review_requeues_the_staged_candidate_on_the_original_run() {
    let (candidate, library) = ambiguous_candidate_and_releases();
    let discovery_connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    let runs = FakeRuns::new(run_with_request(request()));

    let first_import = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &discovery_connector,
        7,
        matching_policy(),
    )
    .unwrap();
    assert!(first_import.is_empty());
    assert_eq!(runs.run.borrow().status, AcquisitionRunStatus::Completed);

    let review_item_id = catalog.review_items.borrow()[0].id;
    catalog
        .set_review_decision(
            review_item_id,
            ReviewDecision::Accept {
                release_edition_id: 402,
            },
        )
        .unwrap();
    let work_key = runs.work.borrow().keys().next().unwrap().clone();
    runs.requeue_completed_work(7, &work_key).unwrap();
    assert_eq!(runs.run.borrow().status, AcquisitionRunStatus::Running);

    let staged_connector = NoDiscoveryConnector {
        downloads: RefCell::new(Vec::new()),
    };
    let imported = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &staged_connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(staged_connector.downloads.borrow().len(), 1);
    assert_eq!(
        catalog.records.borrow()[0].existing_release_edition_id,
        Some(402)
    );
    assert_eq!(runs.run.borrow().status, AcquisitionRunStatus::Completed);
}

#[test]
fn accepted_review_is_not_downloaded_again_after_successful_ingestion() {
    let (candidate, library) = ambiguous_candidate_and_releases();
    let discovery_connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    let runs = FakeRuns::new(run_with_request(request()));

    acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &discovery_connector,
        7,
        matching_policy(),
    )
    .unwrap();
    let review_item_id = catalog.review_items.borrow()[0].id;
    catalog
        .set_review_decision(
            review_item_id,
            ReviewDecision::Accept {
                release_edition_id: 402,
            },
        )
        .unwrap();
    let work_key = runs.work.borrow().keys().next().unwrap().clone();
    runs.requeue_completed_work(7, &work_key).unwrap();

    let staged_connector = NoDiscoveryConnector {
        downloads: RefCell::new(Vec::new()),
    };
    let first_resume = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &staged_connector,
        7,
        matching_policy(),
    )
    .unwrap();
    assert_eq!(first_resume.len(), 1);
    assert_eq!(staged_connector.downloads.borrow().len(), 1);
    assert_eq!(*catalog.finalize_calls.borrow(), 1);

    let second_resume = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &staged_connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert!(second_resume.is_empty());
    assert_eq!(staged_connector.downloads.borrow().len(), 1);
}

#[test]
fn applied_review_decision_downloads_the_candidate_again_in_a_new_run() {
    let (candidate, library) = ambiguous_candidate_and_releases();
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    let first_runs = FakeRuns::new(run_with_request(request()));
    acquire_run_with_connector(
        &first_runs,
        &catalog,
        &FakeStore::default(),
        &FakeConnector {
            downloads: RefCell::new(Vec::new()),
            candidates: vec![candidate.clone()],
        },
        7,
        matching_policy(),
    )
    .unwrap();
    {
        let mut items = catalog.review_items.borrow_mut();
        items[0].decision = Some(ReviewDecision::Accept {
            release_edition_id: 402,
        });
        items[0].status = ReviewStatus::Applied;
    }

    let mut second_run = run_with_request(request());
    second_run.id = 8;
    let second_runs = FakeRuns::new(second_run);
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let imported = acquire_run_with_connector(
        &second_runs,
        &catalog,
        &FakeStore::default(),
        &connector,
        8,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(connector.downloads.borrow().len(), 1);
    assert_eq!(
        catalog
            .records
            .borrow()
            .last()
            .unwrap()
            .existing_release_edition_id,
        Some(402)
    );
}

#[test]
fn accepted_staged_review_resumes_when_discovery_is_unavailable() {
    let (candidate, library) = ambiguous_candidate_and_releases();
    let discovery_connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    let runs = FakeRuns::new(run_with_request(request()));

    acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &discovery_connector,
        7,
        matching_policy(),
    )
    .unwrap();
    let review_item_id = catalog.review_items.borrow()[0].id;
    catalog
        .set_review_decision(
            review_item_id,
            ReviewDecision::Accept {
                release_edition_id: 402,
            },
        )
        .unwrap();
    let work_key = runs.work.borrow().keys().next().unwrap().clone();
    runs.requeue_completed_work(7, &work_key).unwrap();

    let connector = FailingDiscoveryConnector {
        downloads: RefCell::new(Vec::new()),
    };
    let imported = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(connector.downloads.borrow().len(), 1);
}

#[test]
fn unrelated_queued_work_does_not_suppress_a_discovery_failure() {
    let (mut first_candidate, library) = ambiguous_candidate_and_releases();
    first_candidate.provider_candidate_id = Some("review-a".to_owned());
    let second_candidate = AssetCandidate {
        provider_candidate_id: Some("review-b".to_owned()),
        ..first_candidate.clone()
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    let runs = FakeRuns::new(run_with_request(request()));
    let discovery_connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![first_candidate, second_candidate],
    };

    acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &discovery_connector,
        7,
        matching_policy(),
    )
    .unwrap();
    assert_eq!(catalog.review_items.borrow().len(), 2);
    runs.run.borrow_mut().status = AcquisitionRunStatus::Running;
    runs.queue_work(7, "connector:unrelated-work".to_owned())
        .unwrap();

    let connector = FailingDiscoveryConnector {
        downloads: RefCell::new(Vec::new()),
    };
    let error = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap_err();

    assert!(error.to_string().contains("fixture discovery unavailable"));
    assert_eq!(runs.run.borrow().queued_work, 1);
}

#[test]
fn rediscovered_provider_candidate_replaces_the_staged_url() {
    let (mut candidate, library) = ambiguous_candidate_and_releases();
    candidate.provider_candidate_id = Some("provider-release-42".to_owned());
    let first_connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate.clone()],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    let runs = FakeRuns::new(run_with_request(request()));

    acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &first_connector,
        7,
        matching_policy(),
    )
    .unwrap();
    let review_item_id = catalog.review_items.borrow()[0].id;
    catalog
        .set_review_decision(
            review_item_id,
            ReviewDecision::Accept {
                release_edition_id: 402,
            },
        )
        .unwrap();
    let old_work_key = runs.work.borrow().keys().next().unwrap().clone();
    runs.requeue_completed_work(7, &old_work_key).unwrap();

    let rotated_url = "https://cdn.example.invalid/v2/rotated-cover.png";
    let rotated_candidate = AssetCandidate {
        source_url: rotated_url.to_owned(),
        original_filename: "rotated-cover.png".to_owned(),
        ..candidate
    };
    let rotated_connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![rotated_candidate],
    };

    let imported = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &rotated_connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(
        rotated_connector.downloads.borrow().as_slice(),
        [rotated_url]
    );
    assert_eq!(
        catalog.records.borrow()[0].source_location,
        rotated_url.to_owned()
    );
    assert_eq!(runs.work.borrow().len(), 1);
}

#[test]
fn accepted_review_decision_survives_mutable_provider_metadata_changes() {
    let (mut candidate, library) = ambiguous_candidate_and_releases();
    candidate.provider_candidate_id = Some("provider-release-42".to_owned());
    let first_connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate.clone()],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    create_review_then_set_decision(
        &catalog,
        &first_connector,
        ReviewDecision::Accept {
            release_edition_id: 402,
        },
    );

    let equivalent_candidate = AssetCandidate {
        game_title: format!(" {} ", candidate.game_title.to_uppercase()),
        region: candidate.region.to_lowercase(),
        source_asset_label: Some("rotated-label".to_owned()),
        source_url: "https://cdn.example.invalid/v2/rotated-cover.png".to_owned(),
        original_filename: "rotated-cover.png".to_owned(),
        ..candidate
    };
    let second_connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![equivalent_candidate],
    };
    let second_run = FakeRuns::new(run_with_request(request()));

    let imported = acquire_run_with_connector(
        &second_run,
        &catalog,
        &FakeStore::default(),
        &second_connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(second_connector.downloads.borrow().len(), 1);
    assert_eq!(catalog.review_items.borrow().len(), 1);
    assert_eq!(
        catalog.records.borrow()[0].existing_release_edition_id,
        Some(402)
    );
}

#[test]
fn rejected_review_decision_skips_the_same_candidate_identity() {
    let (candidate, library) = ambiguous_candidate_and_releases();
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    create_review_then_set_decision(&catalog, &connector, ReviewDecision::Reject);
    let second_run = FakeRuns::new(run_with_request(request()));

    let imported = acquire_run_with_connector(
        &second_run,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert!(imported.is_empty());
    assert!(connector.downloads.borrow().is_empty());
    assert!(catalog.records.borrow().is_empty());
    assert_eq!(catalog.review_items.borrow().len(), 1);
}

#[test]
fn deferred_review_decision_keeps_the_same_candidate_staged() {
    let (candidate, library) = ambiguous_candidate_and_releases();
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    create_review_then_set_decision(&catalog, &connector, ReviewDecision::Defer);
    let second_run = FakeRuns::new(run_with_request(request()));

    let imported = acquire_run_with_connector(
        &second_run,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert!(imported.is_empty());
    assert!(connector.downloads.borrow().is_empty());
    assert!(catalog.records.borrow().is_empty());
    assert_eq!(catalog.review_items.borrow().len(), 1);
    assert_eq!(
        catalog.review_items.borrow()[0].decision,
        Some(ReviewDecision::Defer)
    );
}

fn threshold_review_candidate_and_release() -> (AssetCandidate, LibraryEntry) {
    let candidate = AssetCandidate {
        provider_candidate_id: None,
        game_title: "Threshold Review Game".to_owned(),
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("libretro-thumbnails"),
        source_asset_label: Some("Named_Boxarts".to_owned()),
        source_url: "https://example.invalid/threshold-review.png".to_owned(),
        original_filename: "threshold-review.png".to_owned(),
    };
    let release = release_for_candidate(&candidate, 501);
    (candidate, release)
}

fn stricter_matching_policy() -> MatchingPolicy {
    MatchingPolicy {
        high_confidence_threshold: 90,
        medium_confidence_threshold: 50,
    }
}

#[test]
fn pending_review_is_re_evaluated_when_matching_policy_changes() {
    let (candidate, release) = threshold_review_candidate_and_release();
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library: vec![release],
    };
    let first_run = FakeRuns::new(run_with_request(request()));

    acquire_run_with_connector(
        &first_run,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        stricter_matching_policy(),
    )
    .unwrap();
    assert_eq!(catalog.review_items.borrow().len(), 1);
    assert_eq!(catalog.review_items.borrow()[0].decision, None);

    let second_run = FakeRuns::new(run_with_request(request()));
    let imported = acquire_run_with_connector(
        &second_run,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(connector.downloads.borrow().len(), 1);
    assert_eq!(
        catalog.review_items.borrow()[0].status,
        ReviewStatus::AutoResolved
    );
    assert_eq!(*catalog.finalize_calls.borrow(), 1);
}

#[test]
fn concurrent_execution_does_not_complete_work_owned_by_a_processing_review() {
    let (candidate, library) = ambiguous_candidate_and_releases();
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    let runs = FakeRuns::new(run_with_request(request()));

    acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();
    assert_eq!(
        catalog.review_items.borrow()[0].status,
        ReviewStatus::Pending
    );
    let work_key = runs.work.borrow().keys().next().unwrap().clone();
    runs.requeue_completed_work(7, &work_key).unwrap();
    catalog.review_items.borrow_mut()[0].status = ReviewStatus::Processing;

    let imported = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert!(imported.is_empty());
    let run = runs.run.borrow();
    assert_eq!(run.status, AcquisitionRunStatus::Running);
    assert_eq!(run.queued_work, 1);
    assert_eq!(run.completed_work, 0);
}

#[test]
fn persisted_pending_review_is_re_evaluated_without_connector_rediscovery() {
    let (candidate, release) = threshold_review_candidate_and_release();
    let discovery_connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library: vec![release],
    };
    let runs = FakeRuns::new(run_with_request(request()));

    acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &discovery_connector,
        7,
        stricter_matching_policy(),
    )
    .unwrap();
    assert_eq!(runs.run.borrow().status, AcquisitionRunStatus::Completed);

    let staged_connector = NoDiscoveryConnector {
        downloads: RefCell::new(Vec::new()),
    };
    let imported = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &staged_connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(staged_connector.downloads.borrow().len(), 1);
    assert_eq!(
        catalog.review_items.borrow()[0].status,
        ReviewStatus::AutoResolved
    );
}

#[test]
fn medium_review_refreshes_competing_matches_when_catalog_changes() {
    let (candidate, library) = ambiguous_candidate_and_releases();
    let discovery_connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate.clone()],
    };
    let mut catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library,
    };
    let runs = FakeRuns::new(run_with_request(request()));

    acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &discovery_connector,
        7,
        matching_policy(),
    )
    .unwrap();
    assert_eq!(catalog.review_items.borrow()[0].competing_matches.len(), 2);

    let new_release = LibraryEntry {
        game_id: 303,
        game_title: candidate.game_title.clone(),
        release_edition_id: 403,
        platform: candidate.platform.clone(),
        region: candidate.region.clone(),
        edition_name: "Limited".to_owned(),
        assertions: Vec::new(),
        assets: Vec::new(),
    };
    catalog.library.push(new_release);
    let staged_connector = NoDiscoveryConnector {
        downloads: RefCell::new(Vec::new()),
    };

    acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &staged_connector,
        7,
        matching_policy(),
    )
    .unwrap();

    let review_items = catalog.review_items.borrow();
    assert_eq!(review_items[0].status, ReviewStatus::Pending);
    assert!(
        review_items[0]
            .competing_matches
            .iter()
            .any(|candidate_match| candidate_match.release_edition_id == 403)
    );
}

#[test]
fn deferred_review_is_re_evaluated_when_matching_policy_changes() {
    let (candidate, release) = threshold_review_candidate_and_release();
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library: vec![release],
    };
    let first_run = FakeRuns::new(run_with_request(request()));

    acquire_run_with_connector(
        &first_run,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        stricter_matching_policy(),
    )
    .unwrap();
    catalog.review_items.borrow_mut()[0].decision = Some(ReviewDecision::Defer);

    let second_run = FakeRuns::new(run_with_request(request()));
    let imported = acquire_run_with_connector(
        &second_run,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(connector.downloads.borrow().len(), 1);
    assert_eq!(
        catalog.review_items.borrow()[0].status,
        ReviewStatus::AutoResolved
    );
}

#[test]
fn pending_review_is_superseded_when_re_evaluation_becomes_low_confidence() {
    let (candidate, release) = threshold_review_candidate_and_release();
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![candidate],
    };
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library: vec![release],
    };
    let first_run = FakeRuns::new(run_with_request(request()));

    acquire_run_with_connector(
        &first_run,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        stricter_matching_policy(),
    )
    .unwrap();

    let low_confidence_policy = MatchingPolicy {
        high_confidence_threshold: 90,
        medium_confidence_threshold: 81,
    };
    let second_run = FakeRuns::new(run_with_request(request()));
    let imported = acquire_run_with_connector(
        &second_run,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        low_confidence_policy,
    )
    .unwrap();

    assert!(imported.is_empty());
    assert!(connector.downloads.borrow().is_empty());
    assert_eq!(
        catalog.review_items.borrow()[0].status,
        ReviewStatus::Superseded
    );
    assert_eq!(*catalog.finalize_calls.borrow(), 1);
}

#[test]
fn packaging_selector_acquires_the_supported_box_front() {
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::Packaging],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();
    let runs = FakeRuns::new(run_with_request(request));
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: Vec::new(),
    };
    let store = FakeStore::default();
    let catalog = FakeCatalog::default();

    let imported =
        acquire_run_with_connector(&runs, &catalog, &store, &connector, 7, matching_policy())
            .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(connector.downloads.borrow().len(), 1);
    assert_eq!(catalog.records.borrow()[0].asset_type, AssetType::BoxFront);
}

#[test]
fn pause_or_cancel_during_the_last_download_preserves_the_requested_run_status() {
    for status in [
        AcquisitionRunStatus::Paused,
        AcquisitionRunStatus::Cancelled,
    ] {
        let runs = FakeRuns::new(run_with_request(request())).with_status_after_complete(status);
        let connector = FakeConnector {
            downloads: RefCell::new(Vec::new()),
            candidates: Vec::new(),
        };

        let imported = acquire_run_with_connector(
            &runs,
            &FakeCatalog::default(),
            &FakeStore::default(),
            &connector,
            7,
            matching_policy(),
        )
        .unwrap();

        assert_eq!(imported.len(), 1);
        assert_eq!(runs.run.borrow().status, status);
        assert_eq!(runs.run.borrow().queued_work, 0);
    }
}

#[test]
fn pause_or_cancel_winning_the_final_completion_race_does_not_fail_execution() {
    for status in [
        AcquisitionRunStatus::Paused,
        AcquisitionRunStatus::Cancelled,
    ] {
        let runs =
            FakeRuns::new(run_with_request(request())).with_status_before_complete_cas(status);
        let connector = FakeConnector {
            downloads: RefCell::new(Vec::new()),
            candidates: Vec::new(),
        };

        let imported = acquire_run_with_connector(
            &runs,
            &FakeCatalog::default(),
            &FakeStore::default(),
            &connector,
            7,
            matching_policy(),
        )
        .unwrap();

        assert_eq!(imported.len(), 1);
        assert_eq!(runs.run.borrow().status, status);
        assert_eq!(runs.run.borrow().queued_work, 0);
    }
}

fn run_with_request(request: AcquisitionRequest) -> AcquisitionRun {
    AcquisitionRun {
        id: 7,
        request,
        status: AcquisitionRunStatus::Running,
        queued_work: 0,
        completed_work: 0,
    }
}

fn execute_error(request: AcquisitionRequest) -> game_media_vault_application::ApplicationError {
    let runs = FakeRuns::new(run_with_request(request));
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: Vec::new(),
    };
    acquire_run_with_connector(
        &runs,
        &FakeCatalog::default(),
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap_err()
}

#[test]
fn rejects_multi_source_execution_until_a_multi_source_plan_exists() {
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec![
            "libretro-thumbnails".to_owned(),
            "screenscraper".to_owned(),
        ]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    let error = execute_error(request);

    assert!(matches!(
        error,
        game_media_vault_application::ApplicationError::UnsupportedConnectorPlan { .. }
    ));
}

#[test]
fn rejects_quality_and_retention_behavior_reserved_for_the_quality_slice() {
    let quality_request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: Some(QualityRequirements {
            accepted_mime_types: vec!["image/jpeg".to_owned()],
            ..QualityRequirements::default()
        }),
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();
    let retention_request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepBestPerType,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    assert!(matches!(
        execute_error(quality_request),
        game_media_vault_application::ApplicationError::UnsupportedConnectorPlan { .. }
    ));
    assert!(matches!(
        execute_error(retention_request),
        game_media_vault_application::ApplicationError::UnsupportedConnectorPlan { .. }
    ));
}

#[test]
fn rejects_acquisition_limits_until_the_scheduler_slice_can_enforce_them() {
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits {
            max_downloads: Some(1),
            ..AcquisitionLimits::default()
        },
    })
    .unwrap();

    assert!(matches!(
        execute_error(request),
        game_media_vault_application::ApplicationError::UnsupportedConnectorPlan { .. }
    ));
}

#[test]
fn rejects_asset_types_not_declared_by_the_connector() {
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::Screenshot],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    assert!(matches!(
        execute_error(request),
        game_media_vault_application::ApplicationError::UnsupportedConnectorPlan { .. }
    ));
}

#[test]
fn distinct_candidates_that_share_a_source_url_keep_distinct_work_items() {
    let first = AssetCandidate {
        provider_candidate_id: None,
        game_title: "A:B".to_owned(),
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("libretro-thumbnails"),
        source_asset_label: Some("Named_Boxarts".to_owned()),
        source_url: "https://example.invalid/Named_Boxarts/A_B.png".to_owned(),
        original_filename: "A_B.png".to_owned(),
    };
    let second = AssetCandidate {
        provider_candidate_id: None,
        game_title: "A?B".to_owned(),
        ..first.clone()
    };
    let runs = FakeRuns::new(run_with_request(request()));
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library: vec![
            release_for_candidate(&first, 81),
            release_for_candidate(&second, 82),
        ],
    };
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![first, second],
    };

    let imported = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 2);
    assert_eq!(catalog.records.borrow().len(), 2);
    assert_eq!(runs.run.borrow().completed_work, 2);
}

#[test]
fn distinct_provider_candidates_with_identical_metadata_keep_distinct_work_items() {
    let first = AssetCandidate {
        provider_candidate_id: Some("provider-release-1".to_owned()),
        game_title: "Shared Game".to_owned(),
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("libretro-thumbnails"),
        source_asset_label: Some("Named_Boxarts".to_owned()),
        source_url: "https://example.invalid/shared.png".to_owned(),
        original_filename: "shared.png".to_owned(),
    };
    let second = AssetCandidate {
        provider_candidate_id: Some("provider-release-2".to_owned()),
        ..first.clone()
    };
    let runs = FakeRuns::new(run_with_request(request()));
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library: vec![release_for_candidate(&first, 83)],
    };
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![first, second],
    };

    let imported = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 2);
    assert_eq!(connector.downloads.borrow().len(), 2);
    assert_eq!(catalog.records.borrow().len(), 2);
    assert_eq!(runs.run.borrow().completed_work, 2);
}

#[test]
fn reviews_with_a_colliding_source_url_keep_independent_decisions() {
    let first = AssetCandidate {
        provider_candidate_id: Some("provider:A:B".to_owned()),
        game_title: "A:B".to_owned(),
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Collector".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("libretro-thumbnails"),
        source_asset_label: Some("Named_Boxarts".to_owned()),
        source_url: "https://example.invalid/Named_Boxarts/A_B.png".to_owned(),
        original_filename: "A_B.png".to_owned(),
    };
    let second = AssetCandidate {
        provider_candidate_id: Some("provider:A?B".to_owned()),
        game_title: "A?B".to_owned(),
        ..first.clone()
    };
    let release = |candidate: &AssetCandidate, game_id, release_edition_id, edition_name: &str| {
        LibraryEntry {
            game_id,
            game_title: candidate.game_title.clone(),
            release_edition_id,
            platform: candidate.platform.clone(),
            region: candidate.region.clone(),
            edition_name: edition_name.to_owned(),
            assertions: Vec::new(),
            assets: Vec::new(),
        }
    };
    let runs = FakeRuns::new(run_with_request(request()));
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library: vec![
            release(&first, 101, 201, "Standard"),
            release(&first, 101, 202, "Deluxe"),
            release(&second, 102, 203, "Standard"),
            release(&second, 102, 204, "Deluxe"),
        ],
    };
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![first, second],
    };

    acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    let review_items = catalog.review_items.borrow().clone();
    assert_eq!(review_items.len(), 2);
    assert_ne!(
        review_items[0].candidate_identity,
        review_items[1].candidate_identity
    );
    catalog
        .set_review_decision(
            review_items[0].id,
            ReviewDecision::Accept {
                release_edition_id: review_items[0].competing_matches[0].release_edition_id,
            },
        )
        .unwrap();
    assert_eq!(catalog.review_items.borrow()[1].decision, None);
}

#[test]
fn candidate_identity_fields_cannot_collide_through_work_key_delimiters() {
    let first = AssetCandidate {
        provider_candidate_id: None,
        game_title: "C".to_owned(),
        platform: "A:B".to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("libretro-thumbnails"),
        source_asset_label: Some("Named_Boxarts".to_owned()),
        source_url: "https://example.invalid/shared.png".to_owned(),
        original_filename: "shared.png".to_owned(),
    };
    let second = AssetCandidate {
        provider_candidate_id: None,
        game_title: "B:C".to_owned(),
        platform: "A".to_owned(),
        ..first.clone()
    };
    let runs = FakeRuns::new(run_with_request(request()));
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        review_items: RefCell::new(Vec::new()),
        finalize_calls: RefCell::new(0),
        library: vec![
            release_for_candidate(&first, 91),
            release_for_candidate(&second, 92),
        ],
    };
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![first, second],
    };

    let imported = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 2);
    assert_eq!(catalog.records.borrow().len(), 2);
    assert_eq!(runs.run.borrow().completed_work, 2);
}
