use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRun,
    AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate, AssetCandidateMatch,
    ConnectorCapabilities, ImportedAsset, LibraryEntry, MatchConfidence, MatchingPolicy,
    NewReviewItem, PersistAsset, RetentionPolicy, ReviewDecision, ReviewItem, ReviewStatus,
    StoredObject, ValidatedMatchingPolicy, confirmed_asset_candidate_match,
    match_asset_candidate_to_release, review_matches_for_asset_candidate,
};
use url::Url;

use crate::{
    ApplicationError, CatalogPort, ConnectorPort, ObjectStorePort, ParkedReview,
    ReviewRepositoryPort, RunRepositoryPort, build_acquisition_request, candidate_identity,
    load_acquisition_run,
};

/// A concurrent human decision can close a Review Item between reading it and writing the
/// automatic outcome. Human decisions are final, so a second attempt always settles.
const REVIEW_RACE_ATTEMPTS: usize = 3;

struct Acquisition<'a> {
    runs: &'a dyn RunRepositoryPort,
    catalog: &'a dyn CatalogPort,
    reviews: &'a dyn ReviewRepositoryPort,
    object_store: &'a dyn ObjectStorePort,
    connector: &'a dyn ConnectorPort,
    run_id: i64,
    matching_policy: ValidatedMatchingPolicy,
    releases: Vec<LibraryEntry>,
}

enum Step {
    Done(Option<ImportedAsset>),
    Retry,
}

/// Executes a running Acquisition Run with one Connector.
///
/// The Source is discovered once per run: its candidates are persisted as queued work, so
/// resuming never rediscovers. Each queued candidate is matched against the library and is
/// either imported, left unattached, or parked on its Review Item until a human decides.
pub fn acquire_run_with_connector(
    runs: &dyn RunRepositoryPort,
    reviews: &dyn ReviewRepositoryPort,
    catalog: &dyn CatalogPort,
    object_store: &dyn ObjectStorePort,
    connector: &dyn ConnectorPort,
    run_id: i64,
    matching_policy: MatchingPolicy,
) -> Result<Vec<ImportedAsset>, ApplicationError> {
    let matching_policy = matching_policy.validate()?;
    let run = load_acquisition_run(runs, run_id)?;
    match run.status {
        AcquisitionRunStatus::Running => {
            let capabilities = validate_connector_plan(&run.request, connector)?;
            if !runs.has_discovered(run_id, connector.source_id())? {
                let work = discover_work(&run.request, connector, &capabilities)?;
                // A cancellation or completion that won the race while discovering stops quietly.
                if !runs.record_discovery(run_id, connector.source_id(), &work)? {
                    return Ok(Vec::new());
                }
            }
        }
        // Nothing is left to discover, but an acceptance may reopen the run right after this
        // read; the drain below executes whatever work it requeued.
        AcquisitionRunStatus::Completed => {}
        status => return Err(ApplicationError::RunNotExecutable { status }),
    }

    let acquisition = Acquisition {
        runs,
        catalog,
        reviews,
        object_store,
        connector,
        run_id,
        matching_policy,
        releases: catalog.list_library()?,
    };
    let mut imported_assets = Vec::new();
    loop {
        while let Some(work) = runs.next_queued_work(run_id)? {
            if let Some(imported) = acquisition.process(&work)? {
                imported_assets.push(imported);
            }
        }
        if runs.compare_and_set_run_status(
            run_id,
            AcquisitionRunStatus::Running,
            AcquisitionRunStatus::Completed,
        )? {
            break;
        }
        // A pause or cancellation that won the race keeps its status; otherwise a decision
        // requeued work after the queue looked empty, and it is executed too.
        if load_acquisition_run(runs, run_id)?.status != AcquisitionRunStatus::Running {
            break;
        }
    }
    Ok(imported_assets)
}

