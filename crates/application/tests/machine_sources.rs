mod support;

use game_media_vault_application::{
    ApplicationError, ConnectorPort, DownloadLimits, ExcludedSource, acquire_run_with_connectors,
    machine_connectors, machine_registry, plan_acquisition, start_acquisition_run_with_connectors,
};
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRunStatus, AssetCandidate, SourceId,
    SourceSelection,
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
        matches!(error, ApplicationError::UnsupportedConnectorPlan { .. }),
        "{error}"
    );
    assert_eq!(*later_disabled.discover_calls.borrow(), 0);
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
