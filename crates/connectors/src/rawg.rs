//! RAWG: a database of games and their media, served by an API that needs the user's own key,
//! sent as a query parameter. Its terms ask for an active link to rawg.io wherever its data
//! shows, which the desktop app gives.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
};

use game_media_vault_application::{ApiKey, ConnectorPort, CredentialStorePort, PortError};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection, SourceId,
};
use serde_json::Value;
use url::Url;

use crate::{
    HttpTransport, ReqwestHttpTransport, RetryPolicy,
    selection::{name_key, platform_key, wanted_games},
};

/// The most games one discovery looks up: the Source answers one request per game, so a
/// request for a whole platform is discovered that many games at a time.
const DISCOVERY_BATCH: usize = 25;

pub const RAWG_SOURCE_ID: &str = "rawg";

const API: &str = "https://api.rawg.io/api/games";

/// The server of RAWG's images, the only one media are downloaded from.
const MEDIA_HOST: &str = "media.rawg.io";

/// The most games one search lists, which is the most RAWG lists on a page.
const SEARCH_PAGE_SIZE: &str = "40";

/// The platforms RAWG names with other words than the No-Intro and Redump catalogs do: each
/// catalog name with the RAWG names it stands for.
const PLATFORM_ALIASES: [(&str, &[&str]); 8] = [
    ("Nintendo - Nintendo Entertainment System", &["NES"]),
    ("Nintendo - Super Nintendo Entertainment System", &["SNES"]),
    ("Sega - Mega Drive - Genesis", &["Genesis"]),
    ("Sega - Master System - Mark III", &["SEGA Master System"]),
    ("Sega - Mega-CD - Sega CD", &["SEGA CD"]),
    ("Sony - PlayStation Portable", &["PSP"]),
    ("Sony - PlayStation Vita", &["PS Vita"]),
    ("Panasonic - 3DO Interactive Multiplayer", &["3DO"]),
];

/// Reads the API key from this machine's credential store at each discovery, so a key stored or
/// cleared meanwhile takes effect at once. Images are downloaded from RAWG's media server,
/// without the key, following no redirect elsewhere.
pub struct RawgConnector<T = ReqwestHttpTransport> {
    transport: T,
    credentials: Arc<dyn CredentialStorePort>,
}

impl RawgConnector<ReqwestHttpTransport> {
    pub fn new(credentials: Arc<dyn CredentialStorePort>) -> Self {
        // A redirect could take a download away from RAWG's media server.
        Self::with_transport(
            ReqwestHttpTransport::following_no_redirect(RetryPolicy::default()),
            credentials,
        )
    }
}

impl<T> RawgConnector<T> {
    pub fn with_transport(transport: T, credentials: Arc<dyn CredentialStorePort>) -> Self {
        Self {
            transport,
            credentials,
        }
    }

    fn api_key(&self) -> Result<Option<ApiKey>, PortError> {
        self.credentials.api_key(RAWG_SOURCE_ID)
    }
}

