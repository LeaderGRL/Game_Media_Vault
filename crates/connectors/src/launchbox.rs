//! LaunchBox Games Database: a downloadable dataset of game records and of the images its
//! community uploaded, which the LaunchBox image host serves.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, BufRead, BufReader, Seek},
};

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection, SourceId,
};
use quick_xml::{Reader, events::Event};
use url::Url;

use crate::{
    HttpTransport, ReqwestHttpTransport, naming::parse_release_name, xml::push_xml_reference,
};

pub const LAUNCHBOX_GAMES_DB_SOURCE_ID: &str = "launchbox-games-db";
/// The whole dataset, refreshed daily by LaunchBox.
pub const LAUNCHBOX_METADATA_URL: &str = "https://gamesdb.launchbox-app.com/Metadata.zip";
const IMAGE_BASE_URL: &str = "https://images.launchbox-app.com/";
const METADATA_ENTRY: &str = "Metadata.xml";

/// The image types acquired and the Asset Type each holds.
const IMAGE_TYPES: [(AssetType, &str); 16] = [
    (AssetType::BoxFront, "Box - Front"),
    (AssetType::BoxBack, "Box - Back"),
    (AssetType::Spine, "Box - Spine"),
    (AssetType::Box3dRender, "Box - 3D"),
    (AssetType::CartridgeFront, "Cart - Front"),
    (AssetType::CartridgeBack, "Cart - Back"),
    (AssetType::Disc, "Disc"),
    (AssetType::Pcb, "Arcade - Circuit Board"),
    (AssetType::Screenshot, "Screenshot - Gameplay"),
    (AssetType::TitleScreen, "Screenshot - Game Title"),
    (AssetType::Logo, "Clear Logo"),
    (AssetType::WallpaperArtwork, "Fanart - Background"),
    (AssetType::Flyer, "Advertisement Flyer - Front"),
    (AssetType::ArcadeCabinet, "Arcade - Cabinet"),
    (AssetType::ControlPanel, "Arcade - Control Panel"),
    (AssetType::Marquee, "Arcade - Marquee"),
];

/// Platforms as No-Intro, Redump and Libretro name them, and as LaunchBox does.
const PLATFORMS: &[(&str, &str)] = &[
    // MAME and FBNeo emulate the arcade boards LaunchBox files under one platform.
    ("FBNeo - Arcade Games", "Arcade"),
    ("MAME", "Arcade"),
    ("Atari - 2600", "Atari 2600"),
    ("Atari - 5200", "Atari 5200"),
    ("Atari - 7800", "Atari 7800"),
    ("Atari - Lynx", "Atari Lynx"),
    ("Microsoft - Xbox", "Microsoft Xbox"),
    ("Microsoft - Xbox 360", "Microsoft Xbox 360"),
    ("Microsoft - Xbox One", "Microsoft Xbox One"),
    ("NEC - PC Engine CD - TurboGrafx-CD", "NEC TurboGrafx-CD"),
    ("NEC - PC Engine - TurboGrafx 16", "NEC TurboGrafx-16"),
    ("Nintendo - Game Boy", "Nintendo Game Boy"),
    ("Nintendo - Game Boy Advance", "Nintendo Game Boy Advance"),
    ("Nintendo - Game Boy Color", "Nintendo Game Boy Color"),
    ("Nintendo - GameCube", "Nintendo GameCube"),
    ("Nintendo - Nintendo 3DS", "Nintendo 3DS"),
    ("Nintendo - Nintendo 64", "Nintendo 64"),
    ("Nintendo - Nintendo DS", "Nintendo DS"),
    ("Nintendo - Nintendo Switch", "Nintendo Switch"),
    (
        "Nintendo - Nintendo Entertainment System",
        "Nintendo Entertainment System",
    ),
    (
        "Nintendo - Super Nintendo Entertainment System",
        "Super Nintendo Entertainment System",
    ),
    ("Nintendo - Virtual Boy", "Nintendo Virtual Boy"),
    ("Nintendo - Wii", "Nintendo Wii"),
    ("Nintendo - Wii U", "Nintendo Wii U"),
    ("Philips - CD-i", "Philips CD-i"),
    ("Sega - 32X", "Sega 32X"),
    ("Sega - Dreamcast", "Sega Dreamcast"),
    ("Sega - Game Gear", "Sega Game Gear"),
    ("Sega - Master System - Mark III", "Sega Master System"),
    ("Sega - Mega Drive - Genesis", "Sega Genesis"),
    ("Sega - Mega-CD - Sega CD", "Sega CD"),
    ("Sega - Saturn", "Sega Saturn"),
    ("SNK - Neo Geo CD", "SNK Neo Geo CD"),
    ("SNK - Neo Geo Pocket Color", "SNK Neo Geo Pocket Color"),
    ("Sony - PlayStation", "Sony Playstation"),
    ("Sony - PlayStation 2", "Sony Playstation 2"),
    ("Sony - PlayStation 3", "Sony Playstation 3"),
    ("Sony - PlayStation 4", "Sony Playstation 4"),
    ("Sony - PlayStation Portable", "Sony PSP"),
    ("Sony - PlayStation Vita", "Sony Playstation Vita"),
    ("The 3DO Company - 3DO", "3DO Interactive Multiplayer"),
];

