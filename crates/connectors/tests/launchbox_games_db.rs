use std::{
    cell::RefCell,
    io::{Cursor, Read, Write},
};

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_connectors::{
    HttpTransport, LAUNCHBOX_GAMES_DB_SOURCE_ID, LAUNCHBOX_METADATA_URL, LaunchBoxGamesDbConnector,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType,
    AssetTypeSelector, GameSelection, RetentionPolicy, SourceId, SourceSelection,
};

const NES: &str = "Nintendo - Nintendo Entertainment System";
const SNES: &str = "Nintendo - Super Nintendo Entertainment System";

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
    requests: RefCell<Vec<String>>,
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
            requests: RefCell::new(Vec::new()),
        }
    }
}

impl HttpTransport for &FixtureTransport {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        self.requests.borrow_mut().push(url.to_owned());
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
fn declares_box_fronts_screenshots_and_title_screens() {
    let transport = FixtureTransport::new();
    let capabilities = LaunchBoxGamesDbConnector::with_transport(&transport).capabilities();

    assert_eq!(
        capabilities.asset_types,
        [
            AssetType::BoxFront,
            AssetType::Screenshot,
            AssetType::TitleScreen
        ]
    );
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
    assert_eq!(*transport.requests.borrow(), [LAUNCHBOX_METADATA_URL]);
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
    assert!(transport.requests.borrow().is_empty());
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
        transport.requests.borrow().last().unwrap(),
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
