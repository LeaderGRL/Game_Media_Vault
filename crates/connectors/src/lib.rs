use std::{
    io::Read,
    path::Path,
    sync::{Arc, OnceLock},
    thread,
    time::{Duration, SystemTime},
};

use game_media_vault_application::{
    ApiKey, ConnectorPort, CredentialStorePort, PortError, ReferenceCatalogRead,
    ReferenceCatalogSourcePort,
};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection, SourceId,
};

mod datafile;
mod launchbox;
mod mame;
mod naming;
mod resume;
mod retry;
mod steamgriddb;
mod xml;

pub use mame::{MAME_SOFTWARE_LISTS_SOURCE_ID, MameSoftwareListCatalog};
pub use retry::RetryPolicy;
pub use steamgriddb::{STEAMGRIDDB_SOURCE_ID, SteamGridDbConnector};

pub use launchbox::{
    DatasetCache, LAUNCHBOX_GAMES_DB_SOURCE_ID, LAUNCHBOX_METADATA_URL, LaunchBoxGamesDbConnector,
};

use datafile::{DatafileSource, read_datafile};
use naming::parse_release_name;
use reqwest::{
    StatusCode,
    blocking::{Client, Response},
    header::{ETAG, HeaderName, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, RETRY_AFTER},
};
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

/// The connector of every implemented Source, in the order `Auto` considers them. Those that
/// need an API key read it from this machine's `credentials`.
pub fn registered_connectors(
    credentials: Arc<dyn CredentialStorePort>,
) -> Vec<Box<dyn ConnectorPort>> {
    vec![
        Box::new(LibretroThumbnailsConnector::new()),
        Box::new(LaunchBoxGamesDbConnector::new()),
        Box::new(SteamGridDbConnector::new(credentials)),
    ]
}

/// What a server said identifies the version of a resource it served.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// What a conditional request gave.
pub enum Fetched {
    /// The resource has not changed since the version the request named.
    Unchanged,
    /// The resource, and the validators of the version served.
    Changed {
        body: Box<dyn Read + Send>,
        validators: Validators,
    },
}

/// Requests media and data from Sources; connectors share it across download threads.
pub trait HttpTransport: Send + Sync {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError>;

    /// Fetches `url` unless its server says it has not changed since the version `known`
    /// names. A transport that cannot ask fetches it whole.
    fn get_if_changed(&self, url: &str, known: &Validators) -> Result<Fetched, PortError> {
        let _ = known;
        Ok(Fetched::Changed {
            body: self.get_stream(url)?,
            validators: Validators::default(),
        })
    }

    /// Fetches `url` from an API that needs `api_key`, sent as a bearer token, and returns the
    /// body, which such APIs keep small. A transport that cannot send a key refuses rather than
    /// dropping it. Errors never show the key.
    fn get_authorized(&self, url: &str, api_key: &ApiKey) -> Result<Vec<u8>, PortError> {
        let _ = api_key;
        Err(PortError::new(format!(
            "this transport cannot send the API key {url} needs"
        )))
    }

    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, PortError> {
        let mut stream = self.get_stream(url)?;
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).map_err(|error| {
            PortError::new(format!("failed to read HTTP response body: {error}"))
        })?;
        Ok(bytes)
    }
}

/// The largest answer an API is trusted to send, which keeps a misbehaving one from filling memory.
const MAX_API_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

pub struct ReqwestHttpTransport {
    client: Client,
    retry: RetryPolicy,
}

impl Default for ReqwestHttpTransport {
    fn default() -> Self {
        Self::with_retry_policy(RetryPolicy::default())
    }
}

impl ReqwestHttpTransport {
    pub fn with_retry_policy(retry: RetryPolicy) -> Self {
        Self {
            client: Client::builder()
                .user_agent("game-media-vault/0.1")
                .build()
                .expect("failed to build the HTTP client"),
            retry,
        }
    }