/// Regions LaunchBox names differently from No-Intro; the others keep their name.
const REGIONS: &[(&str, &str)] = &[
    ("North America", "USA"),
    ("United States", "USA"),
    ("United Kingdom", "UK"),
    ("The Netherlands", "Netherlands"),
];

pub struct LaunchBoxGamesDbConnector<T = ReqwestHttpTransport> {
    transport: T,
}

impl LaunchBoxGamesDbConnector<ReqwestHttpTransport> {
    pub fn new() -> Self {
        Self::with_transport(ReqwestHttpTransport::default())
    }
}

impl Default for LaunchBoxGamesDbConnector<ReqwestHttpTransport> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LaunchBoxGamesDbConnector<T> {
    pub fn with_transport(transport: T) -> Self {
        Self { transport }
    }
}

impl<T> ConnectorPort for LaunchBoxGamesDbConnector<T>
where
    T: HttpTransport,
{
    fn source_id(&self) -> &'static str {
        LAUNCHBOX_GAMES_DB_SOURCE_ID
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: IMAGE_TYPES
                .iter()
                .map(|(asset_type, _)| *asset_type)
                .collect(),
            direct_media_download: true,
        }
    }

    /// Checked against the platforms this connector knows, so planning never downloads the
    /// dataset.
    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        if !request.languages().is_empty() {
            return Ok(Some(
                "LaunchBox Games Database cannot satisfy language filters because its images record no language"
                    .to_owned(),
            ));
        }
        Ok(requested_platforms(request)
            .into_iter()
            .find(|platform| launchbox_platform(platform).is_none())
            .map(|platform| {
                format!("LaunchBox Games Database has no platform matching {platform}")
            }))
    }

    /// Reads the dataset twice, for the requested games and then for their images, so memory
    /// holds only what the request selects. Records missing what a candidate needs are skipped.
    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        let image_types: Vec<(AssetType, &str)> = IMAGE_TYPES
            .iter()
            .copied()
            .filter(|(asset_type, _)| request.requests_asset_type(*asset_type))
            .collect();
        if image_types.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(reason) = self.unsupported_request_reason(request)? {
            return Err(PortError::new(reason));
        }
        let wanted = WantedGames::of(request);
        let mut dataset = self.download_dataset()?;

        let mut games = HashMap::new();
        for_each_record(dataset.metadata()?, "Game", |fields| {
            let (Some(id), Some(name), Some(platform)) = (
                fields.get("DatabaseID"),
                fields.get("Name"),
                fields.get("Platform"),
            ) else {
                return;
            };
            if let Some(game) = wanted.game(name, platform) {
                games.insert(id.clone(), game);
            }
        })?;

        let regions: HashSet<String> = request
            .regions()
            .iter()
            .map(|region| region.trim().to_lowercase())
            .collect();
        let mut candidates = Vec::new();
        for_each_record(dataset.metadata()?, "GameImage", |fields| {
            let (Some(id), Some(file_name), Some(image_type)) = (
                fields.get("DatabaseID"),
                fields.get("FileName"),
                fields.get("Type"),
            ) else {
                return;
            };
            let Some(game) = games.get(id) else {
                return;
            };
            let Some((asset_type, _)) = image_types.iter().find(|(_, name)| name == image_type)
            else {
                return;
            };
            let region = fields
                .get("Region")
                .map(|region| no_intro_region(region))
                .unwrap_or_else(|| "Unknown".to_owned());
            if !regions.is_empty() && !regions.contains(&region.to_lowercase()) {
                return;
            }
            let Some(source_url) = image_url(file_name) else {
                return;
            };
            candidates.push(AssetCandidate {
                provider_candidate_id: Some(format!("{id}/{file_name}")),
                game_title: game.title.clone(),
                platform: game.platform.clone(),
                region,
                edition_name: "Unspecified".to_owned(),
                asset_type: *asset_type,
                source_id: SourceId::from(LAUNCHBOX_GAMES_DB_SOURCE_ID),
                source_asset_label: Some(image_type.clone()),
                source_url,
                original_filename: file_name.clone(),
            });
        })?;
        Ok(candidates)
    }

    fn download(
        &self,
        candidate: &AssetCandidate,
    ) -> Result<Box<dyn std::io::Read + Send>, PortError> {
        self.transport.get_stream(&candidate.source_url)
    }
}

