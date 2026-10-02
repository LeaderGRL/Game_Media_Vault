use game_media_vault_domain::AcquisitionRequestDraft;
use serde::{Deserialize, Serialize};

use crate::{ApplicationError, RunRepositoryPort, load_acquisition_run};

/// Format of the Acquisition Request documents this version writes and reads.
pub const ACQUISITION_REQUEST_DOCUMENT_VERSION: u32 = 1;

/// A portable Acquisition Request: the request alone, without run state, Source credentials or
/// vault paths, so another machine can start the same acquisition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcquisitionRequestDocument {
    pub format_version: u32,
    pub request: AcquisitionRequestDraft,
}

/// The request of run `run_id` as a document.
pub fn export_acquisition_request(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRequestDocument, ApplicationError> {
    let run = load_acquisition_run(runs, run_id)?;
    Ok(AcquisitionRequestDocument {
        format_version: ACQUISITION_REQUEST_DOCUMENT_VERSION,
        request: run.request.to_draft(),
    })
}

/// The draft a document holds, which starting a run validates like any other.
pub fn draft_from_document(
    document: AcquisitionRequestDocument,
) -> Result<AcquisitionRequestDraft, ApplicationError> {
    if document.format_version != ACQUISITION_REQUEST_DOCUMENT_VERSION {
        return Err(ApplicationError::UnsupportedDocumentVersion {
            found: document.format_version,
            supported: ACQUISITION_REQUEST_DOCUMENT_VERSION,
        });
    }
    Ok(document.request)
}