/// Starts an Acquisition Run only if `connector` can execute it, so runs that would fail on
/// every execution are never persisted.
pub fn start_acquisition_run_for_connector(
    runs: &dyn RunRepositoryPort,
    input: AcquisitionRequestDraft,
    connector: &dyn ConnectorPort,
) -> Result<AcquisitionRun, ApplicationError> {
    let request = build_acquisition_request(input)?;
    validate_connector_plan(&request, connector)?;
    Ok(runs.create_run(request)?)
}

fn validate_connector_plan(
    request: &AcquisitionRequest,
    connector: &dyn ConnectorPort,
) -> Result<ConnectorCapabilities, ApplicationError> {
    let source_id = connector.source_id();
    if !request.selects_source(source_id) {
        return Err(ApplicationError::ConnectorNotSelected {
            source_id: source_id.to_owned(),
        });
    }
    let capabilities = connector.capabilities();
    if !capabilities.direct_media_download {
        return Err(ApplicationError::ConnectorCannotDownload {
            source_id: source_id.to_owned(),
        });
    }
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
    if request.limits() != &AcquisitionLimits::default() {
        return Err(unsupported(
            "acquisition limits are not supported by this execution path",
        ));
    }
    if let Some(reason) = connector.unsupported_request_reason(request)? {
        return Err(unsupported(&reason));
    }
    Ok(capabilities)
}

fn discover_work(
    request: &AcquisitionRequest,
    connector: &dyn ConnectorPort,
    capabilities: &ConnectorCapabilities,
) -> Result<Vec<AcquisitionWorkItem>, ApplicationError> {
    let source_id = connector.source_id();
    let mut work = Vec::new();
    for candidate in connector.discover(request)? {
        if candidate.source_id.as_str() != source_id {
            return Err(ApplicationError::ConnectorCandidateSourceMismatch {
                connector_source_id: source_id.to_owned(),
                candidate_source_id: candidate.source_id.as_str().to_owned(),
            });
        }
        if !request.requests_asset_type(candidate.asset_type)
            || !capabilities.asset_types.contains(&candidate.asset_type)
        {
            continue;
        }
        ensure_catalog_safe_locator(source_id, &candidate)?;
        work.push(AcquisitionWorkItem {
            key: candidate_identity(source_id, &candidate),
            candidate,
        });
    }
    Ok(work)
}

fn ensure_catalog_safe_locator(
    source_id: &str,
    candidate: &AssetCandidate,
) -> Result<(), ApplicationError> {
    // Query strings and fragments are where API keys and signatures travel, so a stable
    // locator carries none of them; connectors add request parameters in `download`.
    let catalog_safe = Url::parse(&candidate.source_url).is_ok_and(|url| {
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    });
    if catalog_safe {
        Ok(())
    } else {
        Err(ApplicationError::UnsafeCandidateLocator {
            source_id: source_id.to_owned(),
        })
    }
}

