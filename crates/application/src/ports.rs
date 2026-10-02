use std::{io::Read, path::Path};

use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRun, AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate,
    ConnectorCapabilities, ImportedAsset, ImportedReleaseEdition, LibraryEntry, NewReviewItem,
    Outranked, PersistAsset, QualityShortfall, ReferenceReleaseRecord, RetentionPolicy,
    ReviewDecision, ReviewItem, ReviewStatus, StoredObject,
};
use thiserror::Error;

/// Failure of a port adapter: storage, network or another environmental failure, or a Source
/// that answered with data breaking the connector contract.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub struct PortError {
    message: String,
    invalid_source_data: bool,
}

impl PortError {
    /// An environmental failure, which may succeed when retried.
    pub fn new(message: String) -> Self {
        Self {
            message,
            invalid_source_data: false,
        }
    }

    /// A Source answered, but with data that breaks the connector contract, such as malformed
    /// metadata or a catalog that cannot be parsed.
    pub fn invalid_source_data(message: String) -> Self {
        Self {
            message,
            invalid_source_data: true,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn is_invalid_source_data(&self) -> bool {
        self.invalid_source_data
    }
}

/// Immutable content-addressed store of original bytes.
pub trait ObjectStorePort {
    /// Streams original bytes into the store and returns their identity. Publishing is atomic
    /// and idempotent: identical bytes resolve to the same verified object. Originals are
    /// stored before the catalog references them, so an interrupted import can only leave an
    /// unreferenced object behind.
    fn store_original(&self, reader: &mut dyn Read) -> Result<StoredObject, PortError>;
}

/// Library catalog: Games, Release Editions and their Assets.
pub trait CatalogPort {
    fn persist_asset(&self, record: PersistAsset) -> Result<ImportedAsset, PortError>;

    /// Lists every Release Edition with its Assets and its assertions in observation order (a
    /// claim observed again comes after the claims recorded before), which
    /// Canonical Value selection relies on.
    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError>;
}

/// Outcome of recording a human decision on a Review Item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewDecisionOutcome {
    Recorded(Box<ReviewItem>),
    NotFound,
    /// The item was already decided or closed automatically.
    NotUndecided(ReviewStatus),
    /// The accepted Release Edition is not among the item's current competing matches.
    NotCompeting,
}

impl ReviewDecisionOutcome {
    /// The decided item, if the decision was recorded.
    pub fn recorded(self) -> Option<ReviewItem> {
        match self {
            Self::Recorded(item) => Some(*item),
            _ => None,
        }
    }
}

/// Outcome of parking run work on the Review Item of its candidate identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParkedReview {
    /// The work now waits for a human decision on this undecided item.
    Parked(ReviewItem),
    /// A human already decided the item; the work was left queued.
    AlreadyDecided(ReviewItem),
    /// Another execution of the run already settled the work; no item was opened.
    Settled,
}

/// Outcome of persisting an Asset acquired for a candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateAssetOutcome {
    Linked(ImportedAsset),
    /// Keep Best Per Type: a retained Asset stays preferred, so nothing was linked.
    Outranked(Outranked),
    /// A human rejected the candidate or accepted another Release Edition; nothing changed.
    HumanDecisionConflict,
}

/// Review Items, one per candidate identity. Every method is atomic.
pub trait ReviewRepositoryPort {
    fn list_review_items(&self) -> Result<Vec<ReviewItem>, PortError>;

    fn get_review_item(&self, review_item_id: i64) -> Result<Option<ReviewItem>, PortError>;

    fn find_review_item(&self, candidate_identity: &str) -> Result<Option<ReviewItem>, PortError>;

    /// Opens or refreshes the undecided Review Item for the candidate identity and parks the
    /// queued run work on it; parking work already parked on the item is a no-op, and work
    /// another execution completed is reported `Settled` without touching the item. An item
    /// closed by re-evaluation is reopened as pending.
    fn park_work_for_review(
        &self,
        run_id: i64,
        work_key: &str,
        item: NewReviewItem,
    ) -> Result<ParkedReview, PortError>;

    /// Dismisses the candidate after a low-confidence evaluation: its Review Item becomes
    /// `Superseded` with its parked work completed, its automatic links are detached, and its
    /// work in `run_id` is completed. The item is read in the same transaction, so an item
    /// opened concurrently is superseded too. Returns `false`, changing nothing, when a human
    /// decided the item.
    fn supersede_candidate_review(
        &self,
        run_id: i64,
        candidate_identity: &str,
    ) -> Result<bool, PortError>;

    /// Records a human decision on an undecided item and moves its parked work: accepting
    /// requeues it in runs that are not cancelled (reopening completed runs), rejecting
    /// completes it and detaches the Assets linked to the candidate, deferring keeps it parked.
    /// The item's state and evidence are checked in the same transaction as the decision.
    fn decide_review_item(
        &self,
        review_item_id: i64,
        decision: ReviewDecision,
    ) -> Result<ReviewDecisionOutcome, PortError>;

