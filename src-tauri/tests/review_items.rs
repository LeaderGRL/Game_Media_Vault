use game_media_vault_application::CatalogPort;
use game_media_vault_domain::{
    AssetCandidate, AssetType, MatchEvidence, MatchSignal, NewReviewItem, ReviewDecision,
    ReviewMatchCandidate, SourceId,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::tempdir;

#[test]
fn tauri_lists_review_evidence_and_persists_resolution() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id: 7,
            candidate_identity: "connector:tauri-review".to_owned(),
            candidate: AssetCandidate {
                game_title: "Review Game".to_owned(),
                platform: "Nintendo Entertainment System".to_owned(),
                region: "USA".to_owned(),
                edition_name: "Collector".to_owned(),
                asset_type: AssetType::BoxFront,
                source_id: SourceId::from("fixture-provider"),
                source_asset_label: Some("front".to_owned()),
                source_url: "fixture://review/front".to_owned(),
                original_filename: "front.png".to_owned(),
            },
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

    let reloaded = game_media_vault_tauri::load_review_items(&vault).unwrap();
    assert_eq!(reloaded[0], resolved);
}
