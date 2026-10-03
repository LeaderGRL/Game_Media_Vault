use std::{
    collections::HashMap,
    io::{Cursor, Read},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use game_media_vault_application::{ApiKey, ConnectorPort, CredentialStorePort, PortError};
use game_media_vault_connectors::{HttpTransport, SCREENSCRAPER_SOURCE_ID, ScreenScraperConnector};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType,
    AssetTypeSelector, GameSelection, RetentionPolicy, SourceSelection,
};

const PLAYSTATION: &str = "Sony - PlayStation";
const SEARCH_URL: &str = "https://api.screenscraper.fr/api2/jeuRecherche.php?softname=game-media-vault&output=json&systemeid=57&recherche=Ridge+Racer";

/// A request: its URL without the credentials, and each credential it carried.
type Request = (String, Vec<(String, String)>);

/// Answers ScreenScraper requests from fixtures, by their URL without the credentials, and
/// records each request with the credentials it carried.
struct FixtureApi {
    answers: HashMap<String, String>,
    media: Vec<u8>,
    requests: Mutex<Vec<Request>>,
}

impl FixtureApi {
    fn answering(answers: &[(&str, &str)]) -> Self {
        Self {
            answers: answers
                .iter()
                .map(|(url, body)| ((*url).to_owned(), (*body).to_owned()))
                .collect(),
            media: b"screenscraper media fixture".to_vec(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requested(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    fn record(&self, url: &str, keys: &[(&str, &ApiKey)]) {
        self.requests.lock().unwrap().push((
            url.to_owned(),
            keys.iter()
                .map(|(parameter, key)| ((*parameter).to_owned(), key.expose().to_owned()))
                .collect(),
        ));
    }
}

impl HttpTransport for &FixtureApi {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        panic!("ScreenScraper is never asked without credentials: {url}")
    }

    fn get_with_query_keys(
        &self,
        url: &str,
        keys: &[(&str, &ApiKey)],
    ) -> Result<Vec<u8>, PortError> {
        self.record(url, keys);
        self.answers
            .get(url)
            .map(|body| body.clone().into_bytes())
            .ok_or_else(|| PortError::unavailable(format!("download returned HTTP 404 for {url}")))
    }

    fn get_stream_with_query_keys(
        &self,
        url: &str,
        keys: &[(&str, &ApiKey)],
    ) -> Result<Box<dyn Read + Send>, PortError> {
        self.record(url, keys);
        Ok(Box::new(Cursor::new(self.media.clone())))
    }
}

/// This machine's credential store, holding the ScreenScraper credentials it is given.
struct KeyStore(HashMap<String, String>);

impl KeyStore {
    fn holding(credentials: &[(&str, &str)]) -> Self {
        Self(
            credentials
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        )
    }
}

impl CredentialStorePort for KeyStore {
    fn api_key(&self, name: &str) -> Result<Option<ApiKey>, PortError> {
        assert!(name.starts_with("screenscraper/"), "{name}");
        Ok(self.0.get(name).map(|value| ApiKey::new(value).unwrap()))
    }

    fn set_api_key(&self, _name: &str, _key: &ApiKey) -> Result<(), PortError> {
        unreachable!("the connector never stores a credential")
    }

    fn clear_api_key(&self, _name: &str) -> Result<(), PortError> {
        unreachable!("the connector never forgets a credential")
    }
}

const DEVELOPER: [(&str, &str); 2] = [
    ("screenscraper/dev-id", "zq-dev"),
    ("screenscraper/dev-password", "zq-dev-password"),
];

const EVERY_CREDENTIAL: [(&str, &str); 4] = [
    ("screenscraper/dev-id", "zq-dev"),
    ("screenscraper/dev-password", "zq-dev-password"),
    ("screenscraper/user-id", "zq-user"),
    ("screenscraper/user-password", "zq-user-password"),
];

fn connector<'a>(
    api: &'a FixtureApi,
    credentials: &[(&str, &str)],
) -> ScreenScraperConnector<&'a FixtureApi> {
    ScreenScraperConnector::with_transport(api, Arc::new(KeyStore::holding(credentials)))
}

fn request(change: impl FnOnce(&mut AcquisitionRequestDraft)) -> AcquisitionRequest {
    let mut draft = AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec![SCREENSCRAPER_SOURCE_ID.to_owned()]),
        platforms: vec![PLAYSTATION.to_owned()],
        games: GameSelection::Explicit(vec!["Ridge Racer (USA)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![
            AssetTypeSelector::BoxFront,
            AssetTypeSelector::BoxBack,
            AssetTypeSelector::Disc,
        ],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };
    change(&mut draft);
    AcquisitionRequest::try_from_draft(draft).unwrap()
}

