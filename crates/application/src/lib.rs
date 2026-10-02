//! Use cases of Game Media Vault, shared by the CLI and the Tauri shell.

mod acquisition;
mod derived;
mod documents;
mod error;
mod identity;
mod imports;
mod library;
mod plan;
mod ports;
mod review;
mod runs;

pub use acquisition::{acquire_run_with_connector, start_acquisition_run_for_connector};
pub use derived::{
    DerivationFailure, DerivationSummary, DerivativeRepositoryPort, DerivedStorePort,
    MediaTransformPort, OriginalObject, derive_assets,
};
pub use documents::{
    ACQUISITION_REQUEST_DOCUMENT_VERSION, AcquisitionRequestDocument, draft_from_document,
    export_acquisition_request,
};
pub use error::{ApplicationError, ErrorKind};
pub use game_media_vault_domain::{
    AcquisitionRequestDraft as AcquisitionRequestInput, AcquisitionRequestValidationError,
    match_asset_candidate_to_release, review_matches_for_asset_candidate,
};
pub use identity::candidate_identity;
pub use imports::{
    ImportLocalBoxFrontRequest, ImportReferenceCatalogRequest, ReferenceImportSummary,
    import_local_box_front, import_reference_catalog, list_library,
};
pub use library::{
    DEFAULT_LIBRARY_PAGE_SIZE, LibraryPage, LibraryQuery, LibraryStatus, MAX_LIBRARY_PAGE_SIZE,
    search_library,
};
pub use plan::{
    AcquisitionPlan, ExcludedSource, PlannedSource, SelectorCoverage, plan_acquisition,
};
pub use ports::{
    CandidateAssetOutcome, CatalogPort, ConnectorPort, ObjectStorePort, ParkedReview, PortError,
    ReferenceCatalogRepositoryPort, ReferenceCatalogSourcePort, ReviewDecisionOutcome,
    ReviewRepositoryPort, RunRepositoryPort,
};
pub use review::{
    MAX_REVIEW_PREVIEW_BYTES, ReviewPreview, list_review_items, load_review_preview,
    resolve_review_item,
};
pub use runs::{
    build_acquisition_request, cancel_acquisition_run, complete_acquisition_run,
    list_acquisition_runs, load_acquisition_run, pause_acquisition_run, resume_acquisition_run,
    start_acquisition_run,
};
