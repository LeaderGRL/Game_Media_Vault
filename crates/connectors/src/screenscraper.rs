//! ScreenScraper: a community database of game media, scans of boxes, cartridges, discs and
//! manuals among them, served by an API that needs developer credentials and, optionally, the
//! user's own account, all sent as query parameters, to its media too.

use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read},
    sync::{Arc, Mutex, PoisonError},
};

use game_media_vault_application::{
    ApiKey, ConnectorPort, CredentialField, CredentialStorePort, PortError,
};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection, SourceId,
};
use serde_json::Value;
use url::Url;

use crate::{
    HttpTransport, ReqwestHttpTransport,
    selection::{name_key, platform_key, wanted_games},
};

/// The most games one discovery looks up: the Source answers one request per game, so a
/// request for a whole platform is discovered that many games at a time.
const DISCOVERY_BATCH: usize = 25;

pub const SCREENSCRAPER_SOURCE_ID: &str = "screenscraper";

/// Where every request goes, whatever server a media's own address names, so the credentials
/// requests carry reach ScreenScraper's API alone.
const API: &str = "https://api.screenscraper.fr/api2/";

/// The name ScreenScraper knows this software by.
const SOFTNAME: &str = "game-media-vault";

const DEV_ID: CredentialField = CredentialField {
    id: "dev-id",
    label: "Developer id",
    optional: false,
    single_word: true,
};

const DEV_PASSWORD: CredentialField = CredentialField {
    id: "dev-password",
    label: "Developer password",
    optional: false,
    single_word: false,
};

const USER_ID: CredentialField = CredentialField {
    id: "user-id",
    label: "Account user name",
    optional: true,
    single_word: false,
};

const USER_PASSWORD: CredentialField = CredentialField {
    id: "user-password",
    label: "Account password",
    optional: true,
    single_word: false,
};

/// The developer credentials ScreenScraper grants a software, and the user's own account, which
/// brings the user's own quota.
const CREDENTIALS: [CredentialField; 4] = [DEV_ID, DEV_PASSWORD, USER_ID, USER_PASSWORD];

/// The media ScreenScraper serves that have an Asset Type, by their type. Its scan of the
/// cartridge or disc, `support-2D`, has the Asset Type of what the platform's games come on.
const MEDIA: [(&str, AssetType); 13] = [
    ("box-2D", AssetType::BoxFront),
    ("box-2D-back", AssetType::BoxBack),
    ("box-2D-side", AssetType::Spine),
    ("box-3D", AssetType::Box3dRender),
    ("manuel", AssetType::Manual),
    ("ss", AssetType::Screenshot),
    ("sstitle", AssetType::TitleScreen),
    ("video", AssetType::GameplayVideo),
    ("video-normalized", AssetType::GameplayVideo),
    ("wheel", AssetType::Logo),
    ("wheel-hd", AssetType::Logo),
    ("fanart", AssetType::WallpaperArtwork),
    ("flyer", AssetType::Flyer),
];

const SUPPORT: &str = "support-2D";

const CARTRIDGE: Option<AssetType> = Some(AssetType::CartridgeFront);
const DISC: Option<AssetType> = Some(AssetType::Disc);

