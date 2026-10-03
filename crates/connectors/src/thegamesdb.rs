//! TheGamesDB: community box art and media of games, served by an API that needs the user's own
//! key, sent as a query parameter.

use std::{
    collections::{BTreeSet, HashSet},
    sync::{Arc, Mutex},
};

use game_media_vault_application::{ApiKey, ConnectorPort, CredentialStorePort, PortError};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection, SourceId,
};
use serde_json::Value;
use url::Url;

use crate::{
    HttpTransport, ReqwestHttpTransport,
    selection::{name_key, wanted_games},
};

pub const THEGAMESDB_SOURCE_ID: &str = "thegamesdb";

const API: &str = "https://api.thegamesdb.net";

/// The media TheGamesDB serves, by the image type, and the side of box art, that lists it.
/// Banners have no Asset Type.
const MEDIA: [(AssetType, &str, Option<&str>); 6] = [
    (AssetType::BoxFront, "boxart", Some("front")),
    (AssetType::BoxBack, "boxart", Some("back")),
    (AssetType::Screenshot, "screenshot", None),
    (AssetType::TitleScreen, "titlescreen", None),
    (AssetType::Logo, "clearlogo", None),
    (AssetType::WallpaperArtwork, "fanart", None),
];

/// The most pages of images read for one batch of games, which bounds what a discovery takes
/// from the key's monthly allowance.
const MAX_IMAGE_PAGES: u32 = 10;

/// The most games one request for images names, which keeps its address short.
const IMAGE_BATCH_GAMES: usize = 20;

/// The most pages of a game search one requested game reads.
const MAX_SEARCH_PAGES: u32 = 5;

/// The platforms TheGamesDB names with other words than the No-Intro and Redump catalogs do: each
/// catalog name with the TheGamesDB names it stands for.
const PLATFORM_ALIASES: [(&str, &[&str]); 10] = [
    (
        "Nintendo - Super Nintendo Entertainment System",
        &["Super Nintendo (SNES)"],
    ),
    (
        "Sega - Mega Drive - Genesis",
        &["Sega Genesis", "Sega Mega Drive"],
    ),
    ("Sega - Mega-CD - Sega CD", &["Sega CD"]),
    ("Sega - Master System - Mark III", &["Sega Master System"]),
    ("NEC - PC Engine - TurboGrafx-16", &["TurboGrafx 16"]),
    ("Sony - PlayStation Portable", &["Sony PSP"]),
    ("Bandai - WonderSwan", &["WonderSwan"]),
    ("Bandai - WonderSwan Color", &["WonderSwan Color"]),
    ("SNK - Neo Geo Pocket", &["Neo Geo Pocket"]),
    ("SNK - Neo Geo Pocket Color", &["Neo Geo Pocket Color"]),
];

/// A platform TheGamesDB lists: its id and the words naming it, with and without the words in
/// parentheses, which may be an abbreviation it adds, as `(NES)`, or part of its name, as `(LCD)`.
type ListedPlatform = (u64, [BTreeSet<String>; 2]);

/// Reads the API key from this machine's credential store at each discovery, so a key stored or
/// cleared meanwhile takes effect at once. Images are downloaded from their public location,
/// without the key.
pub struct TheGamesDbConnector<T = ReqwestHttpTransport> {
    transport: T,
    credentials: Arc<dyn CredentialStorePort>,
    /// The platforms TheGamesDB lists, read once.
    platforms: Mutex<Option<Vec<ListedPlatform>>>,
}

impl TheGamesDbConnector<ReqwestHttpTransport> {
    pub fn new(credentials: Arc<dyn CredentialStorePort>) -> Self {
        Self::with_transport(ReqwestHttpTransport::default(), credentials)
    }
}

impl<T> TheGamesDbConnector<T> {
    pub fn with_transport(transport: T, credentials: Arc<dyn CredentialStorePort>) -> Self {
        Self {
            transport,
            credentials,
            platforms: Mutex::new(None),
        }
    }

    fn api_key(&self) -> Result<Option<ApiKey>, PortError> {
        self.credentials.api_key(THEGAMESDB_SOURCE_ID)
    }
}

