use std::{
    collections::HashMap,
    io::{Cursor, Read},
    sync::{Arc, Mutex},
};

use game_media_vault_application::{ApiKey, ConnectorPort, CredentialStorePort, PortError};
use game_media_vault_connectors::{HttpTransport, THEGAMESDB_SOURCE_ID, TheGamesDbConnector};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType,
    AssetTypeSelector, GameSelection, RetentionPolicy, SourceSelection,
};

const API: &str = "https://api.thegamesdb.net";
const NES: &str = "Nintendo - Nintendo Entertainment System";

/// A request: its URL without the key, and the query parameter carrying the key with its value.
type Request = (String, Option<(String, String)>);

/// Answers TheGamesDB API requests from fixtures, by their URL without the key, and records each
/// request with the parameter and key it carried.
struct FixtureApi {
    answers: HashMap<String, String>,
    requests: Mutex<Vec<Request>>,
}

impl FixtureApi {
    fn answering(answers: &[(&str, &str)]) -> Self {
        Self {
            answers: answers
                .iter()
                .map(|(url, body)| ((*url).to_owned(), (*body).to_owned()))
                .collect(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requested(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl HttpTransport for &FixtureApi {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        self.requests.lock().unwrap().push((url.to_owned(), None));
        Ok(Box::new(Cursor::new(b"thegamesdb image fixture".to_vec())))
    }

    fn get_with_query_key(
        &self,
        url: &str,
        parameter: &str,
        api_key: &ApiKey,
    ) -> Result<Vec<u8>, PortError> {
        self.requests.lock().unwrap().push((
            url.to_owned(),
            Some((parameter.to_owned(), api_key.expose().to_owned())),
        ));
        self.answers
            .get(url)
            .map(|body| body.clone().into_bytes())
            .ok_or_else(|| PortError::new(format!("download returned HTTP 404 for {url}")))
    }
}

/// The machine's credential store, holding the TheGamesDB key or none.
struct KeyStore(Option<&'static str>);

impl CredentialStorePort for KeyStore {
    fn api_key(&self, source_id: &str) -> Result<Option<ApiKey>, PortError> {
        assert_eq!(source_id, THEGAMESDB_SOURCE_ID);
        Ok(self.0.map(|key| ApiKey::new(key).unwrap()))
    }

    fn set_api_key(&self, _source_id: &str, _key: &ApiKey) -> Result<(), PortError> {
        unreachable!("the connector never stores a key")
    }

    fn clear_api_key(&self, _source_id: &str) -> Result<(), PortError> {
        unreachable!("the connector never forgets a key")
    }
}

fn connector<'a>(
    api: &'a FixtureApi,
    key: Option<&'static str>,
) -> TheGamesDbConnector<&'a FixtureApi> {
    TheGamesDbConnector::with_transport(api, Arc::new(KeyStore(key)))
}

fn request(change: impl FnOnce(&mut AcquisitionRequestDraft)) -> AcquisitionRequest {
    let mut draft = AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec![THEGAMESDB_SOURCE_ID.to_owned()]),
        platforms: vec![NES.to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront, AssetTypeSelector::BoxBack],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };
    change(&mut draft);
    AcquisitionRequest::try_from_draft(draft).unwrap()
}

const PLATFORMS: &str = r#"{"code":200,"status":"Success","remaining_monthly_allowance":100,"extra_allowance":0,"data":{"count":3,"platforms":{
  "7":{"id":7,"name":"Nintendo Entertainment System (NES)","alias":"nintendo-entertainment-system-nes"},
  "4":{"id":4,"name":"Nintendo Game Boy","alias":"nintendo-gameboy"},
  "10":{"id":10,"name":"Sony Playstation","alias":"sony-playstation"},
  "6":{"id":6,"name":"Super Nintendo (SNES)","alias":"super-nintendo-snes"},
  "18":{"id":18,"name":"Sega Genesis","alias":"sega-genesis"},
  "36":{"id":36,"name":"Sega Mega Drive","alias":"sega-mega-drive"},
  "4052":{"id":4052,"name":"Handheld Electronic Games (LCD)","alias":"handheld-electronic-games-lcd"},
  "4955":{"id":4955,"name":"TurboGrafx CD","alias":"turbografx-cd"}
}}}"#;

