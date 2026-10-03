//! Requests for every game of a platform, served from the game list of each platform.

use std::collections::HashSet;

use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, GameSelection, LibraryEntry,
    PlatformBoundGameSelector, ReleaseAssertionField,
};

use crate::{
    ApplicationError, CatalogPort, PlatformCatalogSourcePort, ReferenceCatalogRepositoryPort,
    build_acquisition_request, sync_platform_catalog,
};

/// The Sources whose datafiles name a release as Sources name its media, such as
/// `Super Mario Bros. (World)`.
const DATAFILE_SOURCES: [&str; 2] = ["no-intro", "redump"];

/// Expands a request for every game of its platforms into the games the vault's releases of
/// those platforms name, so that Sources which look games up one by one serve it too. The game
/// list of a platform the vault holds no release of is fetched and imported first; when no list
/// is known for one, the request stays one for every game, which Sources reading whole platforms
/// serve. A request naming its games, or no platform, stays as it is, and an invalid request is
/// refused before anything is fetched.
pub fn expand_every_game(
    catalog: &dyn CatalogPort,
    references: &dyn ReferenceCatalogRepositoryPort,
    platform_catalogs: &dyn PlatformCatalogSourcePort,
    mut input: AcquisitionRequestDraft,
) -> Result<AcquisitionRequestDraft, ApplicationError> {
    if !matches!(input.games, GameSelection::All) || input.platforms.is_empty() {
        return Ok(input);
    }
    build_acquisition_request(input.clone())?;
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
    let mut games: Vec<PlatformBoundGameSelector> = Vec::new();
    for platform in &input.platforms {
        for release in releases
            .iter()
            .filter(|release| same_platform(&release.platform, platform))
        {
            // Sources compare the platform with the request's own spelling of it.
            let game = PlatformBoundGameSelector {
                game: release_name(release),
                platform: platform.clone(),
            };
            if !games.contains(&game) {
                games.push(game);
            }
        }
    }
    input.games = GameSelection::PlatformBound(games);
    Ok(input)
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

fn same_platform(listed: &str, requested: &str) -> bool {
    listed.trim().eq_ignore_ascii_case(requested.trim())
}

/// The name Sources know a release by: the name of its datafile entry, which carries its region
/// and edition tags, or else its title followed by its region and edition tags.
fn release_name(release: &LibraryEntry) -> String {
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
