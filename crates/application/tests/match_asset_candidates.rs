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

#[test]
fn explicit_region_conflict_prevents_high_confidence_auto_link() {
    let conflicting_release = LibraryEntry {
        region: "Europe".to_owned(),
        ..release()
    };

    let result = match_asset_candidate_to_release(
        &candidate(),
        &[conflicting_release],
        MatchingPolicy {
            high_confidence_threshold: 80,
            medium_confidence_threshold: 50,
        },
    );

    assert_eq!(result.score, 70);
    assert_eq!(result.confidence, MatchConfidence::Medium);
    assert_eq!(result.auto_link_release_edition_id(), None);
    assert_eq!(
        result
            .evidence
            .iter()
            .find(|evidence| evidence.signal == MatchSignal::Region)
            .unwrap()
            .score_delta,
        -15
    );
}

#[test]
fn explicit_edition_conflict_prevents_high_confidence_auto_link() {
    let conflicting_release = LibraryEntry {
        edition_name: "Standard".to_owned(),
        ..release()
    };

    let result = match_asset_candidate_to_release(
        &candidate(),
        &[conflicting_release],
        MatchingPolicy {
            high_confidence_threshold: 80,
            medium_confidence_threshold: 50,
        },
    );

    assert_eq!(result.score, 90);
    assert_eq!(result.confidence, MatchConfidence::Medium);
    assert_eq!(result.auto_link_release_edition_id(), None);
    assert_eq!(
        result
            .evidence
            .iter()
            .find(|evidence| evidence.signal == MatchSignal::Edition)
            .unwrap()
            .score_delta,
        -5
    );
}

#[test]
fn missing_region_and_edition_values_do_not_increase_confidence() {
    let sparse_candidate = AssetCandidate {
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        ..candidate()
    };
    let sparse_release = LibraryEntry {
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        ..release()
    };

    let result = match_asset_candidate_to_release(
        &sparse_candidate,
        &[sparse_release],
        MatchingPolicy {
            high_confidence_threshold: 80,
            medium_confidence_threshold: 50,
        },
    );

    assert_eq!(result.score, 80);
    assert_eq!(result.confidence, MatchConfidence::High);
    assert_eq!(result.auto_link_release_edition_id(), Some(11));
    assert_eq!(
        result
            .evidence
            .iter()
            .filter(|evidence| matches!(
                evidence.signal,
                MatchSignal::Region | MatchSignal::Edition
            ))
            .map(|evidence| evidence.score_delta)
            .collect::<Vec<_>>(),
        vec![0, 0]
    );
}

#[test]
fn equally_strong_release_matches_are_deterministic_but_not_auto_linked() {
    let first = release();
    let second = LibraryEntry {
        release_edition_id: 12,
        ..release()
    };
    let policy = MatchingPolicy {
        high_confidence_threshold: 80,
        medium_confidence_threshold: 50,
    };

    let forward =
        match_asset_candidate_to_release(&candidate(), &[first.clone(), second.clone()], policy);
    let reversed = match_asset_candidate_to_release(&candidate(), &[second, first], policy);

    assert_eq!(forward, reversed);
    assert_eq!(forward.release_edition_id, Some(11));
    assert_eq!(forward.score, 100);
    assert_eq!(forward.confidence, MatchConfidence::Medium);
    assert_eq!(forward.auto_link_release_edition_id(), None);
}