    /// Requests `url` once, asking for it only if it changed since `known`. A success, or a 304
    /// Not Modified answer to a request naming a known version, is a response; anything else
    /// is a failure.
    fn attempt(
        &self,
        url: &str,
        known: &Validators,
        bearer: Option<&str>,
    ) -> Result<Response, FailedRequest> {
        let mut request = self.client.get(url);
        // The client drops it on a redirect to another host.
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        if let Some(etag) = &known.etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = &known.last_modified {
            request = request.header(IF_MODIFIED_SINCE, last_modified);
        }
        let response = request.send().map_err(|error| FailedRequest {
            message: format!("download of {url} failed: {error}"),
            // Only failures to reach the Source may pass; a request that cannot be built or
            // that redirects without end fails the same way every time.
            transient: error.is_connect() || error.is_timeout() || error.is_request(),
            unavailable: false,
            retry_after: None,
        })?;
        let status = response.status();
        let conditional = known.etag.is_some() || known.last_modified.is_some();
        if status.is_success() || (conditional && status == StatusCode::NOT_MODIFIED) {
            return Ok(response);
        }
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| retry::parse_retry_after(value, SystemTime::now()));
        Err(FailedRequest {
            message: format!("download returned HTTP {status} for {url}"),
            transient: status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error(),
            // The server no longer has the resource: asking again cannot help.
            unavailable: matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE),
            retry_after,
        })
    }
}

/// Why one request failed, and whether asking again may succeed.
struct FailedRequest {
    message: String,
    transient: bool,
    unavailable: bool,
    retry_after: Option<Duration>,
}

impl HttpTransport for ReqwestHttpTransport {
    fn get_authorized(&self, url: &str, api_key: &ApiKey) -> Result<Vec<u8>, PortError> {
        let (response, _) = self.send(url, &Validators::default(), Some(api_key.expose()))?;
        let mut body = Vec::new();
        response
            .take(MAX_API_RESPONSE_BYTES + 1)
            .read_to_end(&mut body)
            .map_err(|error| {
                PortError::new(format!("failed to read the answer of {url}: {error}"))
            })?;
        if body.len() as u64 > MAX_API_RESPONSE_BYTES {
            return Err(PortError::invalid_source_data(format!(
                "the answer of {url} exceeds {MAX_API_RESPONSE_BYTES} bytes"
            )));
        }
        Ok(body)
    }

    /// Retries transient failures (connection failures, HTTP 429 and 5xx) as the retry policy
    /// allows; any other refusal fails at once.
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        let (response, attempts) = self.send(url, &Validators::default(), None)?;
        Ok(self.body(response, attempts))
    }

    fn get_if_changed(&self, url: &str, known: &Validators) -> Result<Fetched, PortError> {
        let (response, attempts) = self.send(url, known, None)?;
        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(Fetched::Unchanged);
        }
        let header = |name: HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let validators = Validators {
            etag: header(ETAG),
            last_modified: header(LAST_MODIFIED),
        };
        Ok(Fetched::Changed {
            body: self.body(response, attempts),
            validators,
        })
    }
}

impl ReqwestHttpTransport {
    /// The body of `response`, resumed from where its connection drops when it can be, within
    /// the attempts the request left.
    fn body(&self, response: Response, attempts: u32) -> Box<dyn Read + Send> {
        Box::new(resume::ResumingBody::new(
            self.client.clone(),
            self.retry,
            attempts,
            response,
        ))
    }

    /// Requests `url` as the retry policy allows, with the attempts it took.
    fn send(
        &self,
        url: &str,
        known: &Validators,
        bearer: Option<&str>,
    ) -> Result<(Response, u32), PortError> {
        let mut attempts = 1;
        loop {
            match self.attempt(url, known, bearer) {
                Ok(response) => return Ok((response, attempts)),
                Err(failure) if failure.transient && attempts < self.retry.max_attempts => {
                    thread::sleep(self.retry.delay_before_retry(attempts, failure.retry_after));
                    attempts += 1;
                }
                Err(failure) if failure.unavailable => {
                    return Err(PortError::unavailable(failure.message));
                }
                Err(failure) if attempts > 1 => {
                    return Err(PortError::new(format!(
                        "{} (gave up after {attempts} attempts)",
                        failure.message
                    )));
                }
                Err(failure) => return Err(PortError::new(failure.message)),
            }
        }
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
    ) -> Result<ReferenceCatalogRead, PortError> {
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
    ) -> Result<ReferenceCatalogRead, PortError> {
        read_datafile(&REDUMP, source_path, max_games)
    }
}
