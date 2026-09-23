use game_media_vault_application::match_asset_candidate_to_release;
use game_media_vault_domain::{
    AssetCandidate, AssetType, LibraryEntry, MatchConfidence, MatchSignal, MatchingPolicy, SourceId,
};

fn candidate() -> AssetCandidate {
    AssetCandidate {
        game_title: "Super Mario Bros.".to_owned(),
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Rev 1".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("fixture-source"),
        source_asset_label: Some("box_front".to_owned()),
        source_url: "https://example.invalid/smb.png".to_owned(),
        original_filename: "smb.png".to_owned(),
    }
}

fn release() -> LibraryEntry {
    LibraryEntry {
        game_id: 7,
        game_title: "Super Mario Bros.".to_owned(),
        release_edition_id: 11,
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Rev 1".to_owned(),
        assertions: Vec::new(),
        assets: Vec::new(),
    }
}

#[test]
fn matching_combines_multiple_signals_and_exposes_evidence() {
    let result = match_asset_candidate_to_release(
        &candidate(),
        &[release()],
        MatchingPolicy {
            high_confidence_threshold: 80,
            medium_confidence_threshold: 50,
        },
    );

    assert_eq!(result.release_edition_id, Some(11));
    assert_eq!(result.score, 100);
    assert_eq!(result.confidence, MatchConfidence::High);
    assert_eq!(result.auto_link_release_edition_id(), Some(11));
    assert_eq!(
        result
            .evidence
            .iter()
            .map(|evidence| (evidence.signal, evidence.score_delta))
            .collect::<Vec<_>>(),
        vec![
            (MatchSignal::Title, 50),
            (MatchSignal::Platform, 30),
            (MatchSignal::Region, 15),
            (MatchSignal::Edition, 5),
        ]
    );
}
