//! In-memory port doubles shared by the application tests.
#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
    sync::{Mutex, MutexGuard, mpsc},
};

use game_media_vault_application::{
    API_KEY_FIELD, ApiKey, CandidateAssetOutcome, CatalogPort, ConnectorPort, CredentialField,
    CredentialStorePort, MachineSettingsPort, ObjectStorePort, ParkedReview, PortError,
    ReviewDecisionOutcome, ReviewRepositoryPort, RunRepositoryPort,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRun,
    AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate, AssetType, AssetTypeSelector,
    ConnectorCapabilities, GameSelection, ImportedAsset, LibraryAsset, LibraryEntry,
    MatchingPolicy, MediaInfo, NewReviewItem, Outranked, PersistAsset, QualityShortfall,
    RetentionPolicy, ReviewDecision, ReviewItem, ReviewStatus, SourceDiscovery, SourceFailure,
    SourceFailureStage, SourceId, SourceSelection, StoredObject, outranked_by,
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
    pub shortfalls: Vec<QualityShortfall>,
    pub outranked: Option<Outranked>,
    pub unavailable: Option<String>,
    /// Whether the work settled without keeping its candidate.
    pub dismissed: bool,
}

#[derive(Debug, Clone)]
pub struct FakeRun {
    pub request: AcquisitionRequest,
    pub planned_sources: Vec<String>,
    pub status: AcquisitionRunStatus,
    pub discovered: BTreeSet<String>,
    /// Requested games the batches of each discovery not complete yet recorded.
    pub discovered_games: BTreeMap<String, usize>,
    pub work: Vec<FakeWork>,
}

/// In-memory vault implementing the run, review and catalog ports with their documented
/// atomic semantics.
#[derive(Default)]
pub struct FakeVault {
    pub runs: Shared<BTreeMap<i64, FakeRun>>,
    pub review_items: Shared<Vec<ReviewItem>>,
    pub records: Shared<Vec<PersistAsset>>,
    pub library: Shared<Vec<LibraryEntry>>,
    /// Simulates Release Editions imported by another process right after the next library
    /// listing.
    pub library_added_after_next_listing: Shared<Vec<LibraryEntry>>,
    /// Simulates a human decision committed right before the next automatic review write.
    pub human_decision_before_next_write: Shared<Option<ReviewDecision>>,
    /// Simulates a human decision on the first Review Item committed right before the run
    /// completes.
    pub decision_before_completion: Shared<Option<ReviewDecision>>,
    /// Simulates a human decision on the first Review Item committed right after the next run
    /// read.
    pub decision_after_next_run_read: Shared<Option<ReviewDecision>>,
    /// Simulates a pause or cancellation landing while the next discovery runs.
    pub status_before_next_discovery: Shared<Option<AcquisitionRunStatus>>,
    /// Simulates a status change, such as a resume, landing right after the next run read.
    pub status_after_next_run_read: Shared<Option<AcquisitionRunStatus>>,
    /// Simulates a pause or cancellation landing right after the next completed work item.
    pub status_after_next_completion: Shared<Option<AcquisitionRunStatus>>,
    /// Simulates another run opening a Review Item for the candidate right before the next
    /// automatic write: an auto-link, a supersession or a work completion.
    pub review_opened_before_next_write: Shared<Option<NewReviewItem>>,
    /// Release Edition each acquisition candidate is currently linked to.
    pub candidate_links: Shared<BTreeMap<String, i64>>,
    /// Source failures executions recorded, in recording order.
    pub source_failures: Shared<Vec<SourceFailure>>,
    /// Simulates a pause landing while a download thread reads the run status: the next read
    /// waits this long, then finds the run paused.
    pub pause_during_next_status_read: Shared<Option<std::time::Duration>>,
    /// Simulates a pause the next read of the run status sees, resumed right after it.
    pub paused_at_next_status_read: Shared<bool>,
    /// The runs an execution holds.
    pub executing: Shared<BTreeSet<i64>>,
    /// Simulates a human rejecting the first Review Item right before the Nth next read of a
    /// Review Item, counting from one.
    pub rejection_before_review_read: Shared<Option<usize>>,
    /// Simulates a work item leaving the queue right before the Nth next read of the queue,
    /// counting from one.
    pub work_leaving_before_queue_read: Shared<Option<(usize, WorkLeaving)>>,
    /// Holds the Nth next read of the queue, counting from one, until told to go on.
    pub queue_read_gate: Shared<Option<(usize, mpsc::Receiver<()>)>>,
}

