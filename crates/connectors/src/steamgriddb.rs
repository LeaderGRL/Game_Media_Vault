//! SteamGridDB: community artwork for games, served by an API that needs the user's own key.

use std::{collections::HashMap, sync::Arc};

use game_media_vault_application::{ApiKey, ConnectorPort, CredentialStorePort, PortError};
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

pub const STEAMGRIDDB_SOURCE_ID: &str = "steamgriddb";

const API: &str = "https://www.steamgriddb.com/api/v2";

/// The media SteamGridDB serves, by the API collection that lists a game's media of that kind.
/// Grids, its library capsules, are left out: they are fan-made launcher covers rather than scans
/// of packaging (#158).
const MEDIA: [(AssetType, &str); 3] = [
    (AssetType::Logo, "logos"),
    (AssetType::Icon, "icons"),
    (AssetType::WallpaperArtwork, "heroes"),
];

/// Reads the API key from this machine's credential store at each request, so a key stored or
/// cleared meanwhile takes effect at once. Images are downloaded from their public location,
/// without the key.
pub struct SteamGridDbConnector<T = ReqwestHttpTransport> {
    transport: T,
    credentials: Arc<dyn CredentialStorePort>,
}

impl SteamGridDbConnector<ReqwestHttpTransport> {
    pub fn new(credentials: Arc<dyn CredentialStorePort>) -> Self {
        Self::with_transport(ReqwestHttpTransport::default(), credentials)
    }
}

impl<T> SteamGridDbConnector<T> {
    pub fn with_transport(transport: T, credentials: Arc<dyn CredentialStorePort>) -> Self {
        Self {
            transport,
            credentials,
        }
    }

    fn api_key(&self) -> Result<Option<ApiKey>, PortError> {
        self.credentials.api_key(STEAMGRIDDB_SOURCE_ID)
    }
}

impl<T> ConnectorPort for SteamGridDbConnector<T>
where
    T: HttpTransport,
{
    fn source_id(&self) -> &'static str {
        STEAMGRIDDB_SOURCE_ID
    }

    fn needs_api_key(&self) -> bool {
        true
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: MEDIA.iter().map(|(asset_type, _)| *asset_type).collect(),
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
                    "SteamGridDB needs an API key: store one with `game-media-vault source key set steamgriddb`"
                        .to_owned(),
                ));
            }
            // A credential store that cannot be read leaves this Source out, not the whole plan.
            Err(error) => {
                return Ok(Some(format!(
                    "SteamGridDB's API key could not be read: {}",
                    error.message()
                )));
            }
        }
        if matches!(request.games(), GameSelection::All) {
            return Ok(Some(
                "SteamGridDB needs an explicit game selection, since it lists no platform's games"
                    .to_owned(),
            ));
        }
        if !request.regions().is_empty() {
            return Ok(Some(
                "SteamGridDB cannot satisfy region filters because its media record no region"
                    .to_owned(),
            ));
        }
        if !request.languages().is_empty() {
            return Ok(Some(
                "SteamGridDB cannot satisfy language filters yet".to_owned(),
            ));
        }
        Ok(None)
    }

    /// Games are looked up one by one, so a request for many is discovered a batch at a time,
    /// each recorded as it completes.
    fn discovery_batch_size(&self) -> Option<usize> {
        Some(DISCOVERY_BATCH)
    }

    /// Looks each requested game up by name, keeping only the game SteamGridDB names exactly
    /// so, then lists its media of the requested types, best voted first.
    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        if let Some(reason) = self.unsupported_request_reason(request)? {
            return Err(PortError::new(reason));
        }
        let key = self.api_key()?.ok_or_else(|| {
            PortError::new("SteamGridDB needs an API key that this machine no longer stores".into())
        })?;
        let media: Vec<(AssetType, &str)> = MEDIA
            .iter()
            .copied()
            .filter(|(asset_type, _)| request.requests_asset_type(*asset_type))
            .collect();
        let mut games: HashMap<String, Option<u64>> = HashMap::new();
        let mut candidates = Vec::new();
        for (title, platform) in wanted_games(request) {
            let game_id = match games.get(&title) {
                Some(game_id) => *game_id,
                None => {
                    let game_id = self.game_named(&title, &key)?;
                    games.insert(title.clone(), game_id);
                    game_id
                }
            };
            let Some(game_id) = game_id else {
                continue;
            };
            for (asset_type, collection) in &media {
                let answer = self.get(&[collection, "game", &game_id.to_string()], &key)?;
                for item in answer.as_array().into_iter().flatten() {
                    if let Some(candidate) =
                        candidate(item, &title, &platform, *asset_type, collection)
                    {
                        candidates.push(candidate);
                    }
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

impl<T> SteamGridDbConnector<T>
where
    T: HttpTransport,
{
    /// The id of the game SteamGridDB names exactly `title`, regardless of case and punctuation.
    fn game_named(&self, title: &str, key: &ApiKey) -> Result<Option<u64>, PortError> {
        let answer = self.get(&["search", "autocomplete", title], key)?;
        let wanted = name_key(title);
        Ok(answer.as_array().into_iter().flatten().find_map(|game| {
            let name = game.get("name")?.as_str()?;
            (name_key(name) == wanted)
                .then(|| game.get("id")?.as_u64())
                .flatten()
        }))
    }

    /// The `data` of a successful API answer at the path `segments` name.
    fn get(&self, segments: &[&str], key: &ApiKey) -> Result<Value, PortError> {
        let mut url = Url::parse(API).expect("the API location is a valid URL");
        url.path_segments_mut()
            .expect("the API location has a path")
            .extend(segments);
        let body = self.transport.get_authorized(url.as_str(), key)?;
        let answer: Value = serde_json::from_slice(&body).map_err(|error| {
            PortError::invalid_source_data(format!(
                "SteamGridDB answered {url} with no JSON: {error}"
            ))
        })?;
        if answer.get("success").and_then(Value::as_bool) != Some(true) {
            let errors = answer
                .get("errors")
                .and_then(Value::as_array)
                .map(|errors| {
                    errors
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .unwrap_or_default();
            return Err(PortError::new(format!(
                "SteamGridDB refused {url}: {errors}"
            )));
        }
        Ok(answer.get("data").cloned().unwrap_or(Value::Null))
    }
}

/// The candidate a media item of the API describes, unless it lacks what a candidate needs.
fn candidate(
    item: &Value,
    title: &str,
    platform: &str,
    asset_type: AssetType,
    collection: &str,
) -> Option<AssetCandidate> {
    let id = item.get("id")?.as_u64()?;
    let location = Url::parse(item.get("url")?.as_str()?).ok()?;
    if location.scheme() != "https" {
        return None;
    }
    let original_filename = location.path_segments()?.next_back()?.to_owned();
    if original_filename.is_empty() {
        return None;
    }
    let label = match item.get("style").and_then(Value::as_str) {
        Some(style) => format!("{collection}: {style}"),
        None => collection.to_owned(),
    };
    Some(AssetCandidate {
        // One image applies to every requested platform, each its own candidate: its id names the
        // platform, which candidate identity otherwise lacks.
        provider_candidate_id: Some(format!("{}/{collection}/{id}", platform_key(platform))),
        game_title: title.to_owned(),
        platform: platform.to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type,
        source_id: SourceId::from(STEAMGRIDDB_SOURCE_ID),
        source_asset_label: Some(label),
        source_url: location.to_string(),
        original_filename,
    })
}
