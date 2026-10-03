//! Requests for every game of a platform, served from the game list of each platform.

use std::collections::HashSet;

use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, GameSelection, LibraryEntry,
    PlatformBoundGameSelector, ReleaseAssertionField,
};

use crate::{
    AcquisitionPlan, ApplicationError, CatalogPort, ConnectorPort, PlatformCatalogSourcePort,
    ReferenceCatalogRepositoryPort, build_acquisition_request, plan::ensure_request_supported,
    plan_acquisition, sync_platform_catalog,
};

/// The Sources whose datafiles name a release as Sources name its media, such as
/// `Super Mario Bros. (World)`.
const DATAFILE_SOURCES: [&str; 2] = ["no-intro", "redump"];

/// The language No-Intro and Redump leave implied by a release's region when its name gives
/// none, by region.
const REGION_LANGUAGES: [(&str, &str); 22] = [
    ("Asia", "En"),
    ("Australia", "En"),
    ("Brazil", "Pt"),
    ("Canada", "En"),
    ("China", "Zh"),
    ("Europe", "En"),
    ("France", "Fr"),
    ("Germany", "De"),
    ("Hong Kong", "Zh"),
    ("Italy", "It"),
    ("Japan", "Ja"),
    ("Korea", "Ko"),
    ("Netherlands", "Nl"),
    ("Portugal", "Pt"),
    ("Russia", "Ru"),
    ("Spain", "Es"),
    ("Sweden", "Sv"),
    ("Taiwan", "Zh"),
    ("UK", "En"),
    ("USA", "En"),
    ("United Kingdom", "En"),
    ("World", "En"),
];

/// Expands a request for every game of its platforms into the games the vault's releases of
/// those platforms name, so that Sources which look games up one by one serve it too. The game
/// list of a platform the vault holds no release of is fetched and imported first; when no list
/// is known for one, the request stays one for every game, which Sources reading whole platforms
/// serve. A platform is the vault's regardless of case and punctuation, as game lists are found,
/// and the expanded request names it as the vault does, which is how media are matched to its
/// releases. Requested regions keep the releases of those regions and the worldwide ones, and
/// requested languages the releases whose name lists one of them, or whose region implies it
/// when the name lists none; the expanded request keeps its regions, which Sources telling
/// regions apart keep to, but no languages, which no Source tells apart. A request naming its
/// games, or no platform, stays as it is, and a request that is invalid, or that planning would
/// refuse, is refused before anything is fetched.
pub fn expand_every_game(
    catalog: &dyn CatalogPort,
    references: &dyn ReferenceCatalogRepositoryPort,
    platform_catalogs: &dyn PlatformCatalogSourcePort,
    mut input: AcquisitionRequestDraft,
) -> Result<AcquisitionRequestDraft, ApplicationError> {
    if !matches!(input.games, GameSelection::All) || input.platforms.is_empty() {
        return Ok(input);
    }
    ensure_request_supported(&build_acquisition_request(input.clone())?)?;
    let mut releases = catalog.list_library()?;
    let mut synced = false;
    for platform in &input.platforms {
        if !releases
            .iter()
            .any(|release| same_platform(&release.platform, platform))
        {
            match sync_platform_catalog(references, platform_catalogs, platform) {
                Ok(_) => synced = true,
                Err(ApplicationError::PlatformCatalogNotFound(_)) => return Ok(input),
                Err(error) => return Err(error),
            }
        }
    }
    if synced {
        releases = catalog.list_library()?;
    }
    let mut platforms: Vec<String> = Vec::new();
    let mut games: Vec<PlatformBoundGameSelector> = Vec::new();
    let mut listed = false;
    for requested in &input.platforms {
        for release in releases
            .iter()
            .filter(|release| same_platform(&release.platform, requested))
        {
            listed = true;
            if !in_regions(release, &input.regions) || !speaks(release, &input.languages) {
                continue;
            }
            if !platforms.contains(&release.platform) {
                platforms.push(release.platform.clone());
            }
            let game = PlatformBoundGameSelector {
                game: release_name(release),
                platform: release.platform.clone(),
            };
            if !games.contains(&game) {
                games.push(game);
            }
        }
    }
    if games.is_empty() && listed {
        // Every game of the request is none of the releases listed, rather than every one.
        return Err(ApplicationError::NoMatchingReleases(
            input.platforms.join(", "),
        ));
    }
    if games.is_empty() {
        // No release of these platforms to name: Sources reading whole platforms still serve it.
        return Ok(input);
    }
    input.platforms = platforms;
    input.games = GameSelection::PlatformBound(games);
    input.languages.clear();
    Ok(input)
}

/// Plans `input` across `connectors` as starting a run of it would: a request for every game of
/// its platforms is expanded first, as `expand_every_game` does, which may import a platform's
/// game list into the vault.
pub fn plan_acquisition_request(
    catalog: &dyn CatalogPort,
    references: &dyn ReferenceCatalogRepositoryPort,
    platform_catalogs: &dyn PlatformCatalogSourcePort,
    input: AcquisitionRequestDraft,
    connectors: &[&dyn ConnectorPort],
) -> Result<AcquisitionPlan, ApplicationError> {
    let input = expand_every_game(catalog, references, platform_catalogs, input)?;
    plan_acquisition(&build_acquisition_request(input)?, connectors)
}