/// The catalog platforms ScreenScraper has a system for: the id of that system, and the Asset
/// Type of a scan of what the platform's games come on, if any.
const SYSTEMS: &[(&str, u64, Option<AssetType>)] = &[
    ("Nintendo - Nintendo Entertainment System", 3, CARTRIDGE),
    (
        "Nintendo - Super Nintendo Entertainment System",
        4,
        CARTRIDGE,
    ),
    ("Nintendo - Nintendo 64", 14, CARTRIDGE),
    ("Nintendo - GameCube", 13, DISC),
    ("Nintendo - Wii", 16, DISC),
    ("Nintendo - Wii U", 18, DISC),
    ("Nintendo - Game Boy", 9, CARTRIDGE),
    ("Nintendo - Game Boy Color", 10, CARTRIDGE),
    ("Nintendo - Game Boy Advance", 12, CARTRIDGE),
    ("Nintendo - Nintendo DS", 15, CARTRIDGE),
    ("Nintendo - Nintendo 3DS", 17, CARTRIDGE),
    ("Nintendo - Virtual Boy", 11, CARTRIDGE),
    ("Nintendo - Pokemon Mini", 211, CARTRIDGE),
    ("Sega - SG-1000", 109, CARTRIDGE),
    ("Sega - Master System - Mark III", 2, CARTRIDGE),
    ("Sega - Game Gear", 21, CARTRIDGE),
    ("Sega - Mega Drive - Genesis", 1, CARTRIDGE),
    ("Sega - Mega-CD - Sega CD", 20, DISC),
    ("Sega - 32X", 19, CARTRIDGE),
    ("Sega - Saturn", 22, DISC),
    ("Sega - Dreamcast", 23, DISC),
    ("Sony - PlayStation", 57, DISC),
    ("Sony - PlayStation 2", 58, DISC),
    ("Sony - PlayStation 3", 59, DISC),
    ("Sony - PlayStation Portable", 61, DISC),
    ("Sony - PlayStation Vita", 62, CARTRIDGE),
    ("NEC - PC Engine - TurboGrafx-16", 31, CARTRIDGE),
    ("NEC - PC Engine SuperGrafx", 105, CARTRIDGE),
    ("NEC - PC Engine CD - TurboGrafx-CD", 114, DISC),
    ("NEC - PC-FX", 72, DISC),
    ("SNK - Neo Geo Pocket", 25, CARTRIDGE),
    ("SNK - Neo Geo Pocket Color", 82, CARTRIDGE),
    ("SNK - Neo Geo CD", 70, DISC),
    ("Bandai - WonderSwan", 45, CARTRIDGE),
    ("Bandai - WonderSwan Color", 46, CARTRIDGE),
    ("Atari - 2600", 26, CARTRIDGE),
    ("Atari - 5200", 40, CARTRIDGE),
    ("Atari - 7800", 41, CARTRIDGE),
    ("Atari - Jaguar", 27, CARTRIDGE),
    ("Atari - Lynx", 28, CARTRIDGE),
    ("Coleco - ColecoVision", 48, CARTRIDGE),
    ("Mattel - Intellivision", 115, CARTRIDGE),
    ("GCE - Vectrex", 102, CARTRIDGE),
    ("Sony - PlayStation 4", 60, DISC),
    ("Nintendo - Nintendo Switch", 225, CARTRIDGE),
    ("The 3DO Company - 3DO", 29, DISC),
    ("Panasonic - 3DO Interactive Multiplayer", 29, DISC),
    ("Philips - CD-i", 133, DISC),
    ("Microsoft - Xbox", 32, DISC),
    ("Microsoft - Xbox 360", 33, DISC),
    ("Microsoft - Xbox One", 34, DISC),
    // MAME and FBNeo emulate arcade boards, which ScreenScraper files under one system.
    ("MAME", 75, None),
    ("FBNeo - Arcade Games", 75, None),
];

/// The regions ScreenScraper's media name, by their code, with the catalog region each stands
/// for. A media of another region, or of none, is recorded in no region.
const REGIONS: &[(&str, &str)] = &[
    ("wor", "World"),
    ("us", "USA"),
    ("eu", "Europe"),
    ("jp", "Japan"),
    ("asi", "Asia"),
    ("fr", "France"),
    ("de", "Germany"),
    ("es", "Spain"),
    ("it", "Italy"),
    ("uk", "UK"),
    ("nl", "Netherlands"),
    ("se", "Sweden"),
    ("au", "Australia"),
    ("ca", "Canada"),
    ("br", "Brazil"),
    ("kr", "Korea"),
    ("cn", "China"),
    ("tw", "Taiwan"),
    ("pt", "Portugal"),
    ("ru", "Russia"),
    ("pl", "Poland"),
    ("dk", "Denmark"),
    ("fi", "Finland"),
    ("no", "Norway"),
    ("gr", "Greece"),
    ("hu", "Hungary"),
    ("cz", "Czech"),
    ("sk", "Slovakia"),
    ("tr", "Turkey"),
    ("il", "Israel"),
    ("nz", "New Zealand"),
    ("cl", "Chile"),
    ("pe", "Peru"),
    ("bg", "Bulgaria"),
];

/// The scripts that serve media, one of which each locator names.
const MEDIA_SCRIPTS: [&str; 3] = ["mediaJeu.php", "mediaManuelJeu.php", "mediaVideoJeu.php"];