/// A work item leaving the queue while an execution runs.
pub struct WorkLeaving {
    pub key: String,
    /// Whether another execution of the run parks it on its Review Item, which a human accepts
    /// right after the read, requeuing it; otherwise another execution completes it.
    pub requeued: bool,
}

/// Counts down the reads of `hook` and takes its value on the Nth one.
fn countdown<T>(hook: &Shared<Option<(usize, T)>>) -> Option<T> {
    let mut hook = hook.borrow_mut();
    match hook.take() {
        Some((1, value)) => Some(value),
        Some((reads, value)) => {
            *hook = Some((reads - 1, value));
            None
        }
        None => None,
    }
}

impl FakeVault {
    pub fn with_library(library: Vec<LibraryEntry>) -> Self {
        Self {
            library: Shared::new(library),
            ..Self::default()
        }
    }

    fn open_scheduled_review(&self) {
        if let Some(new_item) = self.review_opened_before_next_write.borrow_mut().take() {
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
    }

    pub fn start_run(&self) -> i64 {
        self.create_run(request(), vec![SOURCE_ID.to_owned()])
            .unwrap()
            .id
    }

    pub fn run(&self, run_id: i64) -> AcquisitionRun {
        self.get_run(run_id).unwrap().unwrap()
    }

    /// Why each unavailable work item of the run could not be acquired, in work order.
    pub fn unavailable_reasons(&self, run_id: i64) -> Vec<String> {
        self.runs.borrow()[&run_id]
            .work
            .iter()
            .filter_map(|work| work.unavailable.clone())
            .collect()
    }

    /// Every quality shortfall recorded on the run's work, in work order.
    pub fn quality_shortfalls(&self, run_id: i64) -> Vec<QualityShortfall> {
        self.runs.borrow()[&run_id]
            .work
            .iter()
            .flat_map(|work| work.shortfalls.clone())
            .collect()
    }

    /// Why each outranked work item of the run was not linked, in work order.
    pub fn outranked(&self, run_id: i64) -> Vec<Outranked> {
        self.runs.borrow()[&run_id]
            .work
            .iter()
            .filter_map(|work| work.outranked.clone())
            .collect()
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

    /// Closes an undecided or automatically closed item with an automatic outcome.
    fn close_automatically(&self, review_item_id: i64, status: ReviewStatus) {
        {
            let mut review_items = self.review_items.borrow_mut();
            let item = review_items
                .iter_mut()
                .find(|item| item.id == review_item_id)
                .unwrap();
            if item.status.is_undecided() {
                item.status = status;
                item.decision = None;
            }
        }
        // A superseded candidate is dismissed; an automatically resolved one was kept.
        let dismissed = status == ReviewStatus::Superseded;
        self.move_parked_work(review_item_id, WorkState::Done, true, dismissed);
    }

    fn move_parked_work(
        &self,
        review_item_id: i64,
        target: WorkState,
        include_cancelled: bool,
        dismissed: bool,
    ) {
        for run in self.runs.borrow_mut().values_mut() {
            if run.status == AcquisitionRunStatus::Cancelled && !include_cancelled {
                continue;
            }
            let mut moved = false;
            for work in &mut run.work {
                if work.state == WorkState::Parked(review_item_id) {
                    work.state = target;
                    work.dismissed = dismissed;
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
    fn create_run(
        &self,
        request: AcquisitionRequest,
        planned_sources: Vec<String>,
    ) -> Result<AcquisitionRun, PortError> {
        let mut runs = self.runs.borrow_mut();
        let id = runs.keys().next_back().copied().unwrap_or(6) + 1;
        runs.insert(
            id,
            FakeRun {
                request,
                planned_sources,
                status: AcquisitionRunStatus::Running,
                discovered: BTreeSet::new(),
                discovered_games: BTreeMap::new(),
                work: Vec::new(),
            },
        );
        drop(runs);
        Ok(self.run(id))
    }

    fn get_run(&self, run_id: i64) -> Result<Option<AcquisitionRun>, PortError> {
        let run = self.runs.borrow().get(&run_id).map(|run| {
            let count = |predicate: fn(&WorkState) -> bool| {
                run.work
                    .iter()
                    .filter(|work| predicate(&work.state))
                    .count() as u64
            };
            AcquisitionRun {
                id: run_id,
                request: run.request.clone(),
                planned_sources: run.planned_sources.clone(),
                status: run.status,
                queued_work: count(|state| *state == WorkState::Queued),
                awaiting_review_work: count(|state| matches!(state, WorkState::Parked(_))),
                completed_work: count(|state| *state == WorkState::Done),
                below_quality_work: run
                    .work
                    .iter()
                    .filter(|work| !work.shortfalls.is_empty())
                    .count() as u64,
                outranked_work: run
                    .work
                    .iter()
                    .filter(|work| work.outranked.is_some())
                    .count() as u64,
                unavailable_work: run
                    .work
                    .iter()
                    .filter(|work| work.unavailable.is_some())
                    .count() as u64,
                dismissed_work: run.work.iter().filter(|work| work.dismissed).count() as u64,
                discoveries: run
                    .planned_sources
                    .iter()
                    .map(|source_id| {
                        let complete = run.discovered.contains(source_id);
                        let games = if complete {
                            0
                        } else {
                            run.discovered_games.get(source_id).copied().unwrap_or(0) as u64
                        };
                        SourceDiscovery::new(source_id.clone(), complete, games)
                    })
                    .collect(),
            }
        });
        if let Some(decision) = self.decision_after_next_run_read.borrow_mut().take() {
            let review_item_id = self.review_items.borrow()[0].id;
            self.decide_review_item(review_item_id, decision)?;
        }
        if let Some(status) = self.status_after_next_run_read.borrow_mut().take() {
            self.runs.borrow_mut().get_mut(&run_id).unwrap().status = status;
        }
        Ok(run)
    }

    fn claim_execution(&self, run_id: i64) -> Result<bool, PortError> {
        Ok(self.executing.borrow_mut().insert(run_id))
    }

    fn release_execution(&self, run_id: i64) -> Result<(), PortError> {
        self.executing.borrow_mut().remove(&run_id);
        Ok(())
    }

    fn run_status(&self, run_id: i64) -> Result<Option<AcquisitionRunStatus>, PortError> {
        if std::mem::take(&mut *self.paused_at_next_status_read.borrow_mut()) {
            return Ok(Some(AcquisitionRunStatus::Paused));
        }
        let pause_after = self.pause_during_next_status_read.borrow_mut().take();
        if let Some(delay) = pause_after {
            std::thread::sleep(delay);
            if let Some(run) = self.runs.borrow_mut().get_mut(&run_id) {
                run.status = AcquisitionRunStatus::Paused;
            }
        }
        Ok(self.runs.borrow().get(&run_id).map(|run| run.status))
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
        let decision = self.decision_before_completion.borrow_mut().take();
        if target == AcquisitionRunStatus::Completed
            && let Some(decision) = decision
        {
            let review_item_id = self.review_items.borrow()[0].id;
            self.decide_review_item(review_item_id, decision)?;
        }
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

    fn discovered_games(&self, run_id: i64, source_id: &str) -> Result<usize, PortError> {
        Ok(self.runs.borrow()[&run_id]
            .discovered_games
            .get(source_id)
            .copied()
            .unwrap_or(0))
    }

    fn record_discovery_batch(
        &self,
        run_id: i64,
        source_id: &str,
        first_game: usize,
        games: usize,
        work: &[AcquisitionWorkItem],
    ) -> Result<bool, PortError> {
        let mut runs = self.runs.borrow_mut();
        let run = runs.get_mut(&run_id).unwrap();
        if let Some(status) = self.status_before_next_discovery.borrow_mut().take() {
            run.status = status;
        }
        if matches!(
            run.status,
            AcquisitionRunStatus::Cancelled | AcquisitionRunStatus::Completed
        ) {
            return Ok(false);
        }
        let recorded = run
            .discovered_games
            .entry(source_id.to_owned())
            .or_default();
        if run.discovered.contains(source_id) || *recorded != first_game {
            return Ok(true);
        }
        *recorded = first_game + games;
        for item in work {
            if !run
                .work
                .iter()
                .any(|existing| existing.item.key == item.key)
            {
                run.work.push(FakeWork {
                    item: item.clone(),
                    state: WorkState::Queued,
                    shortfalls: Vec::new(),
                    outranked: None,
                    unavailable: None,
                    dismissed: false,
                });
            }
        }
        Ok(true)
    }

    fn record_discovery(
        &self,
        run_id: i64,
        source_id: &str,
        work: &[AcquisitionWorkItem],
    ) -> Result<bool, PortError> {
        let mut runs = self.runs.borrow_mut();
        let run = runs.get_mut(&run_id).unwrap();
        if let Some(status) = self.status_before_next_discovery.borrow_mut().take() {
            run.status = status;
        }
        if matches!(
            run.status,
            AcquisitionRunStatus::Cancelled | AcquisitionRunStatus::Completed
        ) {
            return Ok(false);
        }
        if !run.discovered.insert(source_id.to_owned()) {
            return Ok(true);
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
                    shortfalls: Vec::new(),
                    outranked: None,
                    unavailable: None,
                    dismissed: false,
                });
            }
        }
        Ok(true)
    }

    fn next_queued_work(
        &self,
        run_id: i64,
        skipped_sources: &[String],
    ) -> Result<Option<AcquisitionWorkItem>, PortError> {
        if let Some(go_on) = countdown(&self.queue_read_gate) {
            go_on.recv().unwrap();
        }
        let leaving = countdown(&self.work_leaving_before_queue_read);
        let set_state = |key: &str, state: WorkState| {
            let mut runs = self.runs.borrow_mut();
            if let Some(work) = runs
                .get_mut(&run_id)
                .and_then(|run| run.work.iter_mut().find(|work| work.item.key == key))
            {
                work.state = state;
            }
        };
        if let Some(leaving) = &leaving {
            let state = if leaving.requeued {
                WorkState::Parked(0)
            } else {
                WorkState::Done
            };
            set_state(&leaving.key, state);
        }
        let next = {
            let runs = self.runs.borrow();
            let run = &runs[&run_id];
            (run.status == AcquisitionRunStatus::Running)
                .then(|| {
                    run.work.iter().find(|work| {
                        work.state == WorkState::Queued
                            && !skipped_sources
                                .iter()
                                .any(|source| source == work.item.candidate.source_id.as_str())
                    })
                })
                .flatten()
                .map(|work| work.item.clone())
        };
        if let Some(leaving) = leaving.filter(|leaving| leaving.requeued) {
            set_state(&leaving.key, WorkState::Queued);
        }
        Ok(next)
    }

    fn queued_work(
        &self,
        run_id: i64,
        skipped_sources: &[String],
        per_source: usize,
    ) -> Result<Vec<AcquisitionWorkItem>, PortError> {
        let runs = self.runs.borrow();
        let run = &runs[&run_id];
        if run.status != AcquisitionRunStatus::Running {
            return Ok(Vec::new());
        }
        // Each item with its position among its Source's queued items.
        let mut taken: BTreeMap<&str, usize> = BTreeMap::new();
        let mut positioned: Vec<(usize, usize, AcquisitionWorkItem)> = run
            .work
            .iter()
            .filter(|work| work.state == WorkState::Queued)
            .enumerate()
            .filter_map(|(order, work)| {
                let source = work.item.candidate.source_id.as_str();
                if skipped_sources.iter().any(|skipped| skipped == source) {
                    return None;
                }
                let position = taken.entry(source).or_default();
                *position += 1;
                (*position <= per_source).then(|| (*position, order, work.item.clone()))
            })
            .collect();
        positioned.sort_by_key(|(position, order, _)| (*position, *order));
        Ok(positioned.into_iter().map(|(_, _, item)| item).collect())
    }

    fn complete_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError> {
        self.open_scheduled_review();
        let mut runs = self.runs.borrow_mut();
        let run = runs.get_mut(&run_id).unwrap();
        let work = run
            .work
            .iter_mut()
            .find(|work| work.item.key == work_key)
            .ok_or_else(|| PortError::new(format!("work {work_key} does not exist")))?;
        work.state = WorkState::Done;
        if let Some(status) = self.status_after_next_completion.borrow_mut().take() {
            run.status = status;
        }
        Ok(())
    }

    fn dismiss_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError> {
        self.complete_work(run_id, work_key)?;
        let mut runs = self.runs.borrow_mut();
        let run = runs.get_mut(&run_id).unwrap();
        if let Some(work) = run.work.iter_mut().find(|work| work.item.key == work_key) {
            work.dismissed = true;
        }
        Ok(())
    }

    fn complete_unavailable_work(
        &self,
        run_id: i64,
        work_key: &str,
        reason: &str,
    ) -> Result<(), PortError> {
        let mut runs = self.runs.borrow_mut();
        let work = runs
            .get_mut(&run_id)
            .unwrap()
            .work
            .iter_mut()
            .find(|work| work.item.key == work_key)
            .ok_or_else(|| PortError::new(format!("work {work_key} does not exist")))?;
        work.state = WorkState::Done;
        work.unavailable = Some(reason.to_owned());
        Ok(())
    }

    fn record_source_failure(
        &self,
        run_id: i64,
        source_id: &str,
        stage: SourceFailureStage,
        message: &str,
    ) -> Result<(), PortError> {
        let mut failures = self.source_failures.borrow_mut();
        let sequence = failures.len() as i64 + 1;
        failures.push(SourceFailure {
            sequence,
            source_id: source_id.to_owned(),
            run_id,
            stage,
            message: message.to_owned(),
            recorded_at: 0,
        });
        Ok(())
    }

    fn source_failures(&self) -> Result<Vec<SourceFailure>, PortError> {
        Ok(self.source_failures.borrow().clone())
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
        let reject_now = {
            let mut countdown = self.rejection_before_review_read.borrow_mut();
            match *countdown {
                Some(1) => {
                    *countdown = None;
                    true
                }
                Some(reads) => {
                    *countdown = Some(reads - 1);
                    false
                }
                None => false,
            }
        };
        if reject_now {
            let review_item_id = self.review_items.borrow()[0].id;
            self.decide_review_item(review_item_id, ReviewDecision::Reject)?;
        }
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
        let settled = self.runs.borrow()[&run_id]
            .work
            .iter()
            .any(|work| work.item.key == work_key && work.state == WorkState::Done);
        if settled {
            return Ok(ParkedReview::Settled);
        }
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
            .ok_or_else(|| PortError::new(format!("queued work {work_key} does not exist")))?;
        work.state = WorkState::Parked(review_item.id);
        self.candidate_links
            .borrow_mut()
            .remove(&review_item.candidate_identity);
        Ok(ParkedReview::Parked(review_item))
    }

    fn supersede_candidate_review(
        &self,
        run_id: i64,
        candidate_identity: &str,
    ) -> Result<bool, PortError> {
        self.open_scheduled_review();
        if let Some(existing) = self.find_review_item(candidate_identity)? {
            self.apply_pending_human_decision(existing.id);
            match self.get_review_item(existing.id)?.unwrap().status {
                ReviewStatus::Accepted | ReviewStatus::Rejected => return Ok(false),
                ReviewStatus::AutoResolved => {
                    self.review_items
                        .borrow_mut()
                        .iter_mut()
                        .find(|item| item.id == existing.id)
                        .unwrap()
                        .status = ReviewStatus::Superseded;
                }
                _ => self.close_automatically(existing.id, ReviewStatus::Superseded),
            }
        }
        self.candidate_links.borrow_mut().remove(candidate_identity);
        self.dismiss_work(run_id, candidate_identity)?;
        Ok(true)
    }

    fn requeue_review_work(&self, review_item_id: i64, run_id: i64) -> Result<(), PortError> {
        let mut runs = self.runs.borrow_mut();
        let run = runs.get_mut(&run_id).unwrap();
        if run.status == AcquisitionRunStatus::Cancelled {
            return Ok(());
        }
        let mut moved = false;
        for work in &mut run.work {
            if work.state == WorkState::Parked(review_item_id) {
                work.state = WorkState::Queued;
                moved = true;
            }
        }
        if moved && run.status == AcquisitionRunStatus::Completed {
            run.status = AcquisitionRunStatus::Running;
        }
        Ok(())
    }

    fn decide_review_item(
        &self,
        review_item_id: i64,
        decision: ReviewDecision,
    ) -> Result<ReviewDecisionOutcome, PortError> {
        let decided = {
            let mut review_items = self.review_items.borrow_mut();
            let Some(item) = review_items
                .iter_mut()
                .find(|item| item.id == review_item_id)
            else {
                return Ok(ReviewDecisionOutcome::NotFound);
            };
            if !item.status.is_undecided() {
                return Ok(ReviewDecisionOutcome::NotUndecided(item.status));
            }
            if let ReviewDecision::Accept { release_edition_id } = decision
                && !item
                    .competing_matches
                    .iter()
                    .any(|candidate| candidate.release_edition_id == release_edition_id)
            {
                return Ok(ReviewDecisionOutcome::NotCompeting);
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
                self.move_parked_work(review_item_id, WorkState::Queued, false, false)
            }
            ReviewDecision::Reject => {
                links.remove(&decided.candidate_identity);
                drop(links);
                self.move_parked_work(review_item_id, WorkState::Done, true, true)
            }
            ReviewDecision::Defer => {}
        }
        Ok(ReviewDecisionOutcome::Recorded(Box::new(decided)))
    }

    fn persist_candidate_asset(
        &self,
        run_id: i64,
        candidate_identity: &str,
        record: PersistAsset,
        retention: RetentionPolicy,
    ) -> Result<CandidateAssetOutcome, PortError> {
        if let Some(kept) = retention.kept_per_type()
            && let Some(release_edition_id) = record.existing_release_edition_id
            && let Some(outranked) = outranked_by(
                &StoredObject {
                    hash: record.object_hash.clone(),
                    byte_len: record.byte_len,
                    media: record.media.clone(),
                },
                &self.retained_assets(release_edition_id, record.asset_type),
                kept,
            )
        {
            let recorded = outranked.clone();
            return Ok(
                if self.settle_unlinked(run_id, candidate_identity, release_edition_id, |work| {
                    work.outranked = Some(recorded);
                })? {
                    CandidateAssetOutcome::Outranked(outranked)
                } else {
                    CandidateAssetOutcome::HumanDecisionConflict
                },
            );
        }
        self.open_scheduled_review();
        if let Some(existing) = self.find_review_item(candidate_identity)? {
            self.apply_pending_human_decision(existing.id);
            let current = self.get_review_item(existing.id)?.unwrap();
            match (current.status, current.decision) {
                (ReviewStatus::Rejected, _) => {
                    return Ok(CandidateAssetOutcome::HumanDecisionConflict);
                }
                (ReviewStatus::Accepted, Some(ReviewDecision::Accept { release_edition_id }))
                    if record.existing_release_edition_id != Some(release_edition_id) =>
                {
                    return Ok(CandidateAssetOutcome::HumanDecisionConflict);
                }
                (ReviewStatus::Pending | ReviewStatus::Deferred, _) => {
                    self.close_automatically(existing.id, ReviewStatus::AutoResolved);
                }
                (ReviewStatus::Superseded, _) => {
                    let mut review_items = self.review_items.borrow_mut();
                    let item = review_items
                        .iter_mut()
                        .find(|item| item.id == existing.id)
                        .unwrap();
                    item.status = ReviewStatus::AutoResolved;
                }
                _ => {}
            }
        }
        let imported = self.persist_asset(record)?;
        self.candidate_links
            .borrow_mut()
            .insert(candidate_identity.to_owned(), imported.release_edition_id);
        self.complete_work(run_id, candidate_identity)?;
        Ok(CandidateAssetOutcome::Linked(imported))
    }

    fn complete_candidate_below_quality(
        &self,
        run_id: i64,
        candidate_identity: &str,
        release_edition_id: i64,
        shortfalls: &[QualityShortfall],
    ) -> Result<bool, PortError> {
        self.settle_unlinked(run_id, candidate_identity, release_edition_id, |work| {
            work.shortfalls = shortfalls.to_vec();
        })
    }
}

impl FakeVault {
    /// Settles a candidate matched to `release_edition_id` without linking it, as the review
    /// repository does for below-quality and outranked originals; `false` when a human decision
    /// conflicts.
    fn settle_unlinked(
        &self,
        run_id: i64,
        candidate_identity: &str,
        release_edition_id: i64,
        record_outcome: impl FnOnce(&mut FakeWork),
    ) -> Result<bool, PortError> {
        self.open_scheduled_review();
        if let Some(existing) = self.find_review_item(candidate_identity)? {
            self.apply_pending_human_decision(existing.id);
            let current = self.get_review_item(existing.id)?.unwrap();
            match (current.status, current.decision) {
                (ReviewStatus::Rejected, _) => return Ok(false),
                (
                    ReviewStatus::Accepted,
                    Some(ReviewDecision::Accept {
                        release_edition_id: accepted,
                    }),
                ) if accepted != release_edition_id => return Ok(false),
                (ReviewStatus::Pending | ReviewStatus::Deferred | ReviewStatus::Superseded, _) => {
                    {
                        let mut review_items = self.review_items.borrow_mut();
                        let item = review_items
                            .iter_mut()
                            .find(|item| item.id == existing.id)
                            .unwrap();
                        item.status = ReviewStatus::AutoResolved;
                        item.decision = None;
                    }
                    self.move_parked_work(existing.id, WorkState::Queued, false, false);
                }
                _ => {}
            }
        }
        {
            let mut links = self.candidate_links.borrow_mut();
            if links.get(candidate_identity) != Some(&release_edition_id) {
                links.remove(candidate_identity);
            }
        }
        self.complete_work(run_id, candidate_identity)?;
        let mut runs = self.runs.borrow_mut();
        let work = runs
            .get_mut(&run_id)
            .unwrap()
            .work
            .iter_mut()
            .find(|work| work.item.key == candidate_identity)
            .unwrap();
        record_outcome(work);
        Ok(true)
    }

    /// Assets retained for the edition and type: the library's and those persisted since.
    fn retained_assets(&self, release_edition_id: i64, asset_type: AssetType) -> Vec<LibraryAsset> {
        let mut retained: Vec<LibraryAsset> = self
            .library
            .borrow()
            .iter()
            .filter(|entry| entry.release_edition_id == release_edition_id)
            .flat_map(|entry| entry.assets.clone())
            .filter(|asset| asset.asset_type == asset_type)
            .collect();
        retained.extend(
            self.records
                .borrow()
                .iter()
                .enumerate()
                .filter(|(_, record)| {
                    record.existing_release_edition_id == Some(release_edition_id)
                        && record.asset_type == asset_type
                })
                .map(|(index, record)| LibraryAsset {
                    asset_id: index as i64 + 1,
                    asset_type: record.asset_type,
                    object_hash: record.object_hash.clone(),
                    byte_len: record.byte_len,
                    media: record.media.clone(),
                    original_filename: record.original_filename.clone(),
                    provenance: Vec::new(),
                    derived: Vec::new(),
                }),
        );
        retained
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
        let listed = self.library.borrow().clone();
        let added = std::mem::take(&mut *self.library_added_after_next_listing.borrow_mut());
        self.library.borrow_mut().extend(added);
        Ok(listed)
    }
}

/// A value doubles record while executions may use them from several threads, read like a
/// `RefCell`.
#[derive(Debug, Default)]
pub struct Shared<T>(Mutex<T>);

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    pub fn borrow(&self) -> MutexGuard<'_, T> {
        self.0.lock().unwrap()
    }

    pub fn borrow_mut(&self) -> MutexGuard<'_, T> {
        self.0.lock().unwrap()
    }
}

#[derive(Default)]
pub struct FakeStore {
    pub stored: Shared<Vec<Vec<u8>>>,
    /// Media every stored original is read as; unknown by default.
    pub media: Option<MediaInfo>,
}

impl FakeStore {
    pub fn storing(media: MediaInfo) -> Self {
        Self {
            media: Some(media),
            ..Self::default()
        }
    }
}

impl ObjectStorePort for FakeStore {
    fn store_original(&self, reader: &mut dyn Read) -> Result<StoredObject, PortError> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|error| PortError::new(error.to_string()))?;
        let stored = StoredObject {
            hash: format!("hash-of-{}-bytes", bytes.len()),
            byte_len: bytes.len() as u64,
            media: self.media.clone().unwrap_or_else(MediaInfo::unknown),
        };
        self.stored.borrow_mut().push(bytes);
        Ok(stored)
    }
}

