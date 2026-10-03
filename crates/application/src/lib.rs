//! Use cases of Game Media Vault, shared by the CLI and the Tauri shell.

mod acquisition;
mod credentials;
mod derived;
mod documents;
mod error;
mod export;
mod identity;
mod imports;
mod library;
mod packaging;
mod plan;
mod platforms;
mod ports;
mod review;
mod runs;
mod sources;
mod verify;

pub use acquisition::{
    DownloadLimits, acquire_run_with_connectors, start_acquisition_run_with_connectors,
};
pub use credentials::{
    API_KEY_FIELD, ApiKey, CredentialField, CredentialFieldState, CredentialState,
};
pub use derived::{
    DerivationFailure, DerivationSummary, DerivativeRepositoryPort, DerivedStorePort,
    MediaTransformPort, OriginalObject, derive_assets,
};
pub use documents::{
    ACQUISITION_REQUEST_DOCUMENT_VERSION, AcquisitionRequestDocument, draft_from_document,
    export_acquisition_request,
};
pub use error::{ApplicationError, ErrorKind};
pub use export::{ExportSummary, ExportTargetPort, export_library};
pub use game_media_vault_domain::{
    AcquisitionRequestDraft as AcquisitionRequestInput, AcquisitionRequestValidationError,
    match_asset_candidate_to_release, match_asset_candidate_to_release_preferring,
    review_matches_for_asset_candidate,
};
pub use identity::candidate_identity;
pub use imports::{
    DescribedReferenceReviewItem, ImportLocalAssetRequest, ImportReferenceCatalogRequest,
    PlatformCatalogSummary, ReferenceImportSummary, import_local_asset, import_local_box_front,
    import_reference_catalog, keep_reference_review_item_apart, link_reference_review_item,
    list_library, list_reference_review_items, sync_platform_catalog,
};
pub use library::{
    DEFAULT_LIBRARY_PAGE_SIZE, LatestMedium, LibraryPage, LibraryQuery, LibraryStatus,
    MAX_LIBRARY_PAGE_SIZE, latest_media, search_library,
};
pub use packaging::{
    IncompletePackaging, PackagingModelFailure, PackagingModelPort, PackagingModelSummary,
    PackagingScan, PackagingScans, derive_packaging_models,
};
pub use plan::{
    AcquisitionPlan, ExcludedSource, PlannedSource, SelectorCoverage, plan_acquisition,
};
pub use platforms::{expand_every_game, plan_acquisition_request};
pub use ports::{
    CandidateAssetOutcome, CatalogPort, ConnectorPort, CredentialStorePort, MachineSettingsPort,
    ObjectStorePort, ParkedReview, PlatformCatalogSourcePort, PortError, ReferenceCatalogRead,
    ReferenceCatalogRepositoryPort, ReferenceCatalogSourcePort, ReferenceReviewOutcome,
    ReferenceReviewRepositoryPort, ReviewDecisionOutcome, ReviewRepositoryPort, RunRepositoryPort,
};
pub use review::{
    MAX_REVIEW_PREVIEW_BYTES, PendingReviewDecision, PendingReviewSummary, ReviewPage,
    ReviewPreview, decide_pending_reviews, list_review_items, load_review_preview,
    resolve_review_item, review_page,
};
pub use runs::{
    build_acquisition_request, cancel_acquisition_run, complete_acquisition_run,
    list_acquisition_runs, load_acquisition_run, pause_acquisition_run, resume_acquisition_run,
    start_acquisition_run,
};
pub use sources::{
    Machine, MachineConnector, MachineConnectors, SourceDescription, SourceFailureSummary,
    clear_source_api_key, clear_source_credential, describe_sources, machine_connectors,
    machine_registry, set_source_api_key, set_source_credential, set_source_enabled,
    summarize_source_failures,
};
pub use verify::{
    CorruptObject, ObjectArea, ObjectCheck, RecordedDerivative, RepairActions, RepairSummary,
    StaleWork, StaleWorkReason, UnfinishedWork, UnreadableObject, VaultCatalogPort,
    VaultRepairCatalogPort, VaultRepairStorePort, VaultReport, VaultStorePort, repair_vault,
    verify_vault,
};
