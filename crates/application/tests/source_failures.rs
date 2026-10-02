mod support;

use game_media_vault_application::{
    ConnectorPort, acquire_run_with_connectors, start_acquisition_run_with_connectors,
    summarize_source_failures,
};
use game_media_vault_domain::{
    AcquisitionRequestDraft, AssetCandidate, AssetType, AssetTypeSelector, SourceFailureStage,
    SourceId, SourceSelection,
};
use support::*;

const SNAPS: &str = "snap-source";

fn screenshot(title: &str) -> AssetCandidate {
    AssetCandidate {
        asset_type: AssetType::Screenshot,
        source_id: SourceId::from(SNAPS),
        source_url: format!("https://snaps.invalid/{title}.png"),
        ..candidate(title)
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

fn snap_connector(candidates: Vec<AssetCandidate>) -> FakeConnector {
    FakeConnector {
        source_id: SNAPS,
        asset_types: vec![AssetType::Screenshot],
        ..FakeConnector::new(candidates)
    }
}

/// Starts a run with `connectors`, then executes it `executions` times.
fn run_with(vault: &FakeVault, connectors: &[&FakeConnector], executions: usize) -> i64 {
    let registry: Vec<&dyn ConnectorPort> = connectors
        .iter()
        .map(|connector| *connector as &dyn ConnectorPort)
        .collect();
    let run = start_acquisition_run_with_connectors(vault, auto_draft(), &registry).unwrap();
    for _ in 0..executions {
        let _ = acquire_run_with_connectors(
            vault,
            vault,
            vault,
            &FakeStore::default(),
            &registry,
            run.id,
            matching_policy(),
        );
    }
    run.id
}

#[test]
fn an_execution_records_each_failing_source_with_where_it_failed() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let mut boxes = FakeConnector::new(vec![smb.clone()]);
    boxes.failing_downloads.insert(smb.source_url.clone());
    let snaps = FakeConnector {
        discovery_fails: true,
        ..snap_connector(Vec::new())
    };

    let run_id = run_with(&vault, &[&boxes, &snaps], 1);

    let summaries = summarize_source_failures(&vault, 5).unwrap();
    let described: Vec<(&str, usize, i64, SourceFailureStage, bool)> = summaries
        .iter()
        .map(|summary| {
            let latest = &summary.latest[0];
            (
                summary.source_id.as_str(),
                summary.failures,
                latest.run_id,
                latest.stage,
                latest.message.contains("fixture"),
            )
        })
        .collect();
    assert_eq!(
        described,
        [
            (SOURCE_ID, 1, run_id, SourceFailureStage::Download, true),
            (SNAPS, 1, run_id, SourceFailureStage::Discovery, true),
        ]
    );
}

#[test]
fn the_summary_counts_every_failure_and_keeps_the_latest_of_each_source() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let boxes = FakeConnector::new(vec![smb]);
    let snaps = FakeConnector {
        discovery_fails: true,
        ..snap_connector(vec![screenshot("Super Mario Bros.")])
    };

    run_with(&vault, &[&boxes, &snaps], 3);

    let summaries = summarize_source_failures(&vault, 2).unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].source_id, SNAPS);
    assert_eq!(summaries[0].failures, 3);
    assert_eq!(summaries[0].latest.len(), 2);
    // Newest first.
    assert!(summaries[0].latest[0].sequence > summaries[0].latest[1].sequence);
}

#[test]
fn sources_that_work_record_no_failure() {
    let smb = candidate("Super Mario Bros.");
    let vault = FakeVault::with_library(vec![release_for(&smb, 73)]);
    let boxes = FakeConnector::new(vec![smb]);
    let snaps = snap_connector(vec![screenshot("Super Mario Bros.")]);

    run_with(&vault, &[&boxes, &snaps], 1);

    assert!(summarize_source_failures(&vault, 5).unwrap().is_empty());
}
