use std::{
    collections::HashMap,
    io::{Cursor, Read},
    sync::{Arc, Mutex},
};

use game_media_vault_application::{ApiKey, ConnectorPort, CredentialStorePort, PortError};
use game_media_vault_connectors::{HttpTransport, STEAMGRIDDB_SOURCE_ID, SteamGridDbConnector};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType,
    AssetTypeSelector, GameSelection, RetentionPolicy, SourceSelection,
};

const API: &str = "https://www.steamgriddb.com/api/v2";
const NES: &str = "Nintendo - Nintendo Entertainment System";

/// Answers SteamGridDB API requests from fixtures and records each request with the key it
/// carried.
struct FixtureApi {
    answers: HashMap<String, String>,
    requests: Mutex<Vec<(String, Option<String>)>>,
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

    fn requested(&self) -> Vec<(String, Option<String>)> {
        self.requests.lock().unwrap().clone()
    }
}

impl HttpTransport for &FixtureApi {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        self.requests.lock().unwrap().push((url.to_owned(), None));
        Ok(Box::new(Cursor::new(b"steamgriddb image fixture".to_vec())))
    }

    fn get_authorized(&self, url: &str, api_key: &ApiKey) -> Result<Vec<u8>, PortError> {
        self.requests
            .lock()
            .unwrap()
            .push((url.to_owned(), Some(api_key.expose().to_owned())));
        self.answers
            .get(url)
            .map(|body| body.clone().into_bytes())
            .ok_or_else(|| PortError::new(format!("download returned HTTP 404 for {url}")))
    }
}

/// The machine's credential store, holding the SteamGridDB key or none.
struct KeyStore(Option<&'static str>);

impl CredentialStorePort for KeyStore {
    fn api_key(&self, source_id: &str) -> Result<Option<ApiKey>, PortError> {
        assert_eq!(source_id, STEAMGRIDDB_SOURCE_ID);
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
) -> SteamGridDbConnector<&'a FixtureApi> {
    SteamGridDbConnector::with_transport(api, Arc::new(KeyStore(key)))
}

fn request(change: impl FnOnce(&mut AcquisitionRequestDraft)) -> AcquisitionRequest {
    let mut draft = AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec![STEAMGRIDDB_SOURCE_ID.to_owned()]),
        platforms: vec![NES.to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![
            AssetTypeSelector::Logo,
            AssetTypeSelector::Icon,
            AssetTypeSelector::WallpaperArtwork,
        ],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };
    change(&mut draft);
    AcquisitionRequest::try_from_draft(draft).unwrap()
}

const SEARCH: &str = r#"{"success":true,"data":[
  {"id":999,"name":"Super Mario Bros. 3","types":[],"verified":true},
  {"id":2254,"name":"Super Mario Bros.","types":["steam"],"verified":true}
]}"#;

fn media(id: u64, style: &str, url: &str) -> String {
    format!(
        r#"{{"success":true,"page":0,"total":1,"limit":50,"data":[{{"id":{id},"score":5,"style":"{style}","width":800,"height":300,"nsfw":false,"humor":false,"mime":"image/png","language":"en","url":"{url}","thumb":"{url}","lock":false,"epilepsy":false,"upvotes":5,"downvotes":0,"author":{{"name":"someone","steam64":"1","avatar":""}}}}]}}"#
    )
}

fn super_mario_api() -> FixtureApi {
    let logos = media(101, "official", "https://cdn2.steamgriddb.com/logo/abc.png");
    let icons = media(202, "official", "https://cdn2.steamgriddb.com/icon/def.png");
    let heroes = media(
        303,
        "alternate",
        "https://cdn2.steamgriddb.com/hero/ghi.png",
    );
    FixtureApi::answering(&[
        (
            &format!("{API}/search/autocomplete/Super%20Mario%20Bros."),
            SEARCH,
        ),
        (&format!("{API}/logos/game/2254"), &logos),
        (&format!("{API}/icons/game/2254"), &icons),
        (&format!("{API}/heroes/game/2254"), &heroes),
    ])
}