impl<T> ConnectorPort for RawgConnector<T>
where
    T: HttpTransport,
{
    fn source_id(&self) -> &'static str {
        RAWG_SOURCE_ID
    }

    fn needs_api_key(&self) -> bool {
        true
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::Screenshot, AssetType::WallpaperArtwork],
            direct_media_download: true,
        }
    }

    fn rate_limits(&self) -> Option<String> {
        Some("Each free API key has a monthly allowance of requests. A discovery takes one search per requested game, whatever the platforms; images are downloaded from RAWG's media server without the key.".to_owned())
    }

    /// Checked without reaching the API.
    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        match self.api_key() {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Ok(Some(
                    "RAWG needs an API key: store one with `game-media-vault source key set rawg`"
                        .to_owned(),
                ));
            }
            // A credential store that cannot be read leaves this Source out, not the whole plan.
            Err(error) => {
                return Ok(Some(format!(
                    "RAWG's API key could not be read: {}",
                    error.message()
                )));
            }
        }
        if matches!(request.games(), GameSelection::All) {
            return Ok(Some(
                "RAWG needs an explicit game selection, since its monthly allowance cannot list whole platforms"
                    .to_owned(),
            ));
        }
        if !request.regions().is_empty() {
            return Ok(Some(
                "RAWG cannot satisfy region filters because its media record no region".to_owned(),
            ));
        }
        if !request.languages().is_empty() {
            return Ok(Some("RAWG cannot satisfy language filters yet".to_owned()));
        }
        Ok(None)
    }

    /// Searches each requested game by name once, keeping the games named exactly so that RAWG
    /// lists on a requested platform, and takes their background image and the screenshots the
    /// search lists with them.
    /// Games are looked up one by one, so a request for many is discovered a batch at a time,
    /// each recorded as it completes.
    fn discovery_batch_size(&self) -> Option<usize> {
        Some(DISCOVERY_BATCH)
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        if let Some(reason) = self.unsupported_request_reason(request)? {
            return Err(PortError::new(reason));
        }
        let key = self.api_key()?.ok_or_else(|| {
            PortError::new("RAWG needs an API key that this machine no longer stores".into())
        })?;
        let wants_backgrounds = request.requests_asset_type(AssetType::WallpaperArtwork);
        let wants_screenshots = request.requests_asset_type(AssetType::Screenshot);
        if !wants_backgrounds && !wants_screenshots {
            return Ok(Vec::new());
        }
        // The games each title's search found, read once for all of its platforms.
        let mut searched: HashMap<String, Vec<Value>> = HashMap::new();
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        for (title, platform) in wanted_games(request) {
            if !searched.contains_key(&title) {
                let found = self.search(&title, &key)?;
                searched.insert(title.clone(), found);
            }
            let wanted = name_key(&title);
            for game in &searched[&title] {
                let (Some(id), Some(name)) = (
                    game.get("id").and_then(Value::as_u64),
                    game.get("name").and_then(Value::as_str),
                ) else {
                    continue;
                };
                if name_key(name) != wanted || !lists_platform(game, &platform) {
                    continue;
                }
                let mut images = Vec::new();
                if wants_backgrounds
                    && let Some(location) = game.get("background_image").and_then(Value::as_str)
                {
                    images.push((
                        AssetType::WallpaperArtwork,
                        format!("{}/games/{id}/background", platform_key(&platform)),
                        "background",
                        location,
                    ));
                }
                if wants_screenshots {
                    for screenshot in game
                        .get("short_screenshots")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        // The background image comes first, without an id of its own.
                        let (Some(shot), Some(location)) = (
                            screenshot.get("id").and_then(Value::as_u64),
                            screenshot.get("image").and_then(Value::as_str),
                        ) else {
                            continue;
                        };
                        images.push((
                            AssetType::Screenshot,
                            format!("{}/screenshots/{shot}", platform_key(&platform)),
                            "screenshot",
                            location,
                        ));
                    }
                }
                for (asset_type, image_id, label, location) in images {
                    let Some((source_url, original_filename)) = media_location(location) else {
                        continue;
                    };
                    // One image, such as a background, applies to every requested platform, each
                    // its own candidate: its id names the platform, which candidate identity
                    // otherwise lacks.
                    if !seen.insert(image_id.clone()) {
                        continue;
                    }
                    candidates.push(AssetCandidate {
                        provider_candidate_id: Some(image_id),
                        game_title: title.clone(),
                        platform: platform.clone(),
                        region: "Unknown".to_owned(),
                        edition_name: "Unspecified".to_owned(),
                        asset_type,
                        source_id: SourceId::from(RAWG_SOURCE_ID),
                        source_asset_label: Some(label.to_owned()),
                        source_url,
                        original_filename,
                    });
                }
            }
        }
        Ok(candidates)
    }

    fn download(
        &self,
        candidate: &AssetCandidate,
    ) -> Result<Box<dyn std::io::Read + Send>, PortError> {
        self.transport.get_stream(&candidate.source_url)
    }
}

impl<T> RawgConnector<T>
where
    T: HttpTransport,
{
    /// The games a search for exactly `title` lists.
    fn search(&self, title: &str, key: &ApiKey) -> Result<Vec<Value>, PortError> {
        let mut url = Url::parse(API).expect("the API location is a valid URL");
        url.query_pairs_mut().extend_pairs([
            ("search", title),
            ("search_exact", "true"),
            ("page_size", SEARCH_PAGE_SIZE),
        ]);
        let body = self
            .transport
            .get_with_query_key(url.as_str(), "key", key)?;
        let answer: Value = serde_json::from_slice(&body).map_err(|error| {
            PortError::invalid_source_data(format!("RAWG answered {url} with no JSON: {error}"))
        })?;
        // A search without its list of games is no answer that none matched.
        answer
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| {
                PortError::invalid_source_data(format!("RAWG answered {url} without its games"))
            })
    }
}

/// The public location of an image RAWG's media server serves, and its file name.
fn media_location(location: &str) -> Option<(String, String)> {
    let url = Url::parse(location).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some(MEDIA_HOST)
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let filename = url.path_segments()?.next_back()?.to_owned();
    if filename.is_empty() {
        return None;
    }
    Some((url.to_string(), filename))
}

/// Whether RAWG lists the game on the requested catalog platform.
fn lists_platform(game: &Value, platform: &str) -> bool {
    game.get("platforms")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|listed| listed.pointer("/platform/name")?.as_str())
        .any(|listed| same_platform(listed, platform))
}

/// Whether a platform RAWG names is the requested catalog platform: by the same words, with or
/// without the maker the catalog names first, as `PlayStation 4` and `SEGA Saturn` for
/// `Sony - PlayStation 4` and `Sega - Saturn`, or by a known alias, as `SNES` for
/// `Nintendo - Super Nintendo Entertainment System`.
fn same_platform(listed: &str, requested: &str) -> bool {
    let listed = words(listed);
    if let Some((_, names)) = PLATFORM_ALIASES
        .iter()
        .find(|(catalog, _)| words(catalog) == words(requested))
    {
        return names.iter().any(|name| words(name) == listed);
    }
    let without_maker = requested
        .split_once(" - ")
        .map_or(requested, |(_, rest)| rest);
    listed == words(requested) || listed == words(without_maker)
}

/// The words of a name, regardless of case and punctuation.
fn words(name: &str) -> BTreeSet<String> {
    name.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}
