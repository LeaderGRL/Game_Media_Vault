use std::{
    collections::HashSet,
    io::{Cursor, Read, Write},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_connectors::{
    DatasetCache, Fetched, HttpTransport, LAUNCHBOX_GAMES_DB_SOURCE_ID, LAUNCHBOX_METADATA_URL,
    LaunchBoxGamesDbConnector, Validators,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType,
    AssetTypeSelector, GameSelection, PlatformBoundGameSelector, RetentionPolicy, SourceId,
    SourceSelection, packaging_platforms,
};

const NES: &str = "Nintendo - Nintendo Entertainment System";
const SNES: &str = "Nintendo - Super Nintendo Entertainment System";
const GAME_BOY: &str = "Nintendo - Game Boy";

/// A slice of the LaunchBox Games Database, with the records discovery must skip.
const METADATA_XML: &str = r#"<?xml version="1.0" standalone="yes"?>
<LaunchBox>
  <Game>
    <Name>Super Mario Bros.</Name>
    <ReleaseYear>1985</ReleaseYear>
    <DatabaseID>140</DatabaseID>
    <Platform>Nintendo Entertainment System</Platform>
  </Game>
  <Game>
    <Name>Tetris</Name>
    <DatabaseID>220</DatabaseID>
    <Platform>Nintendo Game Boy</Platform>
  </Game>
  <Game>
    <Name>The Legend of Zelda</Name>
    <DatabaseID>141</DatabaseID>
    <Platform>Nintendo Entertainment System</Platform>
  </Game>
  <Game>
    <Name>Record Without An Identifier</Name>
    <Platform>Nintendo Entertainment System</Platform>
  </Game>
  <Game>
    <Name>The Legend of Zelda: A Link to the Past</Name>
    <DatabaseID>300</DatabaseID>
    <Platform>Super Nintendo Entertainment System</Platform>
  </Game>
  <GameAlternateName>
    <AlternateName>Super Mario Brothers</AlternateName>
    <DatabaseID>140</DatabaseID>
  </GameAlternateName>
  <GameImage>
    <DatabaseID>140</DatabaseID>
    <FileName>2d0a6c62-smb-front-us.jpg</FileName>
    <Type>Box - Front</Type>
    <Region>North America</Region>
    <CRC32>1A2B3C4D</CRC32>
  </GameImage>
  <GameImage>
    <DatabaseID>140</DatabaseID>
    <FileName>8a1f-smb-front-jp.jpg</FileName>
    <Type>Box - Front</Type>
    <Region>Japan</Region>
  </GameImage>
  <GameImage>
    <DatabaseID>140</DatabaseID>
    <FileName>77aa-smb-snap.png</FileName>
    <Type>Screenshot - Gameplay</Type>
  </GameImage>
  <GameImage>
    <DatabaseID>140</DatabaseID>
    <Type>Box - Front</Type>
  </GameImage>
  <GameImage>
    <DatabaseID>140</DatabaseID>
    <FileName>../escape.jpg</FileName>
    <Type>Box - Front</Type>
  </GameImage>
  <GameImage>
    <DatabaseID>141</DatabaseID>
    <FileName>zelda-front.jpg</FileName>
    <Type>Box - Front</Type>
    <Region>Europe</Region>
  </GameImage>
  <GameImage>
    <DatabaseID>220</DatabaseID>
    <FileName>tetris-front.jpg</FileName>
    <Type>Box - Front</Type>
  </GameImage>
  <GameImage>
    <DatabaseID>220</DatabaseID>
    <FileName>tetris-front-us.jpg</FileName>
    <Type>Box - Front</Type>
    <Region>United States</Region>
  </GameImage>
  <GameImage>
    <DatabaseID>220</DatabaseID>
    <FileName>tetris-front-uk.jpg</FileName>
    <Type>Box - Front</Type>
    <Region>United Kingdom</Region>
  </GameImage>
  <GameImage>
    <DatabaseID>220</DatabaseID>
    <FileName>tetris-front-nl.jpg</FileName>
    <Type>Box - Front</Type>
    <Region>The Netherlands</Region>
  </GameImage>
  <GameImage>
    <DatabaseID>140</DatabaseID>
    <FileName>smb-title.png</FileName>
    <Type>Screenshot - Game Title</Type>
  </GameImage>
  <GameImage>
    <DatabaseID>300</DatabaseID>
    <FileName>alttp-front.jpg</FileName>
    <Type>Box - Front</Type>
    <Region>North America</Region>
  </GameImage>
</LaunchBox>
"#;

fn metadata_archive(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in entries {
        archive
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(content.as_bytes()).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

struct FixtureTransport {
    archive: Vec<u8>,
    requests: Mutex<Vec<String>>,
}

impl FixtureTransport {
    fn new() -> Self {
        Self::serving(metadata_archive(&[
            ("Platforms.xml", "<LaunchBox />"),
            ("Metadata.xml", METADATA_XML),
        ]))
    }

    fn serving(archive: Vec<u8>) -> Self {
        Self {
            archive,
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl HttpTransport for &FixtureTransport {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        self.requests.lock().unwrap().push(url.to_owned());
        let body = if url == LAUNCHBOX_METADATA_URL {
            self.archive.clone()
        } else {
            b"launchbox image fixture".to_vec()
        };
        Ok(Box::new(Cursor::new(body)))
    }
}

fn request(change: impl FnOnce(&mut AcquisitionRequestDraft)) -> AcquisitionRequest {
    let mut draft = AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec![LAUNCHBOX_GAMES_DB_SOURCE_ID.to_owned()]),
        platforms: vec![NES.to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };
    change(&mut draft);
    AcquisitionRequest::try_from_draft(draft).unwrap()
}

fn locators(candidates: &[AssetCandidate]) -> Vec<&str> {
    candidates
        .iter()
        .map(|candidate| candidate.source_url.as_str())
        .collect()
}

#[test]
fn declares_every_stored_asset_type_but_icons_and_manuals() {
    let transport = FixtureTransport::new();
    let capabilities = LaunchBoxGamesDbConnector::with_transport(&transport).capabilities();

    // Its dataset records neither icons nor manuals.
    let expected: Vec<AssetType> = AssetType::ALL
        .into_iter()
        .filter(|asset_type| !matches!(asset_type, AssetType::Icon | AssetType::Manual))
        .collect();
    assert_eq!(capabilities.asset_types, expected);
    assert!(capabilities.direct_media_download);
}

#[test]
fn discovers_the_images_of_a_requested_game_from_the_dataset() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let candidates = connector.discover(&request(|_| {})).unwrap();

    assert_eq!(
        candidates[0],
        AssetCandidate {
            provider_candidate_id: Some("140/2d0a6c62-smb-front-us.jpg".to_owned()),
            game_title: "Super Mario Bros.".to_owned(),
            platform: NES.to_owned(),
            region: "USA".to_owned(),
            edition_name: "Unspecified".to_owned(),
            asset_type: AssetType::BoxFront,
            source_id: SourceId::from(LAUNCHBOX_GAMES_DB_SOURCE_ID),
            source_asset_label: Some("Box - Front".to_owned()),
            source_url: "https://images.launchbox-app.com/2d0a6c62-smb-front-us.jpg".to_owned(),
            original_filename: "2d0a6c62-smb-front-us.jpg".to_owned(),
        }
    );
    // Records without a file name, or with one that leaves the image host, are skipped.
    assert_eq!(
        locators(&candidates),
        [
            "https://images.launchbox-app.com/2d0a6c62-smb-front-us.jpg",
            "https://images.launchbox-app.com/8a1f-smb-front-jp.jpg",
        ]
    );
    assert_eq!(candidates[1].region, "Japan");
    assert_eq!(
        *transport.requests.lock().unwrap(),
        [LAUNCHBOX_METADATA_URL]
    );
}

#[test]
fn honours_region_filters_with_the_regions_the_dataset_records() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let candidates = connector
        .discover(&request(|draft| draft.regions = vec!["Japan".to_owned()]))
        .unwrap();

    assert_eq!(
        locators(&candidates),
        ["https://images.launchbox-app.com/8a1f-smb-front-jp.jpg"]
    );
}

#[test]
fn enumerates_every_game_of_a_platform_when_all_games_are_requested() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let candidates = connector
        .discover(&request(|draft| {
            draft.games = GameSelection::All;
            draft.asset_types = vec![AssetTypeSelector::BoxFront, AssetTypeSelector::Screenshot];
        }))
        .unwrap();

    assert_eq!(
        locators(&candidates),
        [
            "https://images.launchbox-app.com/2d0a6c62-smb-front-us.jpg",
            "https://images.launchbox-app.com/8a1f-smb-front-jp.jpg",
            "https://images.launchbox-app.com/77aa-smb-snap.png",
            "https://images.launchbox-app.com/zelda-front.jpg",
        ]
    );
    assert_eq!(candidates[2].asset_type, AssetType::Screenshot);
    assert_eq!(candidates[2].region, "Unknown");
    // Every game is named as No-Intro names it, so it matches a Library imported from No-Intro.
    assert_eq!(candidates[3].game_title, "Legend of Zelda, The");
}

#[test]
fn checks_plans_against_the_platforms_it_covers_without_downloading() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let covered = connector
        .unsupported_request_reason(&request(|draft| {
            draft.games = GameSelection::All;
            draft.regions = vec!["Europe".to_owned()];
        }))
        .unwrap();
    let unknown_platform = connector
        .unsupported_request_reason(&request(|draft| {
            draft.platforms = vec!["Philips - Videopac+".to_owned()];
        }))
        .unwrap();
    let languages = connector
        .unsupported_request_reason(&request(|draft| draft.languages = vec!["fr".to_owned()]))
        .unwrap();

    assert_eq!(covered, None);
    assert!(
        unknown_platform
            .as_deref()
            .is_some_and(|reason| reason.contains("Philips - Videopac+")),
        "{unknown_platform:?}"
    );
    assert!(languages.is_some());
    assert!(transport.requests.lock().unwrap().is_empty());
}

