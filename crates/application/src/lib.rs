//! Use cases of Game Media Vault, shared by the CLI and the Tauri shell.

mod acquisition;
mod error;
mod identity;
mod imports;
mod ports;
mod review;
mod runs;

pub use acquisition::acquire_run_with_connector;
pub use error::ApplicationError;
pub use game_media_vault_domain::{
    AcquisitionRequestDraft as AcquisitionRequestInput, AcquisitionRequestValidationError,
    match_asset_candidate_to_release, review_matches_for_asset_candidate,
};
pub use identity::candidate_identity;
pub use imports::{
    ImportLocalBoxFrontRequest, ImportReferenceCatalogRequest, ReferenceImportSummary,
    import_local_box_front, import_reference_catalog, list_library,
};
pub use ports::{
    CatalogPort, ConnectorPort, ObjectStorePort, ParkedReview, PortError,
    ReferenceCatalogRepositoryPort, ReferenceCatalogSourcePort, ReviewRepositoryPort,
    RunRepositoryPort,
};
pub use review::{ReviewPreview, list_review_items, load_review_preview, resolve_review_item};
pub use runs::{
    build_acquisition_request, cancel_acquisition_run, complete_acquisition_run,
    list_acquisition_runs, load_acquisition_run, pause_acquisition_run, resume_acquisition_run,
    start_acquisition_run,
};
