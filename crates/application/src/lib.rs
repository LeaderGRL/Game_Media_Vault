use std::{
    fs,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRun, AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate,
    AssetCandidateMatch, AssetType, ConnectorCapabilities, ImportedAsset, ImportedReleaseEdition,
    LibraryEntry, MatchConfidence, MatchingPolicy, MatchingPolicyValidationError, NewReviewItem,
    PersistAsset, ReferenceReleaseRecord, RetentionPolicy, ReviewDecision, ReviewItem,
    ReviewStatus, SourceId, StoredObject,
};
use thiserror::Error;

pub use game_media_vault_domain::{
    AcquisitionRequestDraft as AcquisitionRequestInput, AcquisitionRequestValidationError,
    match_asset_candidate_to_release, review_matches_for_asset_candidate,
};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct PortError(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewProcessingClaim {
    pub item: ReviewItem,
    pub lease_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewProcessingFinalization {
    Imported(ImportedAsset),
    Requeued,
    Discarded,
}

const REVIEW_LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(300);

struct ReviewLeaseReader<'a> {
    inner: Box<dyn Read + Send>,
    catalog: &'a dyn CatalogPort,
    review_item_id: i64,
    lease_token: &'a str,
    last_renewed: Instant,
}

impl<'a> ReviewLeaseReader<'a> {
    fn new(
        inner: Box<dyn Read + Send>,
        catalog: &'a dyn CatalogPort,
        review_item_id: i64,
        lease_token: &'a str,
    ) -> Self {
        Self {
            inner,
            catalog,
            review_item_id,
            lease_token,
            last_renewed: Instant::now(),
        }
    }

    fn renew_if_needed(&mut self) -> io::Result<()> {
        if self.last_renewed.elapsed() < REVIEW_LEASE_HEARTBEAT_INTERVAL {
            return Ok(());
        }
        if !self
            .catalog
            .renew_review_item_processing(self.review_item_id, self.lease_token)
            .map_err(|error| io::Error::other(error.to_string()))?
        {
            return Err(io::Error::other(format!(
                "review item #{} lost its processing lease",
                self.review_item_id
            )));
        }
        self.last_renewed = Instant::now();
        Ok(())
    }
}

impl Read for ReviewLeaseReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.renew_if_needed()?;
        self.inner.read(buffer)
    }
}

pub trait ObjectStorePort {
    fn store_original(&self, source: &Path) -> Result<StoredObject, PortError>;

    fn store_original_reader(&self, reader: &mut dyn Read) -> Result<StoredObject, PortError>;

    fn store_original_bytes(&self, bytes: &[u8]) -> Result<StoredObject, PortError> {
        let mut reader = Cursor::new(bytes);
        self.store_original_reader(&mut reader)
    }
}

pub trait CatalogPort {
    fn persist_asset(&self, record: PersistAsset) -> Result<ImportedAsset, PortError>;

    fn persist_review_item(&self, item: NewReviewItem) -> Result<(), PortError>;

    fn stage_review_item_and_complete_work(
        &self,
        _item: NewReviewItem,
        _work_key: &str,
    ) -> Result<bool, PortError> {
        Ok(false)
    }

    fn list_review_items(&self) -> Result<Vec<ReviewItem>, PortError>;

    fn list_processable_review_items_for_run(
        &self,
        run_id: i64,
    ) -> Result<Vec<ReviewItem>, PortError> {
        Ok(self
            .list_review_items()?
            .into_iter()
            .filter(|item| {
                item.run_id == run_id
                    && matches!(
                        item.status,
                        ReviewStatus::Pending | ReviewStatus::Deferred | ReviewStatus::Accepted
                    )
            })
            .collect())
    }

    fn get_review_item(&self, review_item_id: i64) -> Result<Option<ReviewItem>, PortError> {
        Ok(self
            .list_review_items()?
            .into_iter()
            .find(|item| item.id == review_item_id))
    }

    fn find_review_item_by_candidate_identity(
        &self,
        candidate_identity: &str,
    ) -> Result<Option<ReviewItem>, PortError> {
        Ok(self
            .list_review_items()?
            .into_iter()
            .find(|item| item.candidate_identity == candidate_identity))
    }

    fn find_review_item_for_run_by_candidate_identity(
        &self,
        run_id: i64,
        candidate_identity: &str,
    ) -> Result<Option<ReviewItem>, PortError> {
        let items = self.list_review_items()?;
        Ok(items
            .iter()
            .find(|item| item.run_id == run_id && item.candidate_identity == candidate_identity)
            .cloned()
            .or_else(|| {
                items.into_iter().find(|item| {
                    item.candidate_identity == candidate_identity
                        && matches!(
                            item.status,
                            ReviewStatus::Accepted | ReviewStatus::Applied | ReviewStatus::Rejected
                        )
                })
            }))
    }