#[test]
fn downloads_a_candidate_from_its_locator() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);
    let candidate = connector.discover(&request(|_| {})).unwrap().remove(0);

    let mut bytes = Vec::new();
    connector
        .download(&candidate)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();

    assert_eq!(bytes, b"launchbox image fixture");
    assert_eq!(
        transport.requests.lock().unwrap().last().unwrap(),
        "https://images.launchbox-app.com/2d0a6c62-smb-front-us.jpg"
    );
}

#[test]
fn a_dataset_without_metadata_is_invalid_source_data() {
    let transport = FixtureTransport::serving(metadata_archive(&[("Platforms.xml", "<x />")]));
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let error = connector.discover(&request(|_| {})).unwrap_err();

    assert!(error.is_invalid_source_data(), "{}", error.message());
}

#[test]
fn matches_requested_releases_whatever_their_article_placement() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let candidates = connector
        .discover(&request(|draft| {
            draft.games = GameSelection::Explicit(vec!["Legend of Zelda, The (Europe)".to_owned()]);
        }))
        .unwrap();

    assert_eq!(
        locators(&candidates),
        ["https://images.launchbox-app.com/zelda-front.jpg"]
    );
    // Candidates keep the title the request names, as the Library imported it.
    assert_eq!(candidates[0].game_title, "Legend of Zelda, The");
}

