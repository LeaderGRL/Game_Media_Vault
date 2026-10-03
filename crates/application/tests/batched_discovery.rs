mod support;

use std::io::{Cursor, Read};

use game_media_vault_application::{
    ApplicationError, ConnectorPort, DownloadLimits, PortError, acquire_run_with_connectors,
    resume_acquisition_run, start_acquisition_run_with_connectors,
};
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRunStatus, AssetCandidate, AssetType,
    ConnectorCapabilities, GameSelection, ImportedAsset, PlatformBoundGameSelector,
    SourceDiscovery, SourceId, SourceSelection,
};
use support::*;

const NES: &str = "Nintendo - Nintendo Entertainment System";

const GAMES: [&str; 5] = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"];

/// A platform the per-game Sources of these tests do not serve.
const SATURN: &str = "Sega - Saturn";

/// A Source that looks games up one by one, as those under a quota do: it serves a Box Front
/// for every game a request names, at most `batch` games per discovery.
struct PerGameSource {
    source_id: &'static str,
    batch: usize,
    /// The games each discovery looked up, in order.
    lookups: Shared<Vec<Vec<String>>>,
    /// The discovery, counting from one, that fails as a spent quota does, if any.
    quota_spent_at: Shared<Option<usize>>,
    /// Media downloaded so far.
    downloads: Shared<usize>,
    /// How many media were downloaded when each discovery started.
    downloads_at_lookups: Shared<Vec<usize>>,
}

impl PerGameSource {
    fn new(source_id: &'static str, batch: usize) -> Self {
        Self {
            source_id,
            batch,
            lookups: Shared::new(Vec::new()),
            quota_spent_at: Shared::new(None),
            downloads: Shared::new(0),
            downloads_at_lookups: Shared::new(Vec::new()),
        }
    }

    fn lookups(&self) -> Vec<Vec<String>> {
        self.lookups.borrow().clone()
    }
}

fn games_of(request: &AcquisitionRequest) -> Vec<String> {
    match request.games() {
        GameSelection::Explicit(games) => games.clone(),
        GameSelection::PlatformBound(games) => games.iter().map(|game| game.game.clone()).collect(),
        other => panic!("the tests name their games: {other:?}"),
    }
}

/// The platforms a request names its games on, or its own platforms.
fn platforms_of(request: &AcquisitionRequest) -> Vec<String> {
    match request.games() {
        GameSelection::PlatformBound(games) => {
            games.iter().map(|game| game.platform.clone()).collect()
        }
        _ => request.platforms().to_vec(),
    }
}

impl ConnectorPort for PerGameSource {
    fn source_id(&self) -> &'static str {
        self.source_id
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::BoxFront],
            direct_media_download: true,
        }
    }

    fn discovery_batch_size(&self) -> Option<usize> {
        Some(self.batch)
    }

    /// Refuses more lookups than one discovery makes, as a Source protecting its quota does: one
    /// per requested game and platform.
    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        if platforms_of(request)
            .iter()
            .any(|platform| platform == SATURN)
        {
            return Ok(Some(format!("{SATURN} is not served")));
        }
        let lookups = match request.games() {
            GameSelection::Explicit(games) => games.len() * request.platforms().len(),
            _ => games_of(request).len(),
        };
        Ok((lookups > self.batch).then(|| format!("more than {} lookups at once", self.batch)))
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        let games = games_of(request);
        self.lookups.borrow_mut().push(games.clone());
        let downloads = *self.downloads.borrow();
        self.downloads_at_lookups.borrow_mut().push(downloads);
        if *self.quota_spent_at.borrow() == Some(self.lookups.borrow().len()) {
            return Err(PortError::new("the daily quota is spent".to_owned()));
        }
        Ok(games
            .iter()
            .map(|game| AssetCandidate {
                source_id: SourceId::from(self.source_id),
                source_url: format!("https://{}.invalid/{game}.png", self.source_id),
                ..candidate(game)
            })
            .collect())
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        *self.downloads.borrow_mut() += 1;
        Ok(Box::new(Cursor::new(
            candidate.source_url.clone().into_bytes(),
        )))
    }
}

