use std::collections::BTreeSet;

use game_media_vault_domain::{
    AssetTypeSelector, CoverageStatus, LibraryRelease, ReleaseAssertionField,
};
use serde::{Deserialize, Serialize};

use crate::{ApplicationError, CatalogPort, ReviewRepositoryPort};

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
    pub platforms: Vec<String>,
    pub regions: Vec<String>,
    /// Sources that provided an Asset or an assertion of the release.
    pub sources: Vec<String>,
    /// Asset Types or families of which the release retains an Asset.
    pub asset_types: Vec<AssetTypeSelector>,
    pub statuses: Vec<LibraryStatus>,
    /// The Release Edition the previous page ended with.
    pub after: Option<i64>,
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
}

/// Searches the Library. Releases are ordered by title, platform, region and edition, then by
/// Release Edition; a page resumes after its cursor's place in that order, so releases added
/// between two pages never repeat or shift the following ones.
pub fn search_library(
    catalog: &dyn CatalogPort,
    reviews: &dyn ReviewRepositoryPort,
    query: &LibraryQuery,
) -> Result<LibraryPage, ApplicationError> {
    let mut releases: Vec<LibraryRelease> = catalog
        .list_library()?
        .into_iter()
        .map(LibraryRelease::from)
        .collect();
    releases.sort_by_cached_key(sort_key);
    let start = match query.after {
        Some(after) => {
            releases
                .iter()
                .position(|release| release.entry.release_edition_id == after)
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
    let mut page = Vec::new();
    let mut more = false;
    for (index, release) in releases.into_iter().enumerate() {
        if !matches(query, &release, &needing_review) {
            continue;
        }
        total += 1;
        if index < start {
            continue;
        }
        if page.len() < limit {
            page.push(release);
        } else {
            more = true;
        }
    }
    let next_after = more
        .then(|| page.last().map(|release| release.entry.release_edition_id))
        .flatten();
    Ok(LibraryPage {
        releases: page,
        total,
        next_after,
    })
}

fn sort_key(release: &LibraryRelease) -> (String, String, String, String, i64) {
    let entry = &release.entry;
    (
        entry.game_title.to_lowercase(),
        entry.platform.to_lowercase(),
        entry.region.to_lowercase(),
        entry.edition_name.to_lowercase(),
        entry.release_edition_id,
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

fn matches(query: &LibraryQuery, release: &LibraryRelease, needing_review: &BTreeSet<i64>) -> bool {
    let entry = &release.entry;
    let text_matches = query.text.as_deref().map(str::trim).is_none_or(|text| {
        let text = text.to_lowercase();
        entry.game_title.to_lowercase().contains(&text)
            || release.canonical_values.iter().any(|canonical| {
                canonical.field == ReleaseAssertionField::Title
                    && canonical.value.to_lowercase().contains(&text)
            })
    });
    text_matches
        && any_or_all(&query.platforms, |platform| {
            platform.trim().eq_ignore_ascii_case(&entry.platform)
        })
        && any_or_all(&query.regions, |region| {
            region.trim().eq_ignore_ascii_case(&entry.region)
        })
        && any_or_all(&query.sources, |source| {
            entry.assets.iter().any(|asset| {
                asset
                    .provenance
                    .iter()
                    .any(|provenance| provenance.source_id.as_str() == source)
            }) || entry
                .assertions
                .iter()
                .any(|assertion| assertion.source_id.as_str() == source)
        })
        && any_or_all(&query.asset_types, |selector| {
            entry.assets.iter().any(|asset| {
                asset.asset_type.selector() == *selector || asset.asset_type.family() == *selector
            })
        })
        && any_or_all(&query.statuses, |status| match status {
            LibraryStatus::Complete => release
                .coverage
                .as_ref()
                .is_some_and(|coverage| coverage.status != CoverageStatus::Partial),
            LibraryStatus::Partial => release
                .coverage
                .as_ref()
                .is_some_and(|coverage| coverage.status == CoverageStatus::Partial),
            LibraryStatus::NeedsReview => needing_review.contains(&entry.release_edition_id),
        })
}

/// Whether some value satisfies `predicate`, or there is no value to satisfy.
fn any_or_all<T>(values: &[T], predicate: impl Fn(&T) -> bool) -> bool {
    values.is_empty() || values.iter().any(predicate)
}
