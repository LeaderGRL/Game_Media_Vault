use game_media_vault_application::{
    match_asset_candidate_to_release, match_asset_candidate_to_release_preferring,
};
use game_media_vault_domain::{
    AssetCandidate, AssetType, LibraryEntry, MatchConfidence, MatchSignal, MatchingPolicy,
    SourceId, ValidatedMatchingPolicy,
};

fn candidate() -> AssetCandidate {
    AssetCandidate {
        provider_candidate_id: None,
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

fn matching_policy(high: u8, medium: u8) -> ValidatedMatchingPolicy {
    MatchingPolicy {
        high_confidence_threshold: high,
        medium_confidence_threshold: medium,
    }
    .validate()
    .unwrap()
}

#[test]
fn matching_combines_multiple_signals_and_exposes_evidence() {
    let result =
        match_asset_candidate_to_release(&candidate(), &[release()], matching_policy(80, 50));

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
        matching_policy(80, 50),
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
fn explicit_platform_conflict_prevents_high_confidence_auto_link() {
    let conflicting_release = LibraryEntry {
        platform: "Super Nintendo Entertainment System".to_owned(),
        ..release()
    };

    let result = match_asset_candidate_to_release(
        &candidate(),
        &[conflicting_release],
        matching_policy(40, 20),
    );

    assert_eq!(result.score, 40);
    assert_eq!(result.confidence, MatchConfidence::Medium);
    assert_eq!(result.auto_link_release_edition_id(), None);
    assert_eq!(
        result
            .evidence
            .iter()
            .find(|evidence| evidence.signal == MatchSignal::Platform)
            .unwrap()
            .score_delta,
        -30
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
        matching_policy(80, 50),
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
        provider_candidate_id: None,
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
        matching_policy(80, 50),
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
fn equally_strong_matches_of_different_games_are_deterministic_but_not_auto_linked() {
    let first = release();
    let second = LibraryEntry {
        game_id: 8,
        release_edition_id: 12,
        ..release()
    };
    let policy = matching_policy(80, 50);

    let forward =
        match_asset_candidate_to_release(&candidate(), &[first.clone(), second.clone()], policy);
    let reversed = match_asset_candidate_to_release(&candidate(), &[second, first], policy);

    assert_eq!(forward, reversed);
    assert_eq!(forward.release_edition_id, Some(11));
    assert_eq!(forward.score, 100);
    assert_eq!(forward.confidence, MatchConfidence::Medium);
    assert_eq!(forward.auto_link_release_edition_id(), None);
}

/// A release of Metroid on the NES, one of the game's four.
fn metroid(release_edition_id: i64, region: &str, edition_name: &str) -> LibraryEntry {
    LibraryEntry {
        game_id: 9,
        game_title: "Metroid".to_owned(),
        release_edition_id,
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: region.to_owned(),
        edition_name: edition_name.to_owned(),
        assertions: Vec::new(),
        assets: Vec::new(),
    }
}

fn metroid_releases() -> Vec<LibraryEntry> {
    vec![
        metroid(1, "Europe", "Standard"),
        metroid(2, "Europe", "Virtual Console"),
        metroid(3, "USA", "Virtual Console"),
        metroid(4, "USA", "Standard"),
    ]
}

/// A map, which records neither region nor edition.
fn metroid_map() -> AssetCandidate {
    AssetCandidate {
        game_title: "Metroid".to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        ..candidate()
    }
}

#[test]
fn media_nothing_tells_between_one_games_releases_go_to_its_representative_release() {
    let result = match_asset_candidate_to_release(
        &metroid_map(),
        &metroid_releases(),
        matching_policy(80, 50),
    );

    // The standard edition, then World, USA, Europe and Japan, stand for the game.
    assert_eq!(result.release_edition_id, Some(4));
    assert_eq!(result.score, 80);
    assert_eq!(result.confidence, MatchConfidence::High);
    assert_eq!(result.auto_link_release_edition_id(), Some(4));
}

#[test]
fn a_release_the_request_names_stands_for_its_game_first() {
    let result = match_asset_candidate_to_release_preferring(
        &metroid_map(),
        &metroid_releases(),
        matching_policy(80, 50),
        &|release| release.region == "Europe",
    );

    assert_eq!(result.release_edition_id, Some(1));
    assert_eq!(result.confidence, MatchConfidence::High);
}

#[test]
fn media_whose_region_conflicts_with_every_release_still_await_review() {
    let japanese = AssetCandidate {
        region: "Japan".to_owned(),
        ..metroid_map()
    };

    let result =
        match_asset_candidate_to_release(&japanese, &metroid_releases(), matching_policy(50, 50));

    assert_ne!(result.confidence, MatchConfidence::High);
    assert_eq!(result.auto_link_release_edition_id(), None);
}

#[test]
fn a_candidate_of_one_region_of_a_release_of_several_matches_it() {
    let european = AssetCandidate {
        region: "Europe".to_owned(),
        ..candidate()
    };
    let usa_and_europe = LibraryEntry {
        region: "USA, Europe".to_owned(),
        ..release()
    };

    let result =
        match_asset_candidate_to_release(&european, &[usa_and_europe], matching_policy(80, 50));

    assert_eq!(result.confidence, MatchConfidence::High);
    assert_eq!(result.score, 100);
}

#[test]
fn a_country_agrees_with_the_region_holding_it_less_than_with_itself() {
    let german = AssetCandidate {
        region: "Germany".to_owned(),
        edition_name: "Unspecified".to_owned(),
        ..candidate()
    };
    let european = LibraryEntry {
        region: "Europe".to_owned(),
        release_edition_id: 21,
        ..release()
    };
    let japanese = LibraryEntry {
        region: "Japan".to_owned(),
        release_edition_id: 22,
        ..release()
    };

    // A German box is the European release's, not the Japanese one's.
    let result = match_asset_candidate_to_release(
        &german,
        &[japanese, european.clone()],
        matching_policy(80, 50),
    );
    assert_eq!(result.release_edition_id, Some(21));
    assert_eq!(result.confidence, MatchConfidence::High);
    let region = result
        .evidence
        .iter()
        .find(|evidence| evidence.signal == MatchSignal::Region)
        .unwrap();
    assert_eq!(region.score_delta, 10);

    // A release of that very country takes it before the region's.
    let german_release = LibraryEntry {
        region: "Germany".to_owned(),
        release_edition_id: 23,
        ..release()
    };
    let result = match_asset_candidate_to_release(
        &german,
        &[european, german_release],
        matching_policy(80, 50),
    );
    assert_eq!(result.release_edition_id, Some(23));
}

#[test]
fn a_worldwide_box_agrees_with_every_region_and_a_canadian_one_with_usa() {
    let european = LibraryEntry {
        region: "Europe".to_owned(),
        ..release()
    };
    for (region, release) in [("World", european), ("Canada", release())] {
        let candidate = AssetCandidate {
            region: region.to_owned(),
            ..candidate()
        };

        let result =
            match_asset_candidate_to_release(&candidate, &[release], matching_policy(80, 50));

        assert_eq!(result.confidence, MatchConfidence::High, "{region}");
    }
}

#[test]
fn two_countries_of_one_region_still_conflict() {
    let german = AssetCandidate {
        region: "Germany".to_owned(),
        ..candidate()
    };
    let french = LibraryEntry {
        region: "France".to_owned(),
        ..release()
    };

    let result = match_asset_candidate_to_release(&german, &[french], matching_policy(80, 50));

    assert_ne!(result.confidence, MatchConfidence::High);
}