/// A Box Front of every game of `GAMES` from `sources`.
fn draft(sources: &[&str]) -> AcquisitionRequestDraft {
    AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(sources.iter().map(|id| (*id).to_owned()).collect()),
        games: GameSelection::Explicit(GAMES.iter().map(|game| (*game).to_owned()).collect()),
        ..request_draft()
    }
}

fn vault_knowing_every_game() -> FakeVault {
    let vault = FakeVault::default();
    *vault.library.borrow_mut() = GAMES
        .iter()
        .zip(1..)
        .map(|(game, id)| release_for(&candidate(game), id))
        .collect();
    vault
}

fn execute(
    vault: &FakeVault,
    sources: &[&PerGameSource],
    run_id: i64,
) -> Result<Vec<ImportedAsset>, ApplicationError> {
    let connectors: Vec<&dyn ConnectorPort> = sources
        .iter()
        .map(|source| *source as &dyn ConnectorPort)
        .collect();
    acquire_run_with_connectors(
        vault,
        vault,
        vault,
        &FakeStore::default(),
        &connectors,
        run_id,
        matching_policy(),
        DownloadLimits::default(),
    )
}

fn start(vault: &FakeVault, sources: &[&PerGameSource]) -> i64 {
    let connectors: Vec<&dyn ConnectorPort> = sources
        .iter()
        .map(|source| *source as &dyn ConnectorPort)
        .collect();
    let ids: Vec<&str> = sources.iter().map(|source| source.source_id).collect();
    start_acquisition_run_with_connectors(vault, draft(&ids), &connectors)
        .unwrap()
        .id
}

fn names(games: &[&str]) -> Vec<String> {
    games.iter().map(|game| (*game).to_owned()).collect()
}

#[test]
fn a_source_looking_games_up_one_by_one_is_discovered_in_batches() {
    let vault = vault_knowing_every_game();
    let source = PerGameSource::new("per-game", 2);
    // Planning asks the Source about its first batch, which it can look up at once.
    let run_id = start(&vault, &[&source]);

    let imported = execute(&vault, &[&source], run_id).unwrap();

    assert_eq!(
        source.lookups(),
        [
            names(&["Alpha", "Bravo"]),
            names(&["Charlie", "Delta"]),
            names(&["Echo"])
        ]
    );
    assert_eq!(imported.len(), 5);
    assert_eq!(vault.run(run_id).status, AcquisitionRunStatus::Completed);
}

#[test]
fn an_execution_a_spent_quota_stopped_resumes_with_the_next_batch() {
    let vault = vault_knowing_every_game();
    let source = PerGameSource::new("per-game", 2);
    let run_id = start(&vault, &[&source]);
    *source.quota_spent_at.borrow_mut() = Some(2);

    let error = execute(&vault, &[&source], run_id).unwrap_err();

    // The batch discovered before the quota ran out is acquired, and the run waits.
    assert!(error.to_string().contains("quota"), "{error}");
    assert_eq!(vault.run(run_id).completed_work, 2);
    // The run says how far the Source's discovery went: two of the five games.
    assert_eq!(
        vault.run(run_id).discoveries,
        [SourceDiscovery::new("per-game", false, 2)]
    );
    assert_eq!(vault.run(run_id).status, AcquisitionRunStatus::Running);

    *source.quota_spent_at.borrow_mut() = None;
    source.lookups.borrow_mut().clear();
    execute(&vault, &[&source], run_id).unwrap();

    assert_eq!(
        source.lookups(),
        [names(&["Charlie", "Delta"]), names(&["Echo"])]
    );
    assert_eq!(vault.run(run_id).completed_work, 5);
    assert_eq!(vault.run(run_id).status, AcquisitionRunStatus::Completed);
}