const SEARCH: &str = r#"{"code":200,"status":"Success","remaining_monthly_allowance":99,"extra_allowance":0,
  "pages":{"previous":null,"current":"","next":null},
  "data":{"count":3,"games":[
    {"id":140,"game_title":"Super Mario Bros. 3","platform":7},
    {"id":2,"game_title":"Super Mario Bros.","platform":7},
    {"id":3,"game_title":"Super Mario Bros.","platform":4}
  ]},
  "include":{"boxart":{"base_url":{},"data":{}}}}"#;

fn images(page_next: &str) -> String {
    format!(
        r#"{{"code":200,"status":"Success","remaining_monthly_allowance":98,"extra_allowance":0,
  "pages":{{"previous":null,"current":"","next":{page_next}}},
  "data":{{"count":3,"base_url":{{"original":"https://cdn.thegamesdb.net/images/original/","small":"","thumb":"","cropped_center_thumb":"","medium":"","large":""}},
    "images":{{"2":[
      {{"id":11,"type":"boxart","side":"front","filename":"boxart/front/2-1.jpg","resolution":"1529x2156"}},
      {{"id":12,"type":"boxart","side":"back","filename":"boxart/back/2-1.jpg","resolution":"1529x2156"}},
      {{"id":13,"type":"fanart","filename":"fanart/2-1.jpg","resolution":"1920x1080"}}
    ]}}}}}}"#
    )
}

const PLATFORMS_URL: &str = "https://api.thegamesdb.net/v1/Platforms";
const SEARCH_URL: &str = "https://api.thegamesdb.net/v1.1/Games/ByGameName?name=Super+Mario+Bros.&filter%5Bplatform%5D=7";

fn images_url(types: &str) -> String {
    format!("{API}/v1/Games/Images?games_id=2&filter%5Btype%5D={types}")
}

fn super_mario_api() -> FixtureApi {
    FixtureApi::answering(&[
        (PLATFORMS_URL, PLATFORMS),
        (SEARCH_URL, SEARCH),
        (&images_url("boxart"), &images("null")),
    ])
}

#[test]
fn acquires_box_art_screenshots_logos_and_fanart_with_an_api_key() {
    let api = FixtureApi::answering(&[]);
    let connector = connector(&api, Some("key"));

    let capabilities = connector.capabilities();

    assert_eq!(
        capabilities.asset_types,
        [
            AssetType::BoxFront,
            AssetType::BoxBack,
            AssetType::Screenshot,
            AssetType::TitleScreen,
            AssetType::Logo,
            AssetType::WallpaperArtwork,
        ]
    );
    assert!(capabilities.direct_media_download);
    assert!(connector.needs_api_key());
}

#[test]
fn refuses_plans_until_this_machine_stores_its_api_key() {
    let api = FixtureApi::answering(&[]);

    let reason = connector(&api, None)
        .unsupported_request_reason(&request(|_| {}))
        .unwrap()
        .unwrap();

    assert!(reason.contains("source key set thegamesdb"), "{reason}");
    assert!(api.requested().is_empty());
}

#[test]
fn refuses_requests_its_media_cannot_serve() {
    let api = FixtureApi::answering(&[]);
    let connector = connector(&api, Some("key"));

    for refused in [
        request(|draft| draft.games = GameSelection::All),
        request(|draft| draft.regions = vec!["USA".to_owned()]),
        request(|draft| draft.languages = vec!["en".to_owned()]),
    ] {
        assert!(
            connector
                .unsupported_request_reason(&refused)
                .unwrap()
                .is_some()
        );
    }
    assert!(api.requested().is_empty());
}