/// A media of a ScreenScraper answer, served from `host`, whose own address carries the
/// credentials of the request it answered, as ScreenScraper's do.
fn media(host: &str, kind: &str, region: Option<&str>, game: u64, format: &str) -> String {
    let named = match region {
        Some(region) => format!("{kind}({region})"),
        None => kind.to_owned(),
    };
    let region = region.map_or(String::new(), |region| format!(r#""region":"{region}","#));
    format!(
        r#"{{"type":"{kind}","parent":"jeu","url":"https://{host}/api2/mediaJeu.php?devid=zq-dev&devpassword=zq-dev-password&softname=game-media-vault&ssid=&sspassword=&systemeid=57&jeuid={game}&media={named}",{region}"crc":"0","md5":"0","sha1":"0","size":"1","format":"{format}"}}"#
    )
}

fn search() -> String {
    let ridge_racer = [
        media(
            "neoclone.screenscraper.fr",
            "box-2D",
            Some("us"),
            1234,
            "png",
        ),
        media(
            "neoclone.screenscraper.fr",
            "box-2D-back",
            Some("eu"),
            1234,
            "jpg",
        ),
        media(
            "neoclone.screenscraper.fr",
            "support-2D",
            Some("jp"),
            1234,
            "png",
        ),
        media("neoclone.screenscraper.fr", "ss", Some("wor"), 1234, "png"),
        media("neoclone.screenscraper.fr", "fanart", None, 1234, "jpg"),
        media(
            "neoclone.screenscraper.fr",
            "mixrbv1",
            Some("us"),
            1234,
            "png",
        ),
        // A media ScreenScraper does not serve itself is left out.
        media("media.example.com", "box-2D", Some("jp"), 1234, "png"),
    ]
    .join(",");
    let revolution = media(
        "neoclone.screenscraper.fr",
        "box-2D",
        Some("us"),
        999,
        "png",
    );
    format!(
        r#"{{"header":{{"APIversion":"2.0","success":"true","error":""}},"response":{{"ssuser":{{"id":"zq-user","maxthreads":"1"}},"jeux":[
  {{"id":"1234","noms":[{{"region":"ss","text":"Ridge Racer"}},{{"region":"jp","text":"リッジレーサー"}}],"systeme":{{"id":"57","text":"Playstation"}},"medias":[{ridge_racer}]}},
  {{"id":"999","noms":[{{"region":"ss","text":"Ridge Racer Revolution"}}],"systeme":{{"id":"57","text":"Playstation"}},"medias":[{revolution}]}}
]}}}}"#
    )
}

fn candidate(asset_type: AssetType, media: &str, region: &str, label: &str) -> AssetCandidate {
    let format = if media.starts_with("box-2D-back") {
        "jpg"
    } else {
        "png"
    };
    AssetCandidate {
        provider_candidate_id: Some(format!("sonyplaystation/57/1234/{media}")),
        game_title: "Ridge Racer".to_owned(),
        platform: PLAYSTATION.to_owned(),
        region: region.to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type,
        source_id: SCREENSCRAPER_SOURCE_ID.into(),
        source_asset_label: Some(label.to_owned()),
        source_url: format!("https://api.screenscraper.fr/api2/mediaJeu.php/57/1234/{media}"),
        original_filename: format!("1234-{media}.{format}"),
    }
}

#[test]
fn screenscraper_asks_for_developer_credentials_and_an_optional_account() {
    let api = FixtureApi::answering(&[]);
    let connector = connector(&api, &[]);

    let fields: Vec<(&str, bool)> = connector
        .credential_fields()
        .iter()
        .map(|field| (field.id, field.optional))
        .collect();
    assert_eq!(
        fields,
        [
            ("dev-id", false),
            ("dev-password", false),
            ("user-id", true),
            ("user-password", true),
        ]
    );
    let capabilities = connector.capabilities();
    for asset_type in [
        AssetType::BoxFront,
        AssetType::BoxBack,
        AssetType::Box3dRender,
        AssetType::CartridgeFront,
        AssetType::Disc,
        AssetType::Screenshot,
        AssetType::TitleScreen,
        AssetType::Logo,
        AssetType::WallpaperArtwork,
        AssetType::Manual,
    ] {
        assert!(
            capabilities.asset_types.contains(&asset_type),
            "{asset_type:?}"
        );
    }
}

