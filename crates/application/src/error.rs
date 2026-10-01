use game_media_vault_domain::{
    AcquisitionRequestValidationError, AcquisitionRunStatus, MatchingPolicyValidationError,
    ReviewStatus,
};
use thiserror::Error;

use crate::PortError;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApplicationError {
    #[error("source path does not contain a file name")]
    MissingSourceFileName,
    #[error("failed to resolve source path: {0}")]
    ResolveSourcePath(String),
    #[error("reference catalog imports require a positive game limit")]
    InvalidReferenceImportLimit,
    #[error("{0}")]
    Port(#[from] PortError),
    #[error("{0}")]
    Validation(#[from] AcquisitionRequestValidationError),
    #[error("{0}")]
    InvalidMatchingPolicy(#[from] MatchingPolicyValidationError),
    #[error("acquisition run #{0} does not exist")]
    RunNotFound(i64),
    #[error("acquisition run still has {queued_work} queued work item(s)")]
    RunHasQueuedWork { queued_work: u64 },
    #[error("cannot transition acquisition run from {from:?} to {to:?}")]
    InvalidRunTransition {
        from: AcquisitionRunStatus,
        to: AcquisitionRunStatus,
    },
    #[error("connector {source_id} is not selected by this acquisition request")]
    ConnectorNotSelected { source_id: String },
    #[error("connector {source_id} does not support direct media downloads")]
    ConnectorCannotDownload { source_id: String },
    #[error(
        "connector {connector_source_id} returned a candidate belonging to source {candidate_source_id}"
    )]
    ConnectorCandidateSourceMismatch {
        connector_source_id: String,
        candidate_source_id: String,
    },
    #[error(
        "connector {source_id} returned a candidate whose locator is not an absolute URL free of userinfo, query and fragment"
    )]
    UnsafeCandidateLocator { source_id: String },
    #[error("acquisition run cannot execute connector work while {status:?}")]
    RunNotExecutable { status: AcquisitionRunStatus },
    #[error("connector {source_id} cannot execute this acquisition plan: {reason}")]
    UnsupportedConnectorPlan { source_id: String, reason: String },
    #[error("review item #{0} does not exist")]
    ReviewItemNotFound(i64),
    #[error(
        "release edition #{release_edition_id} is not a competing release for review item #{review_item_id}"
    )]
    ReviewAcceptanceNotCompeting {
        review_item_id: i64,
        release_edition_id: i64,
    },
    #[error("review item #{review_item_id} cannot be resolved while {status:?}")]
    ReviewItemNotActionable {
        review_item_id: i64,
        status: ReviewStatus,
    },
    #[error("release edition #{0} is not in the library")]
    ReleaseEditionMissing(i64),
    #[error("review item #{0} kept changing while acquisition processed its candidate")]
    ReviewItemContended(i64),
}
