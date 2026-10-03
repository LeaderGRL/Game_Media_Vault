use std::{io::Read, path::Path};

use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRun, AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate,
    ConnectorCapabilities, ImportedAsset, ImportedReleaseEdition, LibraryEntry, NewReviewItem,
    Outranked, PersistAsset, QualityShortfall, ReferenceReleaseRecord, ReferenceReviewEdition,
    ReferenceReviewItem, RetentionPolicy, ReviewDecision, ReviewItem, ReviewStatus, SourceFailure,
    SourceFailureStage, StoredObject,
};
use thiserror::Error;

use crate::{API_KEY_FIELD, ApiKey, CredentialField};

/// Failure of a port adapter: storage, network or another environmental failure, or a Source
/// that answered with data breaking the connector contract.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub struct PortError {
    message: String,
    kind: PortErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortErrorKind {
    Environmental,
    InvalidSourceData,
    Unavailable,
}

impl PortError {
    /// An environmental failure, which may succeed when retried.
    pub fn new(message: String) -> Self {
        Self {
            message,
            kind: PortErrorKind::Environmental,
        }
    }

    /// A Source answered, but with data that breaks the connector contract, such as malformed
    /// metadata or a catalog that cannot be parsed.
    pub fn invalid_source_data(message: String) -> Self {
        Self {
            message,
            kind: PortErrorKind::InvalidSourceData,
        }
    }