impl Acquisition<'_> {
    fn process(
        &self,
        work: &AcquisitionWorkItem,
    ) -> Result<Option<ImportedAsset>, ApplicationError> {
        if work.candidate.source_id.as_str() != self.connector.source_id() {
            return Err(ApplicationError::ConnectorCandidateSourceMismatch {
                connector_source_id: self.connector.source_id().to_owned(),
                candidate_source_id: work.candidate.source_id.as_str().to_owned(),
            });
        }
        let mut contended_review_item_id = None;
        for _ in 0..REVIEW_RACE_ATTEMPTS {
            let review_item = self.reviews.find_review_item(&work.key)?;
            contended_review_item_id = review_item.as_ref().map(|item| item.id);
            if let Step::Done(imported) = self.try_process(work, review_item)? {
                return Ok(imported);
            }
        }
        Err(ApplicationError::ReviewItemContended(
            contended_review_item_id.unwrap_or_default(),
        ))
    }

    fn try_process(
        &self,
        work: &AcquisitionWorkItem,
        review_item: Option<ReviewItem>,
    ) -> Result<Step, ApplicationError> {
        match review_item
            .as_ref()
            .map(|item| (item.status, &item.decision))
        {
            Some((ReviewStatus::Accepted, Some(ReviewDecision::Accept { release_edition_id }))) => {
                let release = self.accepted_release(*release_edition_id)?;
                let candidate_match = confirmed_asset_candidate_match(&work.candidate, &release);
                return self.import(work, &release, candidate_match);
            }
            Some((ReviewStatus::Rejected, _)) => {
                self.runs.complete_work(self.run_id, &work.key)?;
                return Ok(Step::Done(None));
            }
            _ => {}
        }
        let candidate_match =
            match_asset_candidate_to_release(&work.candidate, &self.releases, self.matching_policy);
        match candidate_match.confidence {
            MatchConfidence::High | MatchConfidence::Confirmed => {
                // The matcher only links Release Editions taken from `self.releases`.
                let release =
                    self.release(candidate_match.release_edition_id.unwrap_or_default())?;
                self.import(work, release, candidate_match.clone())
            }
            MatchConfidence::Medium => {
                let review = NewReviewItem {
                    candidate_identity: work.key.clone(),
                    candidate: work.candidate.clone(),
                    competing_matches: review_matches_for_asset_candidate(
                        &work.candidate,
                        &self.releases,
                        self.matching_policy,
                    ),
                };
                match self
                    .reviews
                    .park_work_for_review(self.run_id, &work.key, review)?
                {
                    ParkedReview::Parked(_) | ParkedReview::Settled => Ok(Step::Done(None)),
                    ParkedReview::AlreadyDecided(_) => Ok(Step::Retry),
                }
            }
            MatchConfidence::Low => {
                if self
                    .reviews
                    .supersede_candidate_review(self.run_id, &work.key)?
                {
                    Ok(Step::Done(None))
                } else {
                    Ok(Step::Retry)
                }
            }
        }
    }

    fn release(&self, release_edition_id: i64) -> Result<&LibraryEntry, ApplicationError> {
        self.releases
            .iter()
            .find(|release| release.release_edition_id == release_edition_id)
            .ok_or(ApplicationError::ReleaseEditionMissing(release_edition_id))
    }

    /// The edition a human accepted, which another process may have imported after this
    /// execution read the library.
    fn accepted_release(&self, release_edition_id: i64) -> Result<LibraryEntry, ApplicationError> {
        if let Ok(release) = self.release(release_edition_id) {
            return Ok(release.clone());
        }
        self.catalog
            .list_library()?
            .into_iter()
            .find(|release| release.release_edition_id == release_edition_id)
            .ok_or(ApplicationError::ReleaseEditionMissing(release_edition_id))
    }

    /// Stores the candidate's original, links it to `release` and completes the work. The
    /// repository checks the candidate's Review Item in the same transaction; a conflicting
    /// human decision makes the caller retry with that decision.
    fn import(
        &self,
        work: &AcquisitionWorkItem,
        release: &LibraryEntry,
        candidate_match: AssetCandidateMatch,
    ) -> Result<Step, ApplicationError> {
        let stored = self.store_original(work)?;
        let record = self.asset_record(work, release, candidate_match, stored);
        match self
            .reviews
            .persist_candidate_asset(self.run_id, &work.key, record)?
        {
            Some(imported) => Ok(Step::Done(Some(imported))),
            None => Ok(Step::Retry),
        }
    }

    fn store_original(&self, work: &AcquisitionWorkItem) -> Result<StoredObject, ApplicationError> {
        let mut stream = self.connector.download(&work.candidate)?;
        Ok(self.object_store.store_original(stream.as_mut())?)
    }

    fn asset_record(
        &self,
        work: &AcquisitionWorkItem,
        release: &LibraryEntry,
        candidate_match: AssetCandidateMatch,
        stored: StoredObject,
    ) -> PersistAsset {
        let candidate = &work.candidate;
        PersistAsset {
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
        }
    }
}