#[test]
fn matches_subtitles_whichever_separator_and_article_placement() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let candidates = connector
        .discover(&request(|draft| {
            draft.platforms = vec![SNES.to_owned()];
            draft.games = GameSelection::Explicit(vec![
                "Legend of Zelda, The - A Link to the Past (USA)".to_owned(),
            ]);
        }))
        .unwrap();

    assert_eq!(
        locators(&candidates),
        ["https://images.launchbox-app.com/alttp-front.jpg"]
    );
    assert_eq!(
        candidates[0].game_title,
        "Legend of Zelda, The - A Link to the Past"
    );
}

#[test]
fn names_every_game_of_a_platform_as_no_intro_does() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let candidates = connector
        .discover(&request(|draft| {
            draft.platforms = vec![SNES.to_owned()];
            draft.games = GameSelection::All;
        }))
        .unwrap();

    assert_eq!(
        candidates[0].game_title,
        "Legend of Zelda, The - A Link to the Past"
    );
}

#[test]
fn acquires_title_screens_from_game_title_screenshots() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let candidates = connector
        .discover(&request(|draft| {
            draft.asset_types = vec![AssetTypeSelector::TitleScreen];
        }))
        .unwrap();

    assert_eq!(
        locators(&candidates),
        ["https://images.launchbox-app.com/smb-title.png"]
    );
    assert_eq!(candidates[0].asset_type, AssetType::TitleScreen);
    assert_eq!(
        candidates[0].source_asset_label.as_deref(),
        Some("Screenshot - Game Title")
    );
}