#[test]
fn a_request_without_developer_credentials_is_refused_before_reaching_screenscraper() {
    let api = FixtureApi::answering(&[]);

    let reason = connector(&api, &[("screenscraper/user-id", "zq-user")])
        .unsupported_request_reason(&request(|_| {}))
        .unwrap()
        .unwrap();

    assert!(
        reason.contains("source key set screenscraper --field dev-id"),
        "{reason}"
    );
    assert!(api.requested().is_empty());
}

#[test]
fn requests_screenscraper_cannot_serve_are_refused_before_reaching_it() {
    let api = FixtureApi::answering(&[]);
    let connector = connector(&api, &DEVELOPER);
    let reason = |change: fn(&mut AcquisitionRequestDraft)| {
        connector
            .unsupported_request_reason(&request(change))
            .unwrap()
            .unwrap()
    };

    assert!(reason(|draft| draft.games = GameSelection::All).contains("explicit game selection"));
    assert!(
        reason(|draft| draft.platforms = vec!["Nintendo - Wonder Console".to_owned()])
            .contains("Nintendo - Wonder Console")
    );
    assert!(reason(|draft| draft.regions = vec!["Atlantis".to_owned()]).contains("Atlantis"));
    assert!(reason(|draft| draft.languages = vec!["en".to_owned()]).contains("language"));
    assert!(api.requested().is_empty());
}

#[test]
fn discovery_finds_each_requested_game_by_its_exact_name_and_lists_its_requested_media() {
    let search = search();
    let api = FixtureApi::answering(&[(SEARCH_URL, &search)]);

    let candidates = connector(&api, &EVERY_CREDENTIAL)
        .discover(&request(|_| {}))
        .unwrap();

    assert_eq!(
        candidates,
        [
            candidate(AssetType::BoxFront, "box-2D(us)", "USA", "box-2D (us)"),
            candidate(
                AssetType::BoxBack,
                "box-2D-back(eu)",
                "Europe",
                "box-2D-back (eu)"
            ),
            candidate(
                AssetType::Disc,
                "support-2D(jp)",
                "Japan",
                "support-2D (jp)"
            ),
        ]
    );
    // Locators keep none of the credentials ScreenScraper's own addresses carry.
    assert!(
        candidates
            .iter()
            .all(|candidate| !candidate.source_url.contains("zq-"))
    );
    assert_eq!(
        api.requested(),
        [(
            SEARCH_URL.to_owned(),
            vec![
                ("devid".to_owned(), "zq-dev".to_owned()),
                ("devpassword".to_owned(), "zq-dev-password".to_owned()),
                ("ssid".to_owned(), "zq-user".to_owned()),
                ("sspassword".to_owned(), "zq-user-password".to_owned()),
            ]
        )]
    );
}

#[test]
fn a_region_filter_keeps_the_media_of_the_requested_regions() {
    let search = search();
    let api = FixtureApi::answering(&[(SEARCH_URL, &search)]);

    let candidates = connector(&api, &DEVELOPER)
        .discover(&request(|draft| {
            draft.regions = vec!["USA".to_owned(), "World".to_owned()];
            draft.asset_types = vec![
                AssetTypeSelector::BoxFront,
                AssetTypeSelector::Screenshot,
                AssetTypeSelector::WallpaperArtwork,
            ];
        }))
        .unwrap();

    let found: Vec<(&str, &str)> = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.provider_candidate_id.as_deref().unwrap(),
                candidate.region.as_str(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("sonyplaystation/57/1234/box-2D(us)", "USA"),
            ("sonyplaystation/57/1234/ss(wor)", "World")
        ]
    );
}

#[test]
fn the_account_is_sent_only_with_both_its_name_and_password() {
    let search = search();
    let api = FixtureApi::answering(&[(SEARCH_URL, &search)]);
    let mut credentials = DEVELOPER.to_vec();
    credentials.push(("screenscraper/user-id", "zq-user"));

    connector(&api, &credentials)
        .discover(&request(|_| {}))
        .unwrap();

    let sent: Vec<String> = api.requested()[0]
        .1
        .iter()
        .map(|(parameter, _)| parameter.clone())
        .collect();
    assert_eq!(sent, ["devid", "devpassword"]);
}

