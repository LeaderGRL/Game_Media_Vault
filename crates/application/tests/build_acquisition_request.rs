use game_media_vault_application::{
    AcquisitionRequestInput, AcquisitionRequestValidationError, build_acquisition_request,
};
use game_media_vault_domain::{
    AcquisitionLimits, AssetTypeSelector, GameSelection, PlatformBoundGameSelector,
    QualityRequirements, RetentionPolicy, SourceSelection,
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
            max_compression_ratio: Some(12),
            min_bitrate_kbps: Some(320),
            preferred_scan_type: Some("raw_scan".to_owned()),
            preferred_source_priority: vec!["screenscraper".to_owned(), "game-tdb".to_owned()],
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
                "max_compression_ratio": 12,
                "min_bitrate_kbps": 320,
                "preferred_scan_type": "raw_scan",
                "preferred_source_priority": ["screenscraper", "game-tdb"],
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
fn removes_blank_optional_region_and_language_filters() {
    let request = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::All,
        regions: vec!["".to_owned(), "Europe".to_owned(), "   ".to_owned()],
        languages: vec![" ".to_owned(), "fr".to_owned(), "".to_owned()],
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    let serialized = serde_json::to_value(request).unwrap();
    assert_eq!(serialized["regions"], json!(["Europe"]));
    assert_eq!(serialized["languages"], json!(["fr"]));
}

#[test]
fn removes_blank_optional_quality_selectors() {
    let request = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::All,
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: Some(QualityRequirements {
            accepted_mime_types: vec!["".to_owned(), "image/png".to_owned(), "   ".to_owned()],
            preferred_scan_type: Some("   ".to_owned()),
            preferred_source_priority: vec![
                " ".to_owned(),
                "screenscraper".to_owned(),
                "".to_owned(),
            ],
            ..QualityRequirements::default()
        }),
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    let serialized = serde_json::to_value(request).unwrap();
    assert_eq!(
        serialized["quality"]["accepted_mime_types"],
        json!(["image/png"])
    );
    assert_eq!(serialized["quality"]["preferred_scan_type"], json!(null));
    assert_eq!(
        serialized["quality"]["preferred_source_priority"],
        json!(["screenscraper"])
    );
}

#[test]
fn preserves_asset_type_family_selection() {
    let family_selectors: Vec<AssetTypeSelector> = serde_json::from_value(json!([
        "packaging",
        "physical_media",
        "documentation",
        "digital_media",
        "promotional_and_historical",
        "hardware_arcade",
        "other_family"
    ]))
    .unwrap();

    let request = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::All,
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: family_selectors,
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    assert_eq!(
        serde_json::to_value(request).unwrap()["asset_types"],
        json!([
            "packaging",
            "physical_media",
            "documentation",
            "digital_media",
            "promotional_and_historical",
            "hardware_arcade",
            "other_family"
        ])
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
fn rejects_blank_platform_filters() {
    let error = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: vec!["".to_owned(), "   ".to_owned()],
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
fn removes_blank_platform_filters_when_a_valid_platform_remains() {
    let request = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: vec!["".to_owned(), "Windows".to_owned(), "   ".to_owned()],
        games: GameSelection::All,
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    assert_eq!(
        serde_json::to_value(request).unwrap()["platforms"],
        json!(["Windows"])
    );
}

#[test]
fn accepts_platform_bound_games_without_a_platform_filter() {
    let request = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: Vec::new(),
        games: GameSelection::PlatformBound(vec![PlatformBoundGameSelector {
            game: "Metal Gear Solid 3".to_owned(),
            platform: "PlayStation 2".to_owned(),
        }]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    assert_eq!(
        serde_json::to_value(request).unwrap()["games"],
        json!({
            "mode": "platform_bound",
            "values": [{
                "game": "Metal Gear Solid 3",
                "platform": "PlayStation 2"
            }]
        })
    );
}

#[test]
fn preserves_query_result_game_targeting() {
    let request = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: Vec::new(),
        games: GameSelection::QueryResult(vec![PlatformBoundGameSelector {
            game: "Metal Gear Solid 3".to_owned(),
            platform: "PlayStation 2".to_owned(),
        }]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    assert_eq!(
        serde_json::to_value(request).unwrap()["games"],
        json!({
            "mode": "query_result",
            "values": [{
                "game": "Metal Gear Solid 3",
                "platform": "PlayStation 2"
            }]
        })
    );
}

#[test]
fn rejects_platform_bound_games_with_blank_game_names() {
    let error = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: vec!["PlayStation 2".to_owned()],
        games: GameSelection::PlatformBound(vec![PlatformBoundGameSelector {
            game: "   ".to_owned(),
            platform: "PlayStation 2".to_owned(),
        }]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap_err();

    assert_eq!(
        error,
        AcquisitionRequestValidationError::InvalidGameSelection
    );
}

#[test]
fn rejects_platform_bound_games_with_blank_platforms() {
    let error = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: vec!["PlayStation 2".to_owned()],
        games: GameSelection::PlatformBound(vec![PlatformBoundGameSelector {
            game: "Metal Gear Solid 3".to_owned(),
            platform: "   ".to_owned(),
        }]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap_err();

    assert_eq!(
        error,
        AcquisitionRequestValidationError::InvalidGameSelection
    );
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

#[test]
fn rejects_an_empty_explicit_game_selection() {
    let error = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::Explicit(Vec::new()),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap_err();

    assert_eq!(
        error,
        AcquisitionRequestValidationError::InvalidGameSelection
    );
}

#[test]
fn rejects_explicit_game_selection_with_only_blank_names() {
    let error = build_acquisition_request(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::Explicit(vec!["".to_owned(), "   ".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap_err();

    assert_eq!(
        error,
        AcquisitionRequestValidationError::InvalidGameSelection
    );
}
