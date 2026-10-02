use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
    sync::OnceLock,
};

use game_media_vault_application::{ConnectorPort, PortError, ReferenceCatalogSourcePort};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection,
    ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField, SourceId,
};
use quick_xml::{Reader, escape::resolve_xml_entity, events::Event};

mod naming;

use naming::{parse_release_name, platform_name};
use reqwest::blocking::Client;
use url::Url;

pub const LIBRETRO_THUMBNAILS_SOURCE_ID: &str = "libretro-thumbnails";
pub const NO_INTRO_SOURCE_ID: &str = "no-intro";
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
            .map_err(|error| PortError::new(format!("Libretro download failed: {error}")))?;
        if !response.status().is_success() {
            return Err(PortError::new(format!(
                "Libretro download returned HTTP {} for {url}",
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
        if max_games == 0 {
            return Ok(Vec::new());
        }
        let file = File::open(source_path).map_err(|error| {
            PortError::new(format!(
                "failed to open No-Intro datafile {}: {error}",
                source_path.display()
            ))
        })?;
        let source_location = source_path.to_string_lossy().into_owned();
        parse_no_intro_datafile(BufReader::new(file), &source_location, max_games)
    }
}

fn parse_no_intro_datafile<R: std::io::BufRead>(
    reader: R,
    source_location: &str,
    max_games: usize,
) -> Result<Vec<ReferenceReleaseRecord>, PortError> {
    let mut xml = Reader::from_reader(reader);
    let mut buffer = Vec::new();
    let mut platform = None;
    let mut header_name = String::new();
    let mut in_header = false;
    let mut reading_header_name = false;
    let mut current_game = None;
    let mut releases = Vec::with_capacity(max_games.min(256));

    loop {
        match xml.read_event_into(&mut buffer).map_err(xml_error)? {
            Event::Start(element) => match element.name().as_ref() {
                "header" => in_header = true,
                "name" if in_header => {
                    reading_header_name = true;
                    header_name.clear();
                }
                "game" => {
                    let raw_name = attribute_value(&element, "name")?.ok_or_else(|| {
                        PortError::invalid_source_data(
                            "No-Intro game entry is missing its name".to_owned(),
                        )
                    })?;
                    current_game = Some(NoIntroGame::new(raw_name, source_location));
                }
                "rom" => {
                    if let Some(game) = current_game.as_mut() {
                        game.read_identifiers(&element)?;
                    }
                }
                _ => {}
            },
            Event::Empty(element) if element.name().as_ref() == "rom" => {
                if let Some(game) = current_game.as_mut() {
                    game.read_identifiers(&element)?;
                }
            }
            Event::Text(text) if reading_header_name => {
                header_name.push_str(text.xml10_content().as_ref());
            }
            Event::GeneralRef(reference) if reading_header_name => {
                if let Some(character) = reference.resolve_char_ref().map_err(|error| {
                    PortError::invalid_source_data(format!(
                        "invalid No-Intro XML character reference: {error}"
                    ))
                })? {
                    header_name.push(character);
                } else if let Some(value) = resolve_xml_entity(reference.as_ref()) {
                    header_name.push_str(value);
                } else {
                    return Err(PortError::invalid_source_data(format!(
                        "unsupported No-Intro XML entity reference: &{};",
                        reference.as_ref()
                    )));
                }
            }
            Event::End(element) => match element.name().as_ref() {
                "name" if reading_header_name => {
                    reading_header_name = false;
                    platform = Some(platform_name(&header_name));
                }
                "header" => in_header = false,
                "game" => {
                    let game = current_game.take().ok_or_else(|| {
                        PortError::invalid_source_data(
                            "No-Intro game closing tag has no matching entry".to_owned(),
                        )
                    })?;
                    let platform = platform.as_deref().ok_or_else(|| {
                        PortError::invalid_source_data(
                            "No-Intro datafile header is missing a platform name".to_owned(),
                        )
                    })?;
                    releases.push(game.finish(platform));
                    if releases.len() >= max_games {
                        break;
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    if platform.is_none() {
        return Err(PortError::invalid_source_data(
            "No-Intro datafile header is missing a platform name".to_owned(),
        ));
    }
    Ok(releases)
}

struct NoIntroGame {
    raw_name: String,
    source_location: String,
    identifiers: Vec<(String, String)>,
}

impl NoIntroGame {
    fn new(raw_name: String, source_location: &str) -> Self {
        Self {
            raw_name,
            source_location: source_location.to_owned(),
            identifiers: Vec::new(),
        }
    }

    fn read_identifiers(
        &mut self,
        element: &quick_xml::events::BytesStart<'_>,
    ) -> Result<(), PortError> {
        for name in ["name", "crc", "md5", "sha1", "sha256"] {
            if let Some(value) = attribute_value(element, name)? {
                let qualifier = if name == "name" {
                    "rom_name".to_owned()
                } else {
                    name.to_owned()
                };
                self.identifiers.push((qualifier, value));
            }
        }
        Ok(())
    }

    fn finish(self, platform: &str) -> ReferenceReleaseRecord {
        let title = parse_release_name(&self.raw_name);
        let mut assertions = Vec::with_capacity(4 + self.identifiers.len());
        assertions.push(assertion(
            &self.source_location,
            ReleaseAssertionField::Title,
            None,
            &title.game_title,
        ));
        assertions.push(assertion(
            &self.source_location,
            ReleaseAssertionField::Identifier,
            Some("source_record"),
            &source_record_identifier(platform, &self.raw_name),
        ));
        if title.region != "Unknown" {
            assertions.push(assertion(
                &self.source_location,
                ReleaseAssertionField::Region,
                None,
                &title.region,
            ));
        }
        if let Some(revision) = title.revision.as_deref() {
            assertions.push(assertion(
                &self.source_location,
                ReleaseAssertionField::Revision,
                None,
                revision,
            ));
        }
        assertions.extend(self.identifiers.into_iter().map(|(qualifier, value)| {
            assertion(
                &self.source_location,
                ReleaseAssertionField::Identifier,
                Some(&qualifier),
                &value,
            )
        }));

        ReferenceReleaseRecord {
            game_title: title.game_title,
            platform: platform.to_owned(),
            region: title.region,
            revision: title.revision,
            edition_name: title.edition_name,
            assertions,
        }
    }
}

fn assertion(
    source_location: &str,
    field: ReleaseAssertionField,
    qualifier: Option<&str>,
    value: &str,
) -> ReleaseAssertion {
    ReleaseAssertion {
        source_id: SourceId::from(NO_INTRO_SOURCE_ID),
        source_location: source_location.to_owned(),
        field,
        qualifier: qualifier.map(str::to_owned),
        value: value.to_owned(),
    }
}

fn attribute_value(
    element: &quick_xml::events::BytesStart<'_>,
    name: &str,
) -> Result<Option<String>, PortError> {
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| {
            PortError::invalid_source_data(format!("invalid No-Intro XML attribute: {error}"))
        })?;
        if attribute.key.as_ref() == name {
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| {
                    PortError::invalid_source_data(format!(
                        "invalid No-Intro XML attribute value: {error}"
                    ))
                })?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn source_record_identifier(platform: &str, raw_name: &str) -> String {
    format!("{}:{platform}{raw_name}", platform.len())
}

/// A No-Intro datafile that cannot be read is an environmental failure; one that does not
/// parse is invalid source data.
fn xml_error(error: quick_xml::Error) -> PortError {
    match error {
        quick_xml::Error::Io(error) => {
            PortError::new(format!("failed to read No-Intro datafile: {error}"))
        }
        error => PortError::invalid_source_data(format!("invalid No-Intro XML: {error}")),
    }
}
