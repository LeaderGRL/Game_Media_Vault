use std::cell::Cell;

use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRun, AcquisitionRunStatus,
    AcquisitionWorkItem, AssetCandidate, AssetCandidateMatch, AssetType, ImportedAsset,
    LibraryEntry, MatchConfidence, MatchingPolicy, NewReviewItem, PersistAsset,
    QualityRequirements, RetentionPolicy, ReviewDecision, ReviewItem, ReviewStatus, StoredObject,
    ValidatedMatchingPolicy, confirmed_asset_candidate_match, match_asset_candidate_to_release,
    review_matches_for_asset_candidate,
};
use url::Url;

use crate::{
    ApplicationError, CandidateAssetOutcome, CatalogPort, ConnectorPort, ObjectStorePort,
    ParkedReview, ReviewRepositoryPort, RunRepositoryPort, build_acquisition_request,
    candidate_identity, load_acquisition_run,
    plan::{capable_asset_types, ensure_request_supported},
    plan_acquisition,
};

/// A concurrent human decision can close a Review Item between reading it and writing the
/// automatic outcome. Human decisions are final, so a second attempt always settles.
const REVIEW_RACE_ATTEMPTS: usize = 3;

struct Acquisition<'a> {
    runs: &'a dyn RunRepositoryPort,
    catalog: &'a dyn CatalogPort,
    reviews: &'a dyn ReviewRepositoryPort,
    object_store: &'a dyn ObjectStorePort,
    connectors: &'a [&'a dyn ConnectorPort],
    run_id: i64,
    matching_policy: ValidatedMatchingPolicy,
    releases: Vec<LibraryEntry>,
    quality: Option<QualityRequirements>,
    retention: RetentionPolicy,
    /// Whether the last failure came from the Source of the processed work, such as a failed
    /// download, rather than from the vault.
    source_failed: Cell<bool>,
}

enum Step {
    Done(Option<ImportedAsset>),
    Retry,
}

