mod support;

use game_media_vault_application::{
    ApplicationError, ConnectorPort, RunRepositoryPort, acquire_run_with_connectors,
    start_acquisition_run_with_connectors,
};
use game_media_vault_domain::{
    AcquisitionRequestDraft, AcquisitionRun, AcquisitionRunStatus, AssetCandidate, AssetType,
    AssetTypeSelector, ImportedAsset, SourceId, SourceSelection,
};
use support::*;

const SNAPS: &str = "snap-source";

fn screenshot(title: &str) -> AssetCandidate {
    AssetCandidate {
        asset_type: AssetType::Screenshot,
        source_id: SourceId::from(SNAPS),
        source_asset_label: Some("Named_Snaps".to_owned()),
        source_url: format!("https://snaps.invalid/{title}.png"),
        ..candidate(title)
    }
}

fn snap_connector(candidates: Vec<AssetCandidate>) -> FakeConnector {
    FakeConnector {
        source_id: SNAPS,
        asset_types: vec![AssetType::Screenshot],
        ..FakeConnector::new(candidates)
    }
}

/// Box Fronts and Screenshots from whichever Sources provide them.
fn auto_draft() -> AcquisitionRequestDraft {
    AcquisitionRequestDraft {
        sources: SourceSelection::Auto,
        asset_types: vec![AssetTypeSelector::BoxFront, AssetTypeSelector::Screenshot],
        ..request_draft()
    }
}

fn registry<'a>(connectors: &[&'a FakeConnector]) -> Vec<&'a dyn ConnectorPort> {
    connectors
        .iter()
        .map(|connector| *connector as &dyn ConnectorPort)
        .collect()
}

fn start(
    vault: &FakeVault,
    draft: AcquisitionRequestDraft,
    connectors: &[&FakeConnector],
) -> Result<AcquisitionRun, ApplicationError> {
    start_acquisition_run_with_connectors(vault, draft, &registry(connectors))
}

fn execute(
    vault: &FakeVault,
    connectors: &[&FakeConnector],
    run_id: i64,
) -> Result<Vec<ImportedAsset>, ApplicationError> {
    acquire_run_with_connectors(
        vault,
        vault,
        vault,
        &FakeStore::default(),
        &registry(connectors),
        run_id,
        matching_policy(),
    )
}

#[test]
fn an_auto_run_acquires_each_requested_type_from_the_source_that_provides_it() {
    let smb = candidate("Super Mario Bros.");
    let snap = screenshot("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let boxes = FakeConnector::new(vec![smb.clone()]);
    let snaps = snap_connector(vec![snap.clone()]);
    let run = start(&vault, auto_draft(), &[&boxes, &snaps]).unwrap();

    let imported = execute(&vault, &[&boxes, &snaps], run.id).unwrap();

    assert_eq!(imported.len(), 2);
    assert_eq!(boxes.downloads.borrow().as_slice(), [smb.source_url]);
    assert_eq!(snaps.downloads.borrow().as_slice(), [snap.source_url]);
    assert_eq!(vault.run(run.id).status, AcquisitionRunStatus::Completed);
}

#[test]
fn an_explicit_run_executes_only_its_selected_sources() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let boxes = FakeConnector::new(vec![smb]);
    let snaps = snap_connector(vec![screenshot("Super Mario Bros.")]);
    let run = start(&vault, request_draft(), &[&boxes, &snaps]).unwrap();

    execute(&vault, &[&boxes, &snaps], run.id).unwrap();

    assert_eq!(*snaps.discover_calls.borrow(), 0);
    assert_eq!(vault.run(run.id).status, AcquisitionRunStatus::Completed);
}

#[test]
fn a_source_whose_discovery_fails_leaves_the_others_progressing() {
    let smb = candidate("Super Mario Bros.");
    let snap = screenshot("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let failing = FakeConnector {
        discovery_fails: true,
        ..FakeConnector::new(Vec::new())
    };
    let snaps = snap_connector(vec![snap.clone()]);
    let run = start(&vault, auto_draft(), &[&failing, &snaps]).unwrap();

    let error = execute(&vault, &[&failing, &snaps], run.id).unwrap_err();

    assert!(matches!(error, ApplicationError::Port(_)), "{error}");
    assert_eq!(snaps.downloads.borrow().as_slice(), [snap.source_url]);
    assert_eq!(vault.run(run.id).status, AcquisitionRunStatus::Running);

    // The next execution discovers only the Source that failed.
    let boxes = FakeConnector::new(vec![smb.clone()]);
    let resumed_snaps = snap_connector(Vec::new());
    let imported = execute(&vault, &[&boxes, &resumed_snaps], run.id).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(*resumed_snaps.discover_calls.borrow(), 0);
    assert_eq!(boxes.downloads.borrow().as_slice(), [smb.source_url]);
    assert_eq!(vault.run(run.id).status, AcquisitionRunStatus::Completed);
}

