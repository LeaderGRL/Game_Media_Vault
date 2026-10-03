//! VGMaps: maps of games' levels and worlds, ripped or drawn by its community and published as
//! public web pages, one atlas page per system listing every game it maps. It is read through a
//! transport that honors the site's robots.txt and leaves time between requests.

use std::{collections::HashMap, io::Read};

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection, SourceId,
};
use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::{
    HttpTransport, PoliteTransport, ReqwestHttpTransport,
    selection::{name_key, platform_key, wanted_games},
};

pub const VGMAPS_SOURCE_ID: &str = "vgmaps";

const ATLAS: &str = "https://www.vgmaps.com/Atlas/";

/// The largest page the site is trusted to serve, which keeps a misbehaving one from filling
/// memory: its largest atlas, the NES one, is over 5 MB.
const MAX_PAGE_BYTES: u64 = 32 * 1024 * 1024;

/// The catalog platforms VGMaps has an atlas for, by the directory of its atlas page. Some
/// atlases hold the games of several platforms, such as Game Boy and Game Boy Color.
const ATLASES: &[(&str, &str)] = &[
    ("Nintendo - Nintendo Entertainment System", "NES"),
    ("Nintendo - Family Computer Disk System", "NES"),
    ("Nintendo - Super Nintendo Entertainment System", "SuperNES"),
    ("Nintendo - Nintendo 64", "N64"),
    ("Nintendo - GameCube", "GCN"),
    ("Nintendo - Wii", "Wii"),
    ("Nintendo - Wii U", "WiiU"),
    ("Nintendo - Game Boy", "GB-GBC"),
    ("Nintendo - Game Boy Color", "GB-GBC"),
    ("Nintendo - Game Boy Advance", "GBA"),
    ("Nintendo - Nintendo DS", "DS"),
    ("Nintendo - Nintendo 3DS", "3DS"),
    ("Nintendo - Nintendo Switch", "Switch"),
    ("Nintendo - Virtual Boy", "VirtualBoy"),
    ("Sega - Master System - Mark III", "MasterSystem"),
    ("Sega - Game Gear", "GameGear"),
    ("Sega - Mega Drive - Genesis", "Genesis"),
    ("Sega - Saturn", "Saturn"),
    ("Sega - Dreamcast", "DC"),
    ("Sony - PlayStation", "PSX"),
    ("Sony - PlayStation 2", "PS2"),
    ("Sony - PlayStation 3", "PS3"),
    ("Sony - PlayStation 4", "PS4"),
    ("Sony - PlayStation Portable", "PSP"),
    ("NEC - PC Engine - TurboGrafx-16", "TG16"),
    ("NEC - PC Engine CD - TurboGrafx-CD", "TG16"),
    ("NEC - PC-FX", "PC-FX"),
    ("SNK - Neo Geo Pocket", "NGP-NGPC"),
    ("SNK - Neo Geo Pocket Color", "NGP-NGPC"),
    ("Bandai - WonderSwan", "WS-WSC"),
    ("Bandai - WonderSwan Color", "WS-WSC"),
    ("Atari - 2600", "Atari2600"),
    ("Atari - 5200", "Atari5200"),
    ("Atari - 7800", "Atari7800"),
    ("Atari - Lynx", "Lynx"),
    ("Coleco - ColecoVision", "ColecoVision"),
    ("Mattel - Intellivision", "Intellivision"),
    ("Philips - CD-i", "CD-i"),
    ("Microsoft - Xbox", "Xbox"),
    ("Microsoft - Xbox 360", "X360"),
    // MAME and FBNeo emulate arcade boards, which VGMaps maps in one atlas.
    ("MAME", "Arcade"),
    ("FBNeo - Arcade Games", "Arcade"),
];