impl<T> LaunchBoxGamesDbConnector<T>
where
    T: HttpTransport,
{
    /// Spools the dataset archive to a temporary file, since reading a ZIP needs to seek.
    fn download_dataset(&self) -> Result<Dataset, PortError> {
        let mut stream = self.transport.get_stream(LAUNCHBOX_METADATA_URL)?;
        let mut file = tempfile::tempfile().map_err(|error| {
            PortError::new(format!("failed to buffer the LaunchBox dataset: {error}"))
        })?;
        io::copy(&mut stream, &mut file).map_err(|error| {
            PortError::new(format!("failed to download the LaunchBox dataset: {error}"))
        })?;
        file.rewind().map_err(|error| {
            PortError::new(format!("failed to buffer the LaunchBox dataset: {error}"))
        })?;
        let archive = zip::ZipArchive::new(file).map_err(|error| {
            PortError::invalid_source_data(format!("unreadable LaunchBox dataset archive: {error}"))
        })?;
        Ok(Dataset { archive })
    }
}

struct Dataset {
    archive: zip::ZipArchive<File>,
}

impl Dataset {
    fn metadata(&mut self) -> Result<impl BufRead + '_, PortError> {
        let entry = self.archive.by_name(METADATA_ENTRY).map_err(|error| {
            PortError::invalid_source_data(format!(
                "the LaunchBox dataset has no readable {METADATA_ENTRY}: {error}"
            ))
        })?;
        Ok(BufReader::new(entry))
    }
}

/// A game record the request selects, titled as the request names it (as No-Intro does when
/// the request selects every game) and placed on the platform the request names.
struct WantedGame {
    title: String,
    platform: String,
}

/// The games a request selects on each LaunchBox platform, by LaunchBox platform name.
struct WantedGames {
    platforms: HashMap<&'static str, WantedPlatform>,
}

struct WantedPlatform {
    /// The platform as the request names it.
    requested: String,
    /// Requested titles by their key, or `None` for every game of the platform.
    titles: Option<HashMap<String, String>>,
}

impl WantedGames {
    fn of(request: &AcquisitionRequest) -> Self {
        let mut platforms: HashMap<&'static str, WantedPlatform> = HashMap::new();
        let mut want = |platform: &str, game: Option<&str>| {
            let Some(launchbox) = launchbox_platform(platform) else {
                return;
            };
            let wanted = platforms
                .entry(launchbox)
                .or_insert_with(|| WantedPlatform {
                    requested: platform.to_owned(),
                    titles: Some(HashMap::new()),
                });
            match (game, wanted.titles.as_mut()) {
                (None, _) => wanted.titles = None,
                (Some(game), Some(titles)) => {
                    // Requested releases carry No-Intro tags, which LaunchBox titles have not.
                    let title = parse_release_name(game).game_title;
                    titles.insert(title_key(&title), title);
                }
                (Some(_), None) => {}
            }
        };
        match request.games() {
            GameSelection::All => {
                for platform in request.platforms() {
                    want(platform, None);
                }
            }
            GameSelection::Explicit(games) => {
                for platform in request.platforms() {
                    for game in games {
                        want(platform, Some(game));
                    }
                }
            }
            GameSelection::PlatformBound(games) | GameSelection::QueryResult(games) => {
                // Requested platforms, when there are any, narrow the bound games.
                for game in games.iter().filter(|game| {
                    request.platforms().is_empty()
                        || request
                            .platforms()
                            .iter()
                            .any(|platform| platform == &game.platform)
                }) {
                    want(&game.platform, Some(&game.game));
                }
            }
        }
        Self { platforms }
    }

    fn game(&self, name: &str, platform: &str) -> Option<WantedGame> {
        let wanted = self.platforms.get(platform)?;
        let title = match &wanted.titles {
            None => no_intro_title(name),
            Some(titles) => titles.get(&title_key(name))?.clone(),
        };
        Some(WantedGame {
            title,
            platform: wanted.requested.clone(),
        })
    }
}

fn requested_platforms(request: &AcquisitionRequest) -> Vec<&str> {
    match request.games() {
        GameSelection::PlatformBound(games) | GameSelection::QueryResult(games)
            if request.platforms().is_empty() =>
        {
            games.iter().map(|game| game.platform.as_str()).collect()
        }
        _ => request.platforms().iter().map(String::as_str).collect(),
    }
}