    /// Persists an Asset acquired for the candidate with this identity, in one transaction that
    /// keeps the candidate linked to a single Release Edition: links of the same candidate to
    /// other editions are removed, an undecided Review Item is closed as `AutoResolved`
    /// (completing the work parked on it), and the candidate's work in `run_id` is completed.
    ///
    /// Under Keep Best Per Type, an original that a retained Asset of the same Release Edition
    /// and type outranks is not linked: the candidate is settled as for a below-quality
    /// original (see `complete_candidate_below_quality`) and its work records why. The retained
    /// Assets are compared in the same transaction.
    ///
    /// Changes nothing when a human rejected the candidate or accepted another Release Edition.
    fn persist_candidate_asset(
        &self,
        run_id: i64,
        candidate_identity: &str,
        record: PersistAsset,
        retention: RetentionPolicy,
    ) -> Result<CandidateAssetOutcome, PortError>;

    /// Completes the candidate's work in `run_id` without linking its original, which fell
    /// short of the run's quality requirements, and records the shortfalls so the candidate
    /// stays explainable. The match to `release_edition_id` still settles the candidate in the
    /// same transaction: its links to other editions are removed and an undecided or superseded
    /// Review Item is closed as `AutoResolved`, requeueing the work parked on it so every run
    /// applies its own requirements. Changes nothing and returns `false` when a human rejected
    /// the candidate or accepted another Release Edition.
    fn complete_candidate_below_quality(
        &self,
        run_id: i64,
        candidate_identity: &str,
        release_edition_id: i64,
        shortfalls: &[QualityShortfall],
    ) -> Result<bool, PortError>;
}

/// Persisted Acquisition Runs and their work queue.
pub trait RunRepositoryPort {
    /// Persists a running run of `request` that contacts the `planned_sources` of its plan.
    fn create_run(
        &self,
        request: AcquisitionRequest,
        planned_sources: Vec<String>,
    ) -> Result<AcquisitionRun, PortError>;

    fn get_run(&self, run_id: i64) -> Result<Option<AcquisitionRun>, PortError>;

    fn list_runs(&self) -> Result<Vec<AcquisitionRun>, PortError>;

    /// Changes the run status only if it still equals `expected`. Completing a run fails while
    /// it has queued work.
    fn compare_and_set_run_status(
        &self,
        run_id: i64,
        expected: AcquisitionRunStatus,
        target: AcquisitionRunStatus,
    ) -> Result<bool, PortError>;

    /// Whether `source_id` already recorded its discovered work for the run.
    fn has_discovered(&self, run_id: i64, source_id: &str) -> Result<bool, PortError>;

    /// Queues the work discovered from one Source and marks its discovery complete, atomically.
    /// Work whose key is already recorded for the run is ignored, and so is any later discovery
    /// of an already discovered source: a run keeps a single snapshot per source. Returns
    /// `false`, recording nothing, when the run was cancelled or completed meanwhile.
    fn record_discovery(
        &self,
        run_id: i64,
        source_id: &str,
        work: &[AcquisitionWorkItem],
    ) -> Result<bool, PortError>;

    /// Returns the oldest queued work item of a Source outside `skipped_sources` while the run is
    /// running.
    fn next_queued_work(
        &self,
        run_id: i64,
        skipped_sources: &[String],
    ) -> Result<Option<AcquisitionWorkItem>, PortError>;

    fn complete_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError>;
}

/// Source-specific integration that discovers and downloads Asset Candidates.
///
/// Discovered candidates are persisted, so `AssetCandidate::source_url` must be a stable,
/// absolute URL without userinfo, query or fragment. Connectors that need API keys, sessions or
/// signed URLs add them inside `download`, which receives the persisted candidate.
pub trait ConnectorPort {
    fn source_id(&self) -> &'static str;

    fn capabilities(&self) -> ConnectorCapabilities;

    /// Why this connector cannot execute `request` beyond its declared capabilities (for
    /// example a source that needs an explicit game selection or does not cover a platform),
    /// or `None` when it can. Checked before a run is persisted; it may consult the source.
    fn unsupported_request_reason(
        &self,
        _request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        Ok(None)
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError>;

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError>;
}

pub trait ReferenceCatalogSourcePort {
    /// Reads up to `max_games` releases from the canonical `source_path`. The application then
    /// records the readable location of that file on every returned assertion.
    fn read_releases(
        &self,
        source_path: &Path,
        max_games: usize,
    ) -> Result<Vec<ReferenceReleaseRecord>, PortError>;
}

pub trait ReferenceCatalogRepositoryPort {
    fn persist_reference_release(
        &self,
        record: ReferenceReleaseRecord,
    ) -> Result<ImportedReleaseEdition, PortError>;

    fn persist_reference_releases(
        &self,
        records: Vec<ReferenceReleaseRecord>,
    ) -> Result<Vec<ImportedReleaseEdition>, PortError> {
        records
            .into_iter()
            .map(|record| self.persist_reference_release(record))
            .collect()
    }
}