#[test]
fn names_regions_as_no_intro_does() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let candidates = connector
        .discover(&request(|draft| {
            draft.platforms = vec![GAME_BOY.to_owned()];
            draft.games = GameSelection::Explicit(vec!["Tetris (World) (Rev 1)".to_owned()]);
        }))
        .unwrap();

    let regions: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.region.as_str())
        .collect();
    assert_eq!(regions, ["Unknown", "USA", "UK", "Netherlands"]);
}

#[test]
fn refuses_to_discover_with_language_filters_before_downloading() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let error = connector
        .discover(&request(|draft| draft.languages = vec!["fr".to_owned()]))
        .unwrap_err();

    assert!(error.message().contains("language"), "{}", error.message());
    assert!(transport.requests.lock().unwrap().is_empty());
}

#[test]
fn discovers_the_games_bound_to_their_platforms() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);
    let bound = vec![
        PlatformBoundGameSelector {
            game: "Super Mario Bros. (World)".to_owned(),
            platform: NES.to_owned(),
        },
        PlatformBoundGameSelector {
            game: "Tetris (World)".to_owned(),
            platform: GAME_BOY.to_owned(),
        },
    ];

    for games in [
        GameSelection::PlatformBound(bound.clone()),
        GameSelection::QueryResult(bound.clone()),
    ] {
        let every_platform = connector
            .discover(&request(|draft| {
                draft.platforms = Vec::new();
                draft.games = games.clone();
            }))
            .unwrap();
        // Requested platforms narrow the bound games.
        let narrowed = connector
            .discover(&request(|draft| draft.games = games.clone()))
            .unwrap();

        let platforms: HashSet<&str> = every_platform
            .iter()
            .map(|candidate| candidate.platform.as_str())
            .collect();
        assert_eq!(platforms, HashSet::from([NES, GAME_BOY]));
        assert!(narrowed.iter().all(|candidate| candidate.platform == NES));
        assert!(!narrowed.is_empty());
    }
}

#[test]
fn checks_bound_games_against_the_platforms_it_covers() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let reason = connector
        .unsupported_request_reason(&request(|draft| {
            draft.platforms = Vec::new();
            draft.games = GameSelection::PlatformBound(vec![PlatformBoundGameSelector {
                game: "Pong (World)".to_owned(),
                platform: "Philips - Videopac+".to_owned(),
            }]);
        }))
        .unwrap();

    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("Philips - Videopac+")),
        "{reason:?}"
    );
}

#[test]
fn acquires_each_requested_media_type_it_maps_and_no_sibling() {
    let transport = FixtureTransport::serving(metadata_archive(&[(
        "Metadata.xml",
        r#"<LaunchBox>
  <Game>
    <Name>Super Mario Bros.</Name>
    <DatabaseID>140</DatabaseID>
    <Platform>Nintendo Entertainment System</Platform>
  </Game>
  <GameImage><DatabaseID>140</DatabaseID><FileName>back.jpg</FileName><Type>Box - Back</Type></GameImage>
  <GameImage><DatabaseID>140</DatabaseID><FileName>logo.png</FileName><Type>Clear Logo</Type></GameImage>
  <GameImage><DatabaseID>140</DatabaseID><FileName>cart.png</FileName><Type>Cart - Front</Type></GameImage>
  <GameImage><DatabaseID>140</DatabaseID><FileName>banner.png</FileName><Type>Banner</Type></GameImage>
</LaunchBox>"#,
    )]));
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let candidates = connector
        .discover(&request(|draft| {
            draft.asset_types = vec![AssetTypeSelector::BoxBack, AssetTypeSelector::Logo];
        }))
        .unwrap();

    assert_eq!(
        candidates
            .iter()
            .map(|candidate| (candidate.asset_type, candidate.original_filename.as_str()))
            .collect::<Vec<_>>(),
        [
            (AssetType::BoxBack, "back.jpg"),
            (AssetType::Logo, "logo.png")
        ]
    );
    assert!(
        connector
            .capabilities()
            .asset_types
            .contains(&AssetType::CartridgeFront)
    );
}

#[test]
fn acquires_box_spines() {
    let transport = FixtureTransport::serving(metadata_archive(&[(
        "Metadata.xml",
        r#"<LaunchBox>
  <Game>
    <Name>Super Mario Bros.</Name>
    <DatabaseID>140</DatabaseID>
    <Platform>Nintendo Entertainment System</Platform>
  </Game>
  <GameImage><DatabaseID>140</DatabaseID><FileName>spine.png</FileName><Type>Box - Spine</Type></GameImage>
</LaunchBox>"#,
    )]));
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    let candidates = connector
        .discover(&request(|draft| {
            draft.asset_types = vec![AssetTypeSelector::Spine];
        }))
        .unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].asset_type, AssetType::Spine);
    assert_eq!(
        candidates[0].source_asset_label.as_deref(),
        Some("Box - Spine")
    );
}