    fn claim_review_item_for_processing(
        &self,
        _review_item_id: i64,
    ) -> Result<Option<ReviewProcessingClaim>, PortError> {
        Err(PortError(
            "catalog does not support review processing claims".to_owned(),
        ))
    }

    fn renew_review_item_processing(
        &self,
        _review_item_id: i64,
        _lease_token: &str,
    ) -> Result<bool, PortError> {
        Err(PortError(
            "catalog does not support renewing review processing claims".to_owned(),
        ))
    }

    fn recover_expired_review_processing(&self, _run_id: i64) -> Result<(), PortError> {
        Ok(())
    }

    fn finalize_review_processing_asset(
        &self,
        _review_item_id: i64,
        _lease_token: &str,
        _run_id: i64,
        _work_key: &str,
        _record: PersistAsset,
    ) -> Result<ReviewProcessingFinalization, PortError> {
        Err(PortError(
            "catalog does not support atomic review processing finalization".to_owned(),
        ))
    }

    fn finalize_accepted_review_asset(
        &self,
        _review_item_id: i64,
        _run_id: i64,
        _work_key: &str,
        _record: PersistAsset,
    ) -> Result<ImportedAsset, PortError> {
        Err(PortError(
            "catalog does not support atomic accepted review finalization".to_owned(),
        ))
    }

    fn refresh_review_processing_and_complete_work(
        &self,
        _review_item_id: i64,
        _lease_token: &str,
        _item: NewReviewItem,
        _work_key: &str,
        _status: ReviewStatus,
    ) -> Result<ReviewItem, PortError> {
        Err(PortError(
            "catalog does not support atomic review refresh finalization".to_owned(),
        ))
    }

    fn supersede_review_processing_and_complete_work(
        &self,
        _review_item_id: i64,
        _lease_token: &str,
        _run_id: i64,
        _work_key: &str,
    ) -> Result<Option<ReviewItem>, PortError> {
        Err(PortError(
            "catalog does not support atomic review supersession".to_owned(),
        ))
    }

    fn finish_review_item_processing(
        &self,
        _review_item_id: i64,
        _lease_token: &str,
        _status: ReviewStatus,
    ) -> Result<Option<ReviewItem>, PortError> {
        Err(PortError(
            "catalog does not support completing review processing".to_owned(),
        ))
    }

    fn restore_review_item_processing(
        &self,
        _review_item_id: i64,
        _lease_token: &str,
        _status: ReviewStatus,
    ) -> Result<Option<ReviewItem>, PortError> {
        Err(PortError(
            "catalog does not support restoring review processing".to_owned(),
        ))
    }

    fn set_review_decision(
        &self,
        _review_item_id: i64,
        _decision: ReviewDecision,
    ) -> Result<Option<ReviewItem>, PortError> {
        Err(PortError(
            "catalog does not support review decision persistence".to_owned(),
        ))
    }

    fn accept_review_item_and_requeue(
        &self,
        _review_item_id: i64,
        _release_edition_id: i64,
        _work_key: &str,
    ) -> Result<Option<ReviewItem>, PortError> {
        Err(PortError(
            "catalog does not support atomic review acceptance".to_owned(),
        ))
    }

    fn set_review_status(
        &self,
        _review_item_id: i64,
        _status: ReviewStatus,
    ) -> Result<Option<ReviewItem>, PortError> {
        Err(PortError(
            "catalog does not support review status persistence".to_owned(),
        ))
    }

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError>;
}

pub trait RunRepositoryPort {
    fn create_run(&self, request: AcquisitionRequest) -> Result<AcquisitionRun, PortError>;

    fn get_run(&self, run_id: i64) -> Result<Option<AcquisitionRun>, PortError>;

    fn list_runs(&self) -> Result<Vec<AcquisitionRun>, PortError>;

    fn queue_work(&self, run_id: i64, work_key: String) -> Result<(), PortError>;

    fn requeue_completed_work(&self, _run_id: i64, _work_key: &str) -> Result<(), PortError> {
        Err(PortError(
            "run repository does not support requeuing completed work".to_owned(),
        ))
    }

    fn next_queued_work(&self, run_id: i64) -> Result<Option<AcquisitionWorkItem>, PortError>;

    fn complete_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError>;