    /// A Source no longer serves what was asked, such as media it answers with HTTP 404 for;
    /// retrying cannot help.
    pub fn unavailable(message: String) -> Self {
        Self {
            message,
            kind: PortErrorKind::Unavailable,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn is_invalid_source_data(&self) -> bool {
        self.kind == PortErrorKind::InvalidSourceData
    }

    pub fn is_unavailable(&self) -> bool {
        self.kind == PortErrorKind::Unavailable
    }
}

/// Immutable content-addressed store of original bytes. Executions store originals from several
/// threads at once.
pub trait ObjectStorePort: Send + Sync {
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

    /// The Release Editions holding the `limit` Assets retained last, each with those of its
    /// Assets alone and no assertions, in library order: Assets get larger ids as they are
    /// retained. They are kept from the whole library by default; a catalog reads them alone.
    fn list_latest_assets(&self, limit: usize) -> Result<Vec<LibraryEntry>, PortError> {
        let mut entries = self.list_library()?;
        let mut latest: Vec<i64> = entries
            .iter()
            .flat_map(|entry| entry.assets.iter().map(|asset| asset.asset_id))
            .collect();
        latest.sort_unstable_by(|left, right| right.cmp(left));
        latest.truncate(limit);
        for entry in &mut entries {
            entry.assertions.clear();
            entry
                .assets
                .retain(|asset| latest.contains(&asset.asset_id));
        }
        entries.retain(|entry| !entry.assets.is_empty());
        Ok(entries)
    }
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
    /// Under a policy keeping the best originals of each type (Keep Best Per Type keeps one),
    /// an original that as many retained Assets of the same Release Edition and type outrank as
    /// the policy keeps is not linked: the candidate is settled as for a below-quality
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

/// Persisted Acquisition Runs and their work queue. Download threads read the status of the run
/// they serve.
pub trait RunRepositoryPort: Send + Sync {
    /// Persists a running run of `request` that contacts the `planned_sources` of its plan.
    fn create_run(
        &self,
        request: AcquisitionRequest,
        planned_sources: Vec<String>,
    ) -> Result<AcquisitionRun, PortError>;

    fn get_run(&self, run_id: i64) -> Result<Option<AcquisitionRun>, PortError>;

    /// The status of the run, without anything else about it.
    fn run_status(&self, run_id: i64) -> Result<Option<AcquisitionRunStatus>, PortError>;

    /// Claims the run for one execution on this machine while no other execution holds it, and
    /// returns whether it did. A process that ends releases its claims.
    fn claim_execution(&self, run_id: i64) -> Result<bool, PortError>;

    /// Releases the claim an execution made on the run.
    fn release_execution(&self, run_id: i64) -> Result<(), PortError>;

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

    /// How many of the requested games, in request order, the batches of the discovery of
    /// `source_id` recorded, while that discovery is not complete.
    fn discovered_games(&self, run_id: i64, source_id: &str) -> Result<usize, PortError>;

    /// Queues the work of a batch of the discovery of `source_id` covering `games` requested
    /// games from game `first_game`, counting from zero, and records them, atomically, leaving
    /// the discovery incomplete; its last batch is recorded by `record_discovery`. A batch
    /// starting anywhere but after the games already recorded is ignored, as is work whose key
    /// the run already recorded. The work of a discovery still in batches is queued like any.
    /// Returns `false`, recording nothing, when the run was cancelled or completed meanwhile.
    fn record_discovery_batch(
        &self,
        run_id: i64,
        source_id: &str,
        first_game: usize,
        games: usize,
        work: &[AcquisitionWorkItem],
    ) -> Result<bool, PortError>;

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

    /// The oldest `per_source` queued work items of each Source outside `skipped_sources`, while
    /// the run is running: every Source's oldest item first, in queue order, then every Source's
    /// second, and so on. Reading them changes nothing.
    fn queued_work(
        &self,
        run_id: i64,
        skipped_sources: &[String],
        per_source: usize,
    ) -> Result<Vec<AcquisitionWorkItem>, PortError>;

    fn complete_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError>;

    /// Completes `work_key` without keeping its candidate, which a human rejected. A repository
    /// that does not count dismissed work apart completes it as any other.
    fn dismiss_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError> {
        self.complete_work(run_id, work_key)
    }

    /// Completes `work_key` as unavailable: its Source no longer serves the candidate media, so
    /// retrying cannot help. `reason` says why.
    fn complete_unavailable_work(
        &self,
        run_id: i64,
        work_key: &str,
        reason: &str,
    ) -> Result<(), PortError>;

    /// Records that `source_id` failed at `stage` while `run_id` executed, with what its
    /// connector reported.
    fn record_source_failure(
        &self,
        run_id: i64,
        source_id: &str,
        stage: SourceFailureStage,
        message: &str,
    ) -> Result<(), PortError>;

    /// Every recorded Source failure, in recording order.
    fn source_failures(&self) -> Result<Vec<SourceFailure>, PortError>;
}

/// Source-specific integration that discovers and downloads Asset Candidates.
///
/// Discovered candidates are persisted, so `AssetCandidate::source_url` must be a stable,
/// absolute URL without userinfo, query or fragment. Connectors that need API keys, sessions or
/// signed URLs add them inside `download`, which receives the persisted candidate. Executions
/// download from several threads at once.
pub trait ConnectorPort: Send + Sync {
    fn source_id(&self) -> &'static str;

    /// Whether the Source needs an API key, which the connector reads from this machine's
    /// credential store.
    fn needs_api_key(&self) -> bool {
        false
    }

    /// The credentials the Source asks for, which the connector reads from this machine's
    /// credential store: its API key when it needs one, none otherwise.
    fn credential_fields(&self) -> &'static [CredentialField] {
        if self.needs_api_key() {
            &[API_KEY_FIELD]
        } else {
            &[]
        }
    }

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

    /// The most games one discovery looks up, for a Source that looks games up one by one under
    /// a quota or a pace: a request naming more is discovered, and checked, that many games at
    /// a time, each batch recorded as it completes. `None` discovers a request at once.
    fn discovery_batch_size(&self) -> Option<usize> {
        None
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError>;

    /// Why the Source takes no part in acquisitions at all, such as being disabled on this
    /// machine, or `None` when it does. Executions leave its queued work waiting.
    fn disabled_reason(&self) -> Option<String> {
        None
    }

    /// What the Source is known to limit, such as a monthly allowance of requests or the pace it
    /// is read at, in words, or `None` when nothing is known.
    fn rate_limits(&self) -> Option<String> {
        None
    }
}

/// A borrowed connector serves as the connector it borrows.
impl<T: ConnectorPort + ?Sized> ConnectorPort for &T {
    fn source_id(&self) -> &'static str {
        (**self).source_id()
    }

    fn needs_api_key(&self) -> bool {
        (**self).needs_api_key()
    }

    fn credential_fields(&self) -> &'static [CredentialField] {
        (**self).credential_fields()
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        (**self).capabilities()
    }

    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        (**self).unsupported_request_reason(request)
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        (**self).discover(request)
    }

    fn discovery_batch_size(&self) -> Option<usize> {
        (**self).discovery_batch_size()
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        (**self).download(candidate)
    }

    fn disabled_reason(&self) -> Option<String> {
        (**self).disabled_reason()
    }

