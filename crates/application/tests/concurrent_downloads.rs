mod support;

use std::{
    io::Read,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use game_media_vault_application::{
    ConnectorPort, DownloadLimits, PortError, acquire_run_with_connectors,
    start_acquisition_run_with_connectors,
};
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType, AssetTypeSelector,
    ConnectorCapabilities, SourceId, SourceSelection,
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
        DownloadLimits { max_concurrent: 2 },
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
            DownloadLimits { max_concurrent },
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
        DownloadLimits { max_concurrent: 2 },
    );

    assert_eq!(imported, Ok(1));
    assert!(second.inner.downloads.borrow().is_empty());
    assert_eq!(vault.review_items.borrow().len(), 1);
}
