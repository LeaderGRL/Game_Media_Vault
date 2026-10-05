use std::{
    collections::{HashMap, HashSet},
    io::Read,
};

use game_media_vault_domain::{
    LibraryEntry, MatchingPolicy, ReviewDecision, ReviewItem, ValidatedMatchingPolicy,
    match_asset_candidate_to_release_preferring, review_matches_for_asset_candidate,
};
use serde::{Deserialize, Serialize};

use crate::{
    ApplicationError, CatalogPort, ConnectorPort, PortError, ReviewDecisionOutcome,
    ReviewRepositoryPort,
};

/// Decisions recorded together when deciding every pending Review Item.
const DECISION_BATCH: usize = 250;

/// A page of the Review Items awaiting a decision, pending or deferred, in the order they were
/// opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewPage {
    pub items: Vec<ReviewItem>,
    /// How many items await a decision in all.
    pub undecided: usize,
    /// How many undecided items come before the page.
    pub offset: usize,
}

/// The page of `limit` undecided Review Items after the first `offset` of them, read alone
/// however many items the vault holds.
pub fn review_page(
    reviews: &dyn ReviewRepositoryPort,
    offset: usize,
    limit: usize,
) -> Result<ReviewPage, ApplicationError> {
    let (items, undecided) = reviews.undecided_review_page(offset, limit)?;
    Ok(ReviewPage {
        items,
        undecided,
        offset,
    })
}

/// A decision taken on every Review Item awaiting one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingReviewDecision {
    /// Accepts, for each item, the release its candidate matches best among those it competed
    /// for, as the matcher scores them now.
    AcceptBestMatches,
    RejectAll,
}

/// What deciding every Review Item awaiting a decision did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PendingReviewSummary {
    pub decided: usize,
    /// Items considered that still await a decision once every decision is recorded: those whose
    /// best match ties releases of several Games, and those whose competing releases changed
    /// meanwhile, or that someone deferred meanwhile. Items someone else accepted, rejected or
    /// closed meanwhile are not counted.
    pub left: usize,
}

/// Decides every Review Item awaiting a decision as `decision` says, each as a human deciding it
/// alone would: an accepted candidate's parked work is requeued, a rejected one's is settled.
pub fn decide_pending_reviews(
    reviews: &dyn ReviewRepositoryPort,
    catalog: &dyn CatalogPort,
    decision: PendingReviewDecision,
    matching_policy: MatchingPolicy,
) -> Result<PendingReviewSummary, ApplicationError> {
    // Accepting scores each item against the releases it competed for; rejecting needs neither
    // the matcher nor the catalog.
    let scoring: Option<(ValidatedMatchingPolicy, HashMap<i64, LibraryEntry>)> = match decision {
        PendingReviewDecision::AcceptBestMatches => Some((
            matching_policy.validate()?,
            catalog
                .list_library()?
                .into_iter()
                .map(|release| (release.release_edition_id, release))
                .collect(),
        )),
        PendingReviewDecision::RejectAll => None,
    };
    let mut considered = HashSet::new();
    let mut decisions = Vec::new();
    for item in reviews.list_review_items()? {
        if !item.status.is_undecided() {
            continue;
        }
        considered.insert(item.id);
        let chosen = match &scoring {
            None => Some(ReviewDecision::Reject),
            Some((matching_policy, releases)) => {
                best_competing_release(&item, releases, *matching_policy)
                    .map(|release_edition_id| ReviewDecision::Accept { release_edition_id })
            }
        };
        // Recorded only while the item is as read, so the match chosen for it still holds.
        if let Some(chosen) = chosen {
            decisions.push((item, chosen));
        }
    }
    let mut decided = 0;
    // Batches keep each transaction short, so executions are not held off for long.
    for batch in decisions.chunks(DECISION_BATCH) {
        decided += reviews
            .decide_review_items(batch)?
            .into_iter()
            .filter(|outcome| matches!(outcome, ReviewDecisionOutcome::Recorded(_)))
            .count();
    }
    // What is left is read once every decision is recorded, so items others decided meanwhile,
    // whether this decided them or not, are not counted.
    let left = reviews
        .list_review_items()?
        .into_iter()
        .filter(|item| item.status.is_undecided() && considered.contains(&item.id))
        .count();
    Ok(PendingReviewSummary { decided, left })
}