#[test]
fn acquires_arcade_cabinets_control_panels_and_circuit_boards_of_arcade_games() {
    let transport = FixtureTransport::serving(metadata_archive(&[(
        "Metadata.xml",
        r#"<LaunchBox>
  <Game>
    <Name>1942</Name>
    <DatabaseID>192</DatabaseID>
    <Platform>Arcade</Platform>
  </Game>
  <GameImage><DatabaseID>192</DatabaseID><FileName>cabinet.png</FileName><Type>Arcade - Cabinet</Type></GameImage>
  <GameImage><DatabaseID>192</DatabaseID><FileName>panel.png</FileName><Type>Arcade - Control Panel</Type></GameImage>
  <GameImage><DatabaseID>192</DatabaseID><FileName>board.png</FileName><Type>Arcade - Circuit Board</Type></GameImage>
</LaunchBox>"#,
    )]));
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);

    // MAME and FBNeo both name the platform LaunchBox calls Arcade.
    for platform in ["MAME", "FBNeo - Arcade Games"] {
        let candidates = connector
            .discover(&request(|draft| {
                draft.platforms = vec![platform.to_owned()];
                draft.games = GameSelection::Explicit(vec!["1942".to_owned()]);
                draft.asset_types = vec![
                    AssetTypeSelector::ArcadeCabinet,
                    AssetTypeSelector::ControlPanel,
                    AssetTypeSelector::Pcb,
                ];
            }))
            .unwrap();

        let acquired: Vec<(AssetType, Option<&str>, &str)> = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.asset_type,
                    candidate.source_asset_label.as_deref(),
                    candidate.platform.as_str(),
                )
            })
            .collect();
        assert_eq!(
            acquired,
            [
                (AssetType::ArcadeCabinet, Some("Arcade - Cabinet"), platform),
                (
                    AssetType::ControlPanel,
                    Some("Arcade - Control Panel"),
                    platform
                ),
                (AssetType::Pcb, Some("Arcade - Circuit Board"), platform),
            ],
            "{platform}"
        );
    }
}

#[test]
fn accepts_every_platform_whose_packaging_the_coverage_knows() {
    let transport = FixtureTransport::new();
    let connector = LaunchBoxGamesDbConnector::with_transport(&transport);
    // LaunchBox lists no DSi platform.
    let without_launchbox_platform = ["Nintendo - Nintendo DSi"];

    let refused: Vec<String> = packaging_platforms()
        .filter(|(platform, _)| !without_launchbox_platform.contains(platform))
        .filter_map(|(platform, _)| {
            connector
                .unsupported_request_reason(&request(|draft| {
                    draft.platforms = vec![platform.to_owned()];
                }))
                .unwrap()
        })
        .collect();

    assert!(refused.is_empty(), "{refused:#?}");
}

/// Serves one version of the dataset at a time, answering conditional requests as LaunchBox
/// does, and counts how often it sends the dataset whole.
struct VersionedTransport {
    version: Mutex<(Vec<u8>, &'static str)>,
    dataset_sent: AtomicUsize,
    known: Mutex<Vec<Validators>>,
    /// Told once when the dataset is next sent whole, which then takes a while.
    sending: Mutex<Option<mpsc::Sender<()>>>,
}

impl VersionedTransport {
    fn serving(archive: Vec<u8>, etag: &'static str) -> Self {
        Self {
            version: Mutex::new((archive, etag)),
            dataset_sent: AtomicUsize::new(0),
            known: Mutex::new(Vec::new()),
            sending: Mutex::new(None),
        }
    }

    fn publish(&self, archive: Vec<u8>, etag: &'static str) {
        *self.version.lock().unwrap() = (archive, etag);
    }
}

impl HttpTransport for &VersionedTransport {
    fn get_stream(&self, _url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        Ok(Box::new(Cursor::new(b"launchbox image fixture".to_vec())))
    }

