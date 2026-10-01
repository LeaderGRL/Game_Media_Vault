//! In-memory port doubles shared by the application tests.
#![allow(dead_code)]

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
};

use game_media_vault_application::{
    CatalogPort, ConnectorPort, ObjectStorePort, ParkedReview, PortError, ReviewRepositoryPort,
    RunRepositoryPort,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRun,
    AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate, AssetType, AssetTypeSelector,
    ConnectorCapabilities, GameSelection, ImportedAsset, LibraryEntry, MatchingPolicy,
    NewReviewItem, PersistAsset, RetentionPolicy, ReviewDecision, ReviewItem, ReviewStatus,
    SourceId, SourceSelection, StoredObject,
};

pub const SOURCE_ID: &str = "libretro-thumbnails";

pub fn matching_policy() -> MatchingPolicy {
    MatchingPolicy {
        high_confidence_threshold: 80,
        medium_confidence_threshold: 50,
    }
}

/// Turns the 80-point threshold candidate into a medium-confidence match.
pub fn stricter_matching_policy() -> MatchingPolicy {
    MatchingPolicy {
        high_confidence_threshold: 90,
        medium_confidence_threshold: 50,
    }
}

/// Turns the 80-point threshold candidate into a low-confidence match.
pub fn dismissive_matching_policy() -> MatchingPolicy {
    MatchingPolicy {
        high_confidence_threshold: 100,
        medium_confidence_threshold: 95,
    }
}

pub fn request_draft() -> AcquisitionRequestDraft {
    AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec![SOURCE_ID.to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    }
}

pub fn request() -> AcquisitionRequest {
    AcquisitionRequest::try_from_draft(request_draft()).unwrap()
}

/// A candidate that matches `release_for(&candidate, ..)` with 100 points.
pub fn candidate(title: &str) -> AssetCandidate {
    AssetCandidate {
        provider_candidate_id: None,
        game_title: title.to_owned(),
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Standard".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from(SOURCE_ID),
        source_asset_label: Some("Named_Boxarts".to_owned()),
        source_url: format!("https://example.invalid/Named_Boxarts/{title}.png"),
        original_filename: format!("{title}.png"),
    }
}

