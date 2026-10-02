mod support;

use std::{
    io::Read,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use game_media_vault_application::{
    ConnectorPort, DownloadLimits, PortError, acquire_run_with_connectors, pause_acquisition_run,
    start_acquisition_run_with_connectors,
};
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRunStatus, AssetCandidate, AssetType,
    AssetTypeSelector, ConnectorCapabilities, LibraryEntry, SourceId, SourceSelection,
};
use support::*;

/// Lets downloads wait until `expected` of them are under way at once.
struct Rendezvous {
    arrived: Mutex<usize>,
    everyone: Condvar,
    expected: usize,
}

impl Rendezvous {
    fn new(expected: usize) -> Self {
        Self {
            arrived: Mutex::new(0),
            everyone: Condvar::new(),
            expected,
        }
    }

    /// Whether the others arrived too before giving up.
    fn meet(&self) -> bool {
        let mut arrived = self.arrived.lock().unwrap();
        *arrived += 1;
        self.everyone.notify_all();
        let (arrived, timeout) = self
            .everyone
            .wait_timeout_while(arrived, Duration::from_secs(5), |arrived| {
                *arrived < self.expected
            })
            .unwrap();
        drop(arrived);
        !timeout.timed_out()
    }
}

/// Counts the downloads under way and the most there ever were at once.
#[derive(Default)]
struct InFlight {
    current: AtomicUsize,
    most: AtomicUsize,
}

/// A connector whose downloads take a while, meeting other downloads or counting them.
struct SlowConnector<'a> {
    inner: FakeConnector,
    rendezvous: Option<&'a Rendezvous>,
    in_flight: Option<&'a InFlight>,
    /// Runs while each download is under way, as a human acting meanwhile.
    during_download: Option<&'a (dyn Fn() + Sync)>,
    /// Records the title of each download as it starts.
    started: Option<&'a Shared<Vec<String>>>,
}

impl ConnectorPort for SlowConnector<'_> {
    fn source_id(&self) -> &'static str {
        self.inner.source_id()
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        self.inner.capabilities()
    }

    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        self.inner.unsupported_request_reason(request)
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        self.inner.discover(request)
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        if let Some(started) = self.started {
            started.borrow_mut().push(candidate.game_title.clone());
        }
        if let Some(during_download) = self.during_download {
            during_download();
        }
        if let Some(rendezvous) = self.rendezvous
            && !rendezvous.meet()
        {
            return Err(PortError::new("the downloads did not overlap".to_owned()));
        }
        if let Some(in_flight) = self.in_flight {
            let now = in_flight.current.fetch_add(1, Ordering::SeqCst) + 1;
            in_flight.most.fetch_max(now, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(100));
            in_flight.current.fetch_sub(1, Ordering::SeqCst);
        }
        self.inner.download(candidate)
    }
}

/// A Box Front of `title` from the Source `source_id`.
fn box_front(source_id: &'static str, title: &str) -> AssetCandidate {
    AssetCandidate {
        source_id: SourceId::from(source_id),
        source_url: format!("https://{source_id}.invalid/{title}.png"),
        ..candidate(title)
    }
}

fn slow<'a>(
    source_id: &'static str,
    candidates: Vec<AssetCandidate>,
    rendezvous: Option<&'a Rendezvous>,
    in_flight: Option<&'a InFlight>,
) -> SlowConnector<'a> {
    SlowConnector {
        inner: FakeConnector {
            source_id,
            asset_types: vec![AssetType::BoxFront],
            ..FakeConnector::new(candidates)
        },
        rendezvous,
        in_flight,
        during_download: None,
        started: None,
    }
}