fn launchbox_platform(platform: &str) -> Option<&'static str> {
    PLATFORMS
        .iter()
        .find(|(requested, _)| requested.eq_ignore_ascii_case(platform.trim()))
        .map(|(_, launchbox)| *launchbox)
}

fn no_intro_region(region: &str) -> String {
    REGIONS
        .iter()
        .find(|(launchbox, _)| launchbox.eq_ignore_ascii_case(region.trim()))
        .map(|(_, no_intro)| (*no_intro).to_owned())
        .unwrap_or_else(|| region.trim().to_owned())
}

const ARTICLES: [&str; 3] = ["the", "a", "an"];

/// Compares titles regardless of case, spacing, subtitle separator (No-Intro writes " - ",
/// LaunchBox ": ") and article placement: No-Intro moves a leading article after the main
/// title ("Legend of Zelda, The - A Link to the Past").
fn title_key(title: &str) -> String {
    let words = title.split_whitespace().collect::<Vec<_>>().join(" ");
    words
        .to_lowercase()
        .replace(": ", " - ")
        .split(" - ")
        .map(|part| {
            ARTICLES
                .iter()
                .find_map(|article| {
                    part.strip_suffix(&format!(", {article}"))
                        .map(|rest| format!("{article} {rest}"))
                })
                .unwrap_or_else(|| part.to_owned())
        })
        .collect::<Vec<_>>()
        .join(": ")
}

/// `title` as No-Intro writes it, so a candidate matches a Library imported from No-Intro: a
/// leading article moves after the main title and subtitles follow " - " ("The Legend of
/// Zelda: A Link to the Past" becomes "Legend of Zelda, The - A Link to the Past").
fn no_intro_title(title: &str) -> String {
    let words = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized = words.replace(": ", " - ");
    let (main, subtitles) = match normalized.split_once(" - ") {
        Some((main, subtitles)) => (main, Some(subtitles)),
        None => (normalized.as_str(), None),
    };
    let main = match main.split_once(' ') {
        Some((article, rest)) if ARTICLES.contains(&article.to_lowercase().as_str()) => {
            format!("{rest}, {article}")
        }
        _ => main.to_owned(),
    };
    match subtitles {
        Some(subtitles) => format!("{main} - {subtitles}"),
        None => main,
    }
}

/// The image host locator of `file_name`; names that could address anything else are refused.
fn image_url(file_name: &str) -> Option<String> {
    let name = file_name.trim();
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return None;
    }
    let mut url = Url::parse(IMAGE_BASE_URL).ok()?;
    url.path_segments_mut().ok()?.pop_if_empty().push(name);
    Some(url.into())
}

/// Calls `visit` with the text of each field of every `record` element under the root.
fn for_each_record(
    reader: impl BufRead,
    record: &str,
    mut visit: impl FnMut(&HashMap<String, String>),
) -> Result<(), PortError> {
    let mut xml = Reader::from_reader(reader);
    let mut buffer = Vec::new();
    let mut depth = 0_usize;
    let mut in_record = false;
    let mut field: Option<String> = None;
    let mut fields: HashMap<String, String> = HashMap::new();
    loop {
        match xml
            .read_event_into(&mut buffer)
            .map_err(launchbox_xml_error)?
        {
            Event::Start(element) => {
                depth += 1;
                let name = element.name();
                if depth == 2 && name.as_ref() == record {
                    in_record = true;
                    fields.clear();
                } else if depth == 3 && in_record {
                    field = Some(name.as_ref().to_owned());
                }
            }
            Event::Text(text) => {
                if let (true, Some(field)) = (in_record, &field) {
                    fields
                        .entry(field.clone())
                        .or_default()
                        .push_str(text.xml10_content().as_ref());
                }
            }
            Event::GeneralRef(reference) => {
                if let (true, Some(field)) = (in_record, &field) {
                    push_xml_reference(
                        fields.entry(field.clone()).or_default(),
                        &reference,
                        "LaunchBox",
                    )?;
                }
            }
            Event::End(_) => {
                if depth == 3 {
                    field = None;
                } else if depth == 2 && in_record {
                    in_record = false;
                    fields.retain(|_, value| {
                        *value = value.trim().to_owned();
                        !value.is_empty()
                    });
                    visit(&fields);
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(())
}

/// A dataset that cannot be read is an environmental failure; one that does not parse is
/// invalid source data.
fn launchbox_xml_error(error: quick_xml::Error) -> PortError {
    match error {
        quick_xml::Error::Io(error) => {
            PortError::new(format!("failed to read the LaunchBox dataset: {error}"))
        }
        error => PortError::invalid_source_data(format!("invalid LaunchBox XML: {error}")),
    }
}
