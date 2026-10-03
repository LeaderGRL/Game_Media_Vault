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

#[test]
fn a_platform_spelled_otherwise_is_the_one_the_vault_knows_by_its_list_name() {
    let releases = Releases::holding(vec![entry(1, "Tetris", NES, "USA", "Standard")]);
    let lists = game_lists();

    // Game lists are found regardless of case and punctuation, and so are their releases.
    let expanded = expand_every_game(
        &releases,
        &releases,
        &lists,
        every_game_of(&["nintendo-nintendo entertainment system"]),
    )
    .unwrap();

    // Sources and the matcher then see the platform as the vault names it.
    assert_eq!(expanded.platforms, [NES]);
    assert_eq!(expanded.games, bound(&["Tetris (USA)"]));
    assert!(lists.asked.borrow().is_empty());
}

#[test]
fn an_unsupported_limit_is_refused_before_any_game_list_is_fetched() {
    let releases = Releases::default();
    let lists = game_lists();
    let mut limited = every_game_of(&[NES]);
    limited.limits.max_games = Some(5);

    let error = expand_every_game(&releases, &releases, &lists, limited).unwrap_err();

    assert!(
        matches!(error, ApplicationError::UnsupportedRequest(_)),
        "{error:?}"
    );
    assert!(lists.asked.borrow().is_empty());
}

/// A release a No-Intro datafile names `raw_name`, of `region`.
fn listed(id: i64, title: &str, region: &str, raw_name: &str) -> LibraryEntry {
    let mut release = entry(id, title, NES, region, "Standard");
    release.assertions.push(source_record(raw_name));
    release
}

#[test]
fn every_game_kept_to_a_region_names_its_releases_and_the_worldwide_ones() {
    let releases = Releases::holding(vec![
        listed(1, "Super Mario Bros.", "World", "Super Mario Bros. (World)"),
        listed(2, "Tetris", "USA", "Tetris (USA)"),
        listed(3, "Tetris", "Europe", "Tetris (Europe)"),
        listed(4, "Zelda", "USA, Europe", "Zelda (USA, Europe)"),
    ]);
    let mut european = every_game_of(&[NES]);
    european.regions = vec!["europe".to_owned()];

    let expanded = expand_every_game(&releases, &releases, &game_lists(), european).unwrap();

    assert_eq!(
        expanded.games,
        bound(&[
            "Super Mario Bros. (World)",
            "Tetris (Europe)",
            "Zelda (USA, Europe)"
        ])
    );
    // Sources that tell regions apart still keep to the region's media.
    assert_eq!(expanded.regions, ["europe"]);
}

#[test]
fn every_game_kept_to_a_language_names_the_releases_whose_name_or_region_speaks_it() {
    let releases = Releases::holding(vec![
        listed(1, "Asterix", "Europe", "Asterix (Europe) (En,Fr,De)"),
        listed(2, "Tintin", "France", "Tintin (France)"),
        listed(3, "Tetris", "USA", "Tetris (USA)"),
        listed(4, "Mario", "Japan", "Mario (Japan)"),
    ]);
    let mut french = every_game_of(&[NES]);
    french.languages = vec!["fr".to_owned()];

    let expanded = expand_every_game(&releases, &releases, &game_lists(), french).unwrap();

    assert_eq!(
        expanded.games,
        bound(&["Asterix (Europe) (En,Fr,De)", "Tintin (France)"])
    );
    // No Source tells languages apart: the games named are the language's.
    assert!(expanded.languages.is_empty());
}
