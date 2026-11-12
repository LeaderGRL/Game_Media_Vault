use std::collections::{BTreeSet, HashMap};

use game_media_vault_domain::{
    AssetTypeSelector, CoverageStatus, LibraryAsset, LibraryRelease, LibrarySummary,
    ReleaseAssertionField, canonical_values, release_coverage_of_types,
};
use serde::{Deserialize, Serialize};

use crate::{ApplicationError, CatalogPort, ReviewRepositoryPort, platforms::platform_key};

/// Releases per page when a query asks for none.
pub const DEFAULT_LIBRARY_PAGE_SIZE: usize = 50;
/// Largest page a query can ask for.
pub const MAX_LIBRARY_PAGE_SIZE: usize = 500;

/// States the Library can be filtered by directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryStatus {
    /// Coverage reaches at least Packaging Complete.
    Complete,
    /// Coverage is evaluated and still Partial.
    Partial,
    /// An undecided Review Item offers the Release Edition.
    NeedsReview,
}

/// A Library search, shared by every frontend. Values of one filter widen the search; filters
/// of different kinds narrow it. Empty filters impose nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryQuery {
    /// Text the game title or its canonical title contains, ignoring case.
    pub text: Option<String>,
    /// Platforms, each matching its spellings regardless of case and punctuation.
    pub platforms: Vec<String>,
    pub regions: Vec<String>,
    /// Sources that provided an Asset or an assertion of the release.
    pub sources: Vec<String>,
    /// Asset Types or families of which the release retains an Asset.
    pub asset_types: Vec<AssetTypeSelector>,
    pub statuses: Vec<LibraryStatus>,
    /// The Release Edition the previous page ended with.
    pub after: Option<i64>,
    /// The `as_of` of the first page, so later pages keep its results.
    pub as_of: Option<i64>,
    /// Releases per page; 0 asks for the default.
    pub limit: usize,
}

/// One page of matching releases, in a stable order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryPage {
    pub releases: Vec<LibraryRelease>,
    /// Releases matching the query across every page.
    pub total: usize,
    /// The cursor of the next page, when one follows.
    pub next_after: Option<i64>,
    /// The newest Release Edition the search considered. Release Editions are never deleted
    /// and new ones get larger ids, so passing it back keeps later pages to the same editions.
    pub as_of: i64,
    /// Every platform whose releases retain an Asset, whatever the query, in name order, for a
    /// filter to offer: once, as its first spelling in that order, however it is spelled.
    pub platforms_with_media: Vec<String>,
}

/// A retained Asset, with the release it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LatestMedium {
    pub release_edition_id: i64,
    pub game_title: String,
    pub platform: String,
    pub region: String,
    pub asset: LibraryAsset,
}

/// The `limit` Assets the vault retained last, newest first: Assets get larger ids as they are
/// retained. Only they are read, however large the library.
pub fn latest_media(
    catalog: &dyn CatalogPort,
    limit: usize,
) -> Result<Vec<LatestMedium>, ApplicationError> {
    let mut media: Vec<LatestMedium> = catalog
        .list_latest_assets(limit)?
        .into_iter()
        .flat_map(|entry| {
            let (release_edition_id, game_title, platform, region) = (
                entry.release_edition_id,
                entry.game_title,
                entry.platform,
                entry.region,
            );
            entry.assets.into_iter().map(move |asset| LatestMedium {
                release_edition_id,
                game_title: game_title.clone(),
                platform: platform.clone(),
                region: region.clone(),
                asset,
            })
        })
        .collect();
    media.sort_by_key(|medium| std::cmp::Reverse(medium.asset.asset_id));
    media.truncate(limit);
    Ok(media)
}