/// The words an atlas shared by several platforms adds to a game's title, in parentheses, to say
/// the table is one platform's, as `Prince Of Persia (Game Boy Color)`.
const QUALIFIERS: &[(&str, &[&str])] = &[
    ("Nintendo - Game Boy", &["Game Boy", "GB"]),
    ("Nintendo - Game Boy Color", &["Game Boy Color", "GBC"]),
    (
        "Nintendo - Nintendo Entertainment System",
        &["NES", "Famicom"],
    ),
    (
        "Nintendo - Family Computer Disk System",
        &["Famicom Disk System", "Disk System", "FDS"],
    ),
    (
        "NEC - PC Engine - TurboGrafx-16",
        &["TurboGrafx-16", "PC Engine", "TG16"],
    ),
    (
        "NEC - PC Engine CD - TurboGrafx-CD",
        &["TurboGrafx-CD", "PC Engine CD", "CD-ROM2"],
    ),
    ("SNK - Neo Geo Pocket", &["Neo Geo Pocket", "NGP"]),
    (
        "SNK - Neo Geo Pocket Color",
        &["Neo Geo Pocket Color", "NGPC"],
    ),
    ("Bandai - WonderSwan", &["WonderSwan", "WS"]),
    ("Bandai - WonderSwan Color", &["WonderSwan Color", "WSC"]),
];

/// The formats maps are published in, by the extension of their file: images, and PDF documents
/// for maps drawn as pages.
const MAP_EXTENSIONS: [&str; 5] = [".png", ".gif", ".jpg", ".jpeg", ".pdf"];

/// Reads the atlas page of each requested platform once, finds each requested game by its exact
/// title, and takes the maps its table links.
pub struct VgMapsConnector<T = PoliteTransport<ReqwestHttpTransport>> {
    transport: T,
}

impl VgMapsConnector<PoliteTransport<ReqwestHttpTransport>> {
    pub fn new() -> Self {
        Self::with_transport(PoliteTransport::new(
            ReqwestHttpTransport::for_public_sites(),
        ))
    }
}

impl Default for VgMapsConnector<PoliteTransport<ReqwestHttpTransport>> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> VgMapsConnector<T> {
    pub fn with_transport(transport: T) -> Self {
        Self { transport }
    }
}

impl<T> ConnectorPort for VgMapsConnector<T>
where
    T: HttpTransport,
{
    fn source_id(&self) -> &'static str {
        VGMAPS_SOURCE_ID
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::Map],
            direct_media_download: true,
        }
    }

    fn rate_limits(&self) -> Option<String> {
        Some("Read at most once a second, or slower when its robots.txt asks, each request sent once. A discovery reads one atlas page per requested platform, which lists every game it maps.".to_owned())
    }

    /// Checked without reaching the site.
    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        if matches!(request.games(), GameSelection::All) {
            return Ok(Some(
                "VGMaps needs an explicit game selection, since downloading every map of a platform would take hours at the pace the site is read"
                    .to_owned(),
            ));
        }
        // A request also naming another platform would go partly unserved without a word.
        if let Some((_, platform)) = wanted_games(request)
            .iter()
            .find(|(_, platform)| atlas_of(platform).is_none())
        {
            return Ok(Some(format!(
                "VGMaps has no atlas known for platform {platform}"
            )));
        }
        if !request.regions().is_empty() {
            return Ok(Some(
                "VGMaps cannot satisfy region filters because its maps record no region".to_owned(),
            ));
        }
        if !request.languages().is_empty() {
            return Ok(Some(
                "VGMaps cannot satisfy language filters yet".to_owned(),
            ));
        }
        Ok(None)
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        if let Some(reason) = self.unsupported_request_reason(request)? {
            return Err(PortError::new(reason));
        }
        if !request.requests_asset_type(AssetType::Map) {
            return Ok(Vec::new());
        }
        // The games of each atlas, read once for every requested game and platform it holds.
        let mut atlases: HashMap<&str, Vec<Game>> = HashMap::new();
        let mut candidates = Vec::new();
        for (title, platform) in wanted_games(request) {
            let Some(atlas) = atlas_of(&platform) else {
                continue;
            };
            if !atlases.contains_key(atlas) {
                let page = atlas_url(atlas);
                let games = atlas_games(&self.read(&page)?, &page, atlas);
                // Every atlas maps games: a page without any is of another kind, such as an
                // outage.
                if games.is_empty() {
                    return Err(PortError::invalid_source_data(format!(
                        "VGMaps answered {page} with no game"
                    )));
                }
                atlases.insert(atlas, games);
            }
            let wanted = title_key(&title);
            let named: Vec<&Game> = atlases[atlas]
                .iter()
                .filter(|game| title_key(&game.title) == wanted)
                .collect();
            // A table a shared atlas qualifies with the requested platform is that platform's;
            // an unqualified one serves a platform that has none of its own.
            let requested_key = platform_key(&platform);
            let own: Vec<&Game> = named
                .iter()
                .copied()
                .filter(|game| {
                    game.platform
                        .is_some_and(|its| platform_key(its) == requested_key)
                })
                .collect();
            let chosen: Vec<&Game> = if own.is_empty() {
                named
                    .into_iter()
                    .filter(|game| game.platform.is_none())
                    .collect()
            } else {
                own
            };
            for game in chosen {
                for map in &game.maps {
                    candidates.push(AssetCandidate {
                        // One map may serve several platforms of one atlas, each its own
                        // candidate.
                        provider_candidate_id: Some(format!(
                            "{}/{atlas}/{}",
                            platform_key(&platform),
                            map.file
                        )),
                        game_title: title.clone(),
                        platform: platform.clone(),
                        region: "Unknown".to_owned(),
                        edition_name: "Unspecified".to_owned(),
                        asset_type: AssetType::Map,
                        source_id: SourceId::from(VGMAPS_SOURCE_ID),
                        source_asset_label: Some(map.label.clone()),
                        source_url: map.location.to_string(),
                        original_filename: map.file.clone(),
                    });
                }
            }
        }
        Ok(candidates)
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        self.transport.get_stream(&candidate.source_url)
    }
}