/// The release among those `item` competed for that its candidate matches best now, unless the
/// best score ties releases of several Games.
fn best_competing_release(
    item: &ReviewItem,
    releases: &HashMap<i64, LibraryEntry>,
    matching_policy: ValidatedMatchingPolicy,
) -> Option<i64> {
    let competing: Vec<LibraryEntry> = item
        .competing_matches
        .iter()
        .filter_map(|candidate| releases.get(&candidate.release_edition_id).cloned())
        .collect();
    let scored = review_matches_for_asset_candidate(&item.candidate, &competing, matching_policy);
    let best = scored.first()?;
    if scored
        .iter()
        .take_while(|candidate| candidate.score == best.score)
        .any(|candidate| candidate.game_id != best.game_id)
    {
        return None;
    }
    match_asset_candidate_to_release_preferring(
        &item.candidate,
        &competing,
        matching_policy,
        &|_| false,
    )
    .release_edition_id
}

/// Largest candidate media loaded into memory for a Review preview.
pub const MAX_REVIEW_PREVIEW_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewPreview {
    pub original_filename: String,
    pub bytes: Vec<u8>,
}

pub fn list_review_items(
    reviews: &dyn ReviewRepositoryPort,
) -> Result<Vec<ReviewItem>, ApplicationError> {
    Ok(reviews.list_review_items()?)
}

/// Records a human decision on an undecided Review Item.
pub fn resolve_review_item(
    reviews: &dyn ReviewRepositoryPort,
    review_item_id: i64,
    decision: ReviewDecision,
) -> Result<ReviewItem, ApplicationError> {
    let item = load_review_item(reviews, review_item_id)?;
    if !item.status.is_undecided() {
        return Err(ApplicationError::ReviewItemNotActionable {
            review_item_id,
            status: item.status,
        });
    }
    if let ReviewDecision::Accept { release_edition_id } = decision
        && !item
            .competing_matches
            .iter()
            .any(|candidate| candidate.release_edition_id == release_edition_id)
    {
        return Err(ApplicationError::ReviewAcceptanceNotCompeting {
            review_item_id,
            release_edition_id,
        });
    }
    // The checks above give fast feedback; the repository repeats them atomically because
    // acquisition may close or refresh the item in between.
    match reviews.decide_review_item(review_item_id, decision.clone())? {
        ReviewDecisionOutcome::Recorded(decided) => Ok(*decided),
        ReviewDecisionOutcome::NotFound => {
            Err(ApplicationError::ReviewItemNotFound(review_item_id))
        }
        ReviewDecisionOutcome::NotUndecided(status) => {
            Err(ApplicationError::ReviewItemNotActionable {
                review_item_id,
                status,
            })
        }
        ReviewDecisionOutcome::NotCompeting => {
            Err(ApplicationError::ReviewAcceptanceNotCompeting {
                review_item_id,
                release_edition_id: match decision {
                    ReviewDecision::Accept { release_edition_id } => release_edition_id,
                    _ => 0,
                },
            })
        }
        // Only a batch, whose decisions were taken on items as read, answers this.
        ReviewDecisionOutcome::Changed => {
            Err(ApplicationError::ReviewItemContended(review_item_id))
        }
    }
}

/// Downloads the candidate media of a Review Item through the connector of its Source among the
/// registered `connectors`.
pub fn load_review_preview(
    reviews: &dyn ReviewRepositoryPort,
    connectors: &[&dyn ConnectorPort],
    review_item_id: i64,
) -> Result<ReviewPreview, ApplicationError> {
    let item = load_review_item(reviews, review_item_id)?;
    let source_id = item.candidate.source_id.as_str();
    let connector = connectors
        .iter()
        .copied()
        .find(|connector| connector.source_id() == source_id)
        .ok_or_else(|| ApplicationError::PreviewConnectorUnavailable {
            candidate_source_id: source_id.to_owned(),
        })?;
    if let Some(reason) = connector.disabled_reason() {
        return Err(ApplicationError::SourceDisabled {
            source_id: source_id.to_owned(),
            reason,
        });
    }
    if !connector.capabilities().direct_media_download {
        return Err(ApplicationError::ConnectorCannotDownload {
            source_id: connector.source_id().to_owned(),
        });
    }

    let stream = connector.download(&item.candidate)?;
    let mut bytes = Vec::new();
    stream
        .take(MAX_REVIEW_PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| PortError::new(error.to_string()))?;
    if bytes.len() as u64 > MAX_REVIEW_PREVIEW_BYTES {
        return Err(ApplicationError::ReviewPreviewTooLarge {
            review_item_id,
            max_bytes: MAX_REVIEW_PREVIEW_BYTES,
        });
    }
    Ok(ReviewPreview {
        original_filename: item.candidate.original_filename,
        bytes,
    })
}

fn load_review_item(
    reviews: &dyn ReviewRepositoryPort,
    review_item_id: i64,
) -> Result<ReviewItem, ApplicationError> {
    reviews
        .get_review_item(review_item_id)?
        .ok_or(ApplicationError::ReviewItemNotFound(review_item_id))
}