#[test]
fn discovers_the_box_art_of_a_requested_game_named_exactly_on_its_platform() {
    let api = super_mario_api();

    let candidates = connector(&api, Some("tgdb-key"))
        .discover(&request(|_| {}))
        .unwrap();

    let front = |id: &str, asset_type, label: &str, file: &str| AssetCandidate {
        provider_candidate_id: Some(format!("image/{id}")),
        game_title: "Super Mario Bros.".to_owned(),
        platform: NES.to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type,
        source_id: game_media_vault_domain::SourceId::from(THEGAMESDB_SOURCE_ID),
        source_asset_label: Some(label.to_owned()),
        source_url: format!("https://cdn.thegamesdb.net/images/original/{file}"),
        original_filename: file.rsplit('/').next().unwrap().to_owned(),
    };
    assert_eq!(
        candidates,
        [
            front(
                "11",
                AssetType::BoxFront,
                "boxart: front",
                "boxart/front/2-1.jpg"
            ),
            front(
                "12",
                AssetType::BoxBack,
                "boxart: back",
                "boxart/back/2-1.jpg"
            ),
        ]
    );
    // Every API request carries the key as its `apikey` parameter, and only those.
    let requested = api.requested();
    assert_eq!(
        requested
            .iter()
            .map(|(url, _)| url.as_str())
            .collect::<Vec<_>>(),
        [PLATFORMS_URL, SEARCH_URL, images_url("boxart").as_str()]
    );
    assert!(
        requested
            .iter()
            .all(|(_, key)| key == &Some(("apikey".to_owned(), "tgdb-key".to_owned())))
    );
}

#[test]
fn asks_only_for_the_requested_kinds_of_media() {
    let api = FixtureApi::answering(&[
        (PLATFORMS_URL, PLATFORMS),
        (SEARCH_URL, SEARCH),
        (&images_url("clearlogo%2Cfanart"), &images("null")),
    ]);

    let candidates = connector(&api, Some("key"))
        .discover(&request(|draft| {
            draft.asset_types = vec![AssetTypeSelector::Logo, AssetTypeSelector::WallpaperArtwork];
        }))
        .unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].asset_type, AssetType::WallpaperArtwork);
    assert_eq!(candidates[0].source_asset_label.as_deref(), Some("fanart"));
}

#[test]
fn a_platform_it_does_not_list_is_never_searched() {
    let api = FixtureApi::answering(&[(PLATFORMS_URL, PLATFORMS)]);

    let candidates = connector(&api, Some("key"))
        .discover(&request(|draft| {
            draft.platforms = vec!["Atari - 2600".to_owned()];
        }))
        .unwrap();

    assert!(candidates.is_empty());
    assert_eq!(api.requested().len(), 1);
}