impl<T: HttpTransport> VgMapsConnector<T> {
    fn read(&self, url: &Url) -> Result<String, PortError> {
        let mut bytes = Vec::new();
        self.transport
            .get_stream(url.as_str())?
            .take(MAX_PAGE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| PortError::new(format!("failed to read {url}: {error}")))?;
        if bytes.len() as u64 > MAX_PAGE_BYTES {
            return Err(PortError::invalid_source_data(format!(
                "{url} exceeds {MAX_PAGE_BYTES} bytes"
            )));
        }
        // The site writes its pages in Windows-1252, as they declare, and may move to UTF-8.
        Ok(match String::from_utf8(bytes) {
            Ok(page) => page,
            Err(error) => encoding_rs::WINDOWS_1252
                .decode_without_bom_handling(error.as_bytes())
                .0
                .into_owned(),
        })
    }
}

/// A game an atlas maps: its title, and its maps.
struct Game {
    title: String,
    /// The platform of a shared atlas its title says it is, as `(Game Boy Color)` does.
    platform: Option<&'static str>,
    maps: Vec<Map>,
}

/// A map an atlas links: its image on the site, its file name, and the area and name the atlas
/// gives it.
struct Map {
    location: Url,
    file: String,
    label: String,
}

/// The directory of the atlas page that holds a catalog platform's games, if VGMaps has one.
fn atlas_of(platform: &str) -> Option<&'static str> {
    let key = name_key(platform);
    ATLASES
        .iter()
        .find(|(name, _)| name_key(name) == key)
        .map(|(_, atlas)| *atlas)
}

fn atlas_url(atlas: &str) -> Url {
    Url::parse(ATLAS)
        .and_then(|site| site.join(&format!("{atlas}/index.htm")))
        .expect("the atlas pages have valid locations")
}

