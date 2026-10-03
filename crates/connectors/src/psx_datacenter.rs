//! PSX DataCenter: scans of PlayStation releases, published as public web pages, one list per
//! region and one page per game. It is read through a transport that honors the site's
//! robots.txt and leaves time between requests.

use std::io::Read;

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, GameSelection, SourceId,
};
use scraper::{Html, Selector};
use url::Url;

use crate::{
    HttpTransport, PoliteTransport, ReqwestHttpTransport,
    selection::{name_key, wanted_games},
};

pub const PSX_DATACENTER_SOURCE_ID: &str = "psx-datacenter";

const SITE: &str = "https://psxdatacenter.com/";

/// The catalog name of the one platform the site covers, which every candidate is recorded on.
const PLAYSTATION: &str = "Sony - PlayStation";

/// The largest page the site is trusted to serve, which keeps a misbehaving one from filling
/// memory: its longest list is under a mebibyte.
const MAX_PAGE_BYTES: u64 = 16 * 1024 * 1024;

/// The regions the site lists apart: the letter of their directories, their list page and the
/// region its releases are recorded in.
const REGIONS: [(char, &str, &str); 3] = [
    ('U', "ulist.html", "USA"),
    ('P', "plist.html", "Europe"),
    ('J', "jlist.html", "Japan"),
];

/// The region a requested region name stands for, by the letter of the site's directories. Names
/// are compared by their name key, regardless of case, punctuation and spacing.
/// Countries the PAL list also covers are not among them, since its releases are recorded in
/// Europe.
const REGION_NAMES: [(&str, char); 8] = [
    ("usa", 'U'),
    ("us", 'U'),
    ("northamerica", 'U'),
    ("ntscu", 'U'),
    ("europe", 'P'),
    ("pal", 'P'),
    ("japan", 'J'),
    ("ntscj", 'J'),
];

/// The names a request may give the one platform the site covers, by their name key.
const PLATFORM_NAMES: [&str; 4] = ["sonyplaystation", "playstation", "psx", "ps1"];

/// Reads the region lists the request needs, finds each requested game by its exact title, and
/// reads its page for the high-resolution cover scans and the screenshots it shows.
pub struct PsxDataCenterConnector<T = PoliteTransport<ReqwestHttpTransport>> {
    transport: T,
}

impl PsxDataCenterConnector<PoliteTransport<ReqwestHttpTransport>> {
    pub fn new() -> Self {
        Self::with_transport(PoliteTransport::new(
            ReqwestHttpTransport::for_public_sites(),
        ))
    }
}

impl Default for PsxDataCenterConnector<PoliteTransport<ReqwestHttpTransport>> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PsxDataCenterConnector<T> {
    pub fn with_transport(transport: T) -> Self {
        Self { transport }
    }
}