/// The largest media ScreenScraper is trusted to serve, which keeps a misbehaving answer from
/// filling memory; scanned manuals and videos are the largest.
const MAX_MEDIA_BYTES: u64 = 512 * 1024 * 1024;

/// What ScreenScraper answers a media request with instead of the media: that it has none, or
/// that the one the request names is unchanged.
const NO_MEDIA_REPLIES: [&[u8]; 4] = [b"NOMEDIA", b"CRCOK", b"MD5OK", b"SHA1OK"];

/// Held through each request every ScreenScraper connector of the process sends, so that no two
/// are ever under way at once, whichever run or vault sends them: every run reads the same
/// account from this machine's credential store.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Reads the credentials from this machine's credential store at each request, so credentials
/// stored or cleared meanwhile take effect at once, and sends one request at a time, as a free
/// account must.
pub struct ScreenScraperConnector<T = ReqwestHttpTransport> {
    transport: T,
    credentials: Arc<dyn CredentialStorePort>,
}

impl ScreenScraperConnector<ReqwestHttpTransport> {
    pub fn new(credentials: Arc<dyn CredentialStorePort>) -> Self {
        Self::with_transport(ReqwestHttpTransport::default(), credentials)
    }
}

impl<T> ScreenScraperConnector<T> {
    pub fn with_transport(transport: T, credentials: Arc<dyn CredentialStorePort>) -> Self {
        Self {
            transport,
            credentials,
        }
    }

    /// The credentials this machine stores, once it stores the developer's.
    fn stored_credentials(&self) -> Result<Option<Credentials>, PortError> {
        let read = |field: &CredentialField| {
            self.credentials
                .api_key(&field.stored_as(SCREENSCRAPER_SOURCE_ID))
        };
        let (Some(id), Some(password)) = (read(&DEV_ID)?, read(&DEV_PASSWORD)?) else {
            return Ok(None);
        };
        // An account is sent whole or not at all.
        let account = match (read(&USER_ID)?, read(&USER_PASSWORD)?) {
            (Some(name), Some(password)) => Some((name, password)),
            _ => None,
        };
        Ok(Some(Credentials {
            developer: (id, password),
            account,
        }))
    }

    fn credentials(&self) -> Result<Credentials, PortError> {
        self.stored_credentials()?.ok_or_else(|| {
            PortError::new(
                "ScreenScraper needs developer credentials that this machine no longer stores"
                    .to_owned(),
            )
        })
    }
}

/// The credentials ScreenScraper's requests carry: the developer's, and the user's account when
/// this machine stores both its name and its password.
struct Credentials {
    developer: (ApiKey, ApiKey),
    account: Option<(ApiKey, ApiKey)>,
}

impl Credentials {
    /// Each credential with the query parameter carrying it.
    fn query(&self) -> Vec<(&'static str, &ApiKey)> {
        let mut keys = vec![
            ("devid", &self.developer.0),
            ("devpassword", &self.developer.1),
        ];
        if let Some((name, password)) = &self.account {
            keys.push(("ssid", name));
            keys.push(("sspassword", password));
        }
        keys
    }
}