pub fn release_for(candidate: &AssetCandidate, release_edition_id: i64) -> LibraryEntry {
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

/// A candidate scoring 90 points against two editions: always a medium-confidence match.
pub fn ambiguous_candidate_and_releases() -> (AssetCandidate, Vec<LibraryEntry>) {
    let candidate = AssetCandidate {
        edition_name: "Collector".to_owned(),
        ..candidate("Review Game")
    };
    let standard = LibraryEntry {
        edition_name: "Standard".to_owned(),
        ..release_for(&candidate, 401)
    };
    let deluxe = LibraryEntry {
        edition_name: "Deluxe".to_owned(),
        ..release_for(&candidate, 402)
    };
    (candidate, vec![standard, deluxe])
}

/// A candidate scoring exactly 80 points against its only release.
pub fn threshold_candidate_and_release() -> (AssetCandidate, LibraryEntry) {
    let candidate = AssetCandidate {
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        ..candidate("Threshold Review Game")
    };
    let release = LibraryEntry {
        region: "USA".to_owned(),
        edition_name: "Standard".to_owned(),
        ..release_for(&candidate, 501)
    };
    (candidate, release)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkState {
    Queued,
    Parked(i64),
    Done,
}

#[derive(Debug, Clone)]
pub struct FakeWork {
    pub item: AcquisitionWorkItem,
    pub state: WorkState,
}

#[derive(Debug, Clone)]
pub struct FakeRun {
    pub request: AcquisitionRequest,
    pub status: AcquisitionRunStatus,
    pub discovered: BTreeSet<String>,
    pub work: Vec<FakeWork>,
}

/// In-memory vault implementing the run, review and catalog ports with their documented
/// atomic semantics.
#[derive(Default)]
pub struct FakeVault {
    pub runs: RefCell<BTreeMap<i64, FakeRun>>,
    pub review_items: RefCell<Vec<ReviewItem>>,
    pub records: RefCell<Vec<PersistAsset>>,
    pub library: RefCell<Vec<LibraryEntry>>,
    /// Simulates a human decision committed right before the next automatic review write.
    pub human_decision_before_next_write: RefCell<Option<ReviewDecision>>,
    /// Simulates a pause or cancellation landing right after the next completed work item.
    pub status_after_next_completion: RefCell<Option<AcquisitionRunStatus>>,
    /// Simulates another run opening a Review Item for the candidate right before the next
    /// automatic link is persisted.
    pub review_opened_before_next_auto_link: RefCell<Option<NewReviewItem>>,
    /// Release Edition each acquisition candidate is currently linked to.
    pub candidate_links: RefCell<BTreeMap<String, i64>>,
}

impl FakeVault {
    pub fn with_library(library: Vec<LibraryEntry>) -> Self {
        Self {
            library: RefCell::new(library),
            ..Self::default()
        }
    }

    pub fn start_run(&self) -> i64 {
        self.create_run(request()).unwrap().id
    }

    pub fn run(&self, run_id: i64) -> AcquisitionRun {
        self.get_run(run_id).unwrap().unwrap()
    }

    pub fn work_states(&self, run_id: i64) -> Vec<WorkState> {
        self.runs.borrow()[&run_id]
            .work
            .iter()
            .map(|work| work.state)
            .collect()
    }

    pub fn review_item(&self, index: usize) -> ReviewItem {
        self.review_items.borrow()[index].clone()
    }

    fn apply_pending_human_decision(&self, review_item_id: i64) {
        if let Some(decision) = self.human_decision_before_next_write.borrow_mut().take() {
            self.decide_review_item(review_item_id, decision).unwrap();
        }
    }

    fn move_parked_work(&self, review_item_id: i64, target: WorkState, include_cancelled: bool) {
        for run in self.runs.borrow_mut().values_mut() {
            if run.status == AcquisitionRunStatus::Cancelled && !include_cancelled {
                continue;
            }
            let mut moved = false;
            for work in &mut run.work {
                if work.state == WorkState::Parked(review_item_id) {
                    work.state = target;
                    moved = true;
                }
            }
            if moved && target == WorkState::Queued && run.status == AcquisitionRunStatus::Completed
            {
                run.status = AcquisitionRunStatus::Running;
            }
        }
    }
}

impl RunRepositoryPort for FakeVault {
    fn create_run(&self, request: AcquisitionRequest) -> Result<AcquisitionRun, PortError> {
        let mut runs = self.runs.borrow_mut();
        let id = runs.keys().next_back().copied().unwrap_or(6) + 1;
        runs.insert(
            id,
            FakeRun {
                request,
                status: AcquisitionRunStatus::Running,
                discovered: BTreeSet::new(),
                work: Vec::new(),
            },
        );
        drop(runs);
        Ok(self.run(id))
    }

    fn get_run(&self, run_id: i64) -> Result<Option<AcquisitionRun>, PortError> {
        Ok(self.runs.borrow().get(&run_id).map(|run| {
            let count = |predicate: fn(&WorkState) -> bool| {
                run.work
                    .iter()
                    .filter(|work| predicate(&work.state))
                    .count() as u64
            };
            AcquisitionRun {
                id: run_id,
                request: run.request.clone(),
                status: run.status,
                queued_work: count(|state| *state == WorkState::Queued),
                awaiting_review_work: count(|state| matches!(state, WorkState::Parked(_))),
                completed_work: count(|state| *state == WorkState::Done),
            }
        }))
    }

    fn list_runs(&self) -> Result<Vec<AcquisitionRun>, PortError> {
        let ids = self.runs.borrow().keys().copied().collect::<Vec<_>>();
        Ok(ids.into_iter().map(|id| self.run(id)).collect())
    }

    fn compare_and_set_run_status(
        &self,
        run_id: i64,
        expected: AcquisitionRunStatus,
        target: AcquisitionRunStatus,
    ) -> Result<bool, PortError> {
        let mut runs = self.runs.borrow_mut();
        let run = runs.get_mut(&run_id).unwrap();
        let has_queued_work = run.work.iter().any(|work| work.state == WorkState::Queued);
        if run.status != expected || (target == AcquisitionRunStatus::Completed && has_queued_work)
        {
            return Ok(false);
        }
        run.status = target;
        Ok(true)
    }

    fn has_discovered(&self, run_id: i64, source_id: &str) -> Result<bool, PortError> {
        Ok(self.runs.borrow()[&run_id].discovered.contains(source_id))
    }

    fn record_discovery(
        &self,
        run_id: i64,
        source_id: &str,
        work: &[AcquisitionWorkItem],
    ) -> Result<(), PortError> {
        let mut runs = self.runs.borrow_mut();
        let run = runs.get_mut(&run_id).unwrap();
        if matches!(
            run.status,
            AcquisitionRunStatus::Cancelled | AcquisitionRunStatus::Completed
        ) {
            return Err(PortError("run does not accept new work".to_owned()));
        }
        for item in work {
            if !run
                .work
                .iter()
                .any(|existing| existing.item.key == item.key)
            {
                run.work.push(FakeWork {
                    item: item.clone(),
                    state: WorkState::Queued,
                });
            }
        }
        run.discovered.insert(source_id.to_owned());
        Ok(())
    }

    fn next_queued_work(&self, run_id: i64) -> Result<Option<AcquisitionWorkItem>, PortError> {
        let runs = self.runs.borrow();
        let run = &runs[&run_id];
        if run.status != AcquisitionRunStatus::Running {
            return Ok(None);
        }
        Ok(run
            .work
            .iter()
            .find(|work| work.state == WorkState::Queued)
            .map(|work| work.item.clone()))
    }

    fn complete_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError> {
        let mut runs = self.runs.borrow_mut();
        let run = runs.get_mut(&run_id).unwrap();
        let work = run
            .work
            .iter_mut()
            .find(|work| work.item.key == work_key)
            .ok_or_else(|| PortError(format!("work {work_key} does not exist")))?;
        work.state = WorkState::Done;
        if let Some(status) = self.status_after_next_completion.borrow_mut().take() {
            run.status = status;
        }
        Ok(())
    }
}

impl ReviewRepositoryPort for FakeVault {
    fn list_review_items(&self) -> Result<Vec<ReviewItem>, PortError> {
        Ok(self.review_items.borrow().clone())
    }

    fn get_review_item(&self, review_item_id: i64) -> Result<Option<ReviewItem>, PortError> {
        Ok(self
            .review_items
            .borrow()
            .iter()
            .find(|item| item.id == review_item_id)
            .cloned())
    }

    fn find_review_item(&self, candidate_identity: &str) -> Result<Option<ReviewItem>, PortError> {
        Ok(self
            .review_items
            .borrow()
            .iter()
            .find(|item| item.candidate_identity == candidate_identity)
            .cloned())
    }

    fn park_work_for_review(
        &self,
        run_id: i64,
        work_key: &str,
        new_item: NewReviewItem,
    ) -> Result<ParkedReview, PortError> {
        if let Some(existing) = self.find_review_item(&new_item.candidate_identity)? {
            self.apply_pending_human_decision(existing.id);
        }
        let review_item = {
            let mut review_items = self.review_items.borrow_mut();
            let next_id = review_items.len() as i64 + 1;
            match review_items
                .iter_mut()
                .find(|item| item.candidate_identity == new_item.candidate_identity)
            {
                Some(item)
                    if matches!(item.status, ReviewStatus::Accepted | ReviewStatus::Rejected) =>
                {
                    return Ok(ParkedReview::AlreadyDecided(item.clone()));
                }
                Some(item) => {
                    if !item.status.is_undecided() {
                        item.status = ReviewStatus::Pending;
                        item.decision = None;
                    }
                    item.candidate = new_item.candidate;
                    item.competing_matches = new_item.competing_matches;
                    item.clone()
                }
                None => {
                    let item = ReviewItem {
                        id: next_id,
                        candidate_identity: new_item.candidate_identity,
                        candidate: new_item.candidate,
                        competing_matches: new_item.competing_matches,
                        decision: None,
                        status: ReviewStatus::Pending,
                    };
                    review_items.push(item.clone());
                    item
                }
            }
        };
        let mut runs = self.runs.borrow_mut();
        let work = runs
            .get_mut(&run_id)
            .unwrap()
            .work
            .iter_mut()
            .find(|work| {
                work.item.key == work_key
                    && (work.state == WorkState::Queued
                        || work.state == WorkState::Parked(review_item.id))
            })
            .ok_or_else(|| PortError(format!("queued work {work_key} does not exist")))?;
        work.state = WorkState::Parked(review_item.id);
        Ok(ParkedReview::Parked(review_item))
    }

    fn close_review_item(
        &self,
        review_item_id: i64,
        status: ReviewStatus,
    ) -> Result<bool, PortError> {
        assert!(matches!(
            status,
            ReviewStatus::AutoResolved | ReviewStatus::Superseded
        ));
        self.apply_pending_human_decision(review_item_id);
        {
            let mut review_items = self.review_items.borrow_mut();
            let item = review_items
                .iter_mut()
                .find(|item| item.id == review_item_id)
                .unwrap();
            if !item.status.is_undecided() {
                return Ok(false);
            }
            item.status = status;
            item.decision = None;
        }
        self.move_parked_work(review_item_id, WorkState::Done, true);
        Ok(true)
    }

    fn decide_review_item(
        &self,
        review_item_id: i64,
        decision: ReviewDecision,
    ) -> Result<Option<ReviewItem>, PortError> {
        let decided = {
            let mut review_items = self.review_items.borrow_mut();
            let Some(item) = review_items
                .iter_mut()
                .find(|item| item.id == review_item_id)
            else {
                return Ok(None);
            };
            if !item.status.is_undecided() {
                return Ok(None);
            }
            item.status = match decision {
                ReviewDecision::Accept { .. } => ReviewStatus::Accepted,
                ReviewDecision::Reject => ReviewStatus::Rejected,
                ReviewDecision::Defer => ReviewStatus::Deferred,
            };
            item.decision = Some(decision.clone());
            item.clone()
        };
        let mut links = self.candidate_links.borrow_mut();
        match decision {
            ReviewDecision::Accept { release_edition_id } => {
                if links.get(&decided.candidate_identity) != Some(&release_edition_id) {
                    links.remove(&decided.candidate_identity);
                }
                drop(links);
                self.move_parked_work(review_item_id, WorkState::Queued, false)
            }
            ReviewDecision::Reject => {
                links.remove(&decided.candidate_identity);
                drop(links);
                self.move_parked_work(review_item_id, WorkState::Done, true)
            }
            ReviewDecision::Defer => {}
        }
        Ok(Some(decided))
    }

    fn persist_candidate_asset(
        &self,
        candidate_identity: &str,
        record: PersistAsset,
    ) -> Result<Option<ImportedAsset>, PortError> {
        if let Some(new_item) = self.review_opened_before_next_auto_link.borrow_mut().take() {
            let id = self.review_items.borrow().len() as i64 + 1;
            self.review_items.borrow_mut().push(ReviewItem {
                id,
                candidate_identity: new_item.candidate_identity,
                candidate: new_item.candidate,
                competing_matches: new_item.competing_matches,
                decision: None,
                status: ReviewStatus::Pending,
            });
        }
        if let Some(existing) = self.find_review_item(candidate_identity)? {
            self.apply_pending_human_decision(existing.id);
            let current = self.get_review_item(existing.id)?.unwrap();
            match (current.status, current.decision) {
                (ReviewStatus::Rejected, _) => return Ok(None),
                (ReviewStatus::Accepted, Some(ReviewDecision::Accept { release_edition_id }))
                    if record.existing_release_edition_id != Some(release_edition_id) =>
                {
                    return Ok(None);
                }
                (ReviewStatus::Pending | ReviewStatus::Deferred, _) => {
                    self.close_review_item(existing.id, ReviewStatus::AutoResolved)?;
                }
                _ => {}
            }
        }
        let imported = self.persist_asset(record)?;
        self.candidate_links
            .borrow_mut()
            .insert(candidate_identity.to_owned(), imported.release_edition_id);
        Ok(Some(imported))
    }
}

impl CatalogPort for FakeVault {
    fn persist_asset(&self, record: PersistAsset) -> Result<ImportedAsset, PortError> {
        self.records.borrow_mut().push(record.clone());
        Ok(ImportedAsset {
            game_id: record.existing_game_id.unwrap_or(1),
            release_edition_id: record.existing_release_edition_id.unwrap_or(1),
            asset_id: self.records.borrow().len() as i64,
            object_hash: record.object_hash,
            byte_len: record.byte_len,
        })
    }

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError> {
        Ok(self.library.borrow().clone())
    }
}

#[derive(Default)]
pub struct FakeStore {
    pub stored: RefCell<Vec<Vec<u8>>>,
}

impl ObjectStorePort for FakeStore {
    fn store_original(&self, reader: &mut dyn Read) -> Result<StoredObject, PortError> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|error| PortError(error.to_string()))?;
        let stored = StoredObject {
            hash: format!("hash-of-{}-bytes", bytes.len()),
            byte_len: bytes.len() as u64,
        };
        self.stored.borrow_mut().push(bytes);
        Ok(stored)
    }
}