    fn rate_limits(&self) -> Option<String> {
        (**self).rate_limits()
    }
}

/// A boxed connector serves as the connector it holds.
impl<T: ConnectorPort + ?Sized> ConnectorPort for Box<T> {
    fn source_id(&self) -> &'static str {
        (**self).source_id()
    }

    fn needs_api_key(&self) -> bool {
        (**self).needs_api_key()
    }

    fn credential_fields(&self) -> &'static [CredentialField] {
        (**self).credential_fields()
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        (**self).capabilities()
    }

    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        (**self).unsupported_request_reason(request)
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        (**self).discover(request)
    }

    fn discovery_batch_size(&self) -> Option<usize> {
        (**self).discovery_batch_size()
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        (**self).download(candidate)
    }

    fn disabled_reason(&self) -> Option<String> {
        (**self).disabled_reason()
    }

    fn rate_limits(&self) -> Option<String> {
        (**self).rate_limits()
    }
}

/// The releases a reference catalog file yields, and how many of its records were too malformed
/// to read; those never invalidate the others.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceCatalogRead {
    pub releases: Vec<ReferenceReleaseRecord>,
    pub skipped_records: usize,
}

pub trait ReferenceCatalogSourcePort {
    /// Reads up to `max_games` releases from the canonical `source_path`. The application then
    /// records the readable location of that file on every returned assertion.
    fn read_releases(
        &self,
        source_path: &Path,
        max_games: usize,
    ) -> Result<ReferenceCatalogRead, PortError>;
}

/// The game lists of whole platforms, from a Source that publishes one per platform. Each
/// assertion it returns already names where it was read.
pub trait PlatformCatalogSourcePort {
    /// Every release the Source lists for `platform`, or none when it lists no such platform.
    fn platform_releases(&self, platform: &str) -> Result<Option<ReferenceCatalogRead>, PortError>;
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

/// The reference records awaiting a human to tell which Release Edition, if any, they describe.
pub trait ReferenceReviewRepositoryPort {
    /// The pending items, oldest first.
    fn list_reference_review_items(&self) -> Result<Vec<ReferenceReviewItem>, PortError>;

    /// What the catalog knows of each edition, in order, leaving out those it no longer holds.
    fn describe_reference_review_editions(
        &self,
        release_edition_ids: &[i64],
    ) -> Result<Vec<ReferenceReviewEdition>, PortError>;

    /// Decides that the record of the pending item `item_id` describes the candidate
    /// `release_edition_id`: the record's own edition merges into it, or the record moves to it
    /// alone when its source linked it into another source's edition.
    fn link_reference_review_item(
        &self,
        item_id: i64,
        release_edition_id: i64,
    ) -> Result<ReferenceReviewOutcome, PortError>;

    /// Decides that the record of the pending item `item_id` describes none of its candidates.
    fn keep_reference_review_item_apart(
        &self,
        item_id: i64,
    ) -> Result<ReferenceReviewOutcome, PortError>;
}

/// What deciding a Reference Review Item did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceReviewOutcome {
    Decided,
    /// No pending item has this id.
    ItemNotPending,
    /// The edition is not one the record may describe, or some source holds a record on both
    /// editions, which would make them two releases of it.
    NotACandidate,
    /// The record's edition holds Assets, which merging does not move yet.
    EditionHoldsAssets,
}

/// The settings of this machine, shared by every vault it opens.
pub trait MachineSettingsPort {
    /// The Sources disabled on this machine.
    fn disabled_sources(&self) -> Result<Vec<String>, PortError>;

    /// Records whether `source_id` takes part in acquisitions on this machine.
    fn set_source_enabled(&self, source_id: &str, enabled: bool) -> Result<(), PortError>;
}

/// This machine's secure credential store, such as the OS keychain, which every vault it opens
/// shares. It alone keeps API keys: never the vault, logs, exports or provenance.
pub trait CredentialStorePort: Send + Sync {
    /// The API key stored for `source_id`, if any.
    fn api_key(&self, source_id: &str) -> Result<Option<ApiKey>, PortError>;

    /// Stores `key` for `source_id`, replacing the one stored before.
    fn set_api_key(&self, source_id: &str, key: &ApiKey) -> Result<(), PortError>;

    /// Forgets the API key of `source_id`, if one is stored.
    fn clear_api_key(&self, source_id: &str) -> Result<(), PortError>;
}
