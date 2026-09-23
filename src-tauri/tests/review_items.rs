use game_media_vault_application::{
    AcquisitionRequestInput, CatalogPort, complete_acquisition_run, complete_acquisition_work,
    load_acquisition_run, queue_acquisition_work, start_acquisition_run,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRunStatus, AssetCandidate, AssetType, AssetTypeSelector,
    GameSelection, MatchEvidence, MatchSignal, NewReviewItem, RetentionPolicy, ReviewDecision,
    ReviewMatchCandidate, SourceId, SourceSelection,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::tempdir;

fn review_work_key(candidate: &AssetCandidate) -> String {
    let mut key = "connector".to_owned();
    for part in [
        candidate.source_id.as_str(),
        candidate.platform.as_str(),
        candidate.game_title.as_str(),
        candidate.region.as_str(),
        candidate.edition_name.as_str(),
        "box_front",
        candidate.source_url.as_str(),
    ] {
        key.push(':');
        key.push_str(&part.len().to_string());
        key.push(':');
        key.push_str(part);
    }
    key
}

#[test]
fn tauri_lists_review_evidence_and_persists_resolution() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let run = start_acquisition_run(
        &catalog,
        AcquisitionRequestInput {
            sources: SourceSelection::Explicit(vec!["fixture-provider".to_owned()]),
            platforms: vec!["Nintendo Entertainment System".to_owned()],
            games: GameSelection::Explicit(vec!["Review Game".to_owned()]),
            regions: Vec::new(),
            languages: Vec::new(),
            asset_types: vec![AssetTypeSelector::BoxFront],
            quality: None,
            retention: RetentionPolicy::KeepEverything,
            limits: AcquisitionLimits::default(),
        },
    )
    .unwrap();
    let candidate = AssetCandidate {
        game_title: "Review Game".to_owned(),
        platform: "Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Collector".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("fixture-provider"),
        source_asset_label: Some("front".to_owned()),
        source_url: "fixture://review/front".to_owned(),
        original_filename: "front.png".to_owned(),
    };
    let work_key = review_work_key(&candidate);
    queue_acquisition_work(&catalog, run.id, work_key.clone()).unwrap();
    complete_acquisition_work(&catalog, run.id, &work_key).unwrap();
    complete_acquisition_run(&catalog, run.id).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: run.id,
            candidate_identity: "connector:tauri-review".to_owned(),
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
            }],
        })
        .unwrap();
    let review_item_id = catalog.list_review_items().unwrap()[0].id;
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
        run.id,
    )
    .unwrap();
    assert_eq!(reopened_run.status, AcquisitionRunStatus::Running);
    assert_eq!(reopened_run.queued_work, 1);

    let reloaded = game_media_vault_tauri::load_review_items(&vault).unwrap();
    assert_eq!(reloaded[0], resolved);
}
