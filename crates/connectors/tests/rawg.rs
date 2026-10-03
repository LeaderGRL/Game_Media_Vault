use std::{
    collections::HashMap,
    io::{Cursor, Read},
    sync::{Arc, Mutex},
};

use game_media_vault_application::{ApiKey, ConnectorPort, CredentialStorePort, PortError};
use game_media_vault_connectors::{HttpTransport, RAWG_SOURCE_ID, RawgConnector};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType,
    AssetTypeSelector, GameSelection, RetentionPolicy, SourceSelection,
};

const PS4: &str = "Sony - PlayStation 4";
const SEARCH_URL: &str =
    "https://api.rawg.io/api/games?search=Celeste&search_exact=true&page_size=40";

/// A request: its URL, and the query parameter carrying the key with its value, if any.
type Request = (String, Option<(String, String)>);

/// Answers RAWG API requests from fixtures, by their URL without the key, and records each
/// request with the key it carried.
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
        Ok(Box::new(Cursor::new(b"rawg image fixture".to_vec())))
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

/// This machine's credential store, holding the RAWG key or none.
struct KeyStore(Option<&'static str>);

impl CredentialStorePort for KeyStore {
    fn api_key(&self, name: &str) -> Result<Option<ApiKey>, PortError> {
        assert_eq!(name, RAWG_SOURCE_ID);
        Ok(self.0.map(|key| ApiKey::new(key).unwrap()))
    }

    fn set_api_key(&self, _name: &str, _key: &ApiKey) -> Result<(), PortError> {
        unreachable!("the connector never stores a key")
    }

    fn clear_api_key(&self, _name: &str) -> Result<(), PortError> {
        unreachable!("the connector never forgets a key")
    }
}

fn connector<'a>(api: &'a FixtureApi, key: Option<&'static str>) -> RawgConnector<&'a FixtureApi> {
    RawgConnector::with_transport(api, Arc::new(KeyStore(key)))
}

fn request(change: impl FnOnce(&mut AcquisitionRequestDraft)) -> AcquisitionRequest {
    let mut draft = AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec![RAWG_SOURCE_ID.to_owned()]),
        platforms: vec![PS4.to_owned()],
        games: GameSelection::Explicit(vec!["Celeste".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![
            AssetTypeSelector::Screenshot,
            AssetTypeSelector::WallpaperArtwork,
        ],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };
    change(&mut draft);
    AcquisitionRequest::try_from_draft(draft).unwrap()
}

const SEARCH: &str = r#"{"count":3,"next":null,"previous":null,"results":[
  {"id":7,"slug":"celeste","name":"Celeste",
   "background_image":"https://media.rawg.io/media/games/594/celeste.jpg",
   "platforms":[{"platform":{"id":4,"name":"PC","slug":"pc"}},{"platform":{"id":18,"name":"PlayStation 4","slug":"playstation4"}},{"platform":{"id":167,"name":"Genesis","slug":"genesis"}}],
   "short_screenshots":[
     {"id":-1,"image":"https://media.rawg.io/media/games/594/celeste.jpg"},
     {"id":101,"image":"https://media.rawg.io/media/screenshots/a1/shot-1.jpg"},
     {"id":102,"image":"https://elsewhere.example.com/media/screenshots/shot-2.jpg"}
   ]},
  {"id":8,"slug":"celeste-classic","name":"Celeste Classic",
   "background_image":"https://media.rawg.io/media/games/1/classic.jpg",
   "platforms":[{"platform":{"id":18,"name":"PlayStation 4","slug":"playstation4"}}],
   "short_screenshots":[]},
  {"id":9,"slug":"celeste-2","name":"Celeste",
   "background_image":"https://media.rawg.io/media/games/2/other.jpg",
   "platforms":[{"platform":{"id":4,"name":"PC","slug":"pc"}}],
   "short_screenshots":[]}
]}"#;

fn candidate(
    asset_type: AssetType,
    id: &str,
    label: &str,
    url: &str,
    platform: &str,
) -> AssetCandidate {
    AssetCandidate {
        provider_candidate_id: Some(id.to_owned()),
        game_title: "Celeste".to_owned(),
        platform: platform.to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type,
        source_id: RAWG_SOURCE_ID.into(),
        source_asset_label: Some(label.to_owned()),
        source_url: url.to_owned(),
        original_filename: url.rsplit('/').next().unwrap().to_owned(),
    }
}