#[test]
fn a_game_screenscraper_finds_nothing_for_is_no_failure() {
    let empty = r#"{"header":{"success":"true"},"response":{"jeux":[{}]}}"#;
    let api = FixtureApi::answering(&[(SEARCH_URL, empty)]);
    assert!(
        connector(&api, &DEVELOPER)
            .discover(&request(|_| {}))
            .unwrap()
            .is_empty()
    );

    // ScreenScraper answers HTTP 404 for a search it finds nothing for.
    let api = FixtureApi::answering(&[]);
    assert!(
        connector(&api, &DEVELOPER)
            .discover(&request(|_| {}))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_search_answered_without_its_games_is_invalid_source_data() {
    let api = FixtureApi::answering(&[(SEARCH_URL, r#"{"response":{}}"#)]);

    let error = connector(&api, &DEVELOPER)
        .discover(&request(|_| {}))
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{}", error.message());
}

#[test]
fn a_game_found_without_its_media_is_asked_for_them() {
    let found = r#"{"response":{"jeux":[{"id":"1234","noms":[{"region":"ss","text":"Ridge Racer"}],"systeme":{"id":"57"}}]}}"#;
    let details = format!(
        r#"{{"response":{{"jeu":{{"id":"1234","medias":[{}]}}}}}}"#,
        media(
            "neoclone.screenscraper.fr",
            "box-2D",
            Some("us"),
            1234,
            "png"
        )
    );
    let details_url = "https://api.screenscraper.fr/api2/jeuInfos.php?softname=game-media-vault&output=json&systemeid=57&gameid=1234";
    let api = FixtureApi::answering(&[(SEARCH_URL, found), (details_url, &details)]);

    let candidates = connector(&api, &DEVELOPER)
        .discover(&request(|_| {}))
        .unwrap();

    assert_eq!(
        candidates,
        [candidate(
            AssetType::BoxFront,
            "box-2D(us)",
            "USA",
            "box-2D (us)"
        )]
    );
}

#[test]
fn media_are_downloaded_with_the_credentials_added_only_then() {
    let api = FixtureApi::answering(&[]);
    let connector = connector(&api, &EVERY_CREDENTIAL);

    let mut media = Vec::new();
    connector
        .download(&candidate(
            AssetType::BoxFront,
            "box-2D(us)",
            "USA",
            "box-2D (us)",
        ))
        .unwrap()
        .read_to_end(&mut media)
        .unwrap();

    assert_eq!(media, b"screenscraper media fixture");
    let (url, keys) = api.requested().remove(0);
    assert_eq!(
        url,
        "https://api.screenscraper.fr/api2/mediaJeu.php?softname=game-media-vault&systemeid=57&jeuid=1234&media=box-2D%28us%29"
    );
    assert_eq!(keys.len(), 4);
}

#[test]
fn a_media_screenscraper_no_longer_has_is_unavailable() {
    let mut api = FixtureApi::answering(&[]);
    api.media = b"NOMEDIA".to_vec();

    let error = connector(&api, &DEVELOPER)
        .download(&candidate(
            AssetType::BoxFront,
            "box-2D(us)",
            "USA",
            "box-2D (us)",
        ))
        .err()
        .unwrap();

    assert!(error.is_unavailable(), "{}", error.message());
}

#[test]
fn only_locators_screenscraper_discovered_are_downloaded() {
    let api = FixtureApi::answering(&[]);
    let mut foreign = candidate(AssetType::BoxFront, "box-2D(us)", "USA", "box-2D (us)");
    foreign.source_url =
        "https://media.example.com/api2/mediaJeu.php/57/1234/box-2D(us)".to_owned();

    assert!(connector(&api, &DEVELOPER).download(&foreign).is_err());
    assert!(api.requested().is_empty());
}

#[test]
fn each_arcade_platform_gets_its_own_candidates_from_one_search() {
    // MAME and FBNeo emulate the arcade boards ScreenScraper files under one system.
    let arcade = search()
        .replace(r#""id":"57""#, r#""id":"75""#)
        .replace("systemeid=57", "systemeid=75");
    let url = "https://api.screenscraper.fr/api2/jeuRecherche.php?softname=game-media-vault&output=json&systemeid=75&recherche=Ridge+Racer";
    let api = FixtureApi::answering(&[(url, &arcade)]);

    let candidates = connector(&api, &DEVELOPER)
        .discover(&request(|draft| {
            draft.platforms = vec!["MAME".to_owned(), "FBNeo - Arcade Games".to_owned()];
            draft.asset_types = vec![AssetTypeSelector::BoxFront];
        }))
        .unwrap();

    let found: Vec<(&str, &str)> = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.platform.as_str(),
                candidate.provider_candidate_id.as_deref().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("MAME", "mame/75/1234/box-2D(us)"),
            (
                "FBNeo - Arcade Games",
                "fbneoarcadegames/75/1234/box-2D(us)"
            ),
        ]
    );
    assert_eq!(api.requested().len(), 1);
}

#[test]
fn the_catalogs_own_platform_names_are_known() {
    let api = FixtureApi::answering(&[]);
    let connector = connector(&api, &DEVELOPER);

    for platform in [
        "The 3DO Company - 3DO",
        "Nintendo - Nintendo Switch",
        "Sony - PlayStation 4",
        "Microsoft - Xbox One",
    ] {
        let reason = connector
            .unsupported_request_reason(&request(|draft| {
                draft.platforms = vec![platform.to_owned()];
            }))
            .unwrap();
        assert_eq!(reason, None, "{platform}");
    }
}

#[test]
fn the_regions_of_the_countries_screenscraper_names_are_known() {
    let api = FixtureApi::answering(&[]);
    let connector = connector(&api, &DEVELOPER);

    for region in ["Portugal", "Russia", "Poland", "Denmark", "pt"] {
        let reason = connector
            .unsupported_request_reason(&request(|draft| {
                draft.regions = vec![region.to_owned()];
            }))
            .unwrap();
        assert_eq!(reason, None, "{region}");
    }
}

/// Counts the requests under way, and the most there ever were at once.
#[derive(Default)]
struct SlowApi {
    under_way: AtomicUsize,
    most: AtomicUsize,
}

impl HttpTransport for &SlowApi {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        panic!("ScreenScraper is never asked without credentials: {url}")
    }

    fn get_stream_with_query_keys(
        &self,
        _url: &str,
        _keys: &[(&str, &ApiKey)],
    ) -> Result<Box<dyn Read + Send>, PortError> {
        let now = self.under_way.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(50));
        self.under_way.fetch_sub(1, Ordering::SeqCst);
        Ok(Box::new(Cursor::new(b"media".to_vec())))
    }
}

#[test]
fn the_requests_of_every_connector_go_one_at_a_time() {
    let api = SlowApi::default();
    // Each run builds connectors of its own.
    let runs: Vec<ScreenScraperConnector<&SlowApi>> = (0..2)
        .map(|_| {
            ScreenScraperConnector::with_transport(&api, Arc::new(KeyStore::holding(&DEVELOPER)))
        })
        .collect();
    let media = candidate(AssetType::BoxFront, "box-2D(us)", "USA", "box-2D (us)");

    thread::scope(|scope| {
        for run in &runs {
            let media = &media;
            scope.spawn(move || run.download(media).map(drop).unwrap());
        }
    });

    assert_eq!(api.most.load(Ordering::SeqCst), 1);
}

#[test]
fn gameplay_videos_are_found_and_downloaded_from_the_video_script() {
    let video = r#"{"type":"video-normalized","parent":"jeu","url":"https://neoclone.screenscraper.fr/api2/mediaVideoJeu.php?devid=zq-dev&devpassword=zq-dev-password&softname=game-media-vault&ssid=&sspassword=&systemeid=57&jeuid=1234&media=video-normalized","crc":"0","md5":"0","sha1":"0","size":"1","format":"mp4"}"#;
    let found = format!(
        r#"{{"response":{{"jeux":[{{"id":"1234","noms":[{{"region":"ss","text":"Ridge Racer"}}],"systeme":{{"id":"57"}},"medias":[{video}]}}]}}}}"#
    );
    let api = FixtureApi::answering(&[(SEARCH_URL, &found)]);
    let connector = connector(&api, &DEVELOPER);

    let candidates = connector
        .discover(&request(|draft| {
            draft.asset_types = vec![AssetTypeSelector::GameplayVideo];
        }))
        .unwrap();

    assert!(
        connector
            .capabilities()
            .asset_types
            .contains(&AssetType::GameplayVideo)
    );
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate.asset_type, AssetType::GameplayVideo);
    assert_eq!(candidate.region, "Unknown");
    assert_eq!(
        candidate.source_url,
        "https://api.screenscraper.fr/api2/mediaVideoJeu.php/57/1234/video-normalized"
    );
    assert_eq!(candidate.original_filename, "1234-video-normalized.mp4");

    connector.download(candidate).unwrap();
    assert_eq!(
        api.requested().last().unwrap().0,
        "https://api.screenscraper.fr/api2/mediaVideoJeu.php?softname=game-media-vault&systemeid=57&jeuid=1234&media=video-normalized"
    );
}
