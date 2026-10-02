//! LaunchBox Games Database: a downloadable dataset of game records and of the images its
//! community uploaded, which the LaunchBox image host serves.

use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, BufRead, BufReader, Seek},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection, SourceId,
};
use quick_xml::{Reader, events::Event};
use tempfile::NamedTempFile;
use url::Url;

use crate::{
    Fetched, HttpTransport, ReqwestHttpTransport, Validators, naming::parse_release_name,
    xml::push_xml_reference,
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
    cache: Option<DatasetCache>,
}

impl LaunchBoxGamesDbConnector<ReqwestHttpTransport> {
    /// Keeps the dataset in the machine's cache, when the OS has a cache directory.
    pub fn new() -> Self {
        Self {
            transport: ReqwestHttpTransport::default(),
            cache: DatasetCache::machine(),
        }
    }
}

impl Default for LaunchBoxGamesDbConnector<ReqwestHttpTransport> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LaunchBoxGamesDbConnector<T> {
    /// Downloads the dataset whole on every discovery.
    pub fn with_transport(transport: T) -> Self {
        Self {
            transport,
            cache: None,
        }
    }

    /// Keeps the dataset in `cache` and downloads it again only once LaunchBox republishes it.
    pub fn with_transport_and_cache(transport: T, cache: DatasetCache) -> Self {
        Self {
            transport,
            cache: Some(cache),
        }
    }
}

/// A copy of the LaunchBox dataset kept for the whole machine, so every vault shares it, with
/// the validators LaunchBox sent with it.
#[derive(Debug, Clone)]
pub struct DatasetCache {
    dir: PathBuf,
}

impl DatasetCache {
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The cache in the OS standard cache directory, when the OS has one.
    pub fn machine() -> Option<Self> {
        dirs::cache_dir().map(|dir| Self::at(dir.join("game-media-vault").join("launchbox")))
    }

    fn archive(&self) -> PathBuf {
        self.dir.join("Metadata.zip")
    }

    fn validators_file(&self) -> PathBuf {
        self.dir.join("Metadata.zip.validators.json")
    }