/// Whether `release` is of one of `regions`, any when none is named: a release of several
/// regions, such as `USA, Europe`, is of each, and a worldwide one of every region.
fn in_regions(release: &LibraryEntry, regions: &[String]) -> bool {
    regions.is_empty()
        || release.region.split(',').map(str::trim).any(|region| {
            region.eq_ignore_ascii_case("World")
                || regions
                    .iter()
                    .any(|wanted| wanted.trim().eq_ignore_ascii_case(region))
        })
}

/// Whether `release` speaks one of `languages`, any when none is named: the languages its name
/// lists, such as `(En,Fr,De)`, or else the one its region implies.
fn speaks(release: &LibraryEntry, languages: &[String]) -> bool {
    if languages.is_empty() {
        return true;
    }
    let name = release_name(release);
    let listed: Vec<String> = trailing_tags(&name)
        .into_iter()
        .find(|tag| tag.split(',').all(|part| is_language_code(part.trim())))
        .map(|tag| tag.split(',').map(|part| part.trim().to_owned()).collect())
        .unwrap_or_else(|| {
            release
                .region
                .split(',')
                .filter_map(|region| {
                    REGION_LANGUAGES
                        .iter()
                        .find(|(known, _)| known.eq_ignore_ascii_case(region.trim()))
                        .map(|(_, language)| (*language).to_owned())
                })
                .collect()
        });
    listed.iter().any(|spoken| {
        languages
            .iter()
            .any(|wanted| wanted.trim().eq_ignore_ascii_case(spoken))
    })
}

/// The parenthesized tags ending `name`, in order, such as `Europe` and `En,Fr,De` for
/// `Asterix (Europe) (En,Fr,De)`.
pub(crate) fn trailing_tags(name: &str) -> Vec<&str> {
    let mut rest = name.trim_end();
    let mut tags = Vec::new();
    while let Some(inner) = rest.strip_suffix(')') {
        let Some(open) = inner.rfind(" (") else {
            break;
        };
        tags.push(&inner[open + 2..]);
        rest = inner[..open].trim_end();
    }
    tags.reverse();
    tags
}

/// The ISO 639-1 codes No-Intro and Redump write languages with, so that another tag of the same
/// shape, such as `Unl` or `Alt`, is never read as a language.
const LANGUAGE_CODES: [&str; 46] = [
    "Af", "Ar", "Bg", "Ca", "Cs", "Cy", "Da", "De", "El", "En", "Es", "Et", "Eu", "Fa", "Fi", "Fr",
    "Ga", "Gd", "He", "Hi", "Hr", "Hu", "Id", "Is", "It", "Ja", "Ko", "Lt", "Lv", "Ms", "Nl", "No",
    "Pl", "Pt", "Ro", "Ru", "Sk", "Sl", "Sq", "Sr", "Sv", "Th", "Tr", "Uk", "Vi", "Zh",
];

/// Whether `code` is a language as No-Intro writes it, such as `En`, `Zh` or `Zh-Hant`.
fn is_language_code(code: &str) -> bool {
    let language = code.split_once('-').map_or(code, |(language, _)| language);
    LANGUAGE_CODES.contains(&language)
}

/// The keys of the releases a request names game by game, as `release_key` gives them; none
/// for a request of every game.
pub(crate) fn requested_release_keys(request: &AcquisitionRequest) -> HashSet<String> {
    match request.games() {
        GameSelection::All => HashSet::new(),
        GameSelection::Explicit(games) => games.iter().map(|game| name_key_of(game)).collect(),
        GameSelection::PlatformBound(games) | GameSelection::QueryResult(games) => games
            .iter()
            .map(|selector| name_key_of(&selector.game))
            .collect(),
    }
}

/// The key a release is named by, as a request names its game, regardless of case and
/// surrounding spaces.
pub(crate) fn release_key(release: &LibraryEntry) -> String {
    name_key_of(&release_name(release))
}

fn name_key_of(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Whether two spellings name one platform regardless of case, punctuation and spacing, as
/// game lists are found: `NEC - PC Engine - TurboGrafx-16` is `NEC - PC Engine - TurboGrafx 16`.
fn same_platform(listed: &str, requested: &str) -> bool {
    let key = |platform: &str| -> String {
        platform
            .chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    key(listed) == key(requested)
}

/// The name Sources know a release by: the name of its datafile entry, which carries its region
/// and edition tags, or else its title followed by its region and edition tags.
pub(crate) fn release_name(release: &LibraryEntry) -> String {
    release
        .assertions
        .iter()
        .filter(|assertion| {
            assertion.field == ReleaseAssertionField::Identifier
                && assertion.qualifier.as_deref() == Some("source_record")
                && DATAFILE_SOURCES.contains(&assertion.source_id.as_str())
        })
        .find_map(|assertion| datafile_name(&assertion.value))
        .unwrap_or_else(|| described_name(release))
}

/// The entry name a datafile's `source_record` identifier holds after the platform it is
/// prefixed with, written as `<platform length>:<platform><name>`.
fn datafile_name(record: &str) -> Option<String> {
    let (length, rest) = record.split_once(':')?;
    let name = rest.get(length.parse::<usize>().ok()?..)?;
    (!name.trim().is_empty()).then(|| name.to_owned())
}

fn described_name(release: &LibraryEntry) -> String {
    let mut name = release.game_title.clone();
    if !release.region.eq_ignore_ascii_case("unknown") {
        name.push_str(&format!(" ({})", release.region));
    }
    if !["standard", "unspecified"]
        .iter()
        .any(|plain| release.edition_name.eq_ignore_ascii_case(plain))
    {
        for tag in release.edition_name.split(" · ") {
            name.push_str(&format!(" ({})", tag.trim()));
        }
    }
    name
}