impl<T> ConnectorPort for PsxDataCenterConnector<T>
where
    T: HttpTransport,
{
    fn source_id(&self) -> &'static str {
        PSX_DATACENTER_SOURCE_ID
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![
                AssetType::BoxFront,
                AssetType::BoxBack,
                AssetType::Screenshot,
            ],
            direct_media_download: true,
        }
    }

    /// Checked without reaching the site.
    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        if matches!(request.games(), GameSelection::All) {
            return Ok(Some(
                "PSX DataCenter needs an explicit game selection, since reading every game's page would take hours at the pace the site is read"
                    .to_owned(),
            ));
        }
        // A request also naming another platform would go partly unserved without a word.
        if !wanted_games(request)
            .iter()
            .all(|(_, platform)| is_playstation(platform))
        {
            return Ok(Some(
                "PSX DataCenter covers only Sony - PlayStation releases".to_owned(),
            ));
        }
        if let Some(region) = request
            .regions()
            .iter()
            .find(|region| region_letter(region).is_none())
        {
            return Ok(Some(format!(
                "PSX DataCenter lists NTSC-U, PAL and NTSC-J releases, not {region}"
            )));
        }
        if !request.languages().is_empty() {
            return Ok(Some(
                "PSX DataCenter cannot satisfy language filters yet".to_owned(),
            ));
        }
        Ok(None)
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        if let Some(reason) = self.unsupported_request_reason(request)? {
            return Err(PortError::new(reason));
        }
        let wanted: Vec<String> = wanted_games(request)
            .into_iter()
            .map(|(title, _)| title)
            .collect();
        let regions: Vec<char> = if request.regions().is_empty() {
            REGIONS.iter().map(|(letter, ..)| *letter).collect()
        } else {
            REGIONS
                .iter()
                .map(|(letter, ..)| *letter)
                .filter(|letter| {
                    request
                        .regions()
                        .iter()
                        .any(|region| region_letter(region) == Some(*letter))
                })
                .collect()
        };
        let mut candidates = Vec::new();
        let mut pages_read: Vec<Url> = Vec::new();
        for (letter, list, region) in REGIONS {
            if !regions.contains(&letter) {
                continue;
            }
            let list_url = site_url(list);
            let listed = list_rows(&self.read(&list_url)?);
            // Every list holds games: one without is a page of another kind, such as an outage.
            if listed.is_empty() {
                return Err(PortError::invalid_source_data(format!(
                    "PSX DataCenter answered {list_url} with no list of games"
                )));
            }
            for title in &wanted {
                let key = name_key(title);
                let rows: Vec<&(String, String)> = listed
                    .iter()
                    .filter(|(_, listed)| name_key(listed) == key)
                    .collect();
                // Several releases of one title in a region cannot be told apart by it alone.
                let [(page, _)] = rows.as_slice() else {
                    continue;
                };
                // Only the site's own pages are read, never a host a list points elsewhere.
                let Some(page_url) = list_url.join(page).ok().filter(is_on_site) else {
                    continue;
                };
                if pages_read.contains(&page_url) {
                    continue;
                }
                let html = self.read(&page_url)?;
                pages_read.push(page_url.clone());
                candidates.extend(game_media(
                    &html,
                    &page_url,
                    letter,
                    &GameContext {
                        title,
                        region,
                        request,
                    },
                ));
            }
        }
        Ok(candidates)
    }

    fn download(
        &self,
        candidate: &AssetCandidate,
    ) -> Result<Box<dyn std::io::Read + Send>, PortError> {
        self.transport.get_stream(&candidate.source_url)
    }
}

impl<T: HttpTransport> PsxDataCenterConnector<T> {
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
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// What a game page's media are recorded with.
struct GameContext<'a> {
    title: &'a str,
    region: &'a str,
    request: &'a AcquisitionRequest,
}

fn site_url(path: &str) -> Url {
    Url::parse(SITE)
        .and_then(|site| site.join(path))
        .expect("the site's pages have valid locations")
}

fn is_on_site(location: &Url) -> bool {
    location.origin() == site_url("").origin()
}

fn is_playstation(platform: &str) -> bool {
    PLATFORM_NAMES.contains(&name_key(platform).as_str())
}

fn region_letter(region: &str) -> Option<char> {
    let key = name_key(region);
    REGION_NAMES
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, letter)| *letter)
}

/// The rows of a region list: each game's page, relative to the list, and its title without the
/// number of discs the site adds.
fn list_rows(html: &str) -> Vec<(String, String)> {
    let document = Html::parse_document(html);
    let rows = Selector::parse("tr").expect("a valid selector");
    let link = Selector::parse("td.col1 a[href]").expect("a valid selector");
    let title = Selector::parse("td.col3").expect("a valid selector");
    document
        .select(&rows)
        .filter_map(|row| {
            let page = row.select(&link).next()?.value().attr("href")?.to_owned();
            let text: String = row.select(&title).next()?.text().collect();
            Some((page, listed_title(&text)))
        })
        .collect()
}

/// A listed title without its non-breaking spaces and the `- [ 3 DISCS ]` the site appends.
fn listed_title(text: &str) -> String {
    let text = text.replace('\u{a0}', " ");
    let text = match text.rfind('[') {
        Some(start) if text[start..].to_ascii_uppercase().contains("DISC") => &text[..start],
        _ => text.as_str(),
    };
    text.trim().trim_end_matches('-').trim().to_owned()
}

