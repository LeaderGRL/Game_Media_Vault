use std::io::Read;

use game_media_vault_domain::{ReviewDecision, ReviewItem};

use crate::{
    ApplicationError, ConnectorPort, PortError, ReviewDecisionOutcome, ReviewRepositoryPort,
};

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
