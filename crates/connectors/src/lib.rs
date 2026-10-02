use std::{io::Read, path::Path, sync::OnceLock};

use game_media_vault_application::{ConnectorPort, PortError, ReferenceCatalogSourcePort};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection,
    ReferenceReleaseRecord, SourceId,
};

mod datafile;
mod launchbox;
mod naming;
mod xml;

pub use launchbox::{
    LAUNCHBOX_GAMES_DB_SOURCE_ID, LAUNCHBOX_METADATA_URL, LaunchBoxGamesDbConnector,
};

use datafile::{DatafileSource, read_datafile};
use naming::parse_release_name;
use reqwest::blocking::Client;
use url::Url;

pub const LIBRETRO_THUMBNAILS_SOURCE_ID: &str = "libretro-thumbnails";
pub const NO_INTRO_SOURCE_ID: &str = "no-intro";
pub const REDUMP_SOURCE_ID: &str = "redump";
/// The thumbnail folders of every Libretro repository and the Asset Type each holds.
const THUMBNAIL_FOLDERS: [(AssetType, &str); 3] = [
    (AssetType::BoxFront, "Named_Boxarts"),
    (AssetType::Screenshot, "Named_Snaps"),
    (AssetType::TitleScreen, "Named_Titles"),
];
const LIBRETRO_GITMODULES_URL: &str =
    "https://raw.githubusercontent.com/libretro-thumbnails/libretro-thumbnails/master/.gitmodules";

pub trait HttpTransport {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError>;

    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, PortError> {
        let mut stream = self.get_stream(url)?;
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).map_err(|error| {
            PortError::new(format!("failed to read HTTP response body: {error}"))
        })?;
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
                .expect("failed to build the HTTP client"),
        }
    }
}

impl HttpTransport for ReqwestHttpTransport {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|error| PortError::new(format!("download of {url} failed: {error}")))?;
        if !response.status().is_success() {
            return Err(PortError::new(format!(
                "download returned HTTP {} for {url}",
                response.status()
            )));
        }
        Ok(Box::new(response))
    }
}

pub struct LibretroThumbnailsConnector<T = ReqwestHttpTransport> {
    transport: T,
    /// Repository catalog read once per connector, shared by plan checks and discovery.
    repositories: OnceLock<Vec<LibretroRepository>>,
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
        Self {
            transport,
            repositories: OnceLock::new(),
        }
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
            asset_types: THUMBNAIL_FOLDERS
                .iter()
                .map(|(asset_type, _)| *asset_type)
                .collect(),
            direct_media_download: true,
        }
    }

    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        if let Some(reason) = unsupported_selection_reason(request) {
            return Ok(Some(reason.to_owned()));
        }
        // Discovery needs a repository for every targeted platform; a run whose platforms
        // Libretro does not declare could never execute.
        let repositories = self.repository_catalog()?;
        Ok(acquisition_targets(request)?
            .into_iter()
            .map(|(platform, _)| platform)
            .find(|platform| {
                !repositories
                    .iter()
                    .any(|repository| &repository.platform == platform)
            })
            .map(|platform| {
                format!("Libretro Thumbnails does not declare a repository for platform {platform}")
            }))
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        let folders: Vec<(AssetType, &str)> = THUMBNAIL_FOLDERS
            .iter()
            .copied()
            .filter(|(asset_type, _)| request.requests_asset_type(*asset_type))
            .collect();
        if folders.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(reason) = unsupported_selection_reason(request) {
            return Err(PortError::new(reason.to_owned()));
        }

        let repositories = self.repository_catalog()?;
        let mut candidates = Vec::new();
        for (platform, game_title) in acquisition_targets(request)? {
            let original_filename = thumbnail_filename(&game_title);
            let repository = repositories
                .iter()
                .find(|repository| repository.platform == platform)
                .ok_or_else(|| {
                    PortError::new(format!(
                        "Libretro Thumbnails does not declare a repository for platform {platform}"
                    ))
                })?;
            // Libretro names thumbnails after No-Intro/Redump release names, which encode the
            // region and edition of the release.
            let release = parse_release_name(&game_title);
            for (asset_type, folder) in &folders {
                candidates.push(AssetCandidate {
                    provider_candidate_id: Some(format!(
                        "{}/{folder}/{game_title}",
                        repository.repository
                    )),
                    game_title: release.game_title.clone(),
                    platform: platform.clone(),
                    region: release.region.clone(),
                    edition_name: release.edition_name.clone(),
                    asset_type: *asset_type,
                    source_id: SourceId::from(LIBRETRO_THUMBNAILS_SOURCE_ID),
                    source_asset_label: Some((*folder).to_owned()),
                    source_url: thumbnail_url(repository, folder, &original_filename)?,
                    original_filename: original_filename.clone(),
                });
            }
        }
        Ok(candidates)
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        self.transport.get_stream(&candidate.source_url)
    }
}