/// The high-resolution front and back scans of the game's own region, and the screenshots, of
/// the kinds the request asks for.
fn game_media(
    html: &str,
    page_url: &Url,
    letter: char,
    game: &GameContext<'_>,
) -> Vec<AssetCandidate> {
    let document = Html::parse_document(html);
    let mut candidates = Vec::new();
    let anchors = Selector::parse("a[href]").expect("a valid selector");
    let hires = format!("/images/hires/{letter}/");
    for anchor in document.select(&anchors) {
        let Some(link) = anchor
            .value()
            .attr("href")
            .and_then(|href| page_url.join(href).ok())
        else {
            continue;
        };
        if !is_on_site(&link) {
            continue;
        }
        let Some(rest) = link.path().strip_prefix(&hires) else {
            continue;
        };
        let Some(id) = rest.strip_suffix(".html") else {
            continue;
        };
        let Some(name) = id.rsplit('/').next() else {
            continue;
        };
        // Named after the release's serial, the side and the edition: `SCUS-94163-F-GH`.
        let mut parts = name.splitn(4, '-').skip(2);
        let (Some(side), Some(edition)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (asset_type, side_label) = match side {
            "F" => (AssetType::BoxFront, "front"),
            "B" => (AssetType::BoxBack, "back"),
            _ => continue,
        };
        if !game.request.requests_asset_type(asset_type) {
            continue;
        }
        // A scan of an edition the site names otherwise could be taken for the standard one.
        let (label, edition_name) = match edition {
            "ALL" => (side_label.to_owned(), "Unspecified"),
            "GH" => (format!("{side_label}: GH"), "Greatest Hits"),
            "P" => (format!("{side_label}: P"), "Platinum"),
            _ => continue,
        };
        let mut image = link.clone();
        image.set_path(&format!("/images/hires/{letter}/{id}.jpg"));
        let candidate_id = format!("{letter}/{id}");
        if let Some(candidate) =
            candidate(image, &candidate_id, asset_type, &label, edition_name, game)
        {
            candidates.push(candidate);
        }
    }
    if game.request.requests_asset_type(AssetType::Screenshot) {
        let images = Selector::parse("img[src]").expect("a valid selector");
        for img in document.select(&images) {
            let Some(image) = img
                .value()
                .attr("src")
                .and_then(|src| page_url.join(src).ok())
            else {
                continue;
            };
            // Screenshots of another region's directory show that region's release.
            let screens = format!("screens/{letter}/");
            let Some(id) = Some(&image)
                .filter(|image| is_on_site(image))
                .and_then(|image| image.path().strip_prefix("/images/"))
                .filter(|path| path.starts_with(&screens))
                .and_then(|path| path.strip_suffix(".jpg"))
                .map(str::to_owned)
            else {
                continue;
            };
            if let Some(candidate) = candidate(
                image,
                &id,
                AssetType::Screenshot,
                "screenshot",
                "Unspecified",
                game,
            ) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn candidate(
    location: Url,
    id: &str,
    asset_type: AssetType,
    label: &str,
    edition_name: &str,
    game: &GameContext<'_>,
) -> Option<AssetCandidate> {
    // A location the catalog would refuse, with a query, a fragment or credentials, is skipped
    // rather than failing the discovery.
    if location.query().is_some()
        || location.fragment().is_some()
        || !location.username().is_empty()
        || location.password().is_some()
    {
        return None;
    }
    let original_filename = location.path_segments()?.next_back()?.to_owned();
    if original_filename.is_empty() {
        return None;
    }
    Some(AssetCandidate {
        provider_candidate_id: Some(id.to_owned()),
        game_title: game.title.to_owned(),
        platform: PLAYSTATION.to_owned(),
        region: game.region.to_owned(),
        edition_name: edition_name.to_owned(),
        asset_type,
        source_id: SourceId::from(PSX_DATACENTER_SOURCE_ID),
        source_asset_label: Some(label.to_owned()),
        source_url: location.to_string(),
        original_filename,
    })
}