#[test]
fn acquires_logos_icons_and_heroes_with_an_api_key() {
    let api = FixtureApi::answering(&[]);
    let connector = connector(&api, Some("key"));

    let capabilities = connector.capabilities();

    assert_eq!(
        capabilities.asset_types,
        [
            AssetType::Logo,
            AssetType::Icon,
            AssetType::WallpaperArtwork
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

    assert!(reason.contains("source key set steamgriddb"), "{reason}");
    assert!(api.requested().is_empty());
}

#[test]
fn refuses_requests_its_media_cannot_serve() {
    let api = FixtureApi::answering(&[]);
    let connector = connector(&api, Some("key"));
    let reason = |change: fn(&mut AcquisitionRequestDraft)| {
        connector
            .unsupported_request_reason(&request(change))
            .unwrap()
            .unwrap_or_default()
    };

    assert!(reason(|draft| draft.games = GameSelection::All).contains("explicit game"));
    assert!(reason(|draft| draft.regions = vec!["USA".to_owned()]).contains("region"));
    assert!(reason(|draft| draft.languages = vec!["en".to_owned()]).contains("language"));
    assert!(api.requested().is_empty());
}

#[test]
fn discovers_the_media_of_a_requested_game_found_by_its_exact_name() {
    let api = super_mario_api();

    let candidates = connector(&api, Some("user-key"))
        .discover(&request(|_| {}))
        .unwrap();

    let logo = candidates
        .iter()
        .find(|candidate| candidate.asset_type == AssetType::Logo)
        .unwrap();
    assert_eq!(
        logo,
        &AssetCandidate {
            provider_candidate_id: Some(format!("{NES}/logos/101")),
            game_title: "Super Mario Bros.".to_owned(),
            platform: NES.to_owned(),
            region: "Unknown".to_owned(),
            edition_name: "Unspecified".to_owned(),
            asset_type: AssetType::Logo,
            source_id: STEAMGRIDDB_SOURCE_ID.into(),
            source_asset_label: Some("logos: official".to_owned()),
            source_url: "https://cdn2.steamgriddb.com/logo/abc.png".to_owned(),
            original_filename: "abc.png".to_owned(),
        }
    );
    let kinds: Vec<(AssetType, &str)> = candidates
        .iter()
        .map(|candidate| (candidate.asset_type, candidate.source_url.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            (AssetType::Logo, "https://cdn2.steamgriddb.com/logo/abc.png"),
            (AssetType::Icon, "https://cdn2.steamgriddb.com/icon/def.png"),
            (
                AssetType::WallpaperArtwork,
                "https://cdn2.steamgriddb.com/hero/ghi.png"
            ),
        ]
    );
    // Every API request carried the key, and the media are those of the game named exactly.
    assert!(
        api.requested()
            .iter()
            .all(|(_, key)| key.as_deref() == Some("user-key"))
    );
}

#[test]
fn asks_only_for_the_requested_media() {
    let api = super_mario_api();

    connector(&api, Some("key"))
        .discover(&request(|draft| {
            draft.asset_types = vec![AssetTypeSelector::Logo];
        }))
        .unwrap();

    let urls: Vec<String> = api.requested().into_iter().map(|(url, _)| url).collect();
    assert_eq!(
        urls,
        [
            format!("{API}/search/autocomplete/Super%20Mario%20Bros."),
            format!("{API}/logos/game/2254"),
        ]
    );
}

#[test]
fn a_game_the_search_does_not_name_exactly_has_no_media() {
    let api = FixtureApi::answering(&[(
        &format!("{API}/search/autocomplete/Super%20Mario%20Bros."),
        r#"{"success":true,"data":[{"id":999,"name":"Super Mario Bros. 3","types":[],"verified":true}]}"#,
    )]);

    let candidates = connector(&api, Some("key"))
        .discover(&request(|_| {}))
        .unwrap();

    assert!(candidates.is_empty());
    assert_eq!(api.requested().len(), 1);
}

#[test]
fn downloads_a_candidate_from_its_location_without_the_key() {
    let api = super_mario_api();
    let connector = connector(&api, Some("key"));
    let candidates = connector.discover(&request(|_| {})).unwrap();

    let mut body = Vec::new();
    connector
        .download(&candidates[0])
        .unwrap()
        .read_to_end(&mut body)
        .unwrap();

    assert_eq!(body, b"steamgriddb image fixture");
    let (url, key) = api.requested().last().unwrap().clone();
    assert_eq!(url, candidates[0].source_url);
    assert_eq!(key, None);
}

#[test]
fn an_answer_that_is_not_the_api_s_is_invalid_source_data() {
    let api = FixtureApi::answering(&[(
        &format!("{API}/search/autocomplete/Super%20Mario%20Bros."),
        "<html>maintenance</html>",
    )]);

    let error = connector(&api, Some("key"))
        .discover(&request(|_| {}))
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{}", error.message());
}

#[test]
fn an_answer_saying_it_failed_fails_the_discovery() {
    let api = FixtureApi::answering(&[(
        &format!("{API}/search/autocomplete/Super%20Mario%20Bros."),
        r#"{"success":false,"errors":["Rate limited"]}"#,
    )]);

    let error = connector(&api, Some("key"))
        .discover(&request(|_| {}))
        .unwrap_err();

    assert!(
        error.message().contains("Rate limited"),
        "{}",
        error.message()
    );
}

/// A credential store that cannot be read, as a machine without a running keyring service.
struct LockedStore;

impl CredentialStorePort for LockedStore {
    fn api_key(&self, _source_id: &str) -> Result<Option<ApiKey>, PortError> {
        Err(PortError::new("the credential store is locked".to_owned()))
    }

    fn set_api_key(&self, _source_id: &str, _key: &ApiKey) -> Result<(), PortError> {
        unreachable!("the connector never stores a key")
    }

    fn clear_api_key(&self, _source_id: &str) -> Result<(), PortError> {
        unreachable!("the connector never forgets a key")
    }
}

#[test]
fn a_credential_store_that_cannot_be_read_leaves_it_out_of_plans_without_failing_them() {
    let api = FixtureApi::answering(&[]);
    let connector = SteamGridDbConnector::with_transport(&api, Arc::new(LockedStore));

    let reason = connector
        .unsupported_request_reason(&request(|_| {}))
        .unwrap()
        .unwrap();

    assert!(
        reason.contains("the credential store is locked"),
        "{reason}"
    );
}

#[test]
fn each_requested_platform_gets_a_candidate_identity_of_its_own() {
    let api = super_mario_api();
    let famicom = "Nintendo - Family Computer Disk System";

    let candidates = connector(&api, Some("key"))
        .discover(&request(|draft| {
            draft.platforms = vec![NES.to_owned(), famicom.to_owned()];
            draft.asset_types = vec![AssetTypeSelector::Logo];
        }))
        .unwrap();

    // One image applies to both platforms; candidate identity tells them apart by their id.
    let ids: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.provider_candidate_id.as_deref().unwrap())
        .collect();
    assert_eq!(
        ids,
        [format!("{NES}/logos/101"), format!("{famicom}/logos/101"),]
    );
}