#[test]
fn reads_every_page_of_images() {
    let api = FixtureApi::answering(&[
        (PLATFORMS_URL, PLATFORMS),
        (SEARCH_URL, SEARCH),
        (
            &images_url("boxart"),
            &images(r#""https://api.thegamesdb.net/v1/Games/Images?apikey=SECRET&page=2""#),
        ),
        (&format!("{}&page=2", images_url("boxart")), &images("null")),
    ]);

    let candidates = connector(&api, Some("key"))
        .discover(&request(|_| {}))
        .unwrap();

    // The second page repeats the first here, and media seen already add no candidate.
    assert_eq!(candidates.len(), 2);
    assert_eq!(api.requested().len(), 4);
    assert!(
        api.requested()
            .iter()
            .all(|(url, _)| !url.contains("SECRET"))
    );
}

#[test]
fn downloads_a_candidate_from_its_location_without_the_key() {
    let api = super_mario_api();
    let connector = connector(&api, Some("key"));
    let candidate = connector.discover(&request(|_| {})).unwrap().remove(0);

    let mut bytes = Vec::new();
    connector
        .download(&candidate)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();

    assert_eq!(bytes, b"thegamesdb image fixture");
    assert_eq!(
        api.requested().last().unwrap(),
        &(candidate.source_url, None)
    );
}

#[test]
fn an_answer_that_is_not_the_api_s_is_invalid_source_data() {
    let api = FixtureApi::answering(&[(PLATFORMS_URL, "<html>maintenance</html>")]);

    let error = connector(&api, Some("key"))
        .discover(&request(|_| {}))
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{}", error.message());
}

#[test]
fn an_answer_saying_it_failed_fails_the_discovery() {
    let api = FixtureApi::answering(&[(
        PLATFORMS_URL,
        r#"{"code":403,"status":"This API Key has reached its monthly allowance"}"#,
    )]);

    let error = connector(&api, Some("key"))
        .discover(&request(|_| {}))
        .unwrap_err();

    assert!(
        error.message().contains("monthly allowance"),
        "{}",
        error.message()
    );
}

#[test]
fn a_platform_qualified_as_digital_is_not_taken_for_the_physical_one() {
    let api = FixtureApi::answering(&[(PLATFORMS_URL, PLATFORMS)]);

    let candidates = connector(&api, Some("key"))
        .discover(&request(|draft| {
            draft.platforms = vec![format!("{NES} (Digital)")];
        }))
        .unwrap();

    assert!(candidates.is_empty());
    assert_eq!(api.requested().len(), 1);
}

#[test]
fn platforms_it_names_otherwise_are_known_by_their_catalog_names() {
    for (platform, ids) in [
        ("Nintendo - Super Nintendo Entertainment System", "6"),
        ("Sega - Mega Drive - Genesis", "18%2C36"),
        ("NEC - PC Engine CD - TurboGrafx-CD", "4955"),
    ] {
        let api = FixtureApi::answering(&[(PLATFORMS_URL, PLATFORMS)]);

        // The fixture answers no search: which platforms the search names is what counts.
        let _unanswered = connector(&api, Some("key")).discover(&request(|draft| {
            draft.platforms = vec![platform.to_owned()]
        }));

        let searched = &api.requested()[1].0;
        assert!(
            searched.ends_with(&format!("filter%5Bplatform%5D={ids}")),
            "{platform}: {searched}"
        );
    }
}

#[test]
fn titles_differing_only_in_punctuation_name_one_game() {
    let search = r#"{"code":200,"status":"Success","pages":{"next":null},
      "data":{"count":1,"games":[{"id":50,"game_title":"Spider Man","platform":7}]}}"#;
    let api = FixtureApi::answering(&[
        (PLATFORMS_URL, PLATFORMS),
        (
            "https://api.thegamesdb.net/v1.1/Games/ByGameName?name=Spider-Man&filter%5Bplatform%5D=7",
            search,
        ),
        (
            &format!("{API}/v1/Games/Images?games_id=50&filter%5Btype%5D=boxart"),
            &images("null").replace(r#""2":["#, r#""50":["#),
        ),
    ]);

    let candidates = connector(&api, Some("key"))
        .discover(&request(|draft| {
            draft.games = GameSelection::Explicit(vec!["Spider-Man".to_owned()]);
        }))
        .unwrap();

    assert_eq!(candidates.len(), 2);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.game_title == "Spider-Man")
    );
}

#[test]
fn follows_the_pages_of_a_game_search() {
    let first = r#"{"code":200,"status":"Success","pages":{"next":"https://api.thegamesdb.net/v1.1/Games/ByGameName?apikey=SECRET&page=2"},
      "data":{"count":1,"games":[{"id":140,"game_title":"Super Mario Bros. 3","platform":7}]}}"#;
    let second = r#"{"code":200,"status":"Success","pages":{"next":null},
      "data":{"count":1,"games":[{"id":2,"game_title":"Super Mario Bros.","platform":7}]}}"#;
    let api = FixtureApi::answering(&[
        (PLATFORMS_URL, PLATFORMS),
        (SEARCH_URL, first),
        (&format!("{SEARCH_URL}&page=2"), second),
        (&images_url("boxart"), &images("null")),
    ]);

    let candidates = connector(&api, Some("key"))
        .discover(&request(|_| {}))
        .unwrap();

    assert_eq!(candidates.len(), 2);
    assert!(
        api.requested()
            .iter()
            .all(|(url, _)| !url.contains("SECRET"))
    );
}

