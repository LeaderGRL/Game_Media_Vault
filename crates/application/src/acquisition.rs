use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    io::Read,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc},
    thread,
};

use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRun, AcquisitionRunStatus,
    AcquisitionWorkItem, AssetCandidate, AssetCandidateMatch, AssetType, GameSelection,
    ImportedAsset, LibraryEntry, MatchConfidence, MatchingPolicy, NewReviewItem, PersistAsset,
    PlatformBoundGameSelector, QualityRequirements, RetentionPolicy, ReviewDecision, ReviewItem,
    ReviewStatus, SourceFailureStage, StoredObject, ValidatedMatchingPolicy,
    confirmed_asset_candidate_match, match_asset_candidate_to_release_preferring,
    review_matches_for_asset_candidate,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    ApplicationError, CandidateAssetOutcome, CatalogPort, ConnectorPort, ObjectStorePort,
    ParkedReview, ReviewRepositoryPort, RunRepositoryPort, build_acquisition_request,
    candidate_identity, load_acquisition_run,
    plan::{capable_asset_types, ensure_request_supported},
    plan_acquisition,
    platforms::{release_key, requested_release_keys, trailing_tags},
};

/// A concurrent human decision can close a Review Item between reading it and writing the
/// automatic outcome. Human decisions are final, so a second attempt always settles.
const REVIEW_RACE_ATTEMPTS: usize = 3;

/// How many downloads an execution runs at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadLimits {
    /// Downloads under way at once across every Source; zero counts as one.
    pub max_concurrent: usize,
    /// Downloads under way at once from any one Source; zero counts as one.
    pub max_per_source: usize,
}

impl Default for DownloadLimits {
    fn default() -> Self {
        Self {
            max_concurrent: 4,
            max_per_source: 2,
        }
    }
}

struct Acquisition<'a> {
    runs: &'a dyn RunRepositoryPort,
    catalog: &'a dyn CatalogPort,
    reviews: &'a dyn ReviewRepositoryPort,
    object_store: &'a dyn ObjectStorePort,
    connectors: &'a [&'a dyn ConnectorPort],
    run_id: i64,
    matching_policy: ValidatedMatchingPolicy,
    releases: Vec<LibraryEntry>,
    /// The names of the releases the request names, which stand for their game first.
    requested_releases: HashSet<String>,
    quality: Option<QualityRequirements>,
    retention: RetentionPolicy,
    /// Whether the last failure came from the Source of the processed work, such as a failed
    /// download, rather than from the vault.
    source_failed: Cell<bool>,
    /// Downloads started ahead of processing, by work key.
    prefetched: RefCell<HashMap<String, Ahead>>,
    /// The keys of each Source's queued work as the last round read them ahead, in queue order.
    lookahead: RefCell<HashMap<String, Vec<String>>>,
    /// Starts every download the execution makes, ahead or for the work it processes.
    dispatcher: Arc<Dispatcher<'a>>,
    limits: DownloadLimits,
}

/// A download started ahead of processing its work.
struct Ahead {
    source_id: String,
    result: mpsc::Receiver<Fetched>,
}

/// What downloading and storing a candidate's original gave.
enum Fetched {
    Stored(StoredObject),
    /// The Source no longer serves the media, for this reason.
    Unavailable(String),
    /// The Source failed to serve the media, before or partway through its body.
    SourceFailed(ApplicationError),
    /// The vault failed to store it.
    VaultFailed(ApplicationError),
}

/// Downloads `candidate` with its Source's connector and stores its original.
fn fetch(
    connector: &dyn ConnectorPort,
    object_store: &dyn ObjectStorePort,
    candidate: &AssetCandidate,
) -> Fetched {
    let mut stream = match connector.download(candidate) {
        Ok(stream) => stream,
        Err(error) if error.is_unavailable() => {
            return Fetched::Unavailable(error.message().to_owned());
        }
        Err(error) => return Fetched::SourceFailed(error.into()),
    };
    let failed = Cell::new(false);
    let mut body = SourceBody {
        stream: stream.as_mut(),
        failed: &failed,
    };
    match object_store.store_original(&mut body) {
        Ok(stored) => Fetched::Stored(stored),
        Err(error) if failed.get() => Fetched::SourceFailed(error.into()),
        Err(error) => Fetched::VaultFailed(error.into()),
    }
}

/// Gives the downloads of an execution room in the order its rounds read their work ahead, as
/// room frees up: no more than `max_concurrent` at once, nor more than `max_per_source` from any one
/// Source. A download waiting for room in its Source lets later downloads of other Sources start,
/// never later ones of its own Source, and a download the execution makes itself, for the work
/// it processes, goes before every queued one.
struct Dispatcher<'a> {
    state: Mutex<Dispatch<'a>>,
    changed: Condvar,
    limits: DownloadLimits,
}

