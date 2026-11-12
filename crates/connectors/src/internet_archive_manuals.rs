//! The Internet Archive's `consolemanuals` collection: scans of console game manuals, which many
//! people uploaded under several naming schemes. The connector reads the collection's list once,
//! keeps the manuals whose title follows one of two structured schemes on a platform it knows,
//! and downloads a manual's PDF as the item's file list names it. Every request goes through a
//! transport that honors the Archive's robots.txt and leaves time between requests.

use std::{
    collections::{HashMap, HashSet},
    io::Read,
    sync::{Arc, Mutex, PoisonError},
};

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection, SourceId,
};
use serde_json::Value;
use url::Url;

use crate::{
    HttpTransport, PoliteTransport, ReqwestHttpTransport,
    selection::{name_key, platform_key, requested_platforms, title_key, wanted_games},
};

pub const INTERNET_ARCHIVE_MANUALS_SOURCE_ID: &str = "internet-archive-manuals";

/// The domain whose servers the Archive serves its files from.
const ARCHIVE_DOMAIN: &str = "archive.org";
const ARCHIVE: &str = "https://archive.org";
const SCRAPE: &str = "https://archive.org/services/search/v1/scrape";
const COLLECTION_QUERY: &str = "collection:consolemanuals";

/// Items per page of the collection's list, the most its scrape API serves at once: the 19,076
/// items of 2026-11-12 fill two pages.
const PAGE_SIZE: &str = "10000";

/// Pages of the collection's list read at most, which keeps a cursor that never ends from
/// reading forever.
const MAX_LIST_PAGES: usize = 20;

/// The largest answer the Archive is trusted to give, a page of its list or an item's file list,
/// which keeps a misbehaving one from filling memory: a page of the list is about a mebibyte.
const MAX_ANSWER_BYTES: u64 = 32 * 1024 * 1024;

/// The platforms the collection's structured titles name, as they spell them, regardless of
/// case, with the catalog platforms their manuals serve.
const PLATFORMS: &[(&str, &[&str])] = &[
    ("3DO", &["The 3DO Company - 3DO"]),
    ("Atari 2600", &["Atari - 2600"]),
    ("Atari 5200", &["Atari - 5200"]),
    ("Atari 7800", &["Atari - 7800"]),
    ("Atari Jaguar CD", &["Atari - Jaguar CD"]),
    ("Coleco ColecoVision", &["Coleco - ColecoVision"]),
    // The Famicom's releases are the Japanese ones of the NES catalog, and the Super
    // Famicom's of the SNES catalog.
    ("Famicom", &["Nintendo - Nintendo Entertainment System"]),
    ("FDS", &["Nintendo - Family Computer Disk System"]),
    ("Game Boy", &["Nintendo - Game Boy"]),
    ("Game Boy Color", &["Nintendo - Game Boy Color"]),
    ("Mega-CD", &["Sega - Mega-CD - Sega CD"]),
    ("NeoGeo CD", &["SNK - Neo Geo CD"]),
    ("Odyssey 2", &["Magnavox - Odyssey2"]),
    // HuCard and CD-ROM manuals share one name: the requested title tells which catalog holds
    // the game.
    (
        "PC Engine",
        &[
            "NEC - PC Engine - TurboGrafx 16",
            "NEC - PC Engine CD - TurboGrafx-CD",
        ],
    ),
    ("Phillips CD-I", &["Philips - CD-i"]),
    ("Sega Dreamcast", &["Sega - Dreamcast"]),
    ("Sega Genesis", &["Sega - Mega Drive - Genesis"]),
    ("Sega Saturn", &["Sega - Saturn"]),
    ("SFC", &["Nintendo - Super Nintendo Entertainment System"]),
    ("SG-1000", &["Sega - SG-1000"]),
    ("XBOX", &["Microsoft - Xbox"]),
];

/// The region codes structured titles give, with the catalog region each stands for.
const REGIONS: &[(&str, &str)] = &[
    ("US", "USA"),
    ("JP", "Japan"),
    ("EU", "Europe"),
    ("AS", "Asia"),
    ("AU", "Australia"),
    ("BR", "Brazil"),
    ("CA", "Canada"),
    ("CN", "China"),
    ("DE", "Germany"),
    ("ES", "Spain"),
    ("FR", "France"),
    ("GB", "UK"),
    ("HK", "Hong Kong"),
    ("IT", "Italy"),
    ("KR", "Korea"),
    ("NL", "Netherlands"),
    ("PT", "Portugal"),
    ("SE", "Sweden"),
    ("TW", "Taiwan"),
    ("UK", "UK"),
];