    /// The validators of the cached copy; none without a copy, so it is fetched whole.
    fn validators(&self) -> Validators {
        if !self.archive().is_file() {
            return Validators::default();
        }
        let recorded = fs::read(self.validators_file())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        let field = |name: &str| {
            recorded
                .as_ref()
                .and_then(|recorded| recorded.get(name))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        Validators {
            etag: field("etag"),
            last_modified: field("last_modified"),
        }
    }

    /// The identifier this cache gave its copy when storing it, which tells apart two copies of
    /// the same version; none without a copy.
    fn copy_id(&self) -> Option<String> {
        if !self.archive().is_file() {
            return None;
        }
        let recorded = fs::read(self.validators_file())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        let id = recorded
            .as_ref()
            .and_then(|recorded| recorded.get("copy"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unidentified");
        Some(id.to_owned())
    }

    /// Waits until no other discovery of the machine refreshes the copy, and keeps the others
    /// waiting until the returned lock drops; none when the cache directory is unusable.
    fn lock(&self) -> Option<File> {
        fs::create_dir_all(&self.dir).ok()?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join("Metadata.zip.lock"))
            .ok()?;
        lock.lock().ok()?;
        Some(lock)
    }

    /// Forgets the cached copy `id` names, so the next discovery downloads the dataset whole. A
    /// copy another discovery stored since, even of the same version, stays.
    fn forget(&self, id: Option<&str>) {
        let Some(id) = id else {
            return;
        };
        let _refreshing = self.lock();
        if self.copy_id().as_deref() != Some(id) {
            return;
        }
        // Validators first: a copy left without them is never read again.
        let _ = fs::remove_file(self.validators_file());
        let _ = fs::remove_file(self.archive());
    }

    /// The cached copy, unless it vanished or no longer reads as an archive.
    fn copy(&self) -> Option<File> {
        let mut copy = File::open(self.archive()).ok()?;
        (reads_as_archive(&copy) && copy.rewind().is_ok()).then_some(copy)
    }

    /// Makes a copy of `dataset` the cached one, described by `validators`, and returns the
    /// identifier it gives that copy. A copy another
    /// process holds open may stay as it is.
    fn store(&self, mut dataset: &File, validators: &Validators) -> io::Result<String> {
        fs::create_dir_all(&self.dir)?;
        let mut staged = NamedTempFile::new_in(&self.dir)?;
        dataset.rewind()?;
        io::copy(&mut dataset, &mut staged)?;
        // Validators of the previous copy must never describe the new one.
        match fs::remove_file(self.validators_file()) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        staged
            .persist(self.archive())
            .map_err(|error| error.error)?;
        let id = new_copy_id();
        let recorded = serde_json::json!({
            "etag": validators.etag,
            "last_modified": validators.last_modified,
            "copy": id,
        });
        fs::write(self.validators_file(), recorded.to_string())?;
        Ok(id)
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
        let (dataset, copy) = self.download_dataset()?;
        let first = candidates_in(dataset, request, &wanted, &image_types);
        let Some(cache) = self.cache.as_ref().filter(|_| first.is_err()) else {
            return first;
        };
        // A copy whose metadata does not read is never read again, and a cached one is
        // downloaded whole once more.
        cache.forget(copy.id.as_deref());
        if !copy.cached {
            return first;
        }
        let (dataset, copy) = self.download_dataset()?;
        candidates_in(dataset, request, &wanted, &image_types)
            .inspect_err(|_| cache.forget(copy.id.as_deref()))
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
    /// The dataset archive, read from the machine cache while LaunchBox has not republished
    /// it, and otherwise downloaded to a file, since reading a ZIP needs to seek; with which copy
    /// it is.
    fn download_dataset(&self) -> Result<(Dataset, Copy), PortError> {
        let (file, copy) = match &self.cache {
            Some(cache) => self.cached_dataset(cache)?,
            None => (
                spool(self.transport.get_stream(LAUNCHBOX_METADATA_URL)?)?,
                Copy {
                    cached: false,
                    id: None,
                },
            ),
        };
        let archive = zip::ZipArchive::new(file).map_err(|error| {
            PortError::invalid_source_data(format!("unreadable LaunchBox dataset archive: {error}"))
        })?;
        Ok((Dataset { archive }, copy))
    }
}

impl<T> LaunchBoxGamesDbConnector<T>
where
    T: HttpTransport,
{
    /// The cached copy if it is still current, or the dataset LaunchBox serves now, which
    /// replaces it once it reads as an archive. The dataset is downloaded to a temporary file
    /// first, so a cache that cannot be written never fails the discovery.
    fn cached_dataset(&self, cache: &DatasetCache) -> Result<(File, Copy), PortError> {
        // Discoveries of the machine refresh the copy one at a time, each reading the validators
        // the previous one left, so a republished dataset downloads once.
        let _refreshing = cache.lock();
        let (known, id) = (cache.validators(), cache.copy_id());
        let fetched = match self
            .transport
            .get_if_changed(LAUNCHBOX_METADATA_URL, &known)?
        {
            Fetched::Unchanged => {
                if let Some(copy) = cache.copy() {
                    return Ok((copy, Copy { cached: true, id }));
                }
                // The copy vanished, or no longer reads as an archive, since its validators
                // were read: it is downloaded whole again.
                self.transport
                    .get_if_changed(LAUNCHBOX_METADATA_URL, &Validators::default())?
            }
            changed => changed,
        };
        let Fetched::Changed { body, validators } = fetched else {
            return Err(PortError::invalid_source_data(
                "LaunchBox answered a request for its whole dataset as unchanged".to_owned(),
            ));
        };
        let mut dataset = spool(body)?;
        // This discovery reads its own copy whether the cache keeps one or not.
        let id = reads_as_archive(&dataset)
            .then(|| cache.store(&dataset, &validators).ok())
            .flatten();
        dataset.rewind().map_err(buffer_failed)?;
        Ok((dataset, Copy { cached: false, id }))
    }
}

/// The candidates `dataset` holds for `request`: its records of the `wanted` games, then their
/// images of the requested `image_types`.
fn candidates_in(
    mut dataset: Dataset,
    request: &AcquisitionRequest,
    wanted: &WantedGames,
    image_types: &[(AssetType, &str)],
) -> Result<Vec<AssetCandidate>, PortError> {
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
        let Some((asset_type, _)) = image_types.iter().find(|(_, name)| name == image_type) else {
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

/// Which copy of the dataset a discovery reads.
struct Copy {
    /// Whether it is the cached copy, rather than one just downloaded.
    cached: bool,
    /// The identifier the cache gave it, when the cache keeps it.
    id: Option<String>,
}

/// An identifier no other copy stored on this machine shares.
fn new_copy_id() -> String {
    static STORED: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let count = STORED.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos}-{count}", std::process::id())
}

/// Whether `dataset` opens as a ZIP archive.
fn reads_as_archive(dataset: &File) -> bool {
    zip::ZipArchive::new(dataset).is_ok()
}

/// Downloads `body` to a temporary file.
fn spool(mut body: Box<dyn io::Read + Send>) -> Result<File, PortError> {
    let mut file = tempfile::tempfile().map_err(buffer_failed)?;
    io::copy(&mut body, &mut file).map_err(download_failed)?;
    file.rewind().map_err(buffer_failed)?;
    Ok(file)
}

fn download_failed(error: io::Error) -> PortError {
    PortError::new(format!("failed to download the LaunchBox dataset: {error}"))
}

fn buffer_failed(error: io::Error) -> PortError {
    PortError::new(format!("failed to buffer the LaunchBox dataset: {error}"))
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// Stores `bytes` as the copy of the dataset version `etag`, returning its identifier.
    fn stored(cache: &DatasetCache, bytes: &[u8], etag: &str) -> String {
        let mut copy = tempfile::tempfile().unwrap();
        copy.write_all(bytes).unwrap();
        let validators = Validators {
            etag: Some(etag.to_owned()),
            last_modified: None,
        };
        cache.store(&copy, &validators).unwrap()
    }

    #[test]
    fn forgetting_a_failed_copy_keeps_the_one_another_discovery_stored_since() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DatasetCache::at(dir.path());
        let failed = stored(&cache, b"damaged copy", "\"v1\"");
        // Another discovery repairs the copy of the very same version meanwhile.
        let repaired = stored(&cache, b"repaired copy", "\"v1\"");

        cache.forget(Some(&failed));
        assert_eq!(cache.copy_id(), Some(repaired.clone()));

        cache.forget(Some(&repaired));
        assert_eq!(cache.copy_id(), None);
        assert_eq!(cache.validators(), Validators::default());
    }
}