#[test]
fn a_source_whose_quota_is_spent_leaves_the_others_discovering_every_batch() {
    let vault = vault_knowing_every_game();
    let spent = PerGameSource::new("spent", 2);
    *spent.quota_spent_at.borrow_mut() = Some(1);
    let other = PerGameSource::new("other", 2);
    let run_id = start(&vault, &[&spent, &other]);

    execute(&vault, &[&spent, &other], run_id).unwrap_err();

    // The spent Source is asked once only, and the other one is asked about every game.
    assert_eq!(spent.lookups().len(), 1);
    assert_eq!(other.lookups().len(), 3);
    assert_eq!(vault.run(run_id).completed_work, 5);
    assert_eq!(vault.run(run_id).status, AcquisitionRunStatus::Running);
}

#[test]
fn a_batch_counts_a_lookup_for_each_platform_a_game_is_looked_up_on() {
    let vault = vault_knowing_every_game();
    let source = PerGameSource::new("per-game", 2);
    let connectors: Vec<&dyn ConnectorPort> = vec![&source];
    let mut two_platforms = draft(&["per-game"]);
    two_platforms
        .platforms
        .push("Nintendo - Super Nintendo Entertainment System".to_owned());
    let run_id = start_acquisition_run_with_connectors(&vault, two_platforms, &connectors)
        .unwrap()
        .id;

    execute(&vault, &[&source], run_id).unwrap();

    // Two lookups a batch: one game on each of the two platforms.
    assert_eq!(source.lookups().len(), GAMES.len());
    assert!(source.lookups().iter().all(|games| games.len() == 1));
}

#[test]
fn the_work_of_a_recorded_batch_is_executed_before_the_next_batch_is_discovered() {
    let vault = vault_knowing_every_game();
    let source = PerGameSource::new("per-game", 2);
    let run_id = start(&vault, &[&source]);
    // A pause lands while the first batch is discovered: it is recorded, not executed.
    *vault.status_before_next_discovery.borrow_mut() = Some(AcquisitionRunStatus::Paused);
    execute(&vault, &[&source], run_id).unwrap();
    assert_eq!(source.lookups().len(), 1);
    assert_eq!(*source.downloads.borrow(), 0);

    resume_acquisition_run(&vault, run_id).unwrap();
    execute(&vault, &[&source], run_id).unwrap();

    // Each batch is looked up once the media of the previous one are downloaded.
    assert_eq!(*source.downloads_at_lookups.borrow(), [0, 2, 4]);
    assert_eq!(vault.run(run_id).status, AcquisitionRunStatus::Completed);
}

#[test]
fn a_game_named_twice_is_looked_up_once() {
    let vault = vault_knowing_every_game();
    let source = PerGameSource::new("per-game", 2);
    let connectors: Vec<&dyn ConnectorPort> = vec![&source];
    let mut repeated = draft(&["per-game"]);
    repeated.games = GameSelection::Explicit(names(&[
        "Alpha", "Alpha", "Bravo", "alpha", "Bravo", "Charlie",
    ]));
    let run_id = start_acquisition_run_with_connectors(&vault, repeated, &connectors)
        .unwrap()
        .id;

    execute(&vault, &[&source], run_id).unwrap();

    assert_eq!(
        source.lookups(),
        [names(&["Alpha", "Bravo"]), names(&["Charlie"])]
    );
}

#[test]
fn a_platform_a_later_batch_names_is_checked_when_the_run_starts() {
    let vault = vault_knowing_every_game();
    let source = PerGameSource::new("per-game", 2);
    let connectors: Vec<&dyn ConnectorPort> = vec![&source];
    let mut bound = draft(&["per-game"]);
    bound.platforms.push(SATURN.to_owned());
    bound.games = GameSelection::PlatformBound(
        [
            ("Alpha", NES),
            ("Bravo", NES),
            ("Charlie", NES),
            ("Panzer", SATURN),
        ]
        .iter()
        .map(|(game, platform)| PlatformBoundGameSelector {
            game: (*game).to_owned(),
            platform: (*platform).to_owned(),
        })
        .collect(),
    );

    // The Source refuses the Saturn game of the second batch, so no run starts without it.
    let error = start_acquisition_run_with_connectors(&vault, bound, &connectors).unwrap_err();

    assert!(error.to_string().contains(SATURN), "{error}");
    assert!(source.lookups().is_empty());
}