/// Reads the collection's list of manuals once and downloads the PDF of those a request names.
pub struct InternetArchiveManualsConnector<T = PoliteTransport<ReqwestHttpTransport>> {
    transport: T,
    /// The collection's structured manuals, read by the first discovery that needs them.
    listing: Mutex<Option<Arc<Listing>>>,
}

impl InternetArchiveManualsConnector<PoliteTransport<ReqwestHttpTransport>> {
    pub fn new() -> Self {
        Self::with_transport(PoliteTransport::new(
            ReqwestHttpTransport::for_public_site_within(ARCHIVE_DOMAIN),
        ))
    }
}

impl Default for InternetArchiveManualsConnector<PoliteTransport<ReqwestHttpTransport>> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> InternetArchiveManualsConnector<T> {
    pub fn with_transport(transport: T) -> Self {
        Self {
            transport,
            listing: Mutex::new(None),
        }
    }
}

impl<T> ConnectorPort for InternetArchiveManualsConnector<T>
where
    T: HttpTransport,
{
    fn source_id(&self) -> &'static str {
        INTERNET_ARCHIVE_MANUALS_SOURCE_ID
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::Manual],
            direct_media_download: true,
        }
    }

    fn rate_limits(&self) -> Option<String> {
        Some("Its collection's list is read once per run, in two requests, then each manual's file list before its PDF: at least a second apart, or longer when its robots.txt asks, each request sent once.".to_owned())
    }

    /// Checked against the platforms this connector knows, without reaching the Archive.
    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        if !request.languages().is_empty() {
            return Ok(Some(
                "Internet Archive console manuals cannot satisfy language filters, since the collection's titles name no language"
                    .to_owned(),
            ));
        }
        Ok(requested_platforms(request)
            .into_iter()
            .find(|platform| !is_known(platform))
            .map(|platform| {
                format!(
                    "the Internet Archive's console manuals name no platform matching {platform}"
                )
            }))
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        if !request.requests_asset_type(AssetType::Manual) {
            return Ok(Vec::new());
        }
        if let Some(reason) = self.unsupported_request_reason(request)? {
            return Err(PortError::new(reason));
        }
        let listing = self.listing()?;
        let regions: Vec<String> = request
            .regions()
            .iter()
            .map(|region| name_key(region))
            .collect();
        let in_requested_region = |manual: &&ListedManual| {
            regions.is_empty()
                || manual.region.as_deref().is_some_and(|region| {
                    region
                        .split(", ")
                        .any(|part| regions.contains(&name_key(part)))
                })
        };
        let mut offered = HashSet::new();
        let mut candidates = Vec::new();
        let mut offer = |manual: &ListedManual, title: &str, platform: &str| {
            // Two requested titles one key stands for would offer one manual twice.
            if offered.insert((manual.identifier.clone(), platform_key(platform))) {
                candidates.push(candidate(manual, title, platform));
            }
        };
        match request.games() {
            GameSelection::All => {
                for platform in request.platforms() {
                    for manual in listing
                        .manuals
                        .iter()
                        .filter(|manual| manual.serves(platform))
                        .filter(in_requested_region)
                    {
                        offer(manual, &manual.title, platform);
                    }
                }
            }
            _ => {
                for (title, platform) in wanted_games(request) {
                    let Some(found) = listing.by_key.get(&match_key(&title)) else {
                        continue;
                    };
                    for manual in found
                        .iter()
                        .map(|index| &listing.manuals[*index])
                        .filter(|manual| manual.serves(&platform))
                        .filter(in_requested_region)
                    {
                        offer(manual, &title, &platform);
                    }
                }
            }
        }
        Ok(candidates)
    }

    /// Reads the item's file list, then downloads the PDF it names: the original whose name
    /// names a manual, or else its first original PDF.
    fn download(
        &self,
        candidate: &AssetCandidate,
    ) -> Result<Box<dyn std::io::Read + Send>, PortError> {
        let identifier = item_of(&candidate.source_url).ok_or_else(|| {
            PortError::invalid_source_data(format!(
                "{} is no item of the Internet Archive",
                candidate.source_url
            ))
        })?;
        let item = self.read_json(&archive_url(&["metadata", identifier]))?;
        // The Archive answers an item it no longer holds, or no longer shows, with no file list.
        let Some(files) = item.get("files").and_then(Value::as_array) else {
            return Err(PortError::unavailable(format!(
                "the Internet Archive no longer lists the files of {identifier}"
            )));
        };
        let Some(pdf) = manual_pdf(files) else {
            return Err(PortError::unavailable(format!(
                "the Internet Archive item {identifier} holds no PDF of its manual"
            )));
        };
        let mut path = vec!["download", identifier];
        path.extend(pdf.split('/'));
        self.transport.get_stream(&archive_url(&path))
    }
}

