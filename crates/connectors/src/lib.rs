use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection, SourceKind,
};
use reqwest::blocking::Client;
use url::Url;

pub const LIBRETRO_THUMBNAILS_SOURCE_ID: &str = "libretro-thumbnails";

pub trait HttpTransport {
    fn get(&self, url: &str) -> Result<Vec<u8>, PortError>;
}

pub struct ReqwestHttpTransport {
    client: Client,
}

impl Default for ReqwestHttpTransport {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .user_agent("game-media-vault/0.1")
                .build()
                .expect("failed to build Libretro HTTP client"),
        }
    }
}

impl HttpTransport for ReqwestHttpTransport {
    fn get(&self, url: &str) -> Result<Vec<u8>, PortError> {
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|error| PortError(format!("Libretro download failed: {error}")))?;
        if !response.status().is_success() {
            return Err(PortError(format!(
                "Libretro download returned HTTP {} for {url}",
                response.status()
            )));
        }
        response
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(|error| PortError(format!("failed to read Libretro response body: {error}")))
    }
}

pub struct LibretroThumbnailsConnector<T = ReqwestHttpTransport> {
    transport: T,
}

impl LibretroThumbnailsConnector<ReqwestHttpTransport> {
    pub fn new() -> Self {
        Self::with_transport(ReqwestHttpTransport::default())
    }
}

impl Default for LibretroThumbnailsConnector<ReqwestHttpTransport> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LibretroThumbnailsConnector<T> {
    pub fn with_transport(transport: T) -> Self {
        Self { transport }
    }
}

impl<T> ConnectorPort for LibretroThumbnailsConnector<T>
where
    T: HttpTransport,
{
    fn source_id(&self) -> &'static str {
        LIBRETRO_THUMBNAILS_SOURCE_ID
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::BoxFront],
            direct_media_download: true,
        }
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        if !request.requests_asset_type(AssetType::BoxFront) {
            return Ok(Vec::new());
        }

        let region = if request.regions().len() == 1 {
            request.regions()[0].clone()
        } else {
            "Unknown".to_owned()
        };
        let targets = acquisition_targets(request)?;
        targets
            .into_iter()
            .map(|(platform, game_title)| {
                let original_filename = thumbnail_filename(&game_title);
                let source_url = box_front_url(&platform, &original_filename)?;
                Ok(AssetCandidate {
                    game_title,
                    platform,
                    region: region.clone(),
                    edition_name: "Unspecified".to_owned(),
                    asset_type: AssetType::BoxFront,
                    source_kind: SourceKind::LibretroThumbnails,
                    source_url,
                    original_filename,
                })
            })
            .collect()
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Vec<u8>, PortError> {
        self.transport.get(&candidate.source_url)
    }
}

fn acquisition_targets(request: &AcquisitionRequest) -> Result<Vec<(String, String)>, PortError> {
    match request.games() {
        GameSelection::Explicit(games) => Ok(request
            .platforms()
            .iter()
            .flat_map(|platform| {
                games
                    .iter()
                    .map(|game| (platform.clone(), game.clone()))
                    .collect::<Vec<_>>()
            })
            .collect()),
        GameSelection::PlatformBound(games) | GameSelection::QueryResult(games) => Ok(games
            .iter()
            .map(|game| (game.platform.clone(), game.game.clone()))
            .collect()),
        GameSelection::All => Err(PortError(
            "Libretro Thumbnails requires an explicit bounded game selection".to_owned(),
        )),
    }
}

fn thumbnail_filename(game_title: &str) -> String {
    let sanitized: String = game_title
        .chars()
        .map(|character| {
            if matches!(
                character,
                '&' | '*' | '/' | ':' | '"' | '<' | '>' | '?' | '\\' | '|'
            ) {
                '_'
            } else {
                character
            }
        })
        .collect();
    format!("{sanitized}.png")
}

fn box_front_url(platform: &str, filename: &str) -> Result<String, PortError> {
    let repository = platform.replace(' ', "_");
    let mut url = Url::parse("https://raw.githubusercontent.com/")
        .map_err(|error| PortError(format!("invalid Libretro base URL: {error}")))?;
    url.path_segments_mut()
        .map_err(|_| PortError("Libretro base URL cannot contain path segments".to_owned()))?
        .extend([
            "libretro-thumbnails",
            repository.as_str(),
            "master",
            "Named_Boxarts",
            filename,
        ]);
    Ok(url.into())
}