    fn compare_and_set_run_status(
        &self,
        run_id: i64,
        expected: AcquisitionRunStatus,
        target: AcquisitionRunStatus,
    ) -> Result<bool, PortError>;
}

pub trait ConnectorPort {
    fn source_id(&self) -> &'static str;

    fn capabilities(&self) -> ConnectorCapabilities;

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError>;

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError>;
}

pub trait ReferenceCatalogSourcePort {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportLocalBoxFrontRequest {
    pub existing_game_id: Option<i64>,
    pub game_title: String,
    pub platform: String,
    pub region: String,
    pub edition_name: String,
    pub source_path: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReferenceCatalogRequest {
    pub source_path: PathBuf,
    pub max_games: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceImportSummary {
    pub imported_releases: usize,
}

pub fn build_acquisition_request(
    input: AcquisitionRequestInput,
) -> Result<AcquisitionRequest, AcquisitionRequestValidationError> {
    AcquisitionRequest::try_from_draft(input)
}

pub fn start_acquisition_run(
    runs: &dyn RunRepositoryPort,
    input: AcquisitionRequestInput,
) -> Result<AcquisitionRun, ApplicationError> {
    let request = build_acquisition_request(input)?;
    Ok(runs.create_run(request)?)
}

pub fn load_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    runs.get_run(run_id)?
        .ok_or(ApplicationError::RunNotFound(run_id))
}

pub fn list_acquisition_runs(
    runs: &dyn RunRepositoryPort,
) -> Result<Vec<AcquisitionRun>, ApplicationError> {
    Ok(runs.list_runs()?)
}

pub fn queue_acquisition_work(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
    work_key: String,
) -> Result<(), ApplicationError> {
    let run = load_acquisition_run(runs, run_id)?;
    if matches!(
        run.status,
        AcquisitionRunStatus::Cancelled | AcquisitionRunStatus::Completed
    ) {
        return Err(ApplicationError::RunNotAcceptingWork { status: run.status });
    }
    if work_key.trim().is_empty() {
        return Err(ApplicationError::InvalidWorkKey);
    }
    Ok(runs.queue_work(run_id, work_key)?)
}

pub fn next_acquisition_work(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<Option<AcquisitionWorkItem>, ApplicationError> {
    let run = load_acquisition_run(runs, run_id)?;
    if run.status != AcquisitionRunStatus::Running {
        return Ok(None);
    }
    Ok(runs.next_queued_work(run_id)?)
}

pub fn complete_acquisition_work(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
    work_key: &str,
) -> Result<(), ApplicationError> {
    load_acquisition_run(runs, run_id)?;
    Ok(runs.complete_work(run_id, work_key)?)
}

pub fn pause_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    transition_acquisition_run(runs, run_id, AcquisitionRunStatus::Paused)
}

pub fn resume_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    transition_acquisition_run(runs, run_id, AcquisitionRunStatus::Running)
}

pub fn cancel_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    transition_acquisition_run(runs, run_id, AcquisitionRunStatus::Cancelled)
}

pub fn complete_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    transition_acquisition_run(runs, run_id, AcquisitionRunStatus::Completed)
}

