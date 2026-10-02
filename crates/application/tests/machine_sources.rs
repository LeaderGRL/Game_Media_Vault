mod support;

use game_media_vault_application::{
    ApplicationError, ConnectorPort, DownloadLimits, ErrorKind, ExcludedSource, RunRepositoryPort,
    acquire_run_with_connectors, candidate_identity, load_review_preview, machine_connectors,
    machine_registry, plan_acquisition, start_acquisition_run_with_connectors,
};
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRunStatus, AcquisitionWorkItem,
    AssetCandidate, AssetType, ReviewItem, ReviewStatus, SourceId, SourceSelection,
};
use support::*;

const OTHER_SOURCE: &str = "other-source";

fn other(candidates: Vec<AssetCandidate>) -> FakeConnector {
    FakeConnector {
        source_id: OTHER_SOURCE,
        ..FakeConnector::new(candidates)
    }
}

fn request_of(sources: SourceSelection) -> AcquisitionRequest {
    AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources,
        ..request_draft()
    })
    .unwrap()
}

#[test]
fn a_source_disabled_on_this_machine_is_left_out_of_plans_without_being_consulted() {
    let boxes = FakeConnector::new(Vec::new());
    // Consulting it would fail the plan.
    let disabled = FakeConnector {
        plan_check_fails: true,
        ..other(Vec::new())
    };
    let registered: Vec<&dyn ConnectorPort> = vec![&boxes, &disabled];
    let machine = machine_connectors(&registered, &[OTHER_SOURCE.to_owned()]);

    let plan = plan_acquisition(&request_of(SourceSelection::Auto), &machine.refs()).unwrap();

    assert_eq!(
        plan.sources
            .iter()
            .map(|source| source.source_id.as_str())
            .collect::<Vec<_>>(),
        [SOURCE_ID]
    );
    assert_eq!(
        plan.excluded,
        [ExcludedSource {
            source_id: OTHER_SOURCE.to_owned(),
            reason: "disabled on this machine".to_owned(),
        }]
    );
}

#[test]
fn a_request_selecting_a_disabled_source_is_refused_with_why() {
    let disabled = other(Vec::new());
    let registered: Vec<&dyn ConnectorPort> = vec![&disabled];
    let machine = machine_connectors(&registered, &[OTHER_SOURCE.to_owned()]);

    let error = plan_acquisition(
        &request_of(SourceSelection::Explicit(vec![OTHER_SOURCE.to_owned()])),
        &machine.refs(),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("disabled on this machine"),
        "{error}"
    );
}

#[test]
fn an_execution_never_contacts_a_source_disabled_since_its_run_started() {
    let mario = AssetCandidate {
        source_id: SourceId::from(OTHER_SOURCE),
        ..candidate("Super Mario Bros.")
    };
    let vault = FakeVault::with_library(vec![release_for(&mario, 1)]);
    let boxes = FakeConnector::new(Vec::new());
    let later_disabled = other(vec![mario]);
    let registered: Vec<&dyn ConnectorPort> = vec![&boxes, &later_disabled];
    let run = start_acquisition_run_with_connectors(
        &vault,
        AcquisitionRequestDraft {
            sources: SourceSelection::Auto,
            ..request_draft()
        },
        &registered,
    )
    .unwrap();
    let execute = |disabled_sources: &[String]| {
        let machine = machine_connectors(&registered, disabled_sources);
        acquire_run_with_connectors(
            &vault,
            &vault,
            &vault,
            &FakeStore::default(),
            &machine.refs(),
            run.id,
            matching_policy(),
            DownloadLimits::default(),
        )
    };

    let error = execute(&[OTHER_SOURCE.to_owned()]).unwrap_err();

    assert!(
        matches!(error, ApplicationError::SourceDisabled { .. }),
        "{error}"
    );
    assert_eq!(*later_disabled.discover_calls.borrow(), 0);
    // Disabling a Source is a choice of this machine, no failure of the Source.
    assert!(vault.source_failures.borrow().is_empty());
    assert_eq!(vault.run(run.id).status, AcquisitionRunStatus::Running);
    // Enabled again, the Source is discovered by the next execution.
    assert_eq!(execute(&[]).unwrap().len(), 1);
}