    fn get_if_changed(&self, url: &str, known: &Validators) -> Result<Fetched, PortError> {
        assert_eq!(url, LAUNCHBOX_METADATA_URL);
        self.known.lock().unwrap().push(known.clone());
        let (archive, etag) = self.version.lock().unwrap().clone();
        if known.etag.as_deref() == Some(etag) {
            return Ok(Fetched::Unchanged);
        }
        self.dataset_sent.fetch_add(1, Ordering::SeqCst);
        let sending = self.sending.lock().unwrap().take();
        if let Some(sending) = sending {
            sending.send(()).unwrap();
            thread::sleep(Duration::from_millis(300));
        }
        Ok(Fetched::Changed {
            body: Box::new(Cursor::new(archive)),
            validators: Validators {
                etag: Some(etag.to_owned()),
                last_modified: None,
            },
        })
    }
}

fn dataset_with_image(file_name: &str) -> Vec<u8> {
    metadata_archive(&[(
        "Metadata.xml",
        &format!(
            r#"<LaunchBox>
  <Game>
    <Name>Super Mario Bros.</Name>
    <DatabaseID>140</DatabaseID>
    <Platform>Nintendo Entertainment System</Platform>
  </Game>
  <GameImage><DatabaseID>140</DatabaseID><FileName>{file_name}</FileName><Type>Box - Front</Type></GameImage>
</LaunchBox>"#
        ),
    )])
}

fn cached_connector<'a>(
    transport: &'a VersionedTransport,
    cache: &std::path::Path,
) -> LaunchBoxGamesDbConnector<&'a VersionedTransport> {
    LaunchBoxGamesDbConnector::with_transport_and_cache(transport, DatasetCache::at(cache))
}

#[test]
fn a_dataset_unchanged_since_the_last_discovery_is_read_from_the_machine_cache() {
    let cache = tempfile::tempdir().unwrap();
    let transport = VersionedTransport::serving(dataset_with_image("front-v1.png"), "\"v1\"");
    let connector = cached_connector(&transport, cache.path());

    let first = connector.discover(&request(|_| {})).unwrap();
    let again = connector.discover(&request(|_| {})).unwrap();

    assert_eq!(locators(&first), locators(&again));
    assert_eq!(locators(&first).len(), 1);
    assert_eq!(transport.dataset_sent.load(Ordering::SeqCst), 1);
    // The second discovery named the version it held.
    let known = transport.known.lock().unwrap();
    assert_eq!(known[1].etag.as_deref(), Some("\"v1\""));
}

#[test]
fn a_republished_dataset_replaces_the_cached_one() {
    let cache = tempfile::tempdir().unwrap();
    let transport = VersionedTransport::serving(dataset_with_image("front-v1.png"), "\"v1\"");
    let connector = cached_connector(&transport, cache.path());
    connector.discover(&request(|_| {})).unwrap();

    transport.publish(dataset_with_image("front-v2.png"), "\"v2\"");
    let republished = connector.discover(&request(|_| {})).unwrap();
    let again = connector.discover(&request(|_| {})).unwrap();

    assert!(locators(&republished)[0].ends_with("front-v2.png"));
    assert_eq!(locators(&again), locators(&republished));
    assert_eq!(transport.dataset_sent.load(Ordering::SeqCst), 2);
}

#[test]
fn every_vault_of_the_machine_shares_the_cached_dataset() {
    let cache = tempfile::tempdir().unwrap();
    let transport = VersionedTransport::serving(dataset_with_image("front-v1.png"), "\"v1\"");

    // Each vault, or process, builds its own connector over the one machine cache.
    cached_connector(&transport, cache.path())
        .discover(&request(|_| {}))
        .unwrap();
    let other = cached_connector(&transport, cache.path())
        .discover(&request(|_| {}))
        .unwrap();

    assert_eq!(locators(&other).len(), 1);
    assert_eq!(transport.dataset_sent.load(Ordering::SeqCst), 1);
}

#[test]
fn a_cache_that_cannot_be_written_never_fails_a_discovery() {
    let blocked = tempfile::NamedTempFile::new().unwrap();
    let transport = VersionedTransport::serving(dataset_with_image("front-v1.png"), "\"v1\"");
    // A file where the cache directory should be.
    let connector = cached_connector(&transport, &blocked.path().join("cache"));

    let candidates = connector.discover(&request(|_| {})).unwrap();

    assert_eq!(locators(&candidates).len(), 1);
}

