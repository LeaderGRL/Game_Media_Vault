use std::{
    collections::HashMap,
    io::{Cursor, Read},
    sync::Mutex,
};

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_connectors::{
    HttpTransport, INTERNET_ARCHIVE_MANUALS_SOURCE_ID, InternetArchiveManualsConnector,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType,
    AssetTypeSelector, GameSelection, PlatformBoundGameSelector, RetentionPolicy, SourceId,
    SourceSelection,
};

const SATURN: &str = "Sega - Saturn";

/// The Internet Archive served from fixtures, recording every request.
struct Archive {
    answers: HashMap<String, Vec<u8>>,
    requests: Mutex<Vec<String>>,
}

impl Archive {
    fn with(answers: &[(&str, &[u8])]) -> Self {
        Self {
            answers: answers
                .iter()
                .map(|(url, body)| ((*url).to_owned(), body.to_vec()))
                .collect(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requested(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl HttpTransport for &Archive {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        self.requests.lock().unwrap().push(url.to_owned());
        match self.answers.get(url) {
            Some(body) => Ok(Box::new(Cursor::new(body.clone()))),
            None => Err(PortError::unavailable(format!(
                "download returned HTTP 404 Not Found for {url}"
            ))),
        }
    }
}

fn request(change: impl FnOnce(&mut AcquisitionRequestDraft)) -> AcquisitionRequest {
    let mut draft = AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec![INTERNET_ARCHIVE_MANUALS_SOURCE_ID.to_owned()]),
        platforms: Vec::new(),
        games: GameSelection::PlatformBound(vec![game("3D Baseball (USA)", SATURN)]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::Manual],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };
    change(&mut draft);
    AcquisitionRequest::try_from_draft(draft).unwrap()
}

fn game(game: &str, platform: &str) -> PlatformBoundGameSelector {
    PlatformBoundGameSelector {
        game: game.to_owned(),
        platform: platform.to_owned(),
    }
}

#[test]
fn acquires_manuals() {
    let archive = Archive::with(&[]);
    let connector = InternetArchiveManualsConnector::with_transport(&archive);

    assert_eq!(connector.source_id(), INTERNET_ARCHIVE_MANUALS_SOURCE_ID);
    let capabilities = connector.capabilities();
    assert_eq!(capabilities.asset_types, [AssetType::Manual]);
    assert!(capabilities.direct_media_download);
    assert!(archive.requested().is_empty());
}

#[test]
fn refuses_requests_it_cannot_serve_without_reaching_the_archive() {
    let archive = Archive::with(&[]);
    let connector = InternetArchiveManualsConnector::with_transport(&archive);

    for refused in [
        // The collection's structured titles name no PlayStation manual.
        request(|draft| {
            draft.games = GameSelection::PlatformBound(vec![
                game("3D Baseball (USA)", SATURN),
                game("Ridge Racer (USA)", "Sony - PlayStation"),
            ]);
        }),
        request(|draft| draft.languages = vec!["en".to_owned()]),
    ] {
        assert!(
            connector
                .unsupported_request_reason(&refused)
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(
        connector
            .unsupported_request_reason(&request(|_| {}))
            .unwrap(),
        None
    );
    assert!(archive.requested().is_empty());
}

const LIST: &str = "https://archive.org/services/search/v1/scrape?fields=identifier%2Ctitle&q=collection%3Aconsolemanuals&count=10000";

/// A page of the collection's list holding `items`, each an identifier and its title, and the
/// cursor of the next page unless it is the last.
fn page(items: &[(&str, &str)], cursor: Option<&str>) -> Vec<u8> {
    let items: Vec<serde_json::Value> = items
        .iter()
        .map(|(identifier, title)| serde_json::json!({ "identifier": identifier, "title": title }))
        .collect();
    let mut page = serde_json::json!({ "items": items, "count": items.len(), "total": 19076 });
    if let Some(cursor) = cursor {
        page["cursor"] = serde_json::Value::from(cursor);
    }
    serde_json::to_vec(&page).unwrap()
}

/// The manual candidate of the collection's item `identifier`, listed as `listed`, for the game
/// `title` of `platform`, whose key in candidate ids is `platform_key`.
fn manual(
    identifier: &str,
    listed: &str,
    title: &str,
    (platform, platform_key): (&str, &str),
    region: &str,
) -> AssetCandidate {
    AssetCandidate {
        provider_candidate_id: Some(format!("{identifier}/{platform_key}")),
        game_title: title.to_owned(),
        platform: platform.to_owned(),
        region: region.to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type: AssetType::Manual,
        source_id: SourceId::from(INTERNET_ARCHIVE_MANUALS_SOURCE_ID),
        source_asset_label: Some(listed.to_owned()),
        source_url: format!("https://archive.org/details/{identifier}"),
        original_filename: format!("{identifier}.pdf"),
    }
}

const GAME_BOY: (&str, &str) = ("Nintendo - Game Boy", "nintendogameboy");
const SATURN_KEY: (&str, &str) = (SATURN, "segasaturn");

const BASEBALL: (&str, &str) = (
    "3D_Baseball_1996_Crystal_Dynamics_US_T-15906H",
    "Sega Saturn Manual: 3D Baseball (1996)(Crystal Dynamics)(US)[T-15906H]",
);
const ALLEYWAY: (&str, &str) = (
    "alleyway-dmg-awa-gb-jp-manual-600-dpi",
    "Alleyway [DMG-AWA](Game Boy)(JP) -Manual(600DPI)",
);

#[test]
fn discovers_the_manuals_whose_structured_title_names_a_requested_game() {
    let list = page(
        &[
            BASEBALL,
            ALLEYWAY,
            // Scans of a box, not of its manual.
            (
                "alleyway-dmg-awa-gb-jp-box-300-dpi",
                "Alleyway [DMG-AWA](Game Boy)(JP) -Box(300DPI)",
            ),
            // A free-form title follows no scheme.
            (
                "super-mario-world-snes-manual",
                "Super Mario World SNES Manual",
            ),
            (
                "Virtua_Fighter_1995_Sega_US",
                "Sega Saturn Manual: Virtua Fighter (1995)(Sega)(US)",
            ),
        ],
        None,
    );
    let archive = Archive::with(&[(LIST, &list)]);

    let candidates = InternetArchiveManualsConnector::with_transport(&archive)
        .discover(&request(|draft| {
            draft.games = GameSelection::PlatformBound(vec![
                game("3D Baseball (USA)", SATURN),
                game("Alleyway (World)", GAME_BOY.0),
            ]);
        }))
        .unwrap();

    assert_eq!(
        candidates,
        [
            manual(BASEBALL.0, BASEBALL.1, "3D Baseball", SATURN_KEY, "USA"),
            manual(ALLEYWAY.0, ALLEYWAY.1, "Alleyway", GAME_BOY, "Japan"),
        ]
    );
    assert_eq!(archive.requested(), [LIST]);
}

#[test]
fn compares_titles_regardless_of_subtitle_separator_article_and_punctuation() {
    let list = page(
        &[
            (
                "ninja-jajamaru-ginga-daisakusen-jf-32-fc-jp-manual-600-dpi",
                "Ninja Jajamaru - Ginga Daisakusen [JF-32](Famicom)(JP) -Manual(600DPI)",
            ),
            (
                "Activision_Decathlon_The_1983_Activision",
                "Atari 5200 Manual: The Activision Decathlon (1983)(Activision)",
            ),
            (
                "Spider_Man_2002_Activision_US",
                "XBOX Manual: Spider Man (2002)(Activision)(US)",
            ),
        ],
        None,
    );
    let archive = Archive::with(&[(LIST, &list)]);

    let candidates = InternetArchiveManualsConnector::with_transport(&archive)
        .discover(&request(|draft| {
            draft.games = GameSelection::PlatformBound(vec![
                // The Famicom's releases are the Japanese ones of the NES catalog.
                game(
                    "Ninja Jajamaru: Ginga Daisakusen (Japan)",
                    "Nintendo - Nintendo Entertainment System",
                ),
                game("Activision Decathlon, The (USA)", "Atari - 5200"),
                game("Spider-Man (USA)", "Microsoft - Xbox"),
            ]);
        }))
        .unwrap();

    let found: Vec<(&str, &str, &str)> = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.game_title.as_str(),
                candidate.platform.as_str(),
                candidate.region.as_str(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            (
                "Ninja Jajamaru: Ginga Daisakusen",
                "Nintendo - Nintendo Entertainment System",
                "Japan"
            ),
            // A title naming no region records none.
            ("Activision Decathlon, The", "Atari - 5200", "Unknown"),
            ("Spider-Man", "Microsoft - Xbox", "USA"),
        ]
    );
}

#[test]
fn reads_the_region_each_scheme_gives_and_keeps_the_requested_ones() {
    let list = page(
        &[
            BASEBALL,
            (
                "3D_Baseball_1996_Crystal_Dynamics_JP",
                "Sega Saturn Manual: 3D Baseball (1996)(Crystal Dynamics)(JP)",
            ),
            (
                "3D_Baseball_1996_Crystal_Dynamics_GB",
                "Sega Saturn Manual: 3D Baseball (1996) (Crystal Dynamics) (GB)",
            ),
            (
                "3D_Baseball_1996_Crystal_Dynamics",
                "Sega Saturn Manual: 3D Baseball (1996)(Crystal Dynamics)",
            ),
        ],
        None,
    );
    let archive = Archive::with(&[(LIST, &list)]);
    let connector = InternetArchiveManualsConnector::with_transport(&archive);
    let regions = |regions: &[&str]| {
        connector
            .discover(&request(|draft| {
                draft.regions = regions.iter().map(|region| (*region).to_owned()).collect();
            }))
            .unwrap()
            .into_iter()
            .map(|candidate| candidate.region)
            .collect::<Vec<_>>()
    };

    assert_eq!(regions(&[]), ["USA", "Japan", "UK", "Unknown"]);
    // A manual naming no region is no manual of a requested one.
    assert_eq!(regions(&["Japan", "UK"]), ["Japan", "UK"]);
}

#[test]
fn reads_every_page_of_the_list_once_whatever_the_number_of_discoveries() {
    let next = format!("{LIST}&cursor=W3sic29ydCI6MX1d%3D");
    let first = page(&[BASEBALL], Some("W3sic29ydCI6MX1d="));
    let last = page(&[ALLEYWAY], None);
    let archive = Archive::with(&[(LIST, &first), (&next, &last)]);
    let connector = InternetArchiveManualsConnector::with_transport(&archive);
    let both = request(|draft| {
        draft.games = GameSelection::PlatformBound(vec![
            game("3D Baseball (USA)", SATURN),
            game("Alleyway (World)", GAME_BOY.0),
        ]);
    });

    for _ in 0..3 {
        assert_eq!(connector.discover(&both).unwrap().len(), 2);
    }
    assert_eq!(archive.requested(), [LIST.to_owned(), next]);
}

#[test]
fn a_list_page_without_its_items_is_invalid_source_data() {
    let archive = Archive::with(&[(LIST, br#"{"error":"busy"}"#)]);

    let error = InternetArchiveManualsConnector::with_transport(&archive)
        .discover(&request(|_| {}))
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{error}");
}

#[test]
fn offers_every_manual_of_a_platform_when_the_request_selects_every_game() {
    let list = page(
        &[
            BASEBALL,
            ALLEYWAY,
            (
                "4-in-1-fun-pak-dmg-f-4-j-gb-jp-manual-600-dpi",
                "4-in-1 Fun Pak [DMG-F4J](Game Boy)(JP) -Manual(600DPI)",
            ),
        ],
        None,
    );
    let archive = Archive::with(&[(LIST, &list)]);

    let candidates = InternetArchiveManualsConnector::with_transport(&archive)
        .discover(&request(|draft| {
            draft.platforms = vec![GAME_BOY.0.to_owned()];
            draft.games = GameSelection::All;
        }))
        .unwrap();

    let titles: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.game_title.as_str())
        .collect();
    assert_eq!(titles, ["Alleyway", "4-in-1 Fun Pak"]);
}

#[test]
fn a_pc_engine_manual_serves_the_hucard_and_the_cd_catalogs_apart() {
    const BARUNBA: (&str, &str) = (
        "barunba-nc-90003-pce-jp-manual-600-dpi",
        "Barunba [NC90003](PC Engine)(JP) -Manual(600DPI)",
    );
    let list = page(&[BARUNBA], None);
    let archive = Archive::with(&[(LIST, &list)]);

    let candidates = InternetArchiveManualsConnector::with_transport(&archive)
        .discover(&request(|draft| {
            draft.games = GameSelection::PlatformBound(vec![
                game("Barunba (Japan)", "NEC - PC Engine - TurboGrafx 16"),
                game("Barunba (Japan)", "NEC - PC Engine CD - TurboGrafx-CD"),
            ]);
        }))
        .unwrap();

    assert_eq!(
        candidates,
        [
            manual(
                BARUNBA.0,
                BARUNBA.1,
                "Barunba",
                ("NEC - PC Engine - TurboGrafx 16", "necpcengineturbografx16"),
                "Japan"
            ),
            manual(
                BARUNBA.0,
                BARUNBA.1,
                "Barunba",
                (
                    "NEC - PC Engine CD - TurboGrafx-CD",
                    "necpcenginecdturbografxcd"
                ),
                "Japan"
            ),
        ]
    );
}

const BLODIA: (&str, &str) = (
    "blodia-dmg-bla-gb-jp-manual-600-dpi",
    "Blodia [DMG-BLA](Game Boy)(JP) -Manual(600DPI)",
);

fn blodia() -> AssetCandidate {
    manual(BLODIA.0, BLODIA.1, "Blodia", GAME_BOY, "Japan")
}

/// The file list the Archive gives of an item holding `files`, each a name, its source and its
/// format.
fn metadata(files: &[(&str, &str, &str)]) -> Vec<u8> {
    let files: Vec<serde_json::Value> = files
        .iter()
        .map(|(name, source, format)| {
            serde_json::json!({ "name": name, "source": source, "format": format })
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({
        "metadata": { "identifier": BLODIA.0, "title": BLODIA.1 },
        "files": files,
    }))
    .unwrap()
}

const BLODIA_METADATA: &str = "https://archive.org/metadata/blodia-dmg-bla-gb-jp-manual-600-dpi";

#[test]
fn downloads_the_pdf_the_item_s_file_list_names() {
    let files = metadata(&[
        ("__ia_thumb.jpg", "original", "Item Tile"),
        (
            "Blodia [DMG-BLA](GB)(JP) -Manual(600DPI)_text.pdf",
            "derivative",
            "Additional Text PDF",
        ),
        (
            "Blodia [DMG-BLA](GB)(JP) -Cover(600DPI).png",
            "original",
            "PNG",
        ),
        (
            "Blodia [DMG-BLA](GB)(JP) -Manual(600DPI).pdf",
            "original",
            "Image Container PDF",
        ),
    ]);
    let pdf = "https://archive.org/download/blodia-dmg-bla-gb-jp-manual-600-dpi/Blodia%20[DMG-BLA](GB)(JP)%20-Manual(600DPI).pdf";
    let archive = Archive::with(&[(BLODIA_METADATA, &files), (pdf, b"%PDF-1.4 manual")]);

    let mut body = Vec::new();
    InternetArchiveManualsConnector::with_transport(&archive)
        .download(&blodia())
        .unwrap()
        .read_to_end(&mut body)
        .unwrap();

    assert_eq!(body, b"%PDF-1.4 manual");
    assert_eq!(archive.requested(), [BLODIA_METADATA, pdf]);
}

#[test]
fn a_manual_whose_item_holds_no_pdf_is_unavailable_with_the_reason() {
    let without_pdf = metadata(&[
        (
            "Blodia [DMG-BLA](GB)(JP) -Raw(600DPI).zip",
            "original",
            "ZIP",
        ),
        ("__ia_thumb.jpg", "original", "Item Tile"),
    ]);
    // The Archive answers an item it no longer holds with an empty object.
    for answer in [without_pdf.as_slice(), b"{}"] {
        let archive = Archive::with(&[(BLODIA_METADATA, answer)]);

        let error = InternetArchiveManualsConnector::with_transport(&archive)
            .download(&blodia())
            .err()
            .unwrap();

        assert!(error.is_unavailable(), "{error}");
        assert!(error.message().contains(BLODIA.0), "{error}");
        assert_eq!(archive.requested(), [BLODIA_METADATA]);
    }
}

#[test]
fn downloads_only_items_of_the_archive() {
    let archive = Archive::with(&[]);
    let mut elsewhere = blodia();
    elsewhere.source_url = "https://elsewhere.example/details/blodia".to_owned();

    assert!(
        InternetArchiveManualsConnector::with_transport(&archive)
            .download(&elsewhere)
            .is_err()
    );
    assert!(archive.requested().is_empty());
}