impl<T> ConnectorPort for ScreenScraperConnector<T>
where
    T: HttpTransport,
{
    fn source_id(&self) -> &'static str {
        SCREENSCRAPER_SOURCE_ID
    }

    fn credential_fields(&self) -> &'static [CredentialField] {
        &CREDENTIALS
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![
                AssetType::BoxFront,
                AssetType::BoxBack,
                AssetType::Spine,
                AssetType::Box3dRender,
                AssetType::CartridgeFront,
                AssetType::Disc,
                AssetType::Manual,
                AssetType::Screenshot,
                AssetType::TitleScreen,
                AssetType::GameplayVideo,
                AssetType::Logo,
                AssetType::WallpaperArtwork,
                AssetType::Flyer,
            ],
            direct_media_download: true,
        }
    }

    fn rate_limits(&self) -> Option<String> {
        Some("Each account may send one request at a time, and a daily quota of requests that free accounts keep low; requests are sent one at a time, whatever runs send them. A discovery takes one search per requested game and ScreenScraper system, and one more per game found without its media; each media downloaded takes one more.".to_owned())
    }

    /// Checked without reaching ScreenScraper.
    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        match self.stored_credentials() {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Ok(Some(
                    "ScreenScraper needs developer credentials: store them with `game-media-vault source key set screenscraper --field dev-id` and `--field dev-password`"
                        .to_owned(),
                ));
            }
            // A credential store that cannot be read leaves this Source out, not the whole plan.
            Err(error) => {
                return Ok(Some(format!(
                    "ScreenScraper's credentials could not be read: {}",
                    error.message()
                )));
            }
        }
        if matches!(request.games(), GameSelection::All) {
            return Ok(Some(
                "ScreenScraper needs an explicit game selection, since its daily quota cannot list whole platforms"
                    .to_owned(),
            ));
        }
        // A request also naming another platform would go partly unserved without a word.
        if let Some((_, platform)) = wanted_games(request)
            .iter()
            .find(|(_, platform)| system(platform).is_none())
        {
            return Ok(Some(format!(
                "ScreenScraper has no system known for platform {platform}"
            )));
        }
        if let Some(region) = request
            .regions()
            .iter()
            .find(|region| region_code(region).is_none())
        {
            return Ok(Some(format!(
                "ScreenScraper cannot tell media of {region} apart"
            )));
        }
        if !request.languages().is_empty() {
            return Ok(Some(
                "ScreenScraper cannot satisfy language filters yet".to_owned(),
            ));
        }
        Ok(None)
    }

    /// Games are looked up one by one, so a request for many is discovered a batch at a time,
    /// each recorded as it completes.
    fn discovery_batch_size(&self) -> Option<usize> {
        Some(DISCOVERY_BATCH)
    }

    /// Searches each requested game by name on its platform's system, once for every platform
    /// of that system, keeping only the games named exactly so, and lists their media of the
    /// requested kinds and regions.
    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        if let Some(reason) = self.unsupported_request_reason(request)? {
            return Err(PortError::new(reason));
        }
        let credentials = self.credentials()?;
        let regions: Vec<&str> = request
            .regions()
            .iter()
            .filter_map(|region| region_code(region))
            .collect();
        // The media of the games each search found, read once for every platform of a system,
        // as MAME and FBNeo share the arcade one.
        let mut searched: HashMap<(u64, String), Vec<Value>> = HashMap::new();
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        for (title, platform) in wanted_games(request) {
            let Some((system_id, support)) = system(&platform) else {
                continue;
            };
            let search = (system_id, title.clone());
            if !searched.contains_key(&search) {
                let media = self.media_of_games_named(&title, system_id, &credentials)?;
                searched.insert(search.clone(), media);
            }
            let found = Found {
                title: &title,
                platform: &platform,
                support,
                regions: &regions,
                request,
            };
            for media in &searched[&search] {
                if let Some(candidate) = candidate(media, &found)
                    && seen.insert(candidate.provider_candidate_id.clone())
                {
                    candidates.push(candidate);
                }
            }
        }
        Ok(candidates)
    }

    /// Asks ScreenScraper for the media the locator names, adding the credentials only now.
    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        let (script, system_id, game_id, name) =
            locator_parts(&candidate.source_url).ok_or_else(|| {
                PortError::new(format!(
                    "ScreenScraper cannot download {}, which it never discovered",
                    candidate.source_url
                ))
            })?;
        let credentials = self.credentials()?;
        let mut url = api_url(script);
        url.query_pairs_mut().extend_pairs([
            ("softname", SOFTNAME),
            ("systemeid", &system_id.to_string()),
            ("jeuid", &game_id.to_string()),
            ("media", &name),
        ]);
        let mut media = Vec::new();
        {
            // The whole media arrives before the next request leaves.
            let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
            self.transport
                .get_stream_with_query_keys(url.as_str(), &credentials.query())?
                .take(MAX_MEDIA_BYTES + 1)
                .read_to_end(&mut media)
                .map_err(|error| PortError::new(format!("failed to read {url}: {error}")))?;
        }
        if media.len() as u64 > MAX_MEDIA_BYTES {
            return Err(PortError::invalid_source_data(format!(
                "{url} exceeds {MAX_MEDIA_BYTES} bytes"
            )));
        }
        if NO_MEDIA_REPLIES.contains(&media.trim_ascii()) {
            return Err(PortError::unavailable(format!(
                "ScreenScraper no longer serves {}",
                candidate.source_url
            )));
        }
        Ok(Box::new(Cursor::new(media)))
    }
}