#[derive(Default)]
struct Dispatch<'a> {
    /// The downloads to start, in queue order.
    jobs: VecDeque<Job<'a>>,
    /// The downloads under way, by Source.
    under_way: HashMap<&'static str, usize>,
    /// Downloads the execution waits to make itself.
    made_by_execution: usize,
    workers: usize,
    stopped: bool,
}

impl Dispatch<'_> {
    fn has_room(&self, source_id: &str, limits: DownloadLimits) -> bool {
        self.under_way.values().sum::<usize>() < limits.max_concurrent.max(1)
            && self.under_way.get(source_id).copied().unwrap_or(0) < limits.max_per_source.max(1)
    }
}

impl<'a> Dispatcher<'a> {
    fn new(limits: DownloadLimits) -> Self {
        Self {
            state: Mutex::new(Dispatch::default()),
            changed: Condvar::new(),
            limits,
        }
    }

    fn wait<'d>(&self, state: MutexGuard<'d, Dispatch<'a>>) -> MutexGuard<'d, Dispatch<'a>> {
        self.changed
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues `jobs` behind the downloads still waiting, and returns how many more workers
    /// should start them.
    fn queue(&self, jobs: impl IntoIterator<Item = Job<'a>>) -> usize {
        let mut state = lock(&self.state);
        state.jobs.extend(jobs);
        let more = self
            .limits
            .max_concurrent
            .max(1)
            .saturating_sub(state.workers)
            .min(state.jobs.len());
        state.workers += more;
        self.changed.notify_all();
        more
    }

    /// The next download to start once there is room for it, with that room; none once the
    /// execution stopped.
    fn next_job(&self) -> Option<(Job<'a>, Room<'_, 'a>)> {
        let mut state = lock(&self.state);
        loop {
            if state.stopped {
                return None;
            }
            let next = (state.made_by_execution == 0)
                .then(|| {
                    state
                        .jobs
                        .iter()
                        .position(|job| state.has_room(job.source_id, self.limits))
                })
                .flatten();
            if let Some(job) = next.and_then(|position| state.jobs.remove(position)) {
                *state.under_way.entry(job.source_id).or_default() += 1;
                let source_id = job.source_id;
                return Some((
                    job,
                    Room {
                        dispatcher: self,
                        source_id,
                    },
                ));
            }
            state = self.wait(state);
        }
    }

    /// Waits for room to download from `source_id` for the execution itself.
    fn room_for(&self, source_id: &'static str) -> Room<'_, 'a> {
        let mut state = lock(&self.state);
        state.made_by_execution += 1;
        while !state.has_room(source_id, self.limits) {
            state = self.wait(state);
        }
        state.made_by_execution -= 1;
        *state.under_way.entry(source_id).or_default() += 1;
        self.changed.notify_all();
        Room {
            dispatcher: self,
            source_id,
        }
    }

    /// Starts no further download once the execution ends, and drops the waiting ones, whose
    /// results are never sent.
    fn stop(&self) {
        let mut state = lock(&self.state);
        state.stopped = true;
        state.jobs.clear();
        self.changed.notify_all();
    }
}

/// Room for one download from a Source, freed when it drops.
struct Room<'d, 'a> {
    dispatcher: &'d Dispatcher<'a>,
    source_id: &'static str,
}

impl Drop for Room<'_, '_> {
    fn drop(&mut self) {
        let mut state = lock(&self.dispatcher.state);
        if let Some(under_way) = state.under_way.get_mut(self.source_id) {
            *under_way -= 1;
        }
        self.dispatcher.changed.notify_all();
    }
}

/// A download panicking on another thread leaves the counts it guards valid.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Releases the claim an execution holds on its run, however it ends.
struct ExecutionClaim<'a> {
    runs: &'a dyn RunRepositoryPort,
    run_id: i64,
}

impl Drop for ExecutionClaim<'_> {
    fn drop(&mut self) {
        // A claim that cannot be released ends with the process holding it.
        let _ = self.runs.release_execution(self.run_id);
    }
}

/// Marks the execution stopped however it ends.
struct StopOnExit<'d, 'a>(&'d Dispatcher<'a>);