/// The games the atlas page of `atlas` maps. Each game's table opens with a cell spanning half
/// of it that names it, as `Super Mario Bros. Maps`, followed by a row per map, which names its
/// area and links its image; the cells and links of the page are read in order, so a map belongs
/// to the game named last before it.
fn atlas_games(html: &str, page: &Url, atlas: &str) -> Vec<Game> {
    let document = Html::parse_document(html);
    let cells_and_links = Selector::parse("td, a[href]").expect("a valid selector");
    let tables = Selector::parse("table").expect("a valid selector");
    let mut games: Vec<Game> = Vec::new();
    for element in document.select(&cells_and_links) {
        if element.value().name() == "td" {
            // A game's name spans half its table, unlike the area of a map, which may also end in
            // `Maps`; the site's layout cells hold whole tables, whose text names every game.
            if element.value().attr("colspan").map(str::trim) != Some("4")
                || element.select(&tables).next().is_some()
            {
                continue;
            }
            let text = cell_text(element);
            if let Some(title) = text.strip_suffix(" Maps") {
                let (title, platform) = qualified(title.trim(), atlas);
                games.push(Game {
                    title: with_article_first(title),
                    platform,
                    maps: Vec::new(),
                });
            }
            continue;
        }
        let Some(game) = games.last_mut() else {
            continue;
        };
        if let Some(map) = map(element, page) {
            game.maps.push(map);
        }
    }
    games
}

/// The map a link names, when it links an image or a PDF document on the site itself, relative
/// to its atlas page.
fn map(link: ElementRef<'_>, page: &Url) -> Option<Map> {
    let href = link.value().attr("href")?.trim();
    let lowercase = href.to_ascii_lowercase();
    if href.contains(':')
        || href.starts_with('/')
        || !MAP_EXTENSIONS
            .iter()
            .any(|extension| lowercase.ends_with(extension))
    {
        return None;
    }
    let location = page
        .join(href)
        .ok()
        .filter(|location| location.origin() == page.origin())?;
    let file = location.path_segments()?.next_back()?.to_owned();
    if file.is_empty() {
        return None;
    }
    let name = cell_text(link);
    // The row's first cell names the map's area, such as `World 1`.
    let area = link
        .ancestors()
        .filter_map(ElementRef::wrap)
        .find(|ancestor| ancestor.value().name() == "tr")
        .and_then(|row| {
            row.children()
                .filter_map(ElementRef::wrap)
                .find(|cell| cell.value().name() == "td")
        })
        .map(cell_text)
        .filter(|area| !area.is_empty() && *area != name);
    let label = match area {
        Some(area) => format!("{area} · {name}"),
        None => name,
    };
    Some(Map {
        location,
        file,
        label,
    })
}

/// A title without the parenthesized words that say which platform of the shared atlas `atlas`
/// it is, with that platform; other parenthesized words stay part of the title.
fn qualified<'a>(title: &'a str, atlas: &str) -> (&'a str, Option<&'static str>) {
    let Some((base, words)) = title
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once(" ("))
    else {
        return (title, None);
    };
    let words = name_key(words);
    let platform = QUALIFIERS
        .iter()
        .filter(|(platform, _)| atlas_of(platform) == Some(atlas))
        .find(|(_, names)| names.iter().any(|name| name_key(name) == words))
        .map(|(platform, _)| *platform);
    match platform {
        Some(platform) => (base.trim_end(), Some(platform)),
        None => (title, None),
    }
}

/// The text of an element, its whitespace and non-breaking spaces collapsed.
fn cell_text(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<String>()
        .replace('\u{a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A title an atlas files under its main word, as `Legend of Zelda, The`, with its article
/// back in front.
fn with_article_first(title: &str) -> String {
    for article in ["The", "A", "An"] {
        if let Some(rest) = title.strip_suffix(&format!(", {article}")) {
            return format!("{article} {rest}");
        }
    }
    title.to_owned()
}

/// Roman numerals a title may number a sequel with, from one to twenty.
const ROMAN_NUMERALS: [&str; 20] = [
    "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii", "xiii", "xiv", "xv",
    "xvi", "xvii", "xviii", "xix", "xx",
];

/// A title compared regardless of case, punctuation, an article filed last and the numerals of
/// its number, so that `Legend of Zelda, The` and `The Legend Of Zelda`, or `Mega Man 2` and
/// `Mega Man II`, are one title.
fn title_key(title: &str) -> String {
    with_article_first(title.trim())
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let word = word.to_lowercase();
            match ROMAN_NUMERALS.iter().position(|numeral| *numeral == word) {
                Some(index) => (index + 1).to_string(),
                None => word,
            }
        })
        .collect()
}
