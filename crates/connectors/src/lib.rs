use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use game_media_vault_application::{ConnectorPort, PortError, ReferenceCatalogSourcePort};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection,
    ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField, SourceId,
};
use quick_xml::{Reader, events::Event};
use reqwest::blocking::Client;
use url::Url;

pub const LIBRETRO_THUMBNAILS_SOURCE_ID: &str = "libretro-thumbnails";
pub const NO_INTRO_SOURCE_ID: &str = "no-intro";
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
            PortError(format!(
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
    xml.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut platform = None;
    let mut in_header = false;
    let mut reading_header_name = false;
    let mut current_game = None;
    let mut releases = Vec::with_capacity(max_games.min(256));

    loop {
        match xml
            .read_event_into(&mut buffer)
            .map_err(|error| PortError(format!("invalid No-Intro XML: {error}")))?
        {
            Event::Start(element) => match element.name().as_ref() {
                "header" => in_header = true,
                "name" if in_header => reading_header_name = true,
                "game" => {
                    let raw_name = attribute_value(&element, "name")?.ok_or_else(|| {
                        PortError("No-Intro game entry is missing its name".to_owned())
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
                let value = text.xml_content(quick_xml::XmlVersion::Implicit1_0);
                platform = Some(value.into_owned());
            }
            Event::End(element) => match element.name().as_ref() {
                "name" if reading_header_name => reading_header_name = false,
                "header" => in_header = false,
                "game" => {
                    let game = current_game.take().ok_or_else(|| {
                        PortError("No-Intro game closing tag has no matching entry".to_owned())
                    })?;
                    let platform = platform.as_deref().ok_or_else(|| {
                        PortError("No-Intro datafile header is missing a platform name".to_owned())
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
        return Err(PortError(
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
        let title = parse_no_intro_title(&self.raw_name);
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

struct ParsedNoIntroTitle {
    game_title: String,
    region: String,
    revision: Option<String>,
    edition_name: String,
}

fn parse_no_intro_title(raw: &str) -> ParsedNoIntroTitle {
    let (game_title, tags) = split_trailing_tags(raw);
    let revision = tags.iter().find(|tag| is_revision_tag(tag)).cloned();
    let region = tags
        .iter()
        .find(|tag| is_region_tag(tag))
        .cloned()
        .unwrap_or_else(|| "Unknown".to_owned());
    let edition_tags = tags
        .iter()
        .filter(|tag| !is_region_tag(tag))
        .cloned()
        .collect::<Vec<_>>();
    let edition_name = if edition_tags.is_empty() {
        "Standard".to_owned()
    } else {
        edition_tags.join(" · ")
    };

    ParsedNoIntroTitle {
        game_title,
        region,
        revision,
        edition_name,
    }
}

fn split_trailing_tags(raw: &str) -> (String, Vec<String>) {
    let mut base = raw.trim_end();
    let mut tags = Vec::new();
    while base.ends_with(')') {
        let Some(open_index) = base.rfind(" (") else {
            break;
        };
        let tag = &base[open_index + 2..base.len() - 1];
        if tag.is_empty() {
            break;
        }
        tags.push(tag.to_owned());
        base = base[..open_index].trim_end();
    }
    tags.reverse();
    (base.to_owned(), tags)
}

fn source_record_identifier(platform: &str, raw_name: &str) -> String {
    format!("{}:{platform}{raw_name}", platform.len())
}

fn is_revision_tag(tag: &str) -> bool {
    tag.starts_with("Rev ") || tag.starts_with("Revision ")
}

fn is_region_tag(tag: &str) -> bool {
    tag.split(',').map(str::trim).all(|part| {
        matches!(
            part,
            "Albania"
                | "Argentina"
                | "Asia"
                | "Australia"
                | "Austria"
                | "Belgium"
                | "Benelux"
                | "Bosnia and Herzegovina"
                | "Brazil"
                | "Bulgaria"
                | "Canada"
                | "Chile"
                | "China"
                | "Croatia"
                | "Cyprus"
                | "Czech"
                | "Czech Republic"
                | "Denmark"
                | "Egypt"
                | "Estonia"
                | "Europe"
                | "Finland"
                | "France"
                | "Germany"
                | "Greece"
                | "Hong Kong"
                | "Hungary"
                | "Iceland"
                | "India"
                | "Indonesia"
                | "Iran"
                | "Ireland"
                | "Israel"
                | "Italy"
                | "Japan"
                | "Jordan"
                | "Korea"
                | "Latin America"
                | "Latvia"
                | "Lithuania"
                | "Luxembourg"
                | "Macedonia"
                | "Malaysia"
                | "Mexico"
                | "Middle East"
                | "Mongolia"
                | "Nepal"
                | "Netherlands"
                | "New Zealand"
                | "North America"
                | "Norway"
                | "Oman"
                | "Peru"
                | "Philippines"
                | "Poland"
                | "Portugal"
                | "Qatar"
                | "Romania"
                | "Russia"
                | "Saudi Arabia"
                | "Scandinavia"
                | "Serbia"
                | "Serbia and Montenegro"
                | "Singapore"
                | "Slovakia"
                | "Slovenia"
                | "South Africa"
                | "South America"
                | "South East Asia"
                | "South Korea"
                | "Spain"
                | "Sweden"
                | "Switzerland"
                | "Taiwan"
                | "Thailand"
                | "Turkey"
                | "UK"
                | "Ukraine"
                | "United Arab Emirates"
                | "United Kingdom"
                | "USA"
                | "Vietnam"
                | "World"
                | "Yugoslavia"
        )
    })
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
        let attribute = attribute
            .map_err(|error| PortError(format!("invalid No-Intro XML attribute: {error}")))?;
        if attribute.key.as_ref() == name {
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| {
                    PortError(format!("invalid No-Intro XML attribute value: {error}"))
                })?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}