pub struct FakeConnector {
    pub source_id: &'static str,
    pub candidates: Vec<AssetCandidate>,
    pub discovery_fails: bool,
    pub failing_downloads: BTreeSet<String>,
    /// Source URLs whose download starts but fails partway through the body.
    pub failing_bodies: BTreeSet<String>,
    /// Locators the Source no longer serves, as an HTTP 404 says.
    pub unavailable_downloads: BTreeSet<String>,
    pub discover_calls: Shared<u32>,
    pub downloads: Shared<Vec<String>>,
    /// Why the connector refuses every request, if it does.
    pub unsupported_reason: Option<String>,
    /// Whether checking a plan fails, as when the source cannot be reached.
    pub plan_check_fails: bool,
    /// Asset Types the connector declares it can acquire.
    pub asset_types: Vec<AssetType>,
    /// Whether the Source needs an API key.
    pub needs_api_key: bool,
    /// What the Source is known to limit, in words.
    pub rate_limits: Option<&'static str>,
    /// The credentials the Source asks for, when not just an API key or none.
    pub credential_fields: Option<&'static [CredentialField]>,
}

impl FakeConnector {
    pub fn new(candidates: Vec<AssetCandidate>) -> Self {
        Self {
            source_id: SOURCE_ID,
            candidates,
            discovery_fails: false,
            failing_downloads: BTreeSet::new(),
            failing_bodies: BTreeSet::new(),
            unavailable_downloads: BTreeSet::new(),
            discover_calls: Shared::new(0),
            downloads: Shared::new(Vec::new()),
            unsupported_reason: None,
            plan_check_fails: false,
            asset_types: vec![AssetType::BoxFront],
            needs_api_key: false,
            rate_limits: None,
            credential_fields: None,
        }
    }
}