impl<T> ScreenScraperConnector<T>
where
    T: HttpTransport,
{
    /// The media of the games ScreenScraper names exactly `title` on the system `system_id`.
    fn media_of_games_named(
        &self,
        title: &str,
        system_id: u64,
        credentials: &Credentials,
    ) -> Result<Vec<Value>, PortError> {
        let system_number = system_id.to_string();
        let answer = match self.get(
            "jeuRecherche.php",
            &[("systemeid", &system_number), ("recherche", title)],
            credentials,
        ) {
            Ok(answer) => answer,
            // ScreenScraper answers a search it finds nothing for with HTTP 404.
            Err(error) if error.is_unavailable() => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        // A search without its list of games is no answer that none matched.
        let games = answer
            .pointer("/response/jeux")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                PortError::invalid_source_data(
                    "ScreenScraper answered a game search without its games".to_owned(),
                )
            })?;
        let wanted = name_key(title);
        let mut media = Vec::new();
        for game in games {
            // A search that finds nothing lists one game without anything.
            let Some(game_id) = game.get("id").and_then(number) else {
                continue;
            };
            if !is_named(game, &wanted)
                || game
                    .pointer("/systeme/id")
                    .and_then(number)
                    .is_some_and(|listed| listed != system_id)
            {
                continue;
            }
            match game.get("medias").and_then(Value::as_array) {
                Some(listed) => media.extend(listed.iter().cloned()),
                None => media.extend(self.media_of(game_id, &system_number, credentials)?),
            }
        }
        Ok(media)
    }

    /// The media of the game `game_id`, for a search that listed it without them.
    fn media_of(
        &self,
        game_id: u64,
        system_number: &str,
        credentials: &Credentials,
    ) -> Result<Vec<Value>, PortError> {
        let answer = self.get(
            "jeuInfos.php",
            &[
                ("systemeid", system_number),
                ("gameid", &game_id.to_string()),
            ],
            credentials,
        )?;
        let game = answer.pointer("/response/jeu").ok_or_else(|| {
            PortError::invalid_source_data(format!("ScreenScraper answered without game {game_id}"))
        })?;
        Ok(game
            .get("medias")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// The answer of the API's `script` to the `query` pairs, sent with the credentials.
    fn get(
        &self,
        script: &str,
        query: &[(&str, &str)],
        credentials: &Credentials,
    ) -> Result<Value, PortError> {
        let mut url = api_url(script);
        url.query_pairs_mut()
            .extend_pairs([("softname", SOFTNAME), ("output", "json")])
            .extend_pairs(query);
        let body = {
            let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
            self.transport
                .get_with_query_keys(url.as_str(), &credentials.query())?
        };
        // The body may answer with the credentials it was asked with, so it is left unsaid.
        serde_json::from_slice(&body).map_err(|error| {
            PortError::invalid_source_data(format!(
                "ScreenScraper answered {url} with no JSON: {error}"
            ))
        })
    }
}

/// The address of the API's `script`.
fn api_url(script: &str) -> Url {
    Url::parse(API)
        .and_then(|api| api.join(script))
        .expect("the API's scripts have valid locations")
}

/// The game one search found, which its media are recorded with.
struct Found<'a> {
    title: &'a str,
    platform: &'a str,
    /// The Asset Type of a scan of what the platform's games come on, if any.
    support: Option<AssetType>,
    /// The codes of the requested regions; none when the request names none.
    regions: &'a [&'a str],
    request: &'a AcquisitionRequest,
}

