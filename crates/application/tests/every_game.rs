use std::{cell::RefCell, io::Read};

use game_media_vault_application::{
    ApplicationError, CatalogPort, ConnectorPort, PlatformCatalogSourcePort, PortError,
    ReferenceCatalogRead, ReferenceCatalogRepositoryPort, expand_every_game,
    plan_acquisition_request,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType,
    AssetTypeSelector, ConnectorCapabilities, GameSelection, ImportedAsset, ImportedReleaseEdition,
    LibraryEntry, PersistAsset, PlatformBoundGameSelector, ReferenceReleaseRecord,
    ReleaseAssertion, ReleaseAssertionField, RetentionPolicy, SourceSelection,
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
    limited.limits.max_downloads = Some(5);

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
    // Sources that tell regions apart still keep to the region's media, and worldwide ones.
    assert_eq!(expanded.regions, ["europe", "World"]);
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

#[test]
fn a_tag_that_lists_no_language_leaves_the_language_to_the_region() {
    let releases = Releases::holding(vec![
        listed(1, "Tetris", "USA", "Tetris (USA) (Unl)"),
        listed(2, "Zelda", "Europe", "Zelda (Europe) (Alt)"),
    ]);
    let mut english = every_game_of(&[NES]);
    english.languages = vec!["en".to_owned()];

    let expanded = expand_every_game(&releases, &releases, &game_lists(), english).unwrap();

    assert_eq!(
        expanded.games,
        bound(&["Tetris (USA) (Unl)", "Zelda (Europe) (Alt)"])
    );
}

#[test]
fn every_game_kept_to_a_region_no_release_is_of_is_refused_rather_than_broadened() {
    let releases = Releases::holding(vec![listed(1, "Mario", "Japan", "Mario (Japan)")]);
    let mut european = every_game_of(&[NES]);
    european.regions = vec!["Europe".to_owned()];

    let error = expand_every_game(&releases, &releases, &game_lists(), european).unwrap_err();

    assert!(
        matches!(error, ApplicationError::NoMatchingReleases(_)),
        "{error:?}"
    );
}

/// A Source that refuses language filters, as every Source does.
struct LanguageBlind;

impl ConnectorPort for LanguageBlind {
    fn source_id(&self) -> &'static str {
        "language-blind"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::BoxFront],
            direct_media_download: true,
        }
    }

    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        Ok((!request.languages().is_empty()).then(|| "cannot tell languages apart".to_owned()))
    }

    fn discover(&self, _request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        unreachable!("planning never discovers")
    }

    fn download(&self, _candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        unreachable!("planning never downloads")
    }
}

#[test]
fn a_request_for_every_game_is_planned_as_its_run_would_start() {
    let releases = Releases::holding(vec![listed(1, "Tintin", "France", "Tintin (France)")]);
    let mut french = every_game_of(&[NES]);
    french.languages = vec!["Fr".to_owned()];

    // Expanded, the request names the French games and keeps no language filter.
    let plan = plan_acquisition_request(
        &releases,
        &releases,
        &game_lists(),
        french,
        &[&LanguageBlind as &dyn ConnectorPort],
    )
    .unwrap();

    assert_eq!(plan.sources.len(), 1);
}

#[test]
fn a_request_kept_to_a_region_keeps_worldwide_media_too() {
    let releases = Releases::holding(vec![listed(
        1,
        "Super Mario Bros.",
        "World",
        "Super Mario Bros. (World)",
    )]);
    let mut european = every_game_of(&[NES]);
    european.regions = vec!["Europe".to_owned()];

    let expanded = expand_every_game(&releases, &releases, &game_lists(), european).unwrap();

    // Sources that keep to the request's regions keep to worldwide media, which the worldwide
    // releases kept need, as well.
    assert_eq!(expanded.regions, ["Europe", "World"]);
}

#[test]
fn every_region_no_intro_names_implies_its_language() {
    let releases = Releases::holding(vec![
        listed(1, "Wiedzmin", "Poland", "Wiedzmin (Poland)"),
        listed(2, "Lego", "Denmark", "Lego (Denmark)"),
        listed(3, "Hra", "Czech", "Hra (Czech)"),
        listed(4, "Peli", "Finland", "Peli (Finland)"),
        listed(5, "Elite", "United Kingdom", "Elite (United Kingdom)"),
    ]);
    let mut nordic_and_slavic = every_game_of(&[NES]);
    nordic_and_slavic.languages = ["Pl", "Da", "Cs", "Fi", "En"].map(str::to_owned).to_vec();

    let expanded =
        expand_every_game(&releases, &releases, &game_lists(), nordic_and_slavic).unwrap();

    assert_eq!(
        expanded.games,
        bound(&[
            "Wiedzmin (Poland)",
            "Lego (Denmark)",
            "Hra (Czech)",
            "Peli (Finland)",
            "Elite (United Kingdom)"
        ])
    );
}

/// Four NES games, listed out of alphabetical order.
fn four_games() -> Releases {
    Releases::holding(vec![
        listed(1, "Zelda", "USA", "Zelda (USA)"),
        listed(2, "Asterix", "Europe", "Asterix (Europe)"),
        listed(3, "Tetris", "USA", "Tetris (USA)"),
        listed(4, "Mario", "World", "Mario (World)"),
    ])
}

#[test]
fn every_game_kept_to_a_number_names_the_first_games_of_each_platform_by_name() {
    let releases = four_games();
    let mut two = every_game_of(&[NES]);
    two.limits.max_games = Some(2);

    let expanded = expand_every_game(&releases, &releases, &game_lists(), two).unwrap();

    assert_eq!(
        expanded.games,
        bound(&["Asterix (Europe)", "Mario (World)"])
    );
    // The games named are the limit: planning has none left to apply.
    assert_eq!(expanded.limits, AcquisitionLimits::default());
}

#[test]
fn every_game_kept_to_a_share_names_that_share_of_each_platform_rounded_up() {
    let releases = four_games();
    let mut a_third = every_game_of(&[NES]);
    a_third.limits.games_percent = Some(30);

    let expanded = expand_every_game(&releases, &releases, &game_lists(), a_third).unwrap();

    // 30 % of four games is 1.2, kept as two.
    assert_eq!(
        expanded.games,
        bound(&["Asterix (Europe)", "Mario (World)"])
    );
    assert_eq!(expanded.limits, AcquisitionLimits::default());
}

#[test]
fn a_share_of_games_outside_one_to_a_hundred_percent_is_invalid() {
    for percent in [0, 101] {
        let mut draft = every_game_of(&[NES]);
        draft.limits.games_percent = Some(percent);

        let error =
            expand_every_game(&four_games(), &four_games(), &game_lists(), draft).unwrap_err();

        assert!(
            matches!(error, ApplicationError::Validation(_)),
            "{percent}: {error:?}"
        );
    }
}

#[test]
fn a_game_limit_no_game_list_can_apply_is_left_for_planning_to_refuse() {
    let releases = Releases::default();
    let mut five = every_game_of(&[SNES]);
    five.limits.max_games = Some(5);

    // No list names the games of the platform, so nothing keeps the request to five of them.
    let expanded = expand_every_game(&releases, &releases, &game_lists(), five.clone()).unwrap();
    let error = plan_acquisition_request(
        &releases,
        &releases,
        &game_lists(),
        five.clone(),
        &[&LanguageBlind as &dyn ConnectorPort],
    )
    .unwrap_err();

    assert_eq!(expanded, five);
    assert!(
        matches!(error, ApplicationError::UnsupportedRequest(_)),
        "{error:?}"
    );
}