impl<T: HttpTransport> InternetArchiveManualsConnector<T> {
    /// The collection's structured manuals, read once for this connector: a failure to read
    /// them is kept for no one, so the next discovery reads them again.
    fn listing(&self) -> Result<Arc<Listing>, PortError> {
        let mut listing = self.listing.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(read) = listing.as_ref() {
            return Ok(Arc::clone(read));
        }
        let mut manuals = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let page = self.read_json(&list_url(cursor.as_deref()))?;
            let items = page.get("items").and_then(Value::as_array).ok_or_else(|| {
                PortError::invalid_source_data(
                    "the Internet Archive answered a page of its console manuals without their items"
                        .to_owned(),
                )
            })?;
            manuals.extend(items.iter().filter_map(listed_manual));
            match page.get("cursor").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => cursor = Some(next.to_owned()),
                _ => {
                    let read = Arc::new(Listing::of(manuals));
                    *listing = Some(Arc::clone(&read));
                    return Ok(read);
                }
            }
        }
        Err(PortError::invalid_source_data(format!(
            "the Internet Archive's list of console manuals runs past {MAX_LIST_PAGES} pages"
        )))
    }

    fn read_json(&self, url: &str) -> Result<Value, PortError> {
        let mut bytes = Vec::new();
        self.transport
            .get_stream(url)?
            .take(MAX_ANSWER_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| PortError::new(format!("failed to read {url}: {error}")))?;
        if bytes.len() as u64 > MAX_ANSWER_BYTES {
            return Err(PortError::invalid_source_data(format!(
                "{url} exceeds {MAX_ANSWER_BYTES} bytes"
            )));
        }
        serde_json::from_slice(&bytes).map_err(|error| {
            PortError::invalid_source_data(format!("{url} is not the JSON expected: {error}"))
        })
    }
}

/// The collection's structured manuals, and where each title key finds them.
struct Listing {
    manuals: Vec<ListedManual>,
    by_key: HashMap<String, Vec<usize>>,
}

impl Listing {
    fn of(manuals: Vec<ListedManual>) -> Self {
        let mut by_key: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, manual) in manuals.iter().enumerate() {
            by_key
                .entry(match_key(&manual.title))
                .or_default()
                .push(index);
        }
        Self { manuals, by_key }
    }
}

/// A manual of the collection whose title follows a structured scheme on a known platform.
struct ListedManual {
    identifier: String,
    /// Its title as the collection lists it.
    listed: String,
    /// The game's title, without the tags the scheme adds.
    title: String,
    /// The catalog platforms it serves.
    platforms: &'static [&'static str],
    /// The catalog region its title gives, if any.
    region: Option<String>,
}

impl ListedManual {
    fn serves(&self, platform: &str) -> bool {
        self.platforms
            .iter()
            .any(|served| served.eq_ignore_ascii_case(platform.trim()))
    }
}

/// The manual a listed item is, when its title follows a structured scheme on a known platform.
fn listed_manual(item: &Value) -> Option<ListedManual> {
    let identifier = item.get("identifier")?.as_str()?;
    if !is_identifier(identifier) {
        return None;
    }
    // An item may carry several titles, the first of which it shows.
    let listed = match item.get("title")? {
        Value::Array(titles) => titles.first()?.as_str()?,
        title => title.as_str()?,
    };
    let (title, platform, region) = structured(listed)?;
    let (_, platforms) = PLATFORMS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(platform.trim()))?;
    Some(ListedManual {
        identifier: identifier.to_owned(),
        listed: listed.to_owned(),
        title: title.to_owned(),
        platforms,
        region: region.and_then(catalog_region),
    })
}

/// The game title, platform and region code a structured title gives, by either scheme:
/// `<platform> Manual: <title> (<year>)(<publisher>)(<region>)[<serial>]`, whose tags after the
/// title vary, or `<title> [<serial>](<platform>)(<region>) -Manual(600DPI)`, which may name the
/// further scans the item holds after its manual.
fn structured(listed: &str) -> Option<(&str, &str, Option<&str>)> {
    if let Some((platform, rest)) = listed.split_once(" Manual: ") {
        let (title, tags) = trailing_tags(rest);
        let region = tags.into_iter().find(|tag| catalog_region(tag).is_some());
        return (!title.is_empty()).then_some((title, platform, region));
    }
    let serial_end = listed.find("](")?;
    let serial_start = listed[..serial_end].rfind('[')?;
    let title = listed[..serial_start].trim();
    let (platform, rest) = listed[serial_end + 2..].split_once(")(")?;
    let (region, scans) = rest.split_once(") -")?;
    let names_manual = scans
        .split([',', '('])
        .any(|scan| scan.trim().eq_ignore_ascii_case("manual"));
    (names_manual && !title.is_empty()).then_some((title, platform, Some(region)))
}