/// The candidate a media of ScreenScraper describes, when it is of a requested kind and region,
/// and ScreenScraper serves it. Its locator names the media by the script, system, game and
/// media name of its own address, without the credentials that address carries.
fn candidate(media: &Value, found: &Found<'_>) -> Option<AssetCandidate> {
    let kind = media.get("type")?.as_str()?;
    let asset_type = if kind == SUPPORT {
        found.support?
    } else {
        MEDIA.iter().find(|(known, _)| *known == kind)?.1
    };
    if !found.request.requests_asset_type(asset_type) {
        return None;
    }
    let code = media.get("region").and_then(Value::as_str);
    if !found.regions.is_empty() && !code.is_some_and(|code| found.regions.contains(&code)) {
        return None;
    }
    let region = code
        .and_then(|code| REGIONS.iter().find(|(known, _)| *known == code))
        .map_or("Unknown", |(_, region)| region);
    let (script, system_id, game_id, name) = media_address(media.get("url")?.as_str()?)?;
    let original_filename = match media
        .get("format")
        .and_then(Value::as_str)
        .filter(|format| !format.is_empty() && format.chars().all(|c| c.is_ascii_alphanumeric()))
    {
        Some(format) => format!("{game_id}-{name}.{format}"),
        None => format!("{game_id}-{name}"),
    };
    let label = match code {
        Some(code) => format!("{kind} ({code})"),
        None => kind.to_owned(),
    };
    Some(AssetCandidate {
        // One media may serve several platforms of one system, each its own candidate.
        provider_candidate_id: Some(format!(
            "{}/{system_id}/{game_id}/{name}",
            platform_key(found.platform)
        )),
        game_title: found.title.to_owned(),
        platform: found.platform.to_owned(),
        region: region.to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type,
        source_id: SourceId::from(SCREENSCRAPER_SOURCE_ID),
        source_asset_label: Some(label),
        source_url: format!("{API}{script}/{system_id}/{game_id}/{name}"),
        original_filename,
    })
}

/// The script, system, game and media name a media's own address asks for, when ScreenScraper
/// serves it over HTTPS with one of its media scripts.
fn media_address(address: &str) -> Option<(&'static str, u64, u64, String)> {
    let url = Url::parse(address).ok()?;
    let host = url.host_str()?;
    if url.scheme() != "https"
        || !(host == "screenscraper.fr" || host.ends_with(".screenscraper.fr"))
    {
        return None;
    }
    let script_name = url.path_segments()?.next_back()?;
    let script = MEDIA_SCRIPTS
        .into_iter()
        .find(|known| *known == script_name)?;
    let (mut system_id, mut game_id, mut name) = (None, None, None);
    for (parameter, value) in url.query_pairs() {
        match parameter.as_ref() {
            "systemeid" => system_id = value.parse().ok(),
            "jeuid" => game_id = value.parse().ok(),
            "media" => name = Some(value.into_owned()),
            _ => {}
        }
    }
    Some((
        script,
        system_id?,
        game_id?,
        name.filter(|name| is_media_name(name))?,
    ))
}

/// The script, system, game and media name of a locator discovery made.
fn locator_parts(locator: &str) -> Option<(&'static str, u64, u64, String)> {
    let mut parts = locator.strip_prefix(API)?.split('/');
    let script_name = parts.next()?;
    let script = MEDIA_SCRIPTS
        .into_iter()
        .find(|known| *known == script_name)?;
    let system_id = digits(parts.next()?)?;
    let game_id = digits(parts.next()?)?;
    let name = parts.next().filter(|name| is_media_name(name))?.to_owned();
    if parts.next().is_some() {
        return None;
    }
    Some((script, system_id, game_id, name))
}

fn digits(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Whether `name` can name a media, such as `box-2D(us)`, in a locator's path as it is.
fn is_media_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '(' | ')'))
}

/// A number ScreenScraper writes as a number or as a string of digits.
fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str().and_then(digits))
}

/// Whether one of the game's names is the wanted one, by its name key.
fn is_named(game: &Value, wanted: &str) -> bool {
    game.get("noms")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|name| name.get("text")?.as_str())
        .any(|name| name_key(name) == wanted)
}

/// The id of ScreenScraper's system for a catalog platform, and the Asset Type of a scan of what
/// its games come on, if any.
fn system(platform: &str) -> Option<(u64, Option<AssetType>)> {
    let key = name_key(platform);
    SYSTEMS
        .iter()
        .find(|(name, ..)| name_key(name) == key)
        .map(|(_, id, support)| (*id, *support))
}

/// The code ScreenScraper gives a requested region, named as the catalog names it or by its
/// code, regardless of case and punctuation.
fn region_code(region: &str) -> Option<&'static str> {
    let key = name_key(region);
    REGIONS
        .iter()
        .find(|(code, name)| *code == key || name_key(name) == key)
        .map(|(code, _)| *code)
}