impl<T> ConnectorPort for TheGamesDbConnector<T>
where
    T: HttpTransport,
{
    fn source_id(&self) -> &'static str {
        THEGAMESDB_SOURCE_ID
    }

    fn needs_api_key(&self) -> bool {
        true
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: MEDIA.iter().map(|(asset_type, _, _)| *asset_type).collect(),
            direct_media_download: true,
        }
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
                    "TheGamesDB needs an API key: store one with `game-media-vault source key set thegamesdb`"
                        .to_owned(),
                ));
            }
            // A credential store that cannot be read leaves this Source out, not the whole plan.
            Err(error) => {
                return Ok(Some(format!(
                    "TheGamesDB's API key could not be read: {}",
                    error.message()
                )));
            }
        }
        if matches!(request.games(), GameSelection::All) {
            return Ok(Some(
                "TheGamesDB needs an explicit game selection, since its monthly allowance cannot list whole platforms"
                    .to_owned(),
            ));
        }
        if !request.regions().is_empty() {
            return Ok(Some(
                "TheGamesDB cannot satisfy region filters because its media record no region"
                    .to_owned(),
            ));
        }
        if !request.languages().is_empty() {
            return Ok(Some(
                "TheGamesDB cannot satisfy language filters yet".to_owned(),
            ));
        }
        Ok(None)
    }

    /// Looks each requested game up by name on the TheGamesDB platforms its platform names,
    /// keeping only the games named exactly so, then lists their media of the requested kinds
    /// in one paged request.
    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        if let Some(reason) = self.unsupported_request_reason(request)? {
            return Err(PortError::new(reason));
        }
        let key = self.api_key()?.ok_or_else(|| {
            PortError::new("TheGamesDB needs an API key that this machine no longer stores".into())
        })?;
        let media: Vec<(AssetType, &str, Option<&str>)> = MEDIA
            .iter()
            .copied()
            .filter(|(asset_type, _, _)| request.requests_asset_type(*asset_type))
            .collect();
        if media.is_empty() {
            return Ok(Vec::new());
        }
        // The games found, each with the title and platform it was requested as.
        let mut games: Vec<(u64, String, String)> = Vec::new();
        for (title, platform) in wanted_games(request) {
            let platform_ids = self.platforms_named(&platform, &key)?;
            if platform_ids.is_empty() {
                continue;
            }
            let ids: Vec<String> = platform_ids.iter().map(u64::to_string).collect();
            let ids = ids.join(",");
            let wanted = name_key(&title);
            for page in 1..=MAX_SEARCH_PAGES {
                let page_number = page.to_string();
                let mut query = vec![("name", title.as_str()), ("filter[platform]", &ids)];
                if page > 1 {
                    query.push(("page", &page_number));
                }
                let answer = self.get("v1.1/Games/ByGameName", &query, &key)?;
                // A search without its list of games is no answer that none matched.
                let found = answer
                    .pointer("/data/games")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        PortError::invalid_source_data(
                            "TheGamesDB answered a game search without its games".to_owned(),
                        )
                    })?;
                for game in found {
                    let (Some(id), Some(name), Some(on)) = (
                        game.get("id").and_then(Value::as_u64),
                        game.get("game_title").and_then(Value::as_str),
                        game.get("platform").and_then(Value::as_u64),
                    ) else {
                        continue;
                    };
                    if name_key(name) == wanted
                        && platform_ids.contains(&on)
                        && !games.iter().any(|(found, ..)| *found == id)
                    {
                        games.push((id, title.clone(), platform.clone()));
                    }
                }
                if !has_next_page(&answer) {
                    break;
                }
            }
        }
        if games.is_empty() {
            return Ok(Vec::new());
        }
        let mut types: Vec<&str> = Vec::new();
        for (_, image_type, _) in &media {
            if !types.contains(image_type) {
                types.push(image_type);
            }
        }
        let types = types.join(",");
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        // Batches keep each request's address short, however many games were found.
        for batch in games.chunks(IMAGE_BATCH_GAMES) {
            let game_ids: Vec<String> = batch.iter().map(|(id, ..)| id.to_string()).collect();
            let game_ids = game_ids.join(",");
            for page in 1..=MAX_IMAGE_PAGES {
                let page_number = page.to_string();
                let mut query = vec![("games_id", game_ids.as_str()), ("filter[type]", &types)];
                if page > 1 {
                    query.push(("page", &page_number));
                }
                let answer = self.get("v1/Games/Images", &query, &key)?;
                let base = answer
                    .pointer("/data/base_url/original")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        PortError::invalid_source_data(
                            "TheGamesDB listed images without their location".to_owned(),
                        )
                    })?;
                for (game_id, title, platform) in batch {
                    let path = format!("/data/images/{game_id}");
                    for image in answer
                        .pointer(&path)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(candidate) = candidate(image, base, title, platform, &media)
                            && seen.insert(candidate.provider_candidate_id.clone())
                        {
                            candidates.push(candidate);
                        }
                    }
                }
                if !has_next_page(&answer) {
                    break;
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

impl<T> TheGamesDbConnector<T>
where
    T: HttpTransport,
{
    /// The ids of the TheGamesDB platforms `platform` names: those named by the same words, or
    /// those its alias stands for, as `Sega Genesis` and `Sega Mega Drive` for
    /// `Sega - Mega Drive - Genesis`. A qualifier of the requested name, such as `(Digital)`,
    /// counts, so a digital platform is never taken for the physical one.
    fn platforms_named(&self, platform: &str, key: &ApiKey) -> Result<Vec<u64>, PortError> {
        let mut platforms = self
            .platforms
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if platforms.is_none() {
            let answer = self.get("v1/Platforms", &[], key)?;
            let listed = answer
                .pointer("/data/platforms")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    PortError::invalid_source_data(
                        "TheGamesDB answered without its platforms".to_owned(),
                    )
                })?;
            *platforms = Some(
                listed
                    .values()
                    .filter_map(|listed| {
                        Some((listed.get("id")?.as_u64()?, {
                            let name = listed.get("name")?.as_str()?;
                            [platform_words(name, true), listed_platform_words(name)]
                        }))
                    })
                    .collect(),
            );
        }
        let listed = platforms.as_deref().unwrap_or_default();
        let requested = requested_platform_words(platform);
        let wanted: Vec<BTreeSet<String>> = match PLATFORM_ALIASES
            .iter()
            .find(|(catalog, _)| requested_platform_words(catalog) == requested)
        {
            Some((_, names)) => names
                .iter()
                .map(|name| listed_platform_words(name))
                .collect(),
            None => vec![requested],
        };
        let mut same: Vec<u64> = listed
            .iter()
            .filter(|(_, [whole, abbreviated])| {
                wanted.contains(whole) || wanted.contains(abbreviated)
            })
            .map(|(id, _)| *id)
            .collect();
        same.sort_unstable();
        Ok(same)
    }

    /// The answer of the API at `path` with the `query` pairs, once it says it succeeded.
    fn get(&self, path: &str, query: &[(&str, &str)], key: &ApiKey) -> Result<Value, PortError> {
        let mut url = Url::parse(API).expect("the API location is a valid URL");
        url.set_path(path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let body = self
            .transport
            .get_with_query_key(url.as_str(), "apikey", key)?;
        let answer: Value = serde_json::from_slice(&body).map_err(|error| {
            PortError::invalid_source_data(format!(
                "TheGamesDB answered {url} with no JSON: {error}"
            ))
        })?;
        if answer.get("code").and_then(Value::as_u64) != Some(200) {
            let status = answer
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("no reason given");
            return Err(PortError::new(format!(
                "TheGamesDB refused {url}: {status}"
            )));
        }
        Ok(answer)
    }
}