/// Searches the Library. Releases are ordered by title, platform, region and edition, then by
/// Release Edition; a page resumes after its cursor's place in that order and, given the `as_of`
/// of the first page, searches the same Release Editions in the same order, so releases added
/// between two pages never repeat or shift later pages. Filters see the current state of each
/// release, which can change between pages. Releases are filtered and ordered by their summaries;
/// only those of the page are projected whole.
pub fn search_library(
    catalog: &dyn CatalogPort,
    reviews: &dyn ReviewRepositoryPort,
    query: &LibraryQuery,
) -> Result<LibraryPage, ApplicationError> {
    let mut releases = catalog.list_library_summaries()?;
    let as_of = query.as_of.unwrap_or_else(|| {
        releases
            .iter()
            .map(|release| release.release_edition_id)
            .max()
            .unwrap_or(0)
    });
    // Every platform holding media now, even past the snapshot a later page keeps to.
    let mut spelled = BTreeSet::new();
    let platforms_with_media: Vec<String> = releases
        .iter()
        .filter(|release| !release.asset_types.is_empty())
        .map(|release| release.platform.trim().to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|platform| spelled.insert(platform_key(platform)))
        .collect();
    releases.retain(|release| release.release_edition_id <= as_of);
    releases.sort_by_cached_key(sort_key);
    let start = match query.after {
        Some(after) => {
            releases
                .iter()
                .position(|release| release.release_edition_id == after)
                .ok_or(ApplicationError::ReleaseEditionMissing(after))?
                + 1
        }
        None => 0,
    };
    let needing_review = if query.statuses.contains(&LibraryStatus::NeedsReview) {
        releases_needing_review(reviews)?
    } else {
        BTreeSet::new()
    };
    let limit = match query.limit {
        0 => DEFAULT_LIBRARY_PAGE_SIZE,
        limit => limit.min(MAX_LIBRARY_PAGE_SIZE),
    };

    let mut total = 0;
    let mut page: Vec<i64> = Vec::new();
    let mut more = false;
    for (index, release) in releases.iter().enumerate() {
        if !matches(query, release, &needing_review) {
            continue;
        }
        total += 1;
        if index < start {
            continue;
        }
        if page.len() < limit {
            page.push(release.release_edition_id);
        } else {
            more = true;
        }
    }
    let next_after = more.then(|| page.last().copied()).flatten();
    Ok(LibraryPage {
        releases: project_page(catalog, &page)?,
        total,
        next_after,
        as_of,
        platforms_with_media,
    })
}

/// Projects the releases of a page whole, in the page's order. Release Editions are never
/// deleted, so each one the summaries listed is found again.
fn project_page(
    catalog: &dyn CatalogPort,
    page: &[i64],
) -> Result<Vec<LibraryRelease>, ApplicationError> {
    if page.is_empty() {
        return Ok(Vec::new());
    }
    let mut entries: HashMap<i64, _> = catalog
        .library_entries_of(page)?
        .into_iter()
        .map(|entry| (entry.release_edition_id, entry))
        .collect();
    page.iter()
        .map(|id| {
            entries
                .remove(id)
                .map(LibraryRelease::from)
                .ok_or(ApplicationError::ReleaseEditionMissing(*id))
        })
        .collect()
}

fn sort_key(release: &LibrarySummary) -> (String, String, String, String, i64) {
    (
        release.game_title.to_lowercase(),
        release.platform.to_lowercase(),
        release.region.to_lowercase(),
        release.edition_name.to_lowercase(),
        release.release_edition_id,
    )
}

/// Release Editions an undecided Review Item offers as a competing match.
fn releases_needing_review(
    reviews: &dyn ReviewRepositoryPort,
) -> Result<BTreeSet<i64>, ApplicationError> {
    Ok(reviews
        .list_review_items()?
        .into_iter()
        .filter(|item| item.status.is_undecided())
        .flat_map(|item| item.competing_matches)
        .map(|candidate| candidate.release_edition_id)
        .collect())
}

fn matches(query: &LibraryQuery, release: &LibrarySummary, needing_review: &BTreeSet<i64>) -> bool {
    let text_matches = query.text.as_deref().map(str::trim).is_none_or(|text| {
        let text = text.to_lowercase();
        release.game_title.to_lowercase().contains(&text)
            || canonical_values(&release.title_assertions)
                .iter()
                .any(|canonical| {
                    canonical.field == ReleaseAssertionField::Title
                        && canonical.value.to_lowercase().contains(&text)
                })
    });
    // Coverage depends on the platform and the Asset Types retained alone.
    let coverage_status = || {
        release_coverage_of_types(&release.platform, &release.asset_types)
            .map(|coverage| coverage.status)
    };
    text_matches
        && any_or_all(&query.platforms, |platform| {
            platform_key(platform) == platform_key(&release.platform)
        })
        // A release of several regions, such as `USA, Europe`, is of each.
        && any_or_all(&query.regions, |region| {
            release
                .region
                .split(',')
                .any(|named| region.trim().eq_ignore_ascii_case(named.trim()))
        })
        && any_or_all(&query.sources, |source| {
            release
                .source_ids
                .iter()
                .any(|source_id| source_id.as_str() == source)
        })
        && any_or_all(&query.asset_types, |selector| {
            release
                .asset_types
                .iter()
                .any(|asset_type| selector.selects(*asset_type))
        })
        && any_or_all(&query.statuses, |status| match status {
            LibraryStatus::Complete => {
                coverage_status().is_some_and(|status| status != CoverageStatus::Partial)
            }
            LibraryStatus::Partial => {
                coverage_status().is_some_and(|status| status == CoverageStatus::Partial)
            }
            LibraryStatus::NeedsReview => needing_review.contains(&release.release_edition_id),
        })
}

/// Whether some value satisfies `predicate`, or there is no value to satisfy.
fn any_or_all<T>(values: &[T], predicate: impl Fn(&T) -> bool) -> bool {
    values.is_empty() || values.iter().any(predicate)
}