#[test]
fn a_run_no_selected_source_can_serve_is_never_persisted() {
    let vault = FakeVault::default();
    let boxes = FakeConnector::new(Vec::new());

    // No registered Source acquires Screenshots.
    let error = start(&vault, auto_draft(), &[&boxes]).unwrap_err();

    assert!(
        matches!(error, ApplicationError::UncoveredAssetTypes { .. }),
        "{error}"
    );
    assert!(vault.list_runs().unwrap().is_empty());
}

#[test]
fn a_run_contacts_only_the_sources_its_plan_kept() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let refusing = FakeConnector {
        source_id: "other-boxes",
        unsupported_reason: Some("no repository for this platform".to_owned()),
        ..FakeConnector::new(Vec::new())
    };
    let boxes = FakeConnector::new(vec![smb]);
    let snaps = snap_connector(vec![screenshot("Super Mario Bros.")]);
    let run = start(&vault, auto_draft(), &[&refusing, &boxes, &snaps]).unwrap();
    // By the first execution, the Source left out accepts and another one is registered.
    let accepting = FakeConnector {
        source_id: "other-boxes",
        ..FakeConnector::new(Vec::new())
    };
    let newcomer = FakeConnector {
        source_id: "new-boxes",
        ..FakeConnector::new(Vec::new())
    };

    let imported = execute(&vault, &[&accepting, &boxes, &snaps, &newcomer], run.id).unwrap();

    assert_eq!(imported.len(), 2);
    assert_eq!(*accepting.discover_calls.borrow(), 0);
    assert_eq!(*newcomer.discover_calls.borrow(), 0);
    assert_eq!(vault.run(run.id).planned_sources, [SOURCE_ID, SNAPS]);
    assert_eq!(vault.run(run.id).status, AcquisitionRunStatus::Completed);
}

#[test]
fn a_planned_source_refusing_at_execution_leaves_the_others_progressing() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let boxes = FakeConnector::new(vec![smb.clone()]);
    let snaps = snap_connector(vec![screenshot("Super Mario Bros.")]);
    let run = start(&vault, auto_draft(), &[&boxes, &snaps]).unwrap();
    let refusing_snaps = FakeConnector {
        unsupported_reason: Some("the snapshot index is gone".to_owned()),
        ..snap_connector(Vec::new())
    };

    let error = execute(&vault, &[&boxes, &refusing_snaps], run.id).unwrap_err();

    assert!(
        matches!(error, ApplicationError::UnsupportedConnectorPlan { .. }),
        "{error}"
    );
    assert_eq!(boxes.downloads.borrow().as_slice(), [smb.source_url]);
    assert_eq!(vault.run(run.id).status, AcquisitionRunStatus::Running);
}

#[test]
fn a_pause_during_a_discovery_keeps_it_but_discovers_no_further() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let boxes = FakeConnector::new(vec![smb]);
    let snaps = snap_connector(vec![screenshot("Super Mario Bros.")]);
    let run = start(&vault, auto_draft(), &[&boxes, &snaps]).unwrap();
    // The run is paused while the first Source is being discovered.
    *vault.status_before_next_discovery.borrow_mut() = Some(AcquisitionRunStatus::Paused);

    execute(&vault, &[&boxes, &snaps], run.id).unwrap();

    assert_eq!(*boxes.discover_calls.borrow(), 1);
    assert_eq!(*snaps.discover_calls.borrow(), 0);
    let paused = vault.run(run.id);
    assert_eq!(paused.status, AcquisitionRunStatus::Paused);
    assert_eq!(paused.queued_work, 1);
}

#[test]
fn a_source_whose_downloads_fail_leaves_the_others_progressing() {
    let smb = candidate("Super Mario Bros.");
    let snap = screenshot("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let mut boxes = FakeConnector::new(vec![smb.clone()]);
    boxes.failing_downloads.insert(smb.source_url.clone());
    let snaps = snap_connector(vec![snap.clone()]);
    let run = start(&vault, auto_draft(), &[&boxes, &snaps]).unwrap();

    // The failing Box Front is the oldest queued work.
    let error = execute(&vault, &[&boxes, &snaps], run.id).unwrap_err();

    assert!(matches!(error, ApplicationError::Port(_)), "{error}");
    assert_eq!(snaps.downloads.borrow().as_slice(), [snap.source_url]);
    let running = vault.run(run.id);
    assert_eq!(running.status, AcquisitionRunStatus::Running);
    assert_eq!((running.queued_work, running.completed_work), (1, 1));
}
