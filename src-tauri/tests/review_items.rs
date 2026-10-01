use std::{
    cell::RefCell,
    io::{Cursor, Read},
};

use game_media_vault_application::{
    AcquisitionRequestInput, ConnectorPort, ParkedReview, PortError, ReviewRepositoryPort,
    RunRepositoryPort, candidate_identity, load_acquisition_run, start_acquisition_run,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate, AssetType,
    AssetTypeSelector, ConnectorCapabilities, GameSelection, MatchEvidence, MatchSignal,
    NewReviewItem, RetentionPolicy, ReviewDecision, ReviewMatchCandidate, SourceId,
    SourceSelection,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::tempdir;

struct PreviewConnector {
    downloads: RefCell<Vec<String>>,
}

impl ConnectorPort for PreviewConnector {
    fn source_id(&self) -> &'static str {
        "fixture-provider"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::BoxFront],
            direct_media_download: true,
        }
    }

    fn discover(
        &self,
        _request: &game_media_vault_domain::AcquisitionRequest,
    ) -> Result<Vec<AssetCandidate>, PortError> {
        Ok(Vec::new())
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        self.downloads
            .borrow_mut()
            .push(candidate.source_url.clone());
        Ok(Box::new(Cursor::new(b"preview bytes".to_vec())))
    }
}

fn candidate(title: &str) -> AssetCandidate {
    AssetCandidate {
        provider_candidate_id: None,
        game_title: title.to_owned(),
        platform: "Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Collector".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("fixture-provider"),
        source_asset_label: Some("front".to_owned()),
        source_url: format!("https://example.invalid/{title}/front.png"),
        original_filename: "front.png".to_owned(),
    }
}

/// Starts a run whose only discovered candidate is parked on a new Review Item.
fn seed_parked_review(catalog: &SqliteCatalog, candidate: AssetCandidate) -> (i64, i64) {
    let run = start_acquisition_run(
        catalog,
        AcquisitionRequestInput {
            sources: SourceSelection::Explicit(vec!["fixture-provider".to_owned()]),
            platforms: vec!["Nintendo Entertainment System".to_owned()],
            games: GameSelection::Explicit(vec![candidate.game_title.clone()]),
            regions: Vec::new(),
            languages: Vec::new(),
            asset_types: vec![AssetTypeSelector::BoxFront],
            quality: None,
            retention: RetentionPolicy::KeepEverything,
            limits: AcquisitionLimits::default(),
        },
    )
    .unwrap();
    let key = candidate_identity("fixture-provider", &candidate);
    catalog
        .record_discovery(
            run.id,
            "fixture-provider",
            &[AcquisitionWorkItem {
                key: key.clone(),
                candidate: candidate.clone(),
            }],
        )
        .unwrap();
    let parked = catalog
        .park_work_for_review(
            run.id,
            &key,
            NewReviewItem {
                candidate_identity: key.clone(),
                candidate,
                competing_matches: vec![ReviewMatchCandidate {
                    game_id: 301,
                    release_edition_id: 201,
                    game_title: "Review Game".to_owned(),
                    platform: "Nintendo Entertainment System".to_owned(),
                    region: "USA".to_owned(),
                    edition_name: "Standard".to_owned(),
                    score: 90,
                    evidence: vec![MatchEvidence {
                        signal: MatchSignal::Title,
                        candidate_value: "Review Game".to_owned(),
                        release_value: "Review Game".to_owned(),
                        score_delta: 50,
                    }],
                    assertions: Vec::new(),
                }],
            },
        )
        .unwrap();
    let ParkedReview::Parked(item) = parked else {
        panic!("expected a new review item");
    };
    assert!(
        catalog
            .compare_and_set_run_status(
                run.id,
                AcquisitionRunStatus::Running,
                AcquisitionRunStatus::Completed,
            )
            .unwrap()
    );
    (run.id, item.id)
}

#[test]
fn tauri_lists_review_evidence_and_persists_resolution() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let (run_id, review_item_id) = seed_parked_review(&catalog, candidate("Review Game"));
    drop(catalog);

    let items = game_media_vault_tauri::load_review_items(&vault).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].competing_matches[0].score, 90);
    assert_eq!(
        items[0].competing_matches[0].evidence[0].signal,
        MatchSignal::Title
    );

    let resolved = game_media_vault_tauri::resolve_review_item_in_vault(
        &vault,
        review_item_id,
        ReviewDecision::Accept {
            release_edition_id: 201,
        },
    )
    .unwrap();
    assert_eq!(
        resolved.decision,
        Some(ReviewDecision::Accept {
            release_edition_id: 201
        })
    );
    let reopened_run = load_acquisition_run(
        &SqliteCatalog::open_existing(vault.join("catalog.sqlite3")).unwrap(),
        run_id,
    )
    .unwrap();
    assert_eq!(reopened_run.status, AcquisitionRunStatus::Running);
    assert_eq!(reopened_run.queued_work, 1);

    let reloaded = game_media_vault_tauri::load_review_items(&vault).unwrap();
    assert_eq!(reloaded[0], resolved);
}

#[test]
fn tauri_review_preview_returns_backend_media_bytes() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let preview_candidate = AssetCandidate {
        original_filename: "preview.png".to_owned(),
        ..candidate("Preview Game")
    };
    let (_, review_item_id) = seed_parked_review(&catalog, preview_candidate.clone());
    drop(catalog);
    let connector = PreviewConnector {
        downloads: RefCell::new(Vec::new()),
    };

    let preview = game_media_vault_tauri::load_review_preview_in_vault_with_connector(
        &vault,
        review_item_id,
        &connector,
    )
    .unwrap();

    assert_eq!(preview.media_type, "image/png");
    assert_eq!(preview.bytes, b"preview bytes");
    assert_eq!(
        connector.downloads.borrow().as_slice(),
        &[preview_candidate.source_url]
    );
}
