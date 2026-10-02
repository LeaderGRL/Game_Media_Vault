use game_media_vault_domain::{
    AcquisitionRequestValidationError, AcquisitionRunStatus, AssetTypeSelector,
    MatchingPolicyValidationError, ReviewStatus,
};
use thiserror::Error;

use crate::{ExcludedSource, PortError};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApplicationError {
    #[error("source path does not contain a file name")]
    MissingSourceFileName,
    #[error("failed to resolve source path: {0}")]
    ResolveSourcePath(String),
    #[error("reference catalog imports require a positive game limit")]
    InvalidReferenceImportLimit,
    #[error("thumbnails require a positive longest edge")]
    InvalidThumbnailEdge,
    #[error("a repair needs at least one action")]
    NoRepairAction,
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
    #[error("connector {source_id} does not support direct media downloads")]
    ConnectorCannotDownload { source_id: String },
    #[error(
        "connector {connector_source_id} returned a candidate belonging to source {candidate_source_id}"
    )]
    ConnectorCandidateSourceMismatch {
        connector_source_id: String,
        candidate_source_id: String,
    },
    #[error("previews of {candidate_source_id} candidates need a connector for that source")]
    PreviewConnectorUnavailable { candidate_source_id: String },
    #[error(
        "connector {source_id} returned a candidate whose locator is not an absolute URL free of userinfo, query and fragment"
    )]
    UnsafeCandidateLocator { source_id: String },
    #[error("acquisition run cannot execute connector work while {status:?}")]
    RunNotExecutable { status: AcquisitionRunStatus },
    #[error(
        "acquisition request documents of format version {found} are not supported (this version reads {supported})"
    )]
    UnsupportedDocumentVersion { found: u32, supported: u32 },
    #[error("connector {source_id} cannot execute this acquisition plan: {reason}")]
    UnsupportedConnectorPlan { source_id: String, reason: String },
    #[error("the engine cannot execute this acquisition request yet: {0}")]
    UnsupportedRequest(&'static str),
    #[error(
        "no selected source acquires {}{}",
        selector_names(uncovered),
        excluded_reasons(excluded)
    )]
    UncoveredAssetTypes {
        uncovered: Vec<AssetTypeSelector>,
        excluded: Vec<ExcludedSource>,
    },
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
    #[error("review item #{review_item_id} media exceeds the {max_bytes}-byte preview limit")]
    ReviewPreviewTooLarge { review_item_id: i64, max_bytes: u64 },
}

/// Stable category of an application error, shared by the CLI (exit codes) and the desktop
/// shell (structured errors) so frontends never parse messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The request itself is invalid; retrying it unchanged cannot succeed.
    InvalidRequest,
    /// A referenced run, Review Item or Release Edition does not exist.
    NotFound,
    /// The current state forbids the operation, e.g. an already decided Review Item.
    Conflict,
    /// The operation is valid but not implemented for this Source or plan yet.
    Unsupported,
    /// A Source returned data that breaks the connector contract.
    SourceFailure,
    /// Storage, network or another port failed.
    External,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::Unsupported => "unsupported",
            Self::SourceFailure => "source_failure",
            Self::External => "external",
        }
    }
}

impl ApplicationError {
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::MissingSourceFileName
            | Self::ResolveSourcePath(_)
            | Self::InvalidReferenceImportLimit
            | Self::InvalidThumbnailEdge
            | Self::NoRepairAction
            | Self::Validation(_)
            | Self::InvalidMatchingPolicy(_)
            | Self::ReviewAcceptanceNotCompeting { .. } => ErrorKind::InvalidRequest,
            Self::RunNotFound(_) | Self::ReviewItemNotFound(_) | Self::ReleaseEditionMissing(_) => {
                ErrorKind::NotFound
            }
            Self::RunHasQueuedWork { .. }
            | Self::InvalidRunTransition { .. }
            | Self::RunNotExecutable { .. }
            | Self::ReviewItemNotActionable { .. }
            | Self::ReviewItemContended(_) => ErrorKind::Conflict,
            Self::ConnectorCannotDownload { .. }
            | Self::UnsupportedConnectorPlan { .. }
            | Self::UnsupportedDocumentVersion { .. }
            | Self::UnsupportedRequest(_)
            | Self::UncoveredAssetTypes { .. }
            | Self::PreviewConnectorUnavailable { .. }
            | Self::ReviewPreviewTooLarge { .. } => ErrorKind::Unsupported,
            Self::ConnectorCandidateSourceMismatch { .. } | Self::UnsafeCandidateLocator { .. } => {
                ErrorKind::SourceFailure
            }
            Self::Port(error) if error.is_invalid_source_data() => ErrorKind::SourceFailure,
            Self::Port(_) => ErrorKind::External,
        }
    }
}

fn selector_names(selectors: &[AssetTypeSelector]) -> String {
    let names: Vec<String> = selectors
        .iter()
        .map(|selector| format!("{selector:?}"))
        .collect();
    names.join(", ")
}

/// Why each excluded Source was left out, in parentheses, or nothing when none was selected.
fn excluded_reasons(excluded: &[ExcludedSource]) -> String {
    if excluded.is_empty() {
        return String::new();
    }
    let reasons: Vec<String> = excluded
        .iter()
        .map(|source| format!("{}: {}", source.source_id, source.reason))
        .collect();
    format!(" ({})", reasons.join("; "))
}