/// Executes a running Acquisition Run with the registered `connectors`, one per Source.
///
/// Only the Sources the run's plan kept when it started are contacted. Each is discovered once
/// per run: its candidates are persisted as queued work, so resuming never rediscovers it, and a
/// pause stops further discoveries. Sources progress independently: one without a registered
/// connector, that now refuses the plan, cannot be reached, fails to discover or fails to
/// download leaves the others' work to run, keeps the run running, and its error is returned
/// once that work is done. Each queued candidate is matched against the library through its own
/// Source's connector and is either imported, left unattached, or parked on its Review Item
/// until a human decides. The run completes only once every planned Source was discovered.
pub fn acquire_run_with_connectors(
    runs: &dyn RunRepositoryPort,
    reviews: &dyn ReviewRepositoryPort,
    catalog: &dyn CatalogPort,
    object_store: &dyn ObjectStorePort,
    connectors: &[&dyn ConnectorPort],
    run_id: i64,
    matching_policy: MatchingPolicy,
) -> Result<Vec<ImportedAsset>, ApplicationError> {
    let matching_policy = matching_policy.validate()?;
    let run = load_acquisition_run(runs, run_id)?;
    match run.status {
        AcquisitionRunStatus::Running => ensure_request_supported(&run.request)?,
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
        connectors,
        run_id,
        matching_policy,
        releases: catalog.list_library()?,
        quality: run.request.quality().cloned(),
        retention: run.request.retention(),
        source_failed: Cell::new(false),
    };
    let mut imported_assets = Vec::new();
    let mut source_failure = None;
    // Sources whose work failed; their remaining work waits for a later execution.
    let mut failed_sources: Vec<String> = Vec::new();
    loop {
        for source_id in &run.planned_sources {
            if runs.has_discovered(run_id, source_id)? {
                continue;
            }
            // A pause, even one landing during a failed discovery, keeps the snapshots already
            // recorded but discovers no further.
            if load_acquisition_run(runs, run_id)?.status != AcquisitionRunStatus::Running {
                break;
            }
            let discovered = planned_connector(&run.request, source_id, connectors).and_then(
                |(connector, asset_types)| discover_source(&run.request, connector, &asset_types),
            );
            match discovered {
                // A cancellation or completion that won the race while discovering stops
                // quietly.
                Ok(work) => {
                    if !runs.record_discovery(run_id, source_id, &work)? {
                        return Ok(imported_assets);
                    }
                }
                Err(error) => {
                    source_failure.get_or_insert(error);
                }
            }
        }
        while let Some(work) = runs.next_queued_work(run_id, &failed_sources)? {
            match acquisition.process(&work) {
                Ok(imported) => imported_assets.extend(imported),
                Err(error) if acquisition.source_failed.get() => {
                    failed_sources.push(work.candidate.source_id.as_str().to_owned());
                    source_failure.get_or_insert(error);
                }
                Err(error) => return Err(error),
            }
        }
        // A Source left undiscovered or failing keeps the run running for a later execution.
        if let Some(error) = source_failure.take() {
            return Err(error);
        }
        // A pause may have stopped the discoveries before a resume that this execution then
        // sees; the Sources it left are discovered on the next pass.
        let all_discovered = run
            .planned_sources
            .iter()
            .map(|source_id| runs.has_discovered(run_id, source_id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .all(|discovered| discovered);
        if all_discovered
            && runs.compare_and_set_run_status(
                run_id,
                AcquisitionRunStatus::Running,
                AcquisitionRunStatus::Completed,
            )?
        {
            break;
        }
        // A pause or cancellation that won the race keeps its status; otherwise a decision
        // requeued work after the queue looked empty, or a resume left Sources to discover,
        // and they are executed too.
        if load_acquisition_run(runs, run_id)?.status != AcquisitionRunStatus::Running {
            break;
        }
    }
    Ok(imported_assets)
}

/// The connector of a planned Source and the requested types it acquires, unless the Source has
/// no registered connector anymore or its capabilities no longer serve the request.
fn planned_connector<'a>(
    request: &AcquisitionRequest,
    source_id: &str,
    connectors: &[&'a dyn ConnectorPort],
) -> Result<(&'a dyn ConnectorPort, Vec<AssetType>), ApplicationError> {
    let unsupported = |reason: String| ApplicationError::UnsupportedConnectorPlan {
        source_id: source_id.to_owned(),
        reason,
    };
    let connector = connectors
        .iter()
        .copied()
        .find(|connector| connector.source_id() == source_id)
        .ok_or_else(|| unsupported("no connector is registered for this source".to_owned()))?;
    let asset_types = capable_asset_types(request, connector).map_err(unsupported)?;
    Ok((connector, asset_types))
}

/// Starts an Acquisition Run only if the registered `connectors` can plan it, so runs that
/// would fail on every execution are never persisted.
pub fn start_acquisition_run_with_connectors(
    runs: &dyn RunRepositoryPort,
    input: AcquisitionRequestDraft,
    connectors: &[&dyn ConnectorPort],
) -> Result<AcquisitionRun, ApplicationError> {
    let request = build_acquisition_request(input)?;
    let planned_sources = plan_acquisition(&request, connectors)?
        .sources
        .into_iter()
        .map(|source| source.source_id)
        .collect();
    Ok(runs.create_run(request, planned_sources)?)
}

/// Discovers the work of one Source once its connector accepts the plan, which may consult the
/// Source. Only Sources not yet discovered need it: a persisted snapshot executes without them.
fn discover_source(
    request: &AcquisitionRequest,
    connector: &dyn ConnectorPort,
    asset_types: &[AssetType],
) -> Result<Vec<AcquisitionWorkItem>, ApplicationError> {
    if let Some(reason) = connector.unsupported_request_reason(request)? {
        return Err(ApplicationError::UnsupportedConnectorPlan {
            source_id: connector.source_id().to_owned(),
            reason,
        });
    }
    discover_work(request, connector, asset_types)
}

fn discover_work(
    request: &AcquisitionRequest,
    connector: &dyn ConnectorPort,
    asset_types: &[AssetType],
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
        // Only the requested types this Source acquires were planned from it.
        if !asset_types.contains(&candidate.asset_type) {
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
    /// The connector of the Source that discovered `work`.
    fn connector_for(
        &self,
        work: &AcquisitionWorkItem,
    ) -> Result<&dyn ConnectorPort, ApplicationError> {
        let source_id = work.candidate.source_id.as_str();
        self.connectors
            .iter()
            .copied()
            .find(|connector| connector.source_id() == source_id)
            .ok_or_else(|| {
                self.source_failed.set(true);
                ApplicationError::UnsupportedConnectorPlan {
                    source_id: source_id.to_owned(),
                    reason: "no connector is registered for this source".to_owned(),
                }
            })
    }

    fn process(
        &self,
        work: &AcquisitionWorkItem,
    ) -> Result<Option<ImportedAsset>, ApplicationError> {
        self.source_failed.set(false);
        // Work whose Source has no connector anymore stays queued until one is registered.
        self.connector_for(work)?;
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

    /// Stores the candidate's original, links it to `release` (unless it falls short of the
    /// quality requirements) and completes the work. The repository settles the candidate's
    /// Review Item in the same transaction; a conflicting human decision makes the caller retry
    /// with that decision.
    fn import(
        &self,
        work: &AcquisitionWorkItem,
        release: &LibraryEntry,
        candidate_match: AssetCandidateMatch,
    ) -> Result<Step, ApplicationError> {
        let stored = self.store_original(work)?;
        // Quality is measured on the stored bytes; an original below it is not linked, and its
        // object stays unreferenced until vault verification collects it.
        let shortfalls = self
            .quality
            .as_ref()
            .map(|quality| quality.shortfalls(&stored.media))
            .unwrap_or_default();
        if !shortfalls.is_empty() {
            let settled = self.reviews.complete_candidate_below_quality(
                self.run_id,
                &work.key,
                release.release_edition_id,
                &shortfalls,
            )?;
            return Ok(if settled {
                Step::Done(None)
            } else {
                Step::Retry
            });
        }
        let record = self.asset_record(work, release, candidate_match, stored);
        match self.reviews.persist_candidate_asset(
            self.run_id,
            &work.key,
            record,
            self.retention,
        )? {
            CandidateAssetOutcome::Linked(imported) => Ok(Step::Done(Some(imported))),
            CandidateAssetOutcome::Outranked(_) => Ok(Step::Done(None)),
            CandidateAssetOutcome::HumanDecisionConflict => Ok(Step::Retry),
        }
    }

    fn store_original(&self, work: &AcquisitionWorkItem) -> Result<StoredObject, ApplicationError> {
        let mut stream = self
            .connector_for(work)?
            .download(&work.candidate)
            .inspect_err(|_| self.source_failed.set(true))?;
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
            media: stored.media,
            original_filename: candidate.original_filename.clone(),
            source_id: candidate.source_id.clone(),
            source_asset_label: candidate.source_asset_label.clone(),
            source_location: candidate.source_url.clone(),
        }
    }
}