/// `text` without the parenthesized and bracketed tags it ends with, and its parenthesized ones.
fn trailing_tags(text: &str) -> (&str, Vec<&str>) {
    let mut rest = text.trim();
    let mut tags = Vec::new();
    loop {
        let open = match rest.chars().next_back() {
            Some(')') => '(',
            Some(']') => '[',
            _ => break,
        };
        let Some(start) = rest.rfind(open) else {
            break;
        };
        if open == '(' {
            tags.push(&rest[start + 1..rest.len() - 1]);
        }
        rest = rest[..start].trim_end();
    }
    tags.reverse();
    (rest, tags)
}

/// The catalog region a region tag stands for, several codes giving several regions.
fn catalog_region(tag: &str) -> Option<String> {
    let regions: Option<Vec<&str>> = tag
        .split(',')
        .map(|code| {
            REGIONS
                .iter()
                .find(|(known, _)| *known == code.trim())
                .map(|(_, region)| *region)
        })
        .collect();
    regions.map(|regions| regions.join(", "))
}

fn is_known(platform: &str) -> bool {
    PLATFORMS
        .iter()
        .flat_map(|(_, served)| served.iter())
        .any(|served| served.eq_ignore_ascii_case(platform.trim()))
}

/// The key titles are compared by: regardless of case, punctuation, spacing, subtitle separator
/// and article placement.
fn match_key(title: &str) -> String {
    name_key(&title_key(title))
}

/// Whether `identifier` names an item as the Archive names them, so that it is one path
/// segment of its locators.
fn is_identifier(identifier: &str) -> bool {
    identifier
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && identifier
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
}

fn list_url(cursor: Option<&str>) -> String {
    let mut url = Url::parse(SCRAPE).expect("the scrape API has a valid locator");
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("fields", "identifier,title")
            .append_pair("q", COLLECTION_QUERY)
            .append_pair("count", PAGE_SIZE);
        if let Some(cursor) = cursor {
            query.append_pair("cursor", cursor);
        }
    }
    url.into()
}

/// The Archive's locator of `segments`, each escaped as one path segment.
fn archive_url(segments: &[&str]) -> String {
    let mut url = Url::parse(ARCHIVE).expect("the Archive has a valid locator");
    url.path_segments_mut()
        .expect("the Archive's locator has a path")
        .clear()
        .extend(segments);
    url.into()
}

/// The identifier of the item `source_url`, a candidate's details page, shows.
fn item_of(source_url: &str) -> Option<&str> {
    let rest = source_url
        .strip_prefix(ARCHIVE)?
        .strip_prefix("/details/")?;
    is_identifier(rest).then_some(rest)
}

/// The PDF of an item's manual: the original PDF whose name names a manual, or else its first
/// original PDF. Derivatives, such as the text PDF the Archive adds, are not the scan.
fn manual_pdf(files: &[Value]) -> Option<&str> {
    let pdfs: Vec<&str> = files
        .iter()
        .filter(|file| file.get("source").and_then(Value::as_str) == Some("original"))
        .filter_map(|file| file.get("name")?.as_str())
        .filter(|name| name.to_ascii_lowercase().ends_with(".pdf"))
        .filter(|name| {
            name.split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
        })
        .collect();
    pdfs.iter()
        .find(|name| name.to_ascii_lowercase().contains("manual"))
        .or_else(|| pdfs.first())
        .copied()
}

fn candidate(manual: &ListedManual, title: &str, platform: &str) -> AssetCandidate {
    AssetCandidate {
        // One manual serving two catalogs, as PC Engine manuals do, is a candidate on each.
        provider_candidate_id: Some(format!("{}/{}", manual.identifier, platform_key(platform))),
        game_title: title.to_owned(),
        platform: platform.to_owned(),
        region: manual
            .region
            .clone()
            .unwrap_or_else(|| "Unknown".to_owned()),
        edition_name: "Unspecified".to_owned(),
        asset_type: AssetType::Manual,
        source_id: SourceId::from(INTERNET_ARCHIVE_MANUALS_SOURCE_ID),
        source_asset_label: Some(manual.listed.clone()),
        source_url: format!("{ARCHIVE}/details/{}", manual.identifier),
        original_filename: format!("{}.pdf", manual.identifier),
    }
}