impl ConnectorPort for FakeConnector {
    fn source_id(&self) -> &'static str {
        self.source_id
    }

    fn needs_api_key(&self) -> bool {
        self.needs_api_key
    }

    fn rate_limits(&self) -> Option<String> {
        self.rate_limits.map(str::to_owned)
    }

    fn credential_fields(&self) -> &'static [CredentialField] {
        match self.credential_fields {
            Some(fields) => fields,
            None if self.needs_api_key => &[API_KEY_FIELD],
            None => &[],
        }
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: self.asset_types.clone(),
            direct_media_download: true,
        }
    }

    fn unsupported_request_reason(
        &self,
        _request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        if self.plan_check_fails {
            return Err(PortError::new("fixture source unreachable".to_owned()));
        }
        Ok(self.unsupported_reason.clone())
    }

    fn discover(&self, _request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        *self.discover_calls.borrow_mut() += 1;
        if self.discovery_fails {
            return Err(PortError::new("fixture discovery unavailable".to_owned()));
        }
        Ok(self.candidates.clone())
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        if self.failing_downloads.contains(&candidate.source_url) {
            return Err(PortError::new("fixture download failed".to_owned()));
        }
        if self.unavailable_downloads.contains(&candidate.source_url) {
            return Err(PortError::unavailable("fixture media is gone".to_owned()));
        }
        self.downloads
            .borrow_mut()
            .push(candidate.source_url.clone());
        if self.failing_bodies.contains(&candidate.source_url) {
            return Ok(Box::new(
                Cursor::new(b"partial bytes".to_vec()).chain(FailingRead),
            ));
        }
        Ok(Box::new(Cursor::new(
            format!("bytes of {}", candidate.original_filename).into_bytes(),
        )))
    }
}