#[test]
fn rawg_needs_an_api_key_and_provides_screenshots_and_backgrounds() {
    let api = FixtureApi::answering(&[]);
    let connector = connector(&api, None);

    assert!(connector.needs_api_key());
    assert_eq!(
        connector.capabilities().asset_types,
        [AssetType::Screenshot, AssetType::WallpaperArtwork]
    );
    assert!(connector.rate_limits().is_some());
}

#[test]
fn a_request_without_a_key_is_refused_before_reaching_rawg() {
    let api = FixtureApi::answering(&[]);

    let reason = connector(&api, None)
        .unsupported_request_reason(&request(|_| {}))
        .unwrap()
        .unwrap();

    assert!(reason.contains("source key set rawg"), "{reason}");
    assert!(api.requested().is_empty());
}

#[test]
fn requests_rawg_cannot_serve_are_refused_before_reaching_it() {
    let api = FixtureApi::answering(&[]);
    let connector = connector(&api, Some("zq-key"));
    let reason = |change: fn(&mut AcquisitionRequestDraft)| {
        connector
            .unsupported_request_reason(&request(change))
            .unwrap()
            .unwrap()
    };

    assert!(reason(|draft| draft.games = GameSelection::All).contains("explicit game selection"));
    assert!(reason(|draft| draft.regions = vec!["USA".to_owned()]).contains("region"));
    assert!(reason(|draft| draft.languages = vec!["en".to_owned()]).contains("language"));
    assert!(api.requested().is_empty());
}

#[test]
fn discovery_keeps_the_games_named_exactly_so_on_the_requested_platform() {
    let api = FixtureApi::answering(&[(SEARCH_URL, SEARCH)]);

    let candidates = connector(&api, Some("zq-key"))
        .discover(&request(|_| {}))
        .unwrap();

    assert_eq!(
        candidates,
        [
            candidate(
                AssetType::WallpaperArtwork,
                "sonyplaystation4/games/7/background",
                "background",
                "https://media.rawg.io/media/games/594/celeste.jpg",
                PS4,
            ),
            // Images RAWG's media server does not serve are left out.
            candidate(
                AssetType::Screenshot,
                "sonyplaystation4/screenshots/101",
                "screenshot",
                "https://media.rawg.io/media/screenshots/a1/shot-1.jpg",
                PS4,
            ),
        ]
    );
    assert_eq!(
        api.requested(),
        [(
            SEARCH_URL.to_owned(),
            Some(("key".to_owned(), "zq-key".to_owned()))
        )]
    );
}

#[test]
fn a_platform_rawg_words_otherwise_is_known_by_alias_and_searched_once() {
    let api = FixtureApi::answering(&[(SEARCH_URL, SEARCH)]);

    let candidates = connector(&api, Some("zq-key"))
        .discover(&request(|draft| {
            draft.platforms = vec!["Sega - Mega Drive - Genesis".to_owned(), PS4.to_owned()];
            draft.asset_types = vec![AssetTypeSelector::WallpaperArtwork];
        }))
        .unwrap();

    let platforms: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.platform.as_str())
        .collect();
    assert_eq!(platforms, ["Sega - Mega Drive - Genesis", PS4]);
    // Each platform gets a candidate of its own, which candidate identity tells apart by its id.
    assert_ne!(
        candidates[0].provider_candidate_id,
        candidates[1].provider_candidate_id
    );
    assert_eq!(api.requested().len(), 1);
}

#[test]
fn a_search_answered_without_its_results_is_invalid_source_data() {
    let api = FixtureApi::answering(&[(SEARCH_URL, r#"{"count":0}"#)]);

    let error = connector(&api, Some("zq-key"))
        .discover(&request(|_| {}))
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{}", error.message());
}

#[test]
fn media_are_downloaded_from_rawg_without_the_key() {
    let api = FixtureApi::answering(&[]);
    let url = "https://media.rawg.io/media/screenshots/a1/shot-1.jpg";

    let mut media = Vec::new();
    connector(&api, Some("zq-key"))
        .download(&candidate(
            AssetType::Screenshot,
            "sonyplaystation4/screenshots/101",
            "screenshot",
            url,
            PS4,
        ))
        .unwrap()
        .read_to_end(&mut media)
        .unwrap();

    assert_eq!(media, b"rawg image fixture");
    assert_eq!(api.requested(), [(url.to_owned(), None)]);
}

#[test]
fn looks_games_up_a_few_at_a_time_so_whole_platforms_are_discovered_in_batches() {
    let api = FixtureApi::answering(&[]);

    assert_eq!(
        connector(&api, Some("key")).discovery_batch_size(),
        Some(25)
    );
}
