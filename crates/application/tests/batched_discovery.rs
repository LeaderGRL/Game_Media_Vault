mod support;

use std::io::{Cursor, Read};

use game_media_vault_application::{
    ApplicationError, ConnectorPort, DownloadLimits, PortError, acquire_run_with_connectors,
    start_acquisition_run_with_connectors,
};
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRunStatus, AssetCandidate, AssetType,
    ConnectorCapabilities, GameSelection, ImportedAsset, SourceId, SourceSelection,
};
use support::*;

const GAMES: [&str; 5] = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"];

/// A Source that looks games up one by one, as those under a quota do: it serves a Box Front
/// for every game a request names, at most `batch` games per discovery.
struct PerGameSource {
    source_id: &'static str,
    batch: usize,
    /// The games each discovery looked up, in order.
    lookups: Shared<Vec<Vec<String>>>,
    /// The discovery, counting from one, that fails as a spent quota does, if any.
    quota_spent_at: Shared<Option<usize>>,
}

impl PerGameSource {
    fn new(source_id: &'static str, batch: usize) -> Self {
        Self {
            source_id,
            batch,
            lookups: Shared::new(Vec::new()),
            quota_spent_at: Shared::new(None),
        }
    }

    fn lookups(&self) -> Vec<Vec<String>> {
        self.lookups.borrow().clone()
    }
}

fn games_of(request: &AcquisitionRequest) -> Vec<String> {
    match request.games() {
        GameSelection::Explicit(games) => games.clone(),
        other => panic!("the tests name their games: {other:?}"),
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

    /// Refuses more games than one discovery looks up, as a Source protecting its quota does.
    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        Ok((games_of(request).len() > self.batch)
            .then(|| format!("more than {} games at once", self.batch)))
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        let games = games_of(request);
        self.lookups.borrow_mut().push(games.clone());
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
