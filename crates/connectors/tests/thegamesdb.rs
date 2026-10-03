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
  "10":{"id":10,"name":"Sony Playstation","alias":"sony-playstation"}
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