/// Starts and executes an `Auto` run of Box Fronts with `connectors`, under `limits`.
fn execute(
    vault: &FakeVault,
    connectors: &[&SlowConnector<'_>],
    limits: DownloadLimits,
) -> Result<usize, String> {
    let registry: Vec<&dyn ConnectorPort> = connectors
        .iter()
        .map(|connector| *connector as &dyn ConnectorPort)
        .collect();
    let run = start_acquisition_run_with_connectors(
        vault,
        AcquisitionRequestDraft {
            sources: SourceSelection::Auto,
            asset_types: vec![AssetTypeSelector::BoxFront],
            ..request_draft()
        },
        &registry,
    )
    .map_err(|error| error.to_string())?;
    acquire_run_with_connectors(
        vault,
        vault,
        vault,
        &FakeStore::default(),
        &registry,
        run.id,
        matching_policy(),
        limits,
    )
    .map(|imported| imported.len())
    .map_err(|error| error.to_string())
}

#[test]
fn downloads_of_different_sources_overlap() {
    let (mario, tetris) = (
        box_front("a", "Super Mario Bros."),
        box_front("b", "Tetris"),
    );
    let vault = FakeVault::with_library(vec![release_for(&mario, 1), release_for(&tetris, 2)]);
    // Each download waits for the other: one at a time, the first would give up.
    let rendezvous = Rendezvous::new(2);
    let first = slow("a", vec![mario], Some(&rendezvous), None);
    let second = slow("b", vec![tetris], Some(&rendezvous), None);

    let imported = execute(
        &vault,
        &[&first, &second],
        DownloadLimits {
            max_concurrent: 2,
            max_per_source: 2,
        },
    );

    assert_eq!(imported, Ok(2));
}

#[test]
fn the_limit_bounds_the_downloads_under_way() {
    for max_concurrent in [1, 2] {
        let candidates: Vec<AssetCandidate> = ["a", "b", "c"]
            .iter()
            .map(|source_id| box_front(source_id, &format!("Game {source_id}")))
            .collect();
        let vault = FakeVault::with_library(
            candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| release_for(candidate, index as i64 + 1))
                .collect(),
        );
        let in_flight = InFlight::default();
        let connectors: Vec<SlowConnector<'_>> = ["a", "b", "c"]
            .into_iter()
            .zip(candidates)
            .map(|(source_id, candidate)| slow(source_id, vec![candidate], None, Some(&in_flight)))
            .collect();

        let imported = execute(
            &vault,
            &connectors.iter().collect::<Vec<_>>(),
            DownloadLimits {
                max_concurrent,
                max_per_source: 2,
            },
        );

        assert_eq!(imported, Ok(3), "limit {max_concurrent}");
        assert_eq!(
            in_flight.most.load(Ordering::SeqCst),
            max_concurrent,
            "limit {max_concurrent}"
        );
    }
}

#[test]
fn a_candidate_awaiting_review_is_never_downloaded_ahead() {
    let mario = box_front("a", "Super Mario Bros.");
    let (ambiguous, releases) = ambiguous_candidate_and_releases();
    let ambiguous = AssetCandidate {
        source_id: SourceId::from("b"),
        source_url: "https://b.invalid/review-game.png".to_owned(),
        ..ambiguous
    };
    let mut library = releases;
    library.push(release_for(&mario, 1));
    let vault = FakeVault::with_library(library);
    let first = slow("a", vec![mario], None, None);
    let second = slow("b", vec![ambiguous], None, None);

    let imported = execute(
        &vault,
        &[&first, &second],
        DownloadLimits {
            max_concurrent: 2,
            max_per_source: 2,
        },
    );

    assert_eq!(imported, Ok(1));
    assert!(second.inner.downloads.borrow().is_empty());
    assert_eq!(vault.review_items.borrow().len(), 1);
}

#[test]
fn a_pause_starts_no_further_download() {
    let candidates: Vec<AssetCandidate> = ["a", "b", "c"]
        .iter()
        .map(|source_id| box_front(source_id, &format!("Game {source_id}")))
        .collect();
    let vault = FakeVault::with_library(
        candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| release_for(candidate, index as i64 + 1))
            .collect(),
    );
    // The first download lets a human pause the run; the other two wait their turn.
    let pause = || {
        let run_id = *vault.runs.borrow().keys().next().unwrap();
        pause_acquisition_run(&vault, run_id).unwrap();
    };
    let mut connectors: Vec<SlowConnector<'_>> = ["a", "b", "c"]
        .into_iter()
        .zip(candidates)
        .map(|(source_id, candidate)| slow(source_id, vec![candidate], None, None))
        .collect();
    connectors[0].during_download = Some(&pause);

    execute(
        &vault,
        &connectors.iter().collect::<Vec<_>>(),
        DownloadLimits {
            max_concurrent: 1,
            max_per_source: 2,
        },
    )
    .unwrap();

    assert!(connectors[1].inner.downloads.borrow().is_empty());
    assert!(connectors[2].inner.downloads.borrow().is_empty());
}