impl<T> LibretroThumbnailsConnector<T>
where
    T: HttpTransport,
{
    fn repository_catalog(&self) -> Result<&[LibretroRepository], PortError> {
        if let Some(repositories) = self.repositories.get() {
            return Ok(repositories);
        }
        let bytes = self.transport.get_bytes(LIBRETRO_GITMODULES_URL)?;
        let manifest = std::str::from_utf8(&bytes).map_err(|error| {
            PortError::invalid_source_data(format!("invalid Libretro repository metadata: {error}"))
        })?;
        let repositories = parse_repository_catalog(manifest)?;
        Ok(self.repositories.get_or_init(|| repositories))
    }
}

/// Selections Libretro cannot satisfy whatever its repositories hold.
fn unsupported_selection_reason(request: &AcquisitionRequest) -> Option<&'static str> {
    if matches!(request.games(), GameSelection::All) {
        Some("Libretro Thumbnails requires an explicit bounded game selection")
    } else if !request.regions().is_empty() {
        Some(
            "Libretro Thumbnails cannot satisfy region filters because the source provides no region evidence",
        )
    } else if !request.languages().is_empty() {
        Some(
            "Libretro Thumbnails cannot satisfy language filters because the source provides no language evidence",
        )
    } else {
        None
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
        GameSelection::All => Err(PortError::new(
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

fn thumbnail_url(
    repository: &LibretroRepository,
    folder: &str,
    filename: &str,
) -> Result<String, PortError> {
    let mut url = Url::parse("https://raw.githubusercontent.com/")
        .map_err(|error| PortError::new(format!("invalid Libretro base URL: {error}")))?;
    url.path_segments_mut()
        .map_err(|_| PortError::new("Libretro base URL cannot contain path segments".to_owned()))?
        .extend([
            "libretro-thumbnails",
            repository.repository.as_str(),
            repository.branch.as_str(),
            folder,
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
        return Err(PortError::invalid_source_data(
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
        PortError::invalid_source_data(format!(
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
        .ok_or_else(|| {
            PortError::invalid_source_data(format!("invalid Libretro repository URL: {url}"))
        })?;
    Ok(repository.to_owned())
}

const NO_INTRO: DatafileSource = DatafileSource {
    id: NO_INTRO_SOURCE_ID,
    name: "No-Intro",
};
const REDUMP: DatafileSource = DatafileSource {
    id: REDUMP_SOURCE_ID,
    name: "Redump",
};

/// No-Intro datafiles, which catalog cartridge and other ROM-based releases.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoIntroReferenceCatalog;

impl NoIntroReferenceCatalog {
    pub fn new() -> Self {
        Self
    }
}

impl ReferenceCatalogSourcePort for NoIntroReferenceCatalog {
    fn read_releases(
        &self,
        source_path: &Path,
        max_games: usize,
    ) -> Result<Vec<ReferenceReleaseRecord>, PortError> {
        read_datafile(&NO_INTRO, source_path, max_games)
    }
}

/// Redump datafiles, which catalog disc releases by their tracks.
#[derive(Debug, Default, Clone, Copy)]
pub struct RedumpReferenceCatalog;

impl RedumpReferenceCatalog {
    pub fn new() -> Self {
        Self
    }
}

impl ReferenceCatalogSourcePort for RedumpReferenceCatalog {
    fn read_releases(
        &self,
        source_path: &Path,
        max_games: usize,
    ) -> Result<Vec<ReferenceReleaseRecord>, PortError> {
        read_datafile(&REDUMP, source_path, max_games)
    }
}