impl Drop for StopOnExit<'_, '_> {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// Where processing a work item leads, given its Review Item.
enum Route {
    /// A human accepted it for this Release Edition.
    Accepted(i64),
    Rejected,
    /// The matcher decides, by its confidence.
    Matched(AssetCandidateMatch),
}

enum Step {
    Done(Option<ImportedAsset>),
    Retry,
}

/// Executes a running Acquisition Run with the registered `connectors`, one per Source.
///
/// Only the Sources the run's plan kept when it started are contacted. Each is discovered once
/// per run: its candidates are persisted as queued work, so resuming never rediscovers it, and a
/// pause stops further discoveries. A Source that looks games up a few at a time is discovered
/// one batch of games per pass, each recorded as it completes and its work executed before the
/// next batch, so an execution that stops, as on a spent quota, resumes with the next batch.
/// Sources progress independently: one without a registered connector, that now refuses the
/// plan, cannot be reached, fails to discover or fails to download is asked no further in that
/// execution and leaves the others' work and batches to run, keeps the run running, and its
/// error is returned once that work is done. Each queued candidate is matched against the library through its own
/// Source's connector and is either imported, left unattached, or parked on its Review Item
/// until a human decides. The run completes only once every planned Source was discovered.
///
/// The media of the work the coming rounds will import download ahead, up to
/// `limits.max_concurrent` at once and `limits.max_per_source` from any one Source, while the
/// work is processed in turn as before.
// Each port plays its own role; grouping them would only hide what an execution depends on.
#[allow(clippy::too_many_arguments)]
pub fn acquire_run_with_connectors(
    runs: &dyn RunRepositoryPort,
    reviews: &dyn ReviewRepositoryPort,
    catalog: &dyn CatalogPort,
    object_store: &dyn ObjectStorePort,
    connectors: &[&dyn ConnectorPort],
    run_id: i64,
    matching_policy: MatchingPolicy,
    limits: DownloadLimits,
) -> Result<Vec<ImportedAsset>, ApplicationError> {
    let matching_policy = matching_policy.validate()?;
    // One execution at a time: another would only drain the same queue, and the download limits
    // bound each execution.
    if !runs.claim_execution(run_id)? {
        return Err(ApplicationError::RunAlreadyExecuting(run_id));
    }
    let _claim = ExecutionClaim { runs, run_id };
    let run = load_acquisition_run(runs, run_id)?;
    match run.status {
        AcquisitionRunStatus::Running => ensure_request_supported(&run.request)?,
        // Nothing is left to discover, but an acceptance may reopen the run right after this
        // read; the drain below executes whatever work it requeued.
        AcquisitionRunStatus::Completed => {}
        status => return Err(ApplicationError::RunNotExecutable { status }),
    }

    // The request's cap on concurrent downloads, when it sets one, is the execution's.
    let limits = DownloadLimits {
        max_concurrent: run
            .request
            .limits()
            .max_concurrent_downloads
            .map_or(limits.max_concurrent, usize::from),
        ..limits
    };
    let acquisition = Acquisition {
        runs,
        catalog,
        reviews,
        object_store,
        connectors,
        run_id,
        matching_policy,
        releases: catalog.list_library()?,
        requested_releases: requested_release_keys(&run.request),
        quality: run.request.quality().cloned(),
        retention: run.request.retention(),
        source_failed: Cell::new(false),
        prefetched: RefCell::new(HashMap::new()),
        lookahead: RefCell::new(HashMap::new()),
        dispatcher: Arc::new(Dispatcher::new(limits)),
        limits,
    };
    acquisition.requeue_settled_reviews()?;
    thread::scope(|scope| execute(&acquisition, &run, scope))
}

fn execute<'a, 'scope>(
    acquisition: &Acquisition<'a>,
    run: &AcquisitionRun,
    scope: &'scope thread::Scope<'scope, '_>,
) -> Result<Vec<ImportedAsset>, ApplicationError>
where
    'a: 'scope,
{
    // Downloads not started yet never start once the execution stops, however it stops.
    let _stop = StopOnExit(&acquisition.dispatcher);
    let (runs, connectors, run_id) = (acquisition.runs, acquisition.connectors, run.id);
    let mut imported_assets = Vec::new();
    let mut source_failure = None;
    // Sources whose work failed; their remaining work waits for a later execution.
    let mut failed_sources: Vec<String> = Vec::new();
    // Sources disabled or failing to discover; they are discovered further in a later execution.
    let mut undiscoverable: Vec<String> = Vec::new();
    loop {
        for source_id in &run.planned_sources {
            if runs.has_discovered(run_id, source_id)?
                || undiscoverable.contains(source_id)
                || failed_sources.contains(source_id)
            {
                continue;
            }
            // A pause, even one landing during a failed discovery, keeps the snapshots already
            // recorded but discovers no further.
            if load_acquisition_run(runs, run_id)?.status != AcquisitionRunStatus::Running {
                break;
            }
            // A Source disabled on this machine is not discovered, which is no failure of it.
            if let Some(reason) = disabled_reason_of(connectors, source_id) {
                undiscoverable.push(source_id.clone());
                source_failure.get_or_insert(ApplicationError::SourceDisabled {
                    source_id: source_id.clone(),
                    reason,
                });
                continue;
            }
            let first_game = runs.discovered_games(run_id, source_id)?;
            // The work of the batches already recorded is executed before the next batch is
            // looked up, so a quota goes to one batch at a time, across executions too.
            if first_game > 0 && has_queued_work(runs, run, source_id)? {
                continue;
            }
            let discovered = planned_connector(&run.request, source_id, connectors).and_then(
                |(connector, asset_types)| {
                    discover_source(&run.request, connector, &asset_types, first_game)
                },
            );
            // A cancellation or completion that won the race while discovering stops quietly.
            let recorded = match discovered {
                Ok((work, None)) => runs.record_discovery(run_id, source_id, &work)?,
                Ok((work, Some(games))) => {
                    runs.record_discovery_batch(run_id, source_id, first_game, games, &work)?
                }
                Err(error) => {
                    runs.record_source_failure(
                        run_id,
                        source_id,
                        SourceFailureStage::Discovery,
                        &error.to_string(),
                    )?;
                    undiscoverable.push(source_id.clone());
                    source_failure.get_or_insert(error);
                    true
                }
            };
            if !recorded {
                return Ok(imported_assets);
            }
        }
        // Sources take turns: each round processes the oldest queued work of every Source that
        // has some, so a Source with a long queue or slow downloads never holds back the others.
        let mut served_this_round: Vec<String> = Vec::new();
        loop {
            // A round downloads ahead what it will import, and processes it in order.
            if served_this_round.is_empty() {
                acquisition.prefetch_round(scope, &failed_sources)?;
            }
            let skipped: Vec<String> = failed_sources
                .iter()
                .chain(&served_this_round)
                .cloned()
                .collect();
            let Some(work) = runs.next_queued_work(run_id, &skipped)? else {
                if served_this_round.is_empty() {
                    // Nothing is left to process, or the run stopped: the downloads still ahead
                    // tell whether their Sources failed, their work staying queued.
                    let left = acquisition.downloads_left();
                    defer_failed_sources(
                        runs,
                        run_id,
                        left,
                        &mut failed_sources,
                        &mut source_failure,
                    )?;
                    break;
                }
                served_this_round.clear();
                continue;
            };
            let source_id = work.candidate.source_id.as_str().to_owned();
            served_this_round.push(source_id.clone());
            // A Source disabled since its work was queued leaves that work waiting, untouched.
            if let Some(reason) = acquisition.disabled_reason(&work) {
                failed_sources.push(source_id.clone());
                source_failure
                    .get_or_insert(ApplicationError::SourceDisabled { source_id, reason });
                continue;
            }
            // Work of the Source downloaded ahead but no longer queued before this work, as when
            // another execution of the run settled it, tells whether the Source failed; this
            // work then waits for a later execution too.
            let passed = acquisition.downloads_passed_by(&work);
            if defer_failed_sources(
                runs,
                run_id,
                passed,
                &mut failed_sources,
                &mut source_failure,
            )? {
                continue;
            }
            match acquisition.process(&work) {
                Ok(imported) => imported_assets.extend(imported),
                Err(error) if acquisition.source_failed.get() => {
                    defer_failed_sources(
                        runs,
                        run_id,
                        vec![(source_id, error)],
                        &mut failed_sources,
                        &mut source_failure,
                    )?;
                }
                Err(error) => return Err(error),
            }
        }
        // The batches Sources have left are discovered, and their work executed, in this
        // execution too, unless the run stopped meanwhile.
        let mut batches_left = false;
        for source_id in &run.planned_sources {
            if !undiscoverable.contains(source_id)
                && !failed_sources.contains(source_id)
                && !runs.has_discovered(run_id, source_id)?
            {
                batches_left = true;
            }
        }
        if batches_left
            && load_acquisition_run(runs, run_id)?.status == AcquisitionRunStatus::Running
        {
            continue;
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

/// Records the Sources whose downloads failed and defers their remaining work to a later
/// execution, keeping the first failure for the execution to return. A Source already failing
/// keeps its first failure. Returns whether any Source failed.
fn defer_failed_sources(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
    failures: Vec<(String, ApplicationError)>,
    failed_sources: &mut Vec<String>,
    source_failure: &mut Option<ApplicationError>,
) -> Result<bool, ApplicationError> {
    let any_failed = !failures.is_empty();
    for (source_id, error) in failures {
        if failed_sources.contains(&source_id) {
            continue;
        }
        runs.record_source_failure(
            run_id,
            &source_id,
            SourceFailureStage::Download,
            &error.to_string(),
        )?;
        failed_sources.push(source_id);
        source_failure.get_or_insert(error);
    }
    Ok(any_failed)
}

/// Why the registered connector of `source_id` takes no part in acquisitions, if it does not.
fn disabled_reason_of(connectors: &[&dyn ConnectorPort], source_id: &str) -> Option<String> {
    connectors
        .iter()
        .find(|connector| connector.source_id() == source_id)
        .and_then(|connector| connector.disabled_reason())
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

/// Whether `source_id` has work queued in `run`, which executes before the Source is asked
/// about more games.
fn has_queued_work(
    runs: &dyn RunRepositoryPort,
    run: &AcquisitionRun,
    source_id: &str,
) -> Result<bool, ApplicationError> {
    let others: Vec<String> = run
        .planned_sources
        .iter()
        .filter(|planned| planned.as_str() != source_id)
        .cloned()
        .collect();
    Ok(runs.next_queued_work(run.id, &others)?.is_some())
}

/// Discovers the work of the batch of one Source that starts at game `first_game` of the
/// request, once its connector accepts that batch, which may consult the Source. Tells how many
/// games the batch covered, or `None` when it was the last. Only Sources not yet discovered need
/// it: a persisted snapshot executes without them.
fn discover_source(
    request: &AcquisitionRequest,
    connector: &dyn ConnectorPort,
    asset_types: &[AssetType],
    first_game: usize,
) -> Result<(Vec<AcquisitionWorkItem>, Option<usize>), ApplicationError> {
    let batch = discovery_batch(request, connector, first_game)?;
    let (request, refusal) = as_served_by(&batch.request, connector, request)?;
    if let Some(reason) = refusal {
        return Err(ApplicationError::UnsupportedConnectorPlan {
            source_id: connector.source_id().to_owned(),
            reason,
        });
    }
    let work = discover_work(&request, connector, asset_types)?;
    Ok((work, (!batch.last).then_some(batch.games)))
}

/// `request`, a batch of `whole`, as `connector` is asked about it and discovers it, with why the
/// connector refuses it, if it does: as it is, or without its regions when the connector cannot
/// tell regions apart but every game `whole` names carries one of them in its name, as
/// `Tetris (Europe)` or a worldwide `(World)` release does, which keeps the request to those
/// regions already. Judging the whole request keeps every batch of a discovery served alike.
pub(crate) fn as_served_by(
    request: &AcquisitionRequest,
    connector: &dyn ConnectorPort,
    whole: &AcquisitionRequest,
) -> Result<(AcquisitionRequest, Option<String>), ApplicationError> {
    let refusal = connector.unsupported_request_reason(request)?;
    if refusal.is_none() || request.regions().is_empty() || !names_carry_regions(whole) {
        return Ok((request.clone(), refusal));
    }
    let mut draft = request.to_draft();
    draft.regions.clear();
    let without_regions = build_acquisition_request(draft)?;
    Ok(
        match connector.unsupported_request_reason(&without_regions)? {
            None => (without_regions, None),
            Some(_) => (request.clone(), refusal),
        },
    )
}

/// Whether every game `request` names carries one of its regions, or `World`, in a tag of its
/// name, such as `(USA, Europe)`.
fn names_carry_regions(request: &AcquisitionRequest) -> bool {
    let carries = |name: &str| {
        trailing_tags(name).into_iter().any(|tag| {
            tag.split(',').map(str::trim).any(|region| {
                region.eq_ignore_ascii_case("World")
                    || request
                        .regions()
                        .iter()
                        .any(|wanted| wanted.trim().eq_ignore_ascii_case(region))
            })
        })
    };
    match request.games() {
        GameSelection::All => false,
        GameSelection::Explicit(games) => games.iter().all(|game| carries(game)),
        GameSelection::PlatformBound(selectors) | GameSelection::QueryResult(selectors) => {
            selectors.iter().all(|selector| carries(&selector.game))
        }
    }
}

/// The games of a request one discovery of a Source looks up.
pub(crate) struct DiscoveryBatch {
    pub(crate) request: AcquisitionRequest,
    /// How many games of the request it covers.
    pub(crate) games: usize,
    /// Whether it covers the request's last game.
    pub(crate) last: bool,
}

/// The batch of the discovery of `connector` that starts at game `first_game` of `request`: the
/// whole request, as its only batch, unless the connector looks games up a few at a time and the
/// request names more. A game named without its platform is looked up on every requested
/// platform, each lookup counting toward the batch, and a game named twice, regardless of case
/// and surrounding spaces, counts once.
pub(crate) fn discovery_batch(
    request: &AcquisitionRequest,
    connector: &dyn ConnectorPort,
    first_game: usize,
) -> Result<DiscoveryBatch, ApplicationError> {
    let games = unique_games(request.games());
    let count = game_count(&games);
    let whole = || DiscoveryBatch {
        request: request.clone(),
        games: count,
        last: true,
    };
    let Some(size) = batch_size(request, connector) else {
        return Ok(whole());
    };
    // Games past the last stand for the last batch.
    let start = if first_game < count {
        first_game
    } else {
        count.saturating_sub(size)
    };
    let end = (start + size).min(count);
    if start == 0 && end == count {
        return Ok(whole());
    }
    Ok(DiscoveryBatch {
        request: with_games(request, slice(&games, start, end))?,
        games: end - start,
        last: end == count,
    })
}

/// The requests `connector` is asked about before a run starts: the first batch of its
/// discovery and, for games named with their platforms, a game of each platform that batch
/// leaves out, batched alike, so a platform only a later batch names is checked too.
pub(crate) fn planned_batches(
    request: &AcquisitionRequest,
    connector: &dyn ConnectorPort,
) -> Result<Vec<AcquisitionRequest>, ApplicationError> {
    let first = discovery_batch(request, connector, 0)?;
    let first_platforms: HashSet<String> = platforms_named(first.request.games()).collect();
    let mut checked = vec![first.request];
    let (Some(size), false) = (batch_size(request, connector), first.last) else {
        return Ok(checked);
    };
    let games = unique_games(request.games());
    let (GameSelection::PlatformBound(selectors) | GameSelection::QueryResult(selectors)) = &games
    else {
        return Ok(checked);
    };
    let mut seen = first_platforms;
    let others: Vec<PlatformBoundGameSelector> = selectors
        .iter()
        .filter(|selector| seen.insert(name_key(&selector.platform)))
        .cloned()
        .collect();
    for chunk in others.chunks(size) {
        let part = match &games {
            GameSelection::QueryResult(_) => GameSelection::QueryResult(chunk.to_vec()),
            _ => GameSelection::PlatformBound(chunk.to_vec()),
        };
        checked.push(with_games(request, part)?);
    }
    Ok(checked)
}

/// How many games one discovery of `connector` looks up for `request`, when it looks games up a
/// few at a time: a game named without its platform takes a lookup on each requested platform.
fn batch_size(request: &AcquisitionRequest, connector: &dyn ConnectorPort) -> Option<usize> {
    let lookups = connector.discovery_batch_size().filter(|size| *size > 0)?;
    let lookups_per_game = match request.games() {
        GameSelection::Explicit(_) => request.platforms().len().max(1),
        _ => 1,
    };
    Some((lookups / lookups_per_game).max(1))
}

fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// `games`, each once regardless of case and surrounding spaces, in request order.
fn unique_games(games: &GameSelection) -> GameSelection {
    let mut seen = HashSet::new();
    match games {
        GameSelection::All => GameSelection::All,
        GameSelection::Explicit(games) => GameSelection::Explicit(
            games
                .iter()
                .filter(|game| seen.insert((name_key(game), String::new())))
                .cloned()
                .collect(),
        ),
        GameSelection::PlatformBound(selectors) | GameSelection::QueryResult(selectors) => {
            let unique = selectors
                .iter()
                .filter(|selector| {
                    seen.insert((name_key(&selector.game), name_key(&selector.platform)))
                })
                .cloned()
                .collect();
            match games {
                GameSelection::QueryResult(_) => GameSelection::QueryResult(unique),
                _ => GameSelection::PlatformBound(unique),
            }
        }
    }
}

fn game_count(games: &GameSelection) -> usize {
    match games {
        GameSelection::All => 0,
        GameSelection::Explicit(games) => games.len(),
        GameSelection::PlatformBound(games) | GameSelection::QueryResult(games) => games.len(),
    }
}

/// The games of `games` from `start` to `end`.
fn slice(games: &GameSelection, start: usize, end: usize) -> GameSelection {
    match games {
        GameSelection::All => GameSelection::All,
        GameSelection::Explicit(games) => GameSelection::Explicit(games[start..end].to_vec()),
        GameSelection::PlatformBound(games) => {
            GameSelection::PlatformBound(games[start..end].to_vec())
        }
        GameSelection::QueryResult(games) => GameSelection::QueryResult(games[start..end].to_vec()),
    }
}

/// The platforms `games` names with its games, regardless of case and surrounding spaces.
fn platforms_named(games: &GameSelection) -> impl Iterator<Item = String> + '_ {
    let selectors: &[PlatformBoundGameSelector] = match games {
        GameSelection::PlatformBound(selectors) | GameSelection::QueryResult(selectors) => {
            selectors
        }
        _ => &[],
    };
    selectors
        .iter()
        .map(|selector| name_key(&selector.platform))
}

/// `request` naming `games` instead of its own.
fn with_games(
    request: &AcquisitionRequest,
    games: GameSelection,
) -> Result<AcquisitionRequest, ApplicationError> {
    let mut draft = request.to_draft();
    draft.games = games;
    Ok(build_acquisition_request(draft)?)
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

    /// Requeues the run's work parked on pending Review Items whose candidate now matches one
    /// of the releases it competed for with high confidence, as after the matcher learned that a
    /// region holds a country; this execution then links it as any queued work, closing the
    /// item for the other runs too.
    fn requeue_settled_reviews(&self) -> Result<(), ApplicationError> {
        let releases: HashMap<i64, &LibraryEntry> = self
            .releases
            .iter()
            .map(|release| (release.release_edition_id, release))
            .collect();
        for item in self.reviews.list_review_items()? {
            if item.status != ReviewStatus::Pending {
                continue;
            }
            let competing: Vec<LibraryEntry> = item
                .competing_matches
                .iter()
                .filter_map(|candidate| releases.get(&candidate.release_edition_id))
                .map(|release| (*release).clone())
                .collect();
            let rematched = match_asset_candidate_to_release_preferring(
                &item.candidate,
                &competing,
                self.matching_policy,
                &|release| self.requested_releases.contains(&release_key(release)),
            );
            if rematched.confidence == MatchConfidence::High {
                self.reviews.requeue_review_work(item.id, self.run_id)?;
            }
        }
        Ok(())
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
        let candidate_match = match self.route(work, review_item.as_ref()) {
            Route::Accepted(release_edition_id) => {
                let release = self.accepted_release(release_edition_id)?;
                let candidate_match = confirmed_asset_candidate_match(&work.candidate, &release);
                return self.import(work, &release, candidate_match);
            }
            Route::Rejected => {
                // A download started ahead before the rejection still reports a failing Source.
                let ahead = self.prefetched.borrow_mut().remove(&work.key);
                if let Some(Ok(Fetched::SourceFailed(error))) =
                    ahead.map(|ahead| ahead.result.recv())
                {
                    self.source_failed.set(true);
                    return Err(error);
                }
                self.runs.dismiss_work(self.run_id, &work.key)?;
                return Ok(Step::Done(None));
            }
            Route::Matched(candidate_match) => candidate_match,
        };
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

    fn route(&self, work: &AcquisitionWorkItem, review_item: Option<&ReviewItem>) -> Route {
        match review_item.map(|item| (item.status, &item.decision)) {
            Some((ReviewStatus::Accepted, Some(ReviewDecision::Accept { release_edition_id }))) => {
                Route::Accepted(*release_edition_id)
            }
            Some((ReviewStatus::Rejected, _)) => Route::Rejected,
            _ => Route::Matched(match_asset_candidate_to_release_preferring(
                &work.candidate,
                &self.releases,
                self.matching_policy,
                &|release| self.requested_releases.contains(&release_key(release)),
            )),
        }
    }

    /// Whether processing `work` now would download its media.
    fn will_import(&self, work: &AcquisitionWorkItem) -> Result<bool, ApplicationError> {
        let review_item = self.reviews.find_review_item(&work.key)?;
        Ok(match self.route(work, review_item.as_ref()) {
            Route::Accepted(_) => true,
            Route::Rejected => false,
            Route::Matched(candidate_match) => matches!(
                candidate_match.confidence,
                MatchConfidence::High | MatchConfidence::Confirmed
            ),
        })
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
        let Some(stored) = self.store_original(work)? else {
            return Ok(Step::Done(None));
        };
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

    /// Downloads and stores the candidate's original. Returns `None` when its Source no longer
    /// serves the media, completing the work as unavailable, or when the run stopped before
    /// the download started, leaving the work queued.
    fn store_original(
        &self,
        work: &AcquisitionWorkItem,
    ) -> Result<Option<StoredObject>, ApplicationError> {
        // A download started ahead is awaited; one never planned is made now.
        let ahead = self.prefetched.borrow_mut().remove(&work.key);
        let fetched = match ahead.map(|ahead| ahead.result.recv()) {
            Some(Ok(fetched)) => fetched,
            // A download not made ahead, or whose thread stopped before starting it, starts
            // only while the run still runs.
            _ if !self.still_running()? => return Ok(None),
            _ => {
                let connector = self.connector_for(work)?;
                let _room = self.dispatcher.room_for(connector.source_id());
                fetch(connector, self.object_store, &work.candidate)
            }
        };
        match fetched {
            Fetched::Stored(stored) => Ok(Some(stored)),
            Fetched::Unavailable(reason) => {
                self.runs
                    .complete_unavailable_work(self.run_id, &work.key, &reason)?;
                Ok(None)
            }
            Fetched::SourceFailed(error) => {
                self.source_failed.set(true);
                Err(error)
            }
            Fetched::VaultFailed(error) => Err(error),
        }
    }

    /// Awaits the downloads started ahead for work of the `served` work's Source that no longer
    /// waits before it: work read ahead of it, or not read ahead with it, left the queue
    /// unprocessed, as when another execution of the run settled it, even if it came back
    /// since. Work read ahead behind it keeps its download for its turn.
    fn downloads_passed_by(&self, served: &AcquisitionWorkItem) -> Vec<(String, ApplicationError)> {
        let source_id = served.candidate.source_id.as_str();
        let lookahead = self.lookahead.borrow();
        let behind = lookahead
            .get(source_id)
            .and_then(|keys| {
                let position = keys.iter().position(|key| *key == served.key)?;
                Some(&keys[position + 1..])
            })
            .unwrap_or_default();
        self.awaited_failures(|key, ahead| {
            ahead.source_id == source_id && *key != served.key && !behind.contains(key)
        })
    }

    /// Awaits every download still ahead, once the execution processes no further work.
    fn downloads_left(&self) -> Vec<(String, ApplicationError)> {
        self.awaited_failures(|_, _| true)
    }

    /// Awaits the downloads started ahead that `unclaimed` selects, and returns the first
    /// failure of each of their Sources. Only failures matter: their stored originals stay
    /// unreferenced until vault verification collects them.
    fn awaited_failures(
        &self,
        mut unclaimed: impl FnMut(&String, &Ahead) -> bool,
    ) -> Vec<(String, ApplicationError)> {
        let unclaimed: Vec<Ahead> = self
            .prefetched
            .borrow_mut()
            .extract_if(|key, ahead| unclaimed(key, ahead))
            .map(|(_, ahead)| ahead)
            .collect();
        let mut failures: Vec<(String, ApplicationError)> = Vec::new();
        for ahead in unclaimed {
            if let Ok(Fetched::SourceFailed(error)) = ahead.result.recv()
                && !failures
                    .iter()
                    .any(|(source_id, _)| *source_id == ahead.source_id)
            {
                failures.push((ahead.source_id, error));
            }
        }
        failures
    }

    /// Why the Source of `work` takes no part in acquisitions, if it does not.
    fn disabled_reason(&self, work: &AcquisitionWorkItem) -> Option<String> {
        self.connectors
            .iter()
            .find(|connector| connector.source_id() == work.candidate.source_id.as_str())
            .and_then(|connector| connector.disabled_reason())
    }

    fn still_running(&self) -> Result<bool, ApplicationError> {
        Ok(self.runs.run_status(self.run_id)? == Some(AcquisitionRunStatus::Running))
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

/// A download started ahead of processing.
struct Job<'a> {
    connector: &'a dyn ConnectorPort,
    source_id: &'static str,
    candidate: AssetCandidate,
    result: mpsc::Sender<Fetched>,
}

/// How far a round reads each Source's queued work ahead, in multiples of its per-Source limit,
/// so that work bound for review rarely takes the place of work it will import.
const READ_AHEAD: usize = 4;

impl<'a> Acquisition<'a> {
    /// Starts downloading, on up to `max_concurrent` threads, the media of the work the coming
    /// rounds will import: the oldest `max_per_source` queued work items of every Source
    /// outside `failed_sources` that processing would import, among the oldest `READ_AHEAD`
    /// times as many. Work processing would not import, such as work bound for review, is
    /// never downloaded ahead. Every Source's next download is queued before any Source's
    /// further one. Results are awaited when their work is processed, or once it left the
    /// queue unprocessed.
    fn prefetch_round<'scope>(
        &self,
        scope: &'scope thread::Scope<'scope, '_>,
        failed_sources: &[String],
    ) -> Result<(), ApplicationError>
    where
        'a: 'scope,
    {
        let per_source = self.limits.max_per_source.max(1);
        let queued = self
            .runs
            .queued_work(self.run_id, failed_sources, per_source * READ_AHEAD)?;
        let mut lookahead = self.lookahead.borrow_mut();
        lookahead.clear();
        // Each new download, with how many downloads of its Source come before it.
        let mut ranked: Vec<(usize, Job<'a>)> = Vec::new();
        let mut importable: HashMap<&'static str, usize> = HashMap::new();
        for work in queued {
            lookahead
                .entry(work.candidate.source_id.as_str().to_owned())
                .or_default()
                .push(work.key.clone());
            let connector = self
                .connectors
                .iter()
                .copied()
                .find(|connector| connector.source_id() == work.candidate.source_id.as_str());
            let Some(connector) = connector else {
                continue;
            };
            if connector.disabled_reason().is_some() {
                continue;
            }
            let source_id = connector.source_id();
            let rank = importable.get(source_id).copied().unwrap_or(0);
            if rank == per_source {
                continue;
            }
            if self.prefetched.borrow().contains_key(&work.key) {
                importable.insert(source_id, rank + 1);
                continue;
            }
            if !self.will_import(&work)? {
                continue;
            }
            importable.insert(source_id, rank + 1);
            let (result, awaited) = mpsc::channel();
            let ahead = Ahead {
                source_id: source_id.to_owned(),
                result: awaited,
            };
            self.prefetched.borrow_mut().insert(work.key, ahead);
            ranked.push((
                rank,
                Job {
                    connector,
                    source_id,
                    candidate: work.candidate,
                    result,
                },
            ));
        }
        // The sort is stable, so each rank keeps the queue order.
        ranked.sort_by_key(|(rank, _)| *rank);
        let more_workers = self
            .dispatcher
            .queue(ranked.into_iter().map(|(_, job)| job));
        for _ in 0..more_workers {
            let dispatcher = Arc::clone(&self.dispatcher);
            let (object_store, runs, run_id) = (self.object_store, self.runs, self.run_id);
            scope.spawn(move || {
                while let Some((job, _room)) = dispatcher.next_job() {
                    // A pause or a cancellation starts no further download, even while the
                    // execution still awaits one already under way. The download is dropped, its
                    // work staying queued; a resume the execution sees makes it itself.
                    let running = matches!(
                        runs.run_status(run_id),
                        Ok(Some(AcquisitionRunStatus::Running))
                    );
                    if !running {
                        continue;
                    }
                    // The execution may have stopped awaiting this result.
                    let _ = job
                        .result
                        .send(fetch(job.connector, object_store, &job.candidate));
                }
            });
        }
        Ok(())
    }
}

/// A download body that notes when reading it fails, so a connection dropping partway is a
/// failure of the Source rather than of the vault storing it.
struct SourceBody<'a> {
    stream: &'a mut (dyn Read + Send),
    failed: &'a Cell<bool>,
}

impl Read for SourceBody<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.stream
            .read(buffer)
            .inspect_err(|_| self.failed.set(true))
    }
}