/// A download body whose connection drops: every read fails.
pub struct FailingRead;

impl Read for FailingRead {
    fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("fixture connection reset"))
    }
}

/// Machine settings kept in memory.
#[derive(Default)]
pub struct FakeSettings {
    pub disabled: Shared<Vec<String>>,
}

impl MachineSettingsPort for FakeSettings {
    fn disabled_sources(&self) -> Result<Vec<String>, PortError> {
        Ok(self.disabled.borrow().clone())
    }

    fn set_source_enabled(&self, source_id: &str, enabled: bool) -> Result<(), PortError> {
        let mut disabled = self.disabled.borrow_mut();
        disabled.retain(|disabled| disabled != source_id);
        if !enabled {
            disabled.push(source_id.to_owned());
        }
        Ok(())
    }
}

/// A credential store kept in memory, by Source.
#[derive(Default)]
pub struct FakeCredentials {
    pub keys: Shared<BTreeMap<String, String>>,
}

impl CredentialStorePort for FakeCredentials {
    fn api_key(&self, source_id: &str) -> Result<Option<ApiKey>, PortError> {
        Ok(self
            .keys
            .borrow()
            .get(source_id)
            .map(|key| ApiKey::new(key.as_str()).unwrap()))
    }

    fn set_api_key(&self, source_id: &str, key: &ApiKey) -> Result<(), PortError> {
        self.keys
            .borrow_mut()
            .insert(source_id.to_owned(), key.expose().to_owned());
        Ok(())
    }

    fn clear_api_key(&self, source_id: &str) -> Result<(), PortError> {
        self.keys.borrow_mut().remove(source_id);
        Ok(())
    }
}
