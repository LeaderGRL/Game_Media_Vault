mod support;

use game_media_vault_application::{
    ACQUISITION_REQUEST_DOCUMENT_VERSION, AcquisitionRequestDocument, ApplicationError, ErrorKind,
    RunRepositoryPort, draft_from_document, export_acquisition_request,
};
use support::*;

#[test]
fn a_run_exports_its_request_as_a_versioned_document() {
    let vault = FakeVault::default();
    let run = vault.create_run(request()).unwrap();

    let document = export_acquisition_request(&vault, run.id).unwrap();

    assert_eq!(
        document,
        AcquisitionRequestDocument {
            format_version: ACQUISITION_REQUEST_DOCUMENT_VERSION,
            request: request_draft(),
        }
    );
    // The document holds the request alone: no run state, Source credentials or vault paths.
    let json = serde_json::to_value(&document).unwrap();
    assert_eq!(
        json.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["format_version", "request"]
    );
}

#[test]
fn an_exported_document_reads_back_as_the_same_draft() {
    let vault = FakeVault::default();
    let run = vault.create_run(request()).unwrap();
    let json = serde_json::to_string(&export_acquisition_request(&vault, run.id).unwrap()).unwrap();

    let draft = draft_from_document(serde_json::from_str(&json).unwrap()).unwrap();

    assert_eq!(draft, request_draft());
}

#[test]
fn a_document_of_another_format_version_is_refused() {
    let error = draft_from_document(AcquisitionRequestDocument {
        format_version: ACQUISITION_REQUEST_DOCUMENT_VERSION + 1,
        request: request_draft(),
    })
    .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Unsupported);
}

#[test]
fn exporting_an_unknown_run_is_not_found() {
    let error = export_acquisition_request(&FakeVault::default(), 404).unwrap_err();

    assert!(
        matches!(error, ApplicationError::RunNotFound(404)),
        "{error}"
    );
}