#[test]
fn a_pause_landing_before_a_download_thread_starts_leaves_its_work_queued() {
    let mario = box_front("a", "Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&mario, 1)]);
    // The execution already took the work and awaits its download when the pause lands.
    *vault.pause_during_next_status_read.borrow_mut() = Some(Duration::from_millis(200));
    let only = slow("a", vec![mario], None, None);

    let imported = execute(
        &vault,
        &[&only],
        DownloadLimits {
            max_concurrent: 1,
            max_per_source: 2,
        },
    );

    assert_eq!(imported, Ok(0));
    assert!(only.inner.downloads.borrow().is_empty());
    let run_id = *vault.runs.borrow().keys().next().unwrap();
    let run = vault.run(run_id);
    assert_eq!(run.status, AcquisitionRunStatus::Paused);
    assert_eq!(run.queued_work, 1);
}

#[test]
fn a_download_failing_once_the_run_is_paused_still_fails_its_source() {
    let mario = box_front("a", "Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&mario, 1)]);
    // The execution reads the queue to process the download only once a human paused the run
    // while it was under way.
    let (paused, go_on) = mpsc::channel();
    *vault.queue_read_gate.borrow_mut() = Some((1, go_on));
    let pause = || {
        let run_id = *vault.runs.borrow().keys().next().unwrap();
        pause_acquisition_run(&vault, run_id).unwrap();
        paused.send(()).unwrap();
    };
    let mut only = slow("a", vec![mario.clone()], None, None);
    only.inner
        .failing_downloads
        .insert(mario.source_url.clone());
    only.during_download = Some(&pause);

    let result = execute(
        &vault,
        &[&only],
        DownloadLimits {
            max_concurrent: 1,
            max_per_source: 1,
        },
    );

    assert!(result.is_err(), "{result:?}");
    assert_eq!(vault.source_failures.borrow().len(), 1);
    let run_id = *vault.runs.borrow().keys().next().unwrap();
    let run = vault.run(run_id);
    assert_eq!(run.status, AcquisitionRunStatus::Paused);
    assert_eq!(run.queued_work, 1);
}

#[test]
fn downloads_of_one_source_overlap_up_to_its_limit() {
    let (mario, tetris) = (
        box_front("a", "Super Mario Bros."),
        box_front("a", "Tetris"),
    );
    let vault = FakeVault::with_library(vec![release_for(&mario, 1), release_for(&tetris, 2)]);
    let rendezvous = Rendezvous::new(2);
    let only = slow("a", vec![mario, tetris], Some(&rendezvous), None);

    let imported = execute(
        &vault,
        &[&only],
        DownloadLimits {
            max_concurrent: 4,
            max_per_source: 2,
        },
    );

    assert_eq!(imported, Ok(2));
}

#[test]
fn media_downloaded_ahead_for_a_later_round_is_downloaded_once() {
    let candidates: Vec<AssetCandidate> = ["Mario", "Tetris", "Zelda"]
        .iter()
        .map(|title| box_front("a", title))
        .collect();
    let vault = FakeVault::with_library(
        candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| release_for(candidate, index as i64 + 1))
            .collect(),
    );
    let only = slow("a", candidates, None, None);

    let imported = execute(
        &vault,
        &[&only],
        DownloadLimits {
            max_concurrent: 4,
            max_per_source: 2,
        },
    );

    assert_eq!(imported, Ok(3));
    assert_eq!(only.inner.downloads.borrow().len(), 3);
}

#[test]
fn the_per_source_limit_bounds_the_downloads_of_one_source() {
    for max_per_source in [1, 2] {
        let candidates: Vec<AssetCandidate> = ["Mario", "Tetris", "Zelda"]
            .iter()
            .map(|title| box_front("a", title))
            .collect();
        let vault = FakeVault::with_library(
            candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| release_for(candidate, index as i64 + 1))
                .collect(),
        );
        let in_flight = InFlight::default();
        let only = slow("a", candidates, None, Some(&in_flight));

        let imported = execute(
            &vault,
            &[&only],
            DownloadLimits {
                max_concurrent: 4,
                max_per_source,
            },
        );

        assert_eq!(imported, Ok(3), "limit {max_per_source}");
        assert_eq!(
            in_flight.most.load(Ordering::SeqCst),
            max_per_source,
            "limit {max_per_source}"
        );
    }
}