#[test]
fn a_platform_whose_name_holds_parentheses_is_known_by_its_whole_name() {
    let api = FixtureApi::answering(&[(PLATFORMS_URL, PLATFORMS)]);

    // The fixture answers no search: which platform the search names is what counts.
    let _unanswered = connector(&api, Some("key")).discover(&request(|draft| {
        draft.platforms = vec!["Handheld Electronic Games (LCD)".to_owned()]
    }));

    let searched = &api.requested()[1].0;
    assert!(
        searched.ends_with("filter%5Bplatform%5D=4052"),
        "{searched}"
    );
}

#[test]
fn a_game_search_answered_without_its_games_is_invalid_source_data() {
    let api = FixtureApi::answering(&[
        (PLATFORMS_URL, PLATFORMS),
        (SEARCH_URL, r#"{"code":200,"status":"Success","data":{}}"#),
    ]);

    let error = connector(&api, Some("key"))
        .discover(&request(|_| {}))
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{}", error.message());
}

#[test]
fn images_are_asked_for_in_batches_of_twenty_games() {
    let games: Vec<String> = (1..=21)
        .map(|id| format!(r#"{{"id":{id},"game_title":"Super Mario Bros.","platform":7}}"#))
        .collect();
    let search = format!(
        r#"{{"code":200,"status":"Success","pages":{{"next":null}},"data":{{"count":21,"games":[{}]}}}}"#,
        games.join(",")
    );
    let first: Vec<String> = (1..=20).map(|id| id.to_string()).collect();
    let first_batch = format!(
        "{API}/v1/Games/Images?games_id={}&filter%5Btype%5D=boxart",
        first.join("%2C")
    );
    let second_batch = format!("{API}/v1/Games/Images?games_id=21&filter%5Btype%5D=boxart");
    let api = FixtureApi::answering(&[
        (PLATFORMS_URL, PLATFORMS),
        (SEARCH_URL, &search),
        (&first_batch, &images("null")),
        (&second_batch, &images("null")),
    ]);

    connector(&api, Some("key"))
        .discover(&request(|_| {}))
        .unwrap();

    let requested: Vec<String> = api.requested().into_iter().map(|(url, _)| url).collect();
    assert_eq!(&requested[2..], [first_batch, second_batch]);
}

#[test]
fn an_image_listing_answered_without_its_images_is_invalid_source_data() {
    let api = FixtureApi::answering(&[
        (PLATFORMS_URL, PLATFORMS),
        (SEARCH_URL, SEARCH),
        (
            &images_url("boxart"),
            r#"{"code":200,"status":"Success","pages":{"next":null},"data":{"count":0,"base_url":{"original":"https://cdn.thegamesdb.net/images/original/"}}}"#,
        ),
    ]);

    let error = connector(&api, Some("key"))
        .discover(&request(|_| {}))
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{}", error.message());
}

#[test]
fn describes_the_monthly_allowance_its_requests_draw_on() {
    let api = FixtureApi::answering(&[]);

    let limits = connector(&api, None).rate_limits().unwrap();

    assert!(limits.contains("monthly allowance"), "{limits}");
}

#[test]
fn a_request_for_more_games_than_its_allowance_can_look_up_is_refused() {
    let api = FixtureApi::answering(&[]);
    let games: Vec<game_media_vault_domain::PlatformBoundGameSelector> = (0..101)
        .map(|index| game_media_vault_domain::PlatformBoundGameSelector {
            game: format!("Game {index} (USA)"),
            platform: NES.to_owned(),
        })
        .collect();

    let reason = connector(&api, Some("key"))
        .unsupported_request_reason(&request(|draft| {
            draft.games = GameSelection::PlatformBound(games);
        }))
        .unwrap()
        .unwrap();

    assert!(reason.contains("101 games"), "{reason}");
    assert!(api.requested().is_empty());
}

#[test]
fn looks_games_up_as_many_at_a_time_as_one_request_may_so_whole_platforms_are_discovered_in_batches()
 {
    let api = FixtureApi::answering(&[]);

    assert_eq!(
        connector(&api, Some("key")).discovery_batch_size(),
        Some(100)
    );
}
