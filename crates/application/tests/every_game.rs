use std::cell::RefCell;

use game_media_vault_application::{
    ApplicationError, CatalogPort, PlatformCatalogSourcePort, PortError, ReferenceCatalogRead,
    ReferenceCatalogRepositoryPort, expand_every_game,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequestDraft, AssetTypeSelector, GameSelection, ImportedAsset,
    ImportedReleaseEdition, LibraryEntry, PersistAsset, PlatformBoundGameSelector,
    ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField, RetentionPolicy,
    SourceSelection,
};

const NES: &str = "Nintendo - Nintendo Entertainment System";
const SNES: &str = "Nintendo - Super Nintendo Entertainment System";

/// A vault's releases, to which reference releases add.
#[derive(Default)]
struct Releases {
    entries: RefCell<Vec<LibraryEntry>>,
}

impl Releases {
    fn holding(entries: Vec<LibraryEntry>) -> Self {
        Self {
            entries: RefCell::new(entries),
        }
    }
}

impl CatalogPort for Releases {
    fn persist_asset(&self, _record: PersistAsset) -> Result<ImportedAsset, PortError> {
        unreachable!("expanding a request stores no media")
    }

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError> {
        Ok(self.entries.borrow().clone())
    }
}

impl ReferenceCatalogRepositoryPort for Releases {
    fn persist_reference_release(
        &self,
        record: ReferenceReleaseRecord,
    ) -> Result<ImportedReleaseEdition, PortError> {
        let id = self.entries.borrow().len() as i64 + 1;
        self.entries.borrow_mut().push(LibraryEntry {
            game_id: id,
            game_title: record.game_title,
            release_edition_id: id,
            platform: record.platform,
            region: record.region,
            edition_name: record.edition_name,
            assertions: record.assertions,
            assets: Vec::new(),
        });
        Ok(ImportedReleaseEdition {
            game_id: id,
            release_edition_id: id,
        })
    }
}

/// The game list of the NES alone.
struct NesGameList {
    asked: RefCell<Vec<String>>,
}

impl PlatformCatalogSourcePort for NesGameList {
    fn platform_releases(&self, platform: &str) -> Result<Option<ReferenceCatalogRead>, PortError> {
        self.asked.borrow_mut().push(platform.to_owned());
        Ok((platform == NES).then(|| ReferenceCatalogRead {
            releases: vec![ReferenceReleaseRecord {
                game_title: "Super Mario Bros.".to_owned(),
                platform: NES.to_owned(),
                region: "World".to_owned(),
                revision: None,
                edition_name: "Standard".to_owned(),
                assertions: vec![source_record("Super Mario Bros. (World)")],
            }],
            skipped_records: 0,
        }))
    }
}

fn game_lists() -> NesGameList {
    NesGameList {
        asked: RefCell::new(Vec::new()),
    }
}

/// The record a No-Intro datafile entry asserts, naming the entry as the datafile does.
fn source_record(raw_name: &str) -> ReleaseAssertion {
    ReleaseAssertion {
        source_id: "no-intro".into(),
        source_location: "https://raw.githubusercontent.com/nes.dat".to_owned(),
        field: ReleaseAssertionField::Identifier,
        qualifier: Some("source_record".to_owned()),
        value: format!("{}:{NES}{raw_name}", NES.len()),
    }
}

fn entry(id: i64, title: &str, platform: &str, region: &str, edition: &str) -> LibraryEntry {
    LibraryEntry {
        game_id: id,
        game_title: title.to_owned(),
        release_edition_id: id,
        platform: platform.to_owned(),
        region: region.to_owned(),
        edition_name: edition.to_owned(),
        assertions: Vec::new(),
        assets: Vec::new(),
    }
}

fn every_game_of(platforms: &[&str]) -> AcquisitionRequestDraft {
    AcquisitionRequestDraft {
        sources: SourceSelection::Auto,
        platforms: platforms
            .iter()
            .map(|platform| (*platform).to_owned())
            .collect(),
        games: GameSelection::All,
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    }
}

fn bound(games: &[&str]) -> GameSelection {
    GameSelection::PlatformBound(
        games
            .iter()
            .map(|game| PlatformBoundGameSelector {
                game: (*game).to_owned(),
                platform: NES.to_owned(),
            })
            .collect(),
    )
}

#[test]
fn every_game_of_a_known_platform_becomes_the_games_its_releases_name() {
    let mut mario = entry(1, "Super Mario Bros.", NES, "World", "Standard");
    mario
        .assertions
        .push(source_record("Super Mario Bros. (World)"));
    let releases = Releases::holding(vec![
        mario,
        // A release no datafile names is named by its title, region and edition.
        entry(2, "Tetris", NES, "USA", "Rev 1"),
        entry(3, "Excitebike", NES, "Unknown", "Standard"),
        entry(4, "Super Metroid", SNES, "USA", "Standard"),
    ]);
    let lists = game_lists();

    let expanded = expand_every_game(&releases, &releases, &lists, every_game_of(&[NES])).unwrap();

    assert_eq!(
        expanded.games,
        bound(&[
            "Super Mario Bros. (World)",
            "Tetris (USA) (Rev 1)",
            "Excitebike"
        ])
    );
    // A platform the vault knows needs no game list.
    assert!(lists.asked.borrow().is_empty());
}

#[test]
fn the_game_list_of_a_platform_the_vault_does_not_know_is_fetched_first() {
    let releases = Releases::default();
    let lists = game_lists();

    let expanded = expand_every_game(&releases, &releases, &lists, every_game_of(&[NES])).unwrap();

    assert_eq!(expanded.games, bound(&["Super Mario Bros. (World)"]));
    assert_eq!(*lists.asked.borrow(), [NES]);
    assert_eq!(releases.entries.borrow().len(), 1);
}

#[test]
fn a_request_naming_its_games_stays_as_it_is() {
    let releases = Releases::default();
    let lists = game_lists();
    let mut named = every_game_of(&[NES]);
    named.games = GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]);

    let expanded = expand_every_game(&releases, &releases, &lists, named.clone()).unwrap();

    assert_eq!(expanded, named);
    assert!(lists.asked.borrow().is_empty());
}

#[test]
fn a_platform_without_a_known_game_list_keeps_the_request_for_every_game() {
    let releases = Releases::default();
    let lists = game_lists();

    // Sources that read whole platforms still serve it.
    let expanded = expand_every_game(&releases, &releases, &lists, every_game_of(&[SNES])).unwrap();

    assert_eq!(expanded, every_game_of(&[SNES]));
}

#[test]
fn an_invalid_request_is_refused_before_any_game_list_is_fetched() {
    let releases = Releases::default();
    let lists = game_lists();
    let mut sourceless = every_game_of(&[NES]);
    sourceless.sources = SourceSelection::Explicit(Vec::new());

    let error = expand_every_game(&releases, &releases, &lists, sourceless).unwrap_err();

    assert!(
        matches!(error, ApplicationError::Validation(_)),
        "{error:?}"
    );
    assert!(lists.asked.borrow().is_empty());
}