#[test]
fn a_dataset_that_does_not_read_as_an_archive_is_never_cached() {
    let cache = tempfile::tempdir().unwrap();
    // LaunchBox answers with a page that is no archive, under the dataset's validators.
    let transport = VersionedTransport::serving(b"<html>maintenance</html>".to_vec(), "\"v1\"");
    let connector = cached_connector(&transport, cache.path());
    assert!(connector.discover(&request(|_| {})).is_err());

    transport.publish(dataset_with_image("front-v1.png"), "\"v1\"");
    let candidates = connector.discover(&request(|_| {})).unwrap();

    assert_eq!(locators(&candidates).len(), 1);
}

#[test]
fn a_cached_copy_damaged_on_disk_is_downloaded_again() {
    let cache = tempfile::tempdir().unwrap();
    let transport = VersionedTransport::serving(dataset_with_image("front-v1.png"), "\"v1\"");
    let connector = cached_connector(&transport, cache.path());
    connector.discover(&request(|_| {})).unwrap();
    // The copy is cut short on disk while its validators stay.
    std::fs::write(cache.path().join("Metadata.zip"), b"PK").unwrap();

    let candidates = connector.discover(&request(|_| {})).unwrap();
    let again = connector.discover(&request(|_| {})).unwrap();

    assert_eq!(locators(&candidates).len(), 1);
    assert_eq!(locators(&again), locators(&candidates));
    assert_eq!(transport.dataset_sent.load(Ordering::SeqCst), 2);
}

#[test]
fn discoveries_of_the_machine_refresh_its_cached_dataset_one_at_a_time() {
    let cache = tempfile::tempdir().unwrap();
    let transport = VersionedTransport::serving(dataset_with_image("front-v1.png"), "\"v1\"");
    let (sending, sent) = mpsc::channel();
    *transport.sending.lock().unwrap() = Some(sending);

    let (first, other) = thread::scope(|scope| {
        let first =
            scope.spawn(|| cached_connector(&transport, cache.path()).discover(&request(|_| {})));
        // Another vault discovers while the first one downloads the dataset whole.
        sent.recv().unwrap();
        let other = cached_connector(&transport, cache.path()).discover(&request(|_| {}));
        (first.join().unwrap(), other)
    });

    assert_eq!(locators(&first.unwrap()).len(), 1);
    assert_eq!(locators(&other.unwrap()).len(), 1);
    assert_eq!(transport.dataset_sent.load(Ordering::SeqCst), 1);
}

#[test]
fn a_cached_copy_whose_metadata_no_longer_reads_is_downloaded_again() {
    let cache = tempfile::tempdir().unwrap();
    let transport = VersionedTransport::serving(dataset_with_image("front-v1.png"), "\"v1\"");
    let connector = cached_connector(&transport, cache.path());
    connector.discover(&request(|_| {})).unwrap();
    // The copy still opens as an archive, but its metadata is gone.
    std::fs::write(
        cache.path().join("Metadata.zip"),
        metadata_archive(&[("Other.xml", "<LaunchBox/>")]),
    )
    .unwrap();

    let candidates = connector.discover(&request(|_| {})).unwrap();
    let again = connector.discover(&request(|_| {})).unwrap();

    assert_eq!(locators(&candidates).len(), 1);
    assert_eq!(locators(&again), locators(&candidates));
    assert_eq!(transport.dataset_sent.load(Ordering::SeqCst), 2);
}

#[test]
fn a_dataset_whose_metadata_does_not_read_is_not_kept() {
    let cache = tempfile::tempdir().unwrap();
    let transport =
        VersionedTransport::serving(metadata_archive(&[("Other.xml", "<LaunchBox/>")]), "\"v1\"");
    let connector = cached_connector(&transport, cache.path());
    assert!(connector.discover(&request(|_| {})).is_err());

    transport.publish(dataset_with_image("front-v1.png"), "\"v1\"");
    let candidates = connector.discover(&request(|_| {})).unwrap();

    assert_eq!(locators(&candidates).len(), 1);
    // The second discovery held no copy to name.
    assert_eq!(transport.known.lock().unwrap()[1].etag, None);
}

#[test]
fn describes_how_seldom_it_downloads_its_dataset() {
    let transport = FixtureTransport::new();

    let limits = LaunchBoxGamesDbConnector::with_transport(&transport)
        .rate_limits()
        .unwrap();

    assert!(limits.contains("once per"), "{limits}");
}