fn transition_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
    target: AcquisitionRunStatus,
) -> Result<AcquisitionRun, ApplicationError> {
    let mut current = load_acquisition_run(runs, run_id)?;
    loop {
        if current.status == target {
            return Ok(current);
        }
        if target == AcquisitionRunStatus::Completed && current.queued_work != 0 {
            return Err(ApplicationError::RunHasQueuedWork {
                queued_work: current.queued_work,
            });
        }

        let allowed = matches!(
            (current.status, target),
            (AcquisitionRunStatus::Running, AcquisitionRunStatus::Paused)
                | (AcquisitionRunStatus::Paused, AcquisitionRunStatus::Running)
                | (
                    AcquisitionRunStatus::Running,
                    AcquisitionRunStatus::Cancelled
                )
                | (
                    AcquisitionRunStatus::Paused,
                    AcquisitionRunStatus::Cancelled
                )
                | (
                    AcquisitionRunStatus::Running,
                    AcquisitionRunStatus::Completed
                )
        );
        if !allowed {
            return Err(ApplicationError::InvalidRunTransition {
                from: current.status,
                to: target,
            });
        }

        if runs.compare_and_set_run_status(run_id, current.status, target)? {
            return load_acquisition_run(runs, run_id);
        }
        current = load_acquisition_run(runs, run_id)?;
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApplicationError {
    #[error("source path does not contain a file name")]
    MissingSourceFileName,
    #[error("failed to resolve source path: {0}")]
    ResolveSourcePath(String),
    #[error("reference catalog imports require a positive game limit")]
    InvalidReferenceImportLimit,
    #[error("{0}")]
    Port(#[from] PortError),
    #[error("{0}")]
    Validation(#[from] AcquisitionRequestValidationError),
    #[error("{0}")]
    InvalidMatchingPolicy(#[from] MatchingPolicyValidationError),
    #[error("acquisition run #{0} does not exist")]
    RunNotFound(i64),
    #[error("acquisition work key must not be blank")]
    InvalidWorkKey,
    #[error("acquisition run cannot accept new work while {status:?}")]
    RunNotAcceptingWork { status: AcquisitionRunStatus },
    #[error("acquisition run still has {queued_work} queued work item(s)")]
    RunHasQueuedWork { queued_work: u64 },
    #[error("cannot transition acquisition run from {from:?} to {to:?}")]
    InvalidRunTransition {
        from: AcquisitionRunStatus,
        to: AcquisitionRunStatus,
    },
    #[error("connector {source_id} is not selected by this acquisition request")]
    ConnectorNotSelected { source_id: String },
    #[error("connector {source_id} does not support direct media downloads")]
    ConnectorCannotDownload { source_id: String },
    #[error("acquisition run cannot execute connector work while {status:?}")]
    RunNotExecutable { status: AcquisitionRunStatus },
    #[error("connector {source_id} cannot execute this acquisition plan: {reason}")]
    UnsupportedConnectorPlan { source_id: String, reason: String },
    #[error("review item #{0} does not exist")]
    ReviewItemNotFound(i64),
    #[error(
        "release edition #{release_edition_id} is not a competing release for review item #{review_item_id}"
    )]
    ReviewAcceptanceNotCompeting {
        review_item_id: i64,
        release_edition_id: i64,
    },
    #[error("review item #{review_item_id} cannot be resolved while {status:?}")]
    ReviewItemNotActionable {
        review_item_id: i64,
        status: ReviewStatus,
    },
}

pub fn list_review_items(catalog: &dyn CatalogPort) -> Result<Vec<ReviewItem>, ApplicationError> {
    let items = catalog.list_review_items()?;
    let processing_run_ids = items
        .iter()
        .filter(|item| item.status == ReviewStatus::Processing)
        .map(|item| item.run_id)
        .collect::<std::collections::BTreeSet<_>>();
    if processing_run_ids.is_empty() {
        return Ok(items);
    }
    for run_id in processing_run_ids {
        catalog.recover_expired_review_processing(run_id)?;
    }
    Ok(catalog.list_review_items()?)
}

pub fn resolve_review_item(
    catalog: &dyn CatalogPort,
    review_item_id: i64,
    decision: ReviewDecision,
) -> Result<ReviewItem, ApplicationError> {
    let mut item = catalog
        .get_review_item(review_item_id)?
        .ok_or(ApplicationError::ReviewItemNotFound(review_item_id))?;
    if item.status == ReviewStatus::Processing {
        catalog.recover_expired_review_processing(item.run_id)?;
        item = catalog
            .get_review_item(review_item_id)?
            .ok_or(ApplicationError::ReviewItemNotFound(review_item_id))?;
    }
    if !matches!(item.status, ReviewStatus::Pending | ReviewStatus::Deferred) {
        return Err(ApplicationError::ReviewItemNotActionable {
            review_item_id,
            status: item.status,
        });
    }
    if let ReviewDecision::Accept { release_edition_id } = decision
        && !item
            .competing_matches
            .iter()
            .any(|candidate| candidate.release_edition_id == release_edition_id)
    {
        return Err(ApplicationError::ReviewAcceptanceNotCompeting {
            review_item_id,
            release_edition_id,
        });
    }

    match decision {
        ReviewDecision::Accept { release_edition_id } => {
            let work_key = connector_work_key(item.candidate.source_id.as_str(), &item.candidate);
            catalog
                .accept_review_item_and_requeue(review_item_id, release_edition_id, &work_key)?
                .ok_or(ApplicationError::ReviewItemNotFound(review_item_id))
        }
        decision => catalog
            .set_review_decision(review_item_id, decision)?
            .ok_or(ApplicationError::ReviewItemNotFound(review_item_id)),
    }
}

pub fn import_reference_catalog(
    catalog: &dyn ReferenceCatalogRepositoryPort,
    source: &dyn ReferenceCatalogSourcePort,
    request: ImportReferenceCatalogRequest,
) -> Result<ReferenceImportSummary, ApplicationError> {
    if request.max_games == 0 {
        return Err(ApplicationError::InvalidReferenceImportLimit);
    }

    let source_path = resolve_source_path(&request.source_path)?;
    let releases = source.read_releases(&source_path, request.max_games)?;
    let mut imported_releases = 0;
    let mut batch = Vec::with_capacity(REFERENCE_IMPORT_BATCH_SIZE);
    for release in releases.into_iter().take(request.max_games) {
        batch.push(release);
        if batch.len() == REFERENCE_IMPORT_BATCH_SIZE {
            imported_releases += catalog
                .persist_reference_releases(std::mem::take(&mut batch))?
                .len();
        }
    }
    if !batch.is_empty() {
        imported_releases += catalog.persist_reference_releases(batch)?.len();
    }

    Ok(ReferenceImportSummary { imported_releases })
}

const REFERENCE_IMPORT_BATCH_SIZE: usize = 256;

pub fn acquire_run_with_connector(
    runs: &dyn RunRepositoryPort,
    catalog: &dyn CatalogPort,
    object_store: &dyn ObjectStorePort,
    connector: &dyn ConnectorPort,
    run_id: i64,
    matching_policy: MatchingPolicy,
) -> Result<Vec<ImportedAsset>, ApplicationError> {
    let matching_policy = matching_policy.validate()?;
    let mut run = load_acquisition_run(runs, run_id)?;
    if !matches!(
        run.status,
        AcquisitionRunStatus::Running | AcquisitionRunStatus::Completed
    ) {
        return Err(ApplicationError::RunNotExecutable { status: run.status });
    }
    if !run.request.selects_source(connector.source_id()) {
        return Err(ApplicationError::ConnectorNotSelected {
            source_id: connector.source_id().to_owned(),
        });
    }

    let capabilities = connector.capabilities();
    if !capabilities.direct_media_download {
        return Err(ApplicationError::ConnectorCannotDownload {
            source_id: connector.source_id().to_owned(),
        });
    }
    validate_connector_plan(&run.request, connector.source_id(), &capabilities)?;
    catalog.recover_expired_review_processing(run_id)?;
    let review_items = catalog.list_processable_review_items_for_run(run_id)?;
    if run.status == AcquisitionRunStatus::Completed {
        let mut requeued = false;
        for review_item in review_items.iter().filter(|review_item| {
            review_item.run_id == run_id
                && review_item.candidate.source_id.as_str() == connector.source_id()
                && matches!(
                    review_item.status,
                    ReviewStatus::Pending | ReviewStatus::Deferred | ReviewStatus::Accepted
                )
        }) {
            let work_key = connector_work_key(connector.source_id(), &review_item.candidate);
            runs.requeue_completed_work(run_id, &work_key)?;
            requeued = true;
        }
        if !requeued {
            return Ok(Vec::new());
        }
        run = load_acquisition_run(runs, run_id)?;
    }
    let releases = catalog.list_library()?;

    let mut candidates_by_work_key = std::collections::HashMap::new();
    let mut persisted_work_key_by_identity = std::collections::HashMap::new();
    for review_item in &review_items {
        if review_item.run_id != run_id
            || review_item.candidate.source_id.as_str() != connector.source_id()
        {
            continue;
        }
        let work_key = connector_work_key(connector.source_id(), &review_item.candidate);
        persisted_work_key_by_identity
            .insert(review_item.candidate_identity.clone(), work_key.clone());
        candidates_by_work_key.insert(work_key, review_item.candidate.clone());
    }
    let review_only_resume = run.queued_work > 0
        && !persisted_work_key_by_identity.is_empty()
        && run.queued_work as usize <= persisted_work_key_by_identity.len();

    let discovery_error = match connector.discover(&run.request) {
        Ok(candidates) => {
            for candidate in candidates {
                if !run.request.requests_asset_type(candidate.asset_type)
                    || !capabilities.asset_types.contains(&candidate.asset_type)
                {
                    continue;
                }
                let candidate_identity =
                    review_candidate_identity(connector.source_id(), &candidate);
                if review_only_resume
                    && let Some(work_key) = persisted_work_key_by_identity.get(&candidate_identity)
                {
                    candidates_by_work_key.insert(work_key.clone(), candidate);
                    continue;
                }
                let work_key = connector_work_key(connector.source_id(), &candidate);
                queue_acquisition_work(runs, run_id, work_key.clone())?;
                candidates_by_work_key.insert(work_key, candidate);
            }
            None
        }
        Err(error) if !candidates_by_work_key.is_empty() => Some(error),
        Err(error) => return Err(error.into()),
    };

    let mut imported_assets = Vec::new();
    while let Some(work) = next_acquisition_work(runs, run_id)? {
        let Some(candidate) = candidates_by_work_key.get(&work.key) else {
            if let Some(error) = discovery_error.as_ref() {
                return Err(error.clone().into());
            }
            break;
        };
        let candidate_identity = review_candidate_identity(connector.source_id(), candidate);
        let existing_review_item =
            catalog.find_review_item_for_run_by_candidate_identity(run_id, &candidate_identity)?;
        if existing_review_item
            .as_ref()
            .is_some_and(|item| item.status == ReviewStatus::Processing)
        {
            break;
        }
        if existing_review_item.as_ref().is_some_and(|item| {
            item.run_id == run_id
                && matches!(
                    item.status,
                    ReviewStatus::Applied | ReviewStatus::AutoResolved | ReviewStatus::Superseded
                )
        }) {
            complete_acquisition_work(runs, run_id, &work.key)?;
            continue;
        }
        let review_processing = if let Some(review_item) = existing_review_item.as_ref()
            && matches!(
                review_item.status,
                ReviewStatus::Pending | ReviewStatus::Deferred
            ) {
            let Some(claimed) = catalog.claim_review_item_for_processing(review_item.id)? else {
                continue;
            };
            let previous_status = if claimed.item.decision == Some(ReviewDecision::Defer) {
                ReviewStatus::Deferred
            } else {
                ReviewStatus::Pending
            };
            Some((review_item.id, previous_status, claimed.lease_token))
        } else {
            None
        };
        let accepted_review_item_id = existing_review_item
            .as_ref()
            .filter(|item| item.run_id == run_id && item.status == ReviewStatus::Accepted)
            .map(|item| item.id);
        let reviewed_match = if let Some(review_item) = existing_review_item {
            match review_item.decision {
                Some(ReviewDecision::Accept { release_edition_id }) => {
                    let accepted = review_item
                        .competing_matches
                        .iter()
                        .find(|candidate| candidate.release_edition_id == release_edition_id)
                        .ok_or(ApplicationError::ReviewAcceptanceNotCompeting {
                            review_item_id: review_item.id,
                            release_edition_id,
                        })?;
                    Some(AssetCandidateMatch {
                        release_edition_id: Some(release_edition_id),
                        score: accepted.score,
                        confidence: MatchConfidence::Medium,
                        evidence: accepted.evidence.clone(),
                    })
                }
                Some(ReviewDecision::Reject) => {
                    complete_acquisition_work(runs, run_id, &work.key)?;
                    continue;
                }
                Some(ReviewDecision::Defer) | None => None,
            }
        } else {
            None
        };
        let reviewed_release_edition_id = reviewed_match
            .as_ref()
            .and_then(|candidate_match| candidate_match.release_edition_id);
        let candidate_match = reviewed_match.unwrap_or_else(|| {
            match_asset_candidate_to_release(candidate, &releases, matching_policy)
        });
        if candidate_match.confidence == MatchConfidence::Medium
            && reviewed_release_edition_id.is_none()
        {
            let item = NewReviewItem {
                run_id,
                candidate_identity: candidate_identity.clone(),
                candidate: candidate.clone(),
                competing_matches: review_matches_for_asset_candidate(
                    candidate,
                    &releases,
                    matching_policy,
                ),
            };
            if review_processing.is_none() {
                if !catalog.stage_review_item_and_complete_work(item.clone(), &work.key)? {
                    catalog.persist_review_item(item)?;
                    complete_acquisition_work(runs, run_id, &work.key)?;
                }
            } else {
                let (review_item_id, previous_status, lease_token) = review_processing
                    .as_ref()
                    .expect("processing review checked above");
                let refreshed = catalog.refresh_review_processing_and_complete_work(
                    *review_item_id,
                    lease_token,
                    item,
                    &work.key,
                    *previous_status,
                )?;
                if refreshed.status != ReviewStatus::Accepted {
                    complete_acquisition_work(runs, run_id, &work.key)?;
                }
            }
            if review_processing.is_none()
                && catalog
                    .find_review_item_for_run_by_candidate_identity(run_id, &candidate_identity)?
                    .is_some_and(|item| item.status == ReviewStatus::Accepted)
            {
                runs.requeue_completed_work(run_id, &work.key)?;
            }
            continue;
        }
        let Some(release_edition_id) =
            reviewed_release_edition_id.or_else(|| candidate_match.auto_link_release_edition_id())
        else {
            if let Some((review_item_id, _, lease_token)) = review_processing {
                catalog.supersede_review_processing_and_complete_work(
                    review_item_id,
                    &lease_token,
                    run_id,
                    &work.key,
                )?;
            }
            complete_acquisition_work(runs, run_id, &work.key)?;
            continue;
        };
        let Some(release) = releases
            .iter()
            .find(|release| release.release_edition_id == release_edition_id)
        else {
            if let Some((review_item_id, _, lease_token)) = review_processing {
                catalog.supersede_review_processing_and_complete_work(
                    review_item_id,
                    &lease_token,
                    run_id,
                    &work.key,
                )?;
            }
            complete_acquisition_work(runs, run_id, &work.key)?;
            continue;
        };
        let import_result = (|| -> Result<ReviewProcessingFinalization, ApplicationError> {
            let stream = connector.download(candidate)?;
            let stored = if let Some((review_item_id, _, lease_token)) = review_processing.as_ref()
            {
                ensure_review_processing_lease(catalog, *review_item_id, lease_token)?;
                let mut stream =
                    ReviewLeaseReader::new(stream, catalog, *review_item_id, lease_token);
                let stored = object_store.store_original_reader(&mut stream)?;
                ensure_review_processing_lease(catalog, *review_item_id, lease_token)?;
                stored
            } else {
                let mut stream = stream;
                object_store.store_original_reader(stream.as_mut())?
            };
            let record = PersistAsset {
                existing_game_id: Some(release.game_id),
                existing_release_edition_id: Some(release.release_edition_id),
                match_decision: Some(candidate_match),
                game_title: candidate.game_title.clone(),
                platform: candidate.platform.clone(),
                region: candidate.region.clone(),
                edition_name: candidate.edition_name.clone(),
                asset_type: candidate.asset_type,
                object_hash: stored.hash,
                byte_len: stored.byte_len,
                original_filename: candidate.original_filename.clone(),
                source_id: candidate.source_id.clone(),
                source_asset_label: candidate.source_asset_label.clone(),
                source_location: candidate.source_url.clone(),
            };
            if let Some((review_item_id, _, lease_token)) = review_processing.as_ref() {
                Ok(catalog.finalize_review_processing_asset(
                    *review_item_id,
                    lease_token,
                    run_id,
                    &work.key,
                    record,
                )?)
            } else if let Some(review_item_id) = accepted_review_item_id {
                Ok(ReviewProcessingFinalization::Imported(
                    catalog.finalize_accepted_review_asset(
                        review_item_id,
                        run_id,
                        &work.key,
                        record,
                    )?,
                ))
            } else {
                Ok(ReviewProcessingFinalization::Imported(
                    catalog.persist_asset(record)?,
                ))
            }
        })();
        let finalization = match import_result {
            Ok(finalization) => finalization,
            Err(error) => {
                if let Some((review_item_id, previous_status, lease_token)) = review_processing {
                    catalog.restore_review_item_processing(
                        review_item_id,
                        &lease_token,
                        previous_status,
                    )?;
                }
                return Err(error);
            }
        };
        match finalization {
            ReviewProcessingFinalization::Imported(imported) => {
                complete_acquisition_work(runs, run_id, &work.key)?;
                imported_assets.push(imported);
            }
            ReviewProcessingFinalization::Requeued => continue,
            ReviewProcessingFinalization::Discarded => {
                complete_acquisition_work(runs, run_id, &work.key)?;
                continue;
            }
        }
    }

    if let Some(error) = discovery_error
        && !review_only_resume
    {
        return Err(error.into());
    }
    runs.compare_and_set_run_status(
        run_id,
        AcquisitionRunStatus::Running,
        AcquisitionRunStatus::Completed,
    )?;

    Ok(imported_assets)
}

fn ensure_review_processing_lease(
    catalog: &dyn CatalogPort,
    review_item_id: i64,
    lease_token: &str,
) -> Result<(), ApplicationError> {
    if catalog.renew_review_item_processing(review_item_id, lease_token)? {
        Ok(())
    } else {
        Err(PortError(format!(
            "review item #{review_item_id} lost its processing lease"
        ))
        .into())
    }
}

fn connector_work_key(source_id: &str, candidate: &AssetCandidate) -> String {
    let mut key = "connector".to_owned();
    if let Some(provider_candidate_id) = candidate.provider_candidate_id.as_deref() {
        for part in [
            source_id,
            "provider_candidate_id",
            provider_candidate_id,
            asset_type_work_key(candidate.asset_type),
        ] {
            push_work_key_part(&mut key, part);
        }
        return key;
    }
    for part in [
        source_id,
        candidate.platform.as_str(),
        candidate.game_title.as_str(),
        candidate.region.as_str(),
        candidate.edition_name.as_str(),
        asset_type_work_key(candidate.asset_type),
        candidate.source_url.as_str(),
    ] {
        push_work_key_part(&mut key, part);
    }
    key
}

fn review_candidate_identity(source_id: &str, candidate: &AssetCandidate) -> String {
    let mut identity = "candidate".to_owned();
    if let Some(provider_candidate_id) = candidate.provider_candidate_id.as_deref() {
        for part in [
            source_id,
            provider_candidate_id,
            asset_type_work_key(candidate.asset_type),
        ] {
            push_work_key_part(&mut identity, part);
        }
        return identity;
    }
    for part in [
        source_id,
        normalize_review_identity_part(candidate.platform.as_str()).as_str(),
        normalize_review_identity_part(candidate.game_title.as_str()).as_str(),
        normalize_review_identity_part(candidate.region.as_str()).as_str(),
        normalize_review_identity_part(candidate.edition_name.as_str()).as_str(),
        asset_type_work_key(candidate.asset_type),
        candidate
            .source_asset_label
            .as_deref()
            .unwrap_or(candidate.original_filename.as_str()),
        candidate.source_url.as_str(),
    ] {
        push_work_key_part(&mut identity, part);
    }
    identity
}

fn normalize_review_identity_part(value: &str) -> String {
    value.trim().to_lowercase()
}

fn push_work_key_part(key: &mut String, value: &str) {
    key.push(':');
    key.push_str(&value.len().to_string());
    key.push(':');
    key.push_str(value);
}

fn asset_type_work_key(asset_type: AssetType) -> &'static str {
    match asset_type {
        AssetType::BoxFront => "box_front",
    }
}

fn validate_connector_plan(
    request: &AcquisitionRequest,
    source_id: &str,
    capabilities: &ConnectorCapabilities,
) -> Result<(), ApplicationError> {
    let unsupported = |reason: &str| ApplicationError::UnsupportedConnectorPlan {
        source_id: source_id.to_owned(),
        reason: reason.to_owned(),
    };

    if !request.selects_only_source(source_id) {
        return Err(unsupported(
            "this execution path requires one explicitly selected source",
        ));
    }
    if !request.requested_asset_types_supported_by(&capabilities.asset_types) {
        return Err(unsupported(
            "one or more requested asset types are not supported by this connector",
        ));
    }
    if request.quality().is_some() {
        return Err(unsupported(
            "quality requirements are not supported by this execution path",
        ));
    }
    if request.retention() != RetentionPolicy::KeepEverything {
        return Err(unsupported(
            "Keep Best Per Type is not supported by this execution path",
        ));
    }
    if request.limits() != &game_media_vault_domain::AcquisitionLimits::default() {
        return Err(unsupported(
            "acquisition limits are not supported by this execution path",
        ));
    }
    Ok(())
}

pub fn import_local_box_front(
    catalog: &dyn CatalogPort,
    object_store: &dyn ObjectStorePort,
    request: ImportLocalBoxFrontRequest,
) -> Result<ImportedAsset, ApplicationError> {
    let original_filename = request
        .source_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or(ApplicationError::MissingSourceFileName)?;
    let resolved_source_path = resolve_source_path(&request.source_path)?;
    let source_location = source_location(&resolved_source_path);
    let stored = object_store.store_original(&resolved_source_path)?;

    Ok(catalog.persist_asset(PersistAsset {
        existing_game_id: request.existing_game_id,
        existing_release_edition_id: None,
        match_decision: None,
        game_title: request.game_title,
        platform: request.platform,
        region: request.region,
        edition_name: request.edition_name,
        asset_type: AssetType::BoxFront,
        object_hash: stored.hash,
        byte_len: stored.byte_len,
        original_filename,
        source_id: SourceId::from("local_import"),
        source_asset_label: None,
        source_location,
    })?)
}

fn resolve_source_path(path: &Path) -> Result<PathBuf, ApplicationError> {
    fs::canonicalize(path).map_err(|error| ApplicationError::ResolveSourcePath(error.to_string()))
}

fn source_location(path: &Path) -> String {
    let location = path.to_string_lossy();

    #[cfg(windows)]
    {
        if let Some(network_path) = location.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{network_path}");
        }
        if let Some(local_path) = location.strip_prefix(r"\\?\") {
            return local_path.to_owned();
        }
    }

    location.into_owned()
}

pub fn list_library(catalog: &dyn CatalogPort) -> Result<Vec<LibraryEntry>, ApplicationError> {
    Ok(catalog.list_library()?)
}
