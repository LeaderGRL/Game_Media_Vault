use std::io::Read;

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection, SourceId,
};
use reqwest::blocking::Client;
use url::Url;

pub const LIBRETRO_THUMBNAILS_SOURCE_ID: &str = "libretro-thumbnails";
const LIBRETRO_GITMODULES_URL: &str =
    "https://raw.githubusercontent.com/libretro-thumbnails/libretro-thumbnails/master/.gitmodules";

pub trait HttpTransport {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError>;

    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, PortError> {
        let mut stream = self.get_stream(url)?;
        let mut bytes = Vec::new();
        stream
            .read_to_end(&mut bytes)
            .map_err(|error| PortError(format!("failed to read HTTP response body: {error}")))?;
        Ok(bytes)
    }
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
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
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
        Ok(Box::new(response))
    }
}

pub struct LibretroThumbnailsConnector<T = ReqwestHttpTransport> {
    transport: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LibretroRepository {
    platform: String,
    repository: String,
    branch: String,
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
        if !request.regions().is_empty() {
            return Err(PortError(
                "Libretro Thumbnails cannot satisfy region filters because the source provides no region evidence"
                    .to_owned(),
            ));
        }
        if !request.languages().is_empty() {
            return Err(PortError(
                "Libretro Thumbnails cannot satisfy language filters because the source provides no language evidence"
                    .to_owned(),
            ));
        }

        let repositories = self.repository_catalog()?;
        let targets = acquisition_targets(request)?;
        targets
            .into_iter()
            .map(|(platform, game_title)| {
                let original_filename = thumbnail_filename(&game_title);
                let repository = repositories
                    .iter()
                    .find(|repository| repository.platform == platform)
                    .ok_or_else(|| {
                        PortError(format!(
                            "Libretro Thumbnails does not declare a repository for platform {platform}"
                        ))
                    })?;
                let source_url = box_front_url(repository, &original_filename)?;
                Ok(AssetCandidate {
                    game_title,
                    platform,
                    region: "Unknown".to_owned(),
                    edition_name: "Unspecified".to_owned(),
                    asset_type: AssetType::BoxFront,
                    source_id: SourceId::from(LIBRETRO_THUMBNAILS_SOURCE_ID),
                    source_asset_label: Some("Named_Boxarts".to_owned()),
                    source_url,
                    original_filename,
                })
            })
            .collect()
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        self.transport.get_stream(&candidate.source_url)
    }
}

impl<T> LibretroThumbnailsConnector<T>
where
    T: HttpTransport,
{
    fn repository_catalog(&self) -> Result<Vec<LibretroRepository>, PortError> {
        let bytes = self.transport.get_bytes(LIBRETRO_GITMODULES_URL)?;
        let manifest = std::str::from_utf8(&bytes)
            .map_err(|error| PortError(format!("invalid Libretro repository metadata: {error}")))?;
        parse_repository_catalog(manifest)
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
            .filter(|game| {
                request.platforms().is_empty()
                    || request
                        .platforms()
                        .iter()
                        .any(|platform| platform == &game.platform)
            })
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
                '&' | '*' | '/' | ':' | '"' | '<' | '>' | '?' | '\\' | '|' | '`'
            ) {
                '_'
            } else {
                character
            }
        })
        .collect();
    format!("{sanitized}.png")
}

fn box_front_url(repository: &LibretroRepository, filename: &str) -> Result<String, PortError> {
    let mut url = Url::parse("https://raw.githubusercontent.com/")
        .map_err(|error| PortError(format!("invalid Libretro base URL: {error}")))?;
    url.path_segments_mut()
        .map_err(|_| PortError("Libretro base URL cannot contain path segments".to_owned()))?
        .extend([
            "libretro-thumbnails",
            repository.repository.as_str(),
            repository.branch.as_str(),
            "Named_Boxarts",
            filename,
        ]);
    Ok(url.into())
}

fn parse_repository_catalog(manifest: &str) -> Result<Vec<LibretroRepository>, PortError> {
    let mut repositories = Vec::new();
    let mut platform = None;
    let mut repository = None;
    let mut branch = None;

    for line in manifest.lines().map(str::trim) {
        if line.starts_with("[submodule ") {
            push_repository(
                &mut repositories,
                &mut platform,
                &mut repository,
                &mut branch,
            )?;
            continue;
        }
        if let Some(value) = line.strip_prefix("path = ") {
            platform = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("url = ") {
            repository = Some(repository_name(value)?);
        } else if let Some(value) = line.strip_prefix("branch = ") {
            branch = Some(value.to_owned());
        }
    }

    push_repository(
        &mut repositories,
        &mut platform,
        &mut repository,
        &mut branch,
    )?;
    if repositories.is_empty() {
        return Err(PortError(
            "Libretro repository metadata did not contain any usable repositories".to_owned(),
        ));
    }
    Ok(repositories)
}

fn push_repository(
    repositories: &mut Vec<LibretroRepository>,
    platform: &mut Option<String>,
    repository: &mut Option<String>,
    branch: &mut Option<String>,
) -> Result<(), PortError> {
    let Some(platform_value) = platform.take() else {
        repository.take();
        branch.take();
        return Ok(());
    };
    let repository_value = repository.take().ok_or_else(|| {
        PortError(format!(
            "Libretro repository metadata is missing a URL for platform {platform_value}"
        ))
    })?;
    repositories.push(LibretroRepository {
        platform: platform_value,
        repository: repository_value,
        branch: branch.take().unwrap_or_else(|| "master".to_owned()),
    });
    Ok(())
}

fn repository_name(url: &str) -> Result<String, PortError> {
    let repository = url
        .trim_end_matches(".git")
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| PortError(format!("invalid Libretro repository URL: {url}")))?;
    Ok(repository.to_owned())
}
