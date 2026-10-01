use game_media_vault_application::{
    AcquisitionRequestValidationError, ApplicationError, ErrorKind, PortError,
};
use game_media_vault_domain::{AcquisitionRunStatus, ReviewStatus};

#[test]
fn application_errors_expose_a_stable_kind_for_frontends() {
    let cases = [
        (
            ApplicationError::Validation(AcquisitionRequestValidationError::MissingSources),
            ErrorKind::InvalidRequest,
            "invalid_request",
        ),
        (
            ApplicationError::RunNotFound(4),
            ErrorKind::NotFound,
            "not_found",
        ),
        (
            ApplicationError::ReviewItemNotActionable {
                review_item_id: 1,
                status: ReviewStatus::Rejected,
            },
            ErrorKind::Conflict,
            "conflict",
        ),
        (
            ApplicationError::RunNotExecutable {
                status: AcquisitionRunStatus::Paused,
            },
            ErrorKind::Conflict,
            "conflict",
        ),
        (
            ApplicationError::UnsupportedConnectorPlan {
                source_id: "fixture".to_owned(),
                reason: "limits".to_owned(),
            },
            ErrorKind::Unsupported,
            "unsupported",
        ),
        (
            ApplicationError::PreviewConnectorUnavailable {
                connector_source_id: "libretro-thumbnails".to_owned(),
                candidate_source_id: "fixture".to_owned(),
            },
            ErrorKind::Unsupported,
            "unsupported",
        ),
        (
            ApplicationError::UnsafeCandidateLocator {
                source_id: "fixture".to_owned(),
            },
            ErrorKind::SourceFailure,
            "source_failure",
        ),
        (
            ApplicationError::Port(PortError("disk full".to_owned())),
            ErrorKind::External,
            "external",
        ),
    ];

    for (error, kind, name) in cases {
        assert_eq!(error.kind(), kind, "{error}");
        assert_eq!(kind.as_str(), name);
    }
}
