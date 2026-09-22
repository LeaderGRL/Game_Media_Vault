use game_media_vault_application::{
    AcquisitionRequestInput, AcquisitionRequestValidationError, build_acquisition_request,
};
use game_media_vault_domain::{
    AcquisitionLimits, AssetTypeSelector, GameSelection, QualityRequirements, RetentionPolicy,
    SourceSelection,
};
use serde_json::json;

#[test]
fn rejects_an_acquisition_request_without_sources() {
    let error = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Explicit(Vec::new()),
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::All,
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap_err();

    assert_eq!(error, AcquisitionRequestValidationError::MissingSources);
}

#[test]
fn rejects_an_acquisition_request_with_only_blank_sources() {
    let error = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Explicit(vec!["".to_owned(), "   ".to_owned()]),
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::All,
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap_err();

    assert_eq!(error, AcquisitionRequestValidationError::MissingSources);
}

#[test]
fn rejects_an_acquisition_request_without_asset_types() {
    let error = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Explicit(vec!["local-import".to_owned()]),
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::All,
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: Vec::new(),
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap_err();

    assert_eq!(error, AcquisitionRequestValidationError::MissingAssetTypes);
}

#[test]
fn preserves_optional_filters_and_independent_asset_type_selection() {
    let request = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Explicit(vec!["screenscraper".to_owned(), "game-tdb".to_owned()]),
        platforms: vec!["PlayStation 2".to_owned()],
        games: GameSelection::Explicit(vec!["Metal Gear Solid 3".to_owned()]),
        regions: vec!["France".to_owned(), "Europe".to_owned()],
        languages: vec!["fr".to_owned(), "en".to_owned()],
        asset_types: vec![
            AssetTypeSelector::BoxFront,
            AssetTypeSelector::Manual,
            AssetTypeSelector::Screenshot,
        ],
        quality: Some(QualityRequirements {
            min_width: Some(1600),
            min_height: Some(1200),
            min_longest_edge: Some(2000),
            min_pixel_count: Some(2_000_000),
            original_only: true,
            accepted_mime_types: vec!["image/png".to_owned(), "image/jpeg".to_owned()],
            min_bitrate_kbps: Some(320),
            best_available: true,
        }),
        retention: RetentionPolicy::KeepBestPerType,
        limits: AcquisitionLimits {
            max_games: Some(25),
            max_downloads: Some(100),
            max_concurrent_downloads: Some(4),
            max_bytes: Some(5_000_000_000),
        },
    })
    .unwrap();

    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({
            "sources": { "mode": "explicit", "values": ["screenscraper", "game-tdb"] },
            "platforms": ["PlayStation 2"],
            "games": { "mode": "explicit", "values": ["Metal Gear Solid 3"] },
            "regions": ["France", "Europe"],
            "languages": ["fr", "en"],
            "asset_types": ["box_front", "manual", "screenshot"],
            "quality": {
                "min_width": 1600,
                "min_height": 1200,
                "min_longest_edge": 2000,
                "min_pixel_count": 2_000_000,
                "original_only": true,
                "accepted_mime_types": ["image/png", "image/jpeg"],
                "min_bitrate_kbps": 320,
                "best_available": true
            },
            "retention": "keep_best_per_type",
            "limits": {
                "max_games": 25,
                "max_downloads": 100,
                "max_concurrent_downloads": 4,
                "max_bytes": 5_000_000_000_u64
            }
        })
    );
}

#[test]
fn rejects_all_games_targeting_without_a_platform() {
    let error = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: Vec::new(),
        games: GameSelection::All,
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap_err();

    assert_eq!(error, AcquisitionRequestValidationError::MissingPlatforms);
}

#[test]
fn rejects_explicit_game_names_without_a_platform() {
    let error = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: Vec::new(),
        games: GameSelection::Explicit(vec!["Metal Gear Solid 3".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap_err();

    assert_eq!(error, AcquisitionRequestValidationError::MissingPlatforms);
}