pub struct FakeConnector {
    pub candidates: Vec<AssetCandidate>,
    pub discovery_fails: bool,
    pub failing_downloads: BTreeSet<String>,
    pub discover_calls: RefCell<u32>,
    pub downloads: RefCell<Vec<String>>,
}

impl FakeConnector {
    pub fn new(candidates: Vec<AssetCandidate>) -> Self {
        Self {
            candidates,
            discovery_fails: false,
            failing_downloads: BTreeSet::new(),
            discover_calls: RefCell::new(0),
            downloads: RefCell::new(Vec::new()),
        }
    }
}

impl ConnectorPort for FakeConnector {
    fn source_id(&self) -> &'static str {
        SOURCE_ID
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::BoxFront],
            direct_media_download: true,
        }
    }

    fn discover(&self, _request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        *self.discover_calls.borrow_mut() += 1;
        if self.discovery_fails {
            return Err(PortError("fixture discovery unavailable".to_owned()));
        }
        Ok(self.candidates.clone())
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        if self.failing_downloads.contains(&candidate.source_url) {
            return Err(PortError("fixture download failed".to_owned()));
        }
        self.downloads
            .borrow_mut()
            .push(candidate.source_url.clone());
        Ok(Box::new(Cursor::new(
            format!("bytes of {}", candidate.original_filename).into_bytes(),
        )))
    }
}