#[test]
fn every_sources_next_download_starts_before_any_source_looks_further_ahead() {
    let (first, second, other) = (
        box_front("a", "First"),
        box_front("a", "Second"),
        box_front("b", "Other"),
    );
    let vault = FakeVault::with_library(vec![
        release_for(&first, 1),
        release_for(&second, 2),
        release_for(&other, 3),
    ]);
    let started = Shared::new(Vec::new());
    let mut a = slow("a", vec![first, second], None, None);
    let mut b = slow("b", vec![other], None, None);
    a.started = Some(&started);
    b.started = Some(&started);

    // One download at a time: the Source queued first must not take it twice in a row.
    let imported = execute(
        &vault,
        &[&a, &b],
        DownloadLimits {
            max_concurrent: 1,
            max_per_source: 2,
        },
    );

    assert_eq!(imported, Ok(3));
    assert_eq!(*started.borrow(), ["First", "Other", "Second"]);
}

#[test]
fn a_sources_downloads_start_in_queue_order_whichever_round_read_them() {
    let candidates: Vec<AssetCandidate> = ["First", "Second", "Third"]
        .iter()
        .map(|title| box_front("a", title))
        .chain([box_front("b", "Other")])
        .collect();
    let vault = FakeVault::with_library(
        candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| release_for(candidate, index as i64 + 1))
            .collect(),
    );
    let started = Shared::new(Vec::new());
    let mut a = slow("a", candidates[..3].to_vec(), None, None);
    let mut b = slow("b", candidates[3..].to_vec(), None, None);
    a.started = Some(&started);
    b.started = Some(&started);

    // The second round reads Third ahead while Second, read by the first, may still wait.
    let imported = execute(
        &vault,
        &[&a, &b],
        DownloadLimits {
            max_concurrent: 1,
            max_per_source: 2,
        },
    );

    assert_eq!(imported, Ok(4));
    assert_eq!(*started.borrow(), ["First", "Other", "Second", "Third"]);
}

/// A candidate of `source_id` bound for review: it matches two editions of `title` alike.
fn review_bound(
    source_id: &'static str,
    title: &str,
    first_release: i64,
) -> (AssetCandidate, Vec<LibraryEntry>) {
    let candidate = AssetCandidate {
        edition_name: "Collector".to_owned(),
        ..box_front(source_id, title)
    };
    let editions = ["Standard", "Deluxe"]
        .into_iter()
        .zip(first_release..)
        .map(|(edition_name, release_edition_id)| LibraryEntry {
            edition_name: edition_name.to_owned(),
            ..release_for(&candidate, release_edition_id)
        })
        .collect();
    (candidate, editions)
}

#[test]
fn work_bound_for_review_takes_no_place_in_a_sources_lookahead() {
    let (first_review, mut library) = review_bound("a", "Review Game", 401);
    let (second_review, more) = review_bound("a", "Other Review Game", 403);
    library.extend(more);
    let (mario, tetris) = (
        box_front("a", "Super Mario Bros."),
        box_front("a", "Tetris"),
    );
    library.extend([release_for(&mario, 1), release_for(&tetris, 2)]);
    let vault = FakeVault::with_library(library);
    // Each importable download waits for the other, past the work bound for review.
    let rendezvous = Rendezvous::new(2);
    let only = slow(
        "a",
        vec![first_review, mario, second_review, tetris],
        Some(&rendezvous),
        None,
    );

    let imported = execute(
        &vault,
        &[&only],
        DownloadLimits {
            max_concurrent: 4,
            max_per_source: 2,
        },
    );

    assert_eq!(imported, Ok(2));
    assert_eq!(vault.review_items.borrow().len(), 2);
}

#[test]
fn an_execution_resumed_after_a_download_saw_it_paused_downloads_its_later_work() {
    let (done, finished) = mpsc::channel();
    // A hung execution fails the test instead of holding up the suite.
    thread::spawn(move || {
        let candidates: Vec<AssetCandidate> = ["First", "Second", "Third"]
            .iter()
            .map(|title| box_front("a", title))
            .collect();
        let vault = FakeVault::with_library(
            candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| release_for(candidate, index as i64 + 1))
                .collect(),
        );
        // The first download ahead sees the run paused, which a human resumes right after.
        *vault.paused_at_next_status_read.borrow_mut() = true;
        let only = slow("a", candidates, None, None);

        let imported = execute(
            &vault,
            &[&only],
            DownloadLimits {
                max_concurrent: 1,
                max_per_source: 1,
            },
        );
        let _ = done.send(imported);
    });

    let imported = finished
        .recv_timeout(Duration::from_secs(10))
        .expect("the execution hung");

    assert_eq!(imported, Ok(3));
}
