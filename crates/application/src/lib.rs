//! Use cases of Game Media Vault, shared by the CLI and the Tauri shell.

mod acquisition;
mod credentials;
mod derived;
mod documents;
mod error;
mod identity;
mod imports;
mod library;
mod packaging;
mod plan;
mod ports;
mod review;
mod runs;
mod sources;
mod verify;

pub use acquisition::{
    DownloadLimits, acquire_run_with_connectors, start_acquisition_run_with_connectors,
};
pub use credentials::{ApiKey, CredentialState};
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
    DescribedReferenceReviewItem, ImportLocalBoxFrontRequest, ImportReferenceCatalogRequest,
    ReferenceImportSummary, import_local_box_front, import_reference_catalog,
    keep_reference_review_item_apart, link_reference_review_item, list_library,
    list_reference_review_items,
};
pub use library::{
    DEFAULT_LIBRARY_PAGE_SIZE, LibraryPage, LibraryQuery, LibraryStatus, MAX_LIBRARY_PAGE_SIZE,
    search_library,
};
pub use packaging::{
    IncompletePackaging, PackagingModelFailure, PackagingModelPort, PackagingModelSummary,
    PackagingScan, PackagingScans, derive_packaging_models,
};
pub use plan::{
    AcquisitionPlan, ExcludedSource, PlannedSource, SelectorCoverage, plan_acquisition,
};
pub use ports::{
    CandidateAssetOutcome, CatalogPort, ConnectorPort, CredentialStorePort, MachineSettingsPort,
    ObjectStorePort, ParkedReview, PortError, ReferenceCatalogRead, ReferenceCatalogRepositoryPort,
    ReferenceCatalogSourcePort, ReferenceReviewOutcome, ReferenceReviewRepositoryPort,
    ReviewDecisionOutcome, ReviewRepositoryPort, RunRepositoryPort,
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
pub use sources::{
    Machine, MachineConnector, MachineConnectors, SourceDescription, SourceFailureSummary,
    clear_source_api_key, describe_sources, machine_connectors, machine_registry,
    set_source_api_key, set_source_enabled, summarize_source_failures,
};
pub use verify::{
    CorruptObject, ObjectArea, ObjectCheck, RecordedDerivative, RepairActions, RepairSummary,
    StaleWork, StaleWorkReason, UnfinishedWork, UnreadableObject, VaultCatalogPort,
    VaultRepairCatalogPort, VaultRepairStorePort, VaultReport, VaultStorePort, repair_vault,
    verify_vault,
};
