use std::io::Read;

use game_media_vault_domain::{ReviewDecision, ReviewItem};

use crate::{
    ApplicationError, ConnectorPort, PortError, ReviewDecisionOutcome, ReviewRepositoryPort,
};

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

/// Downloads the candidate media of a Review Item through its Connector.
pub fn load_review_preview(
    reviews: &dyn ReviewRepositoryPort,
    connector: &dyn ConnectorPort,
    review_item_id: i64,
) -> Result<ReviewPreview, ApplicationError> {
    let item = load_review_item(reviews, review_item_id)?;
    if item.candidate.source_id.as_str() != connector.source_id() {
        return Err(ApplicationError::PreviewConnectorUnavailable {
            connector_source_id: connector.source_id().to_owned(),
            candidate_source_id: item.candidate.source_id.as_str().to_owned(),
        });
    }
    if !connector.capabilities().direct_media_download {
        return Err(ApplicationError::ConnectorCannotDownload {
            source_id: connector.source_id().to_owned(),
        });
    }

    let mut stream = connector.download(&item.candidate)?;
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .map_err(|error| PortError(error.to_string()))?;
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