/// Whether a paged answer has a next page. Its address carries the key, so only whether there
/// is one counts.
fn has_next_page(answer: &Value) -> bool {
    answer
        .pointer("/pages/next")
        .and_then(Value::as_str)
        .is_some_and(|next| !next.is_empty())
}

/// The candidate an image of the API describes, when it is of a requested kind and has a
/// public location under `base`.
fn candidate(
    image: &Value,
    base: &str,
    title: &str,
    platform: &str,
    media: &[(AssetType, &str, Option<&str>)],
) -> Option<AssetCandidate> {
    let id = image.get("id")?.as_u64()?;
    let image_type = image.get("type")?.as_str()?;
    let side = image.get("side").and_then(Value::as_str);
    let (asset_type, _, _) = media.iter().find(|(_, kind, wanted_side)| {
        *kind == image_type && wanted_side.is_none_or(|wanted| Some(wanted) == side)
    })?;
    let location = Url::parse(base)
        .ok()?
        .join(image.get("filename")?.as_str()?)
        .ok()?;
    if location.scheme() != "https" {
        return None;
    }
    let original_filename = location.path_segments()?.next_back()?.to_owned();
    if original_filename.is_empty() {
        return None;
    }
    let label = match side {
        Some(side) => format!("{image_type}: {side}"),
        None => image_type.to_owned(),
    };
    Some(AssetCandidate {
        provider_candidate_id: Some(format!("image/{id}")),
        game_title: title.to_owned(),
        platform: platform.to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type: *asset_type,
        source_id: SourceId::from(THEGAMESDB_SOURCE_ID),
        source_asset_label: Some(label),
        source_url: location.to_string(),
        original_filename,
    })
}

/// The words of a platform TheGamesDB lists, without the abbreviation in parentheses it adds,
/// such as `(NES)`.
fn listed_platform_words(name: &str) -> BTreeSet<String> {
    platform_words(name, false)
}

/// The words of a requested platform, its qualifiers in parentheses, such as `(Digital)`,
/// among them.
fn requested_platform_words(name: &str) -> BTreeSet<String> {
    platform_words(name, true)
}

/// The words naming a platform, regardless of case, punctuation and a maker named twice, with
/// or without its words in parentheses.
fn platform_words(name: &str, parenthesized: bool) -> BTreeSet<String> {
    let mut words = BTreeSet::new();
    let mut word = String::new();
    let mut depth = 0_usize;
    for character in name.chars().chain([' ']) {
        match character {
            '(' if !parenthesized => depth += 1,
            ')' if !parenthesized => depth = depth.saturating_sub(1),
            _ if depth == 0 && character.is_alphanumeric() => {
                word.extend(character.to_lowercase());
            }
            _ => {}
        }
        if !(depth == 0 && character.is_alphanumeric()) && !word.is_empty() {
            words.insert(std::mem::take(&mut word));
        }
    }
    words
}