#[test]
fn an_owned_registry_keeps_the_sources_disabled_on_this_machine_out_of_plans() {
    let registry: Vec<Box<dyn ConnectorPort>> = vec![
        Box::new(FakeConnector::new(Vec::new())),
        Box::new(other(Vec::new())),
    ];

    let machine = machine_registry(registry, &[OTHER_SOURCE.to_owned()]);
    let connectors: Vec<&dyn ConnectorPort> =
        machine.iter().map(|connector| connector.as_ref()).collect();
    let plan = plan_acquisition(&request_of(SourceSelection::Auto), &connectors).unwrap();

    assert_eq!(plan.sources.len(), 1);
    assert_eq!(plan.excluded[0].source_id, OTHER_SOURCE);
    assert_eq!(plan.excluded[0].reason, "disabled on this machine");
}

#[test]
fn work_a_disabled_source_already_queued_waits_untouched() {
    // Bound for review: processing it would download nothing.
    let (ambiguous, releases) = ambiguous_candidate_and_releases();
    let ambiguous = AssetCandidate {
        source_id: SourceId::from(OTHER_SOURCE),
        ..ambiguous
    };
    let vault = FakeVault::with_library(releases);
    let boxes = FakeConnector::new(Vec::new());
    let disabled = other(Vec::new());
    let registered: Vec<&dyn ConnectorPort> = vec![&boxes, &disabled];
    let run = start_acquisition_run_with_connectors(
        &vault,
        AcquisitionRequestDraft {
            sources: SourceSelection::Auto,
            ..request_draft()
        },
        &registered,
    )
    .unwrap();
    // The Source was discovered before this machine disabled it.
    vault
        .record_discovery(
            run.id,
            OTHER_SOURCE,
            &[AcquisitionWorkItem {
                key: candidate_identity(OTHER_SOURCE, &ambiguous),
                candidate: ambiguous,
            }],
        )
        .unwrap();
    let machine = machine_connectors(&registered, &[OTHER_SOURCE.to_owned()]);

    let error = acquire_run_with_connectors(
        &vault,
        &vault,
        &vault,
        &FakeStore::default(),
        &machine.refs(),
        run.id,
        matching_policy(),
        DownloadLimits::default(),
    )
    .unwrap_err();

    assert!(
        matches!(error, ApplicationError::SourceDisabled { .. }),
        "{error}"
    );
    assert!(vault.review_items.borrow().is_empty());
    let run = vault.run(run.id);
    assert_eq!(run.status, AcquisitionRunStatus::Running);
    assert_eq!(run.queued_work, 1);
}

#[test]
fn a_disabled_source_is_left_out_as_disabled_whatever_it_acquires() {
    let boxes = FakeConnector::new(Vec::new());
    // It acquires none of the requested types, yet its exclusion says it is disabled.
    let disabled = FakeConnector {
        asset_types: vec![AssetType::Screenshot],
        ..other(Vec::new())
    };
    let registered: Vec<&dyn ConnectorPort> = vec![&boxes, &disabled];
    let machine = machine_connectors(&registered, &[OTHER_SOURCE.to_owned()]);

    let plan = plan_acquisition(&request_of(SourceSelection::Auto), &machine.refs()).unwrap();

    assert_eq!(
        plan.excluded,
        [ExcludedSource {
            source_id: OTHER_SOURCE.to_owned(),
            reason: "disabled on this machine".to_owned(),
        }]
    );
}

#[test]
fn a_review_preview_of_a_disabled_source_is_refused_as_unsupported() {
    let candidate = AssetCandidate {
        source_id: SourceId::from(OTHER_SOURCE),
        ..candidate("Review Game")
    };
    let vault = FakeVault::default();
    vault.review_items.borrow_mut().push(ReviewItem {
        id: 1,
        candidate_identity: candidate_identity(OTHER_SOURCE, &candidate),
        candidate,
        competing_matches: Vec::new(),
        decision: None,
        status: ReviewStatus::Pending,
    });
    let disabled = other(Vec::new());
    let registered: Vec<&dyn ConnectorPort> = vec![&disabled];
    let machine = machine_connectors(&registered, &[OTHER_SOURCE.to_owned()]);

    let error = load_review_preview(&vault, &machine.refs(), 1).unwrap_err();

    assert!(
        matches!(error, ApplicationError::SourceDisabled { .. }),
        "{error}"
    );
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(disabled.downloads.borrow().is_empty());
}
