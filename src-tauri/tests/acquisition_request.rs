use game_media_vault_application::{
    AcquisitionRequestInput, AcquisitionRequestValidationError, ApplicationError,
    build_acquisition_request,
};
use game_media_vault_domain::{
    AcquisitionLimits, AssetTypeSelector, GameSelection, RetentionPolicy, SourceSelection,
};

#[test]
fn tauri_uses_the_same_acquisition_request_validation_as_the_application() {
    let input = AcquisitionRequestInput {
        sources: SourceSelection::Explicit(Vec::new()),
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::All,
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };
    let expected = build_acquisition_request(input.clone()).unwrap_err();

    let actual = game_media_vault_tauri::validate_acquisition_request(input).unwrap_err();

    assert_eq!(expected, AcquisitionRequestValidationError::MissingSources);
    assert_eq!(actual.kind, "invalid_request");
    assert_eq!(
        actual.message,
        ApplicationError::Validation(expected).to_string()
    );
}

#[test]
fn tauri_accepts_partial_quality_requirements() {
    let input: AcquisitionRequestInput = serde_json::from_value(serde_json::json!({
        "sources": { "mode": "auto" },
        "platforms": ["Windows"],
        "games": { "mode": "all" },
        "regions": [],
        "languages": [],
        "asset_types": ["box_front"],
        "quality": {
            "min_width": 1600
        },
        "retention": "keep_everything",
        "limits": {}
    }))
    .unwrap();

    let request = game_media_vault_tauri::validate_acquisition_request(input).unwrap();
    let serialized = serde_json::to_value(request).unwrap();

    assert_eq!(serialized["quality"]["min_width"], 1600);
    assert_eq!(serialized["quality"]["original_only"], false);
    assert_eq!(
        serialized["quality"]["accepted_mime_types"],
        serde_json::json!([])
    );
    assert_eq!(serialized["quality"]["best_available"], false);
}

#[test]
fn the_desktop_request_draft_deserializes_into_the_shared_request_model() {
    // Shape produced by ui/src/acquisition.ts::buildAcquisitionRequest.
    let draft = serde_json::json!({
        "sources": { "mode": "explicit", "values": ["libretro-thumbnails"] },
        "platforms": ["Nintendo - Game Boy"],
        "games": { "mode": "explicit", "values": ["Tetris (World) (Rev 1)"] },
        "regions": ["Europe"],
        "languages": [],
        "asset_types": ["box_front", "box_3d_model", "3d_model", "other_family"],
        "quality": null,
        "retention": "keep_best_per_type",
        "limits": {}
    });

    let input: AcquisitionRequestInput = serde_json::from_value(draft).unwrap();
    let request = game_media_vault_tauri::validate_acquisition_request(input).unwrap();

    assert!(request.selects_only_source("libretro-thumbnails"));
    assert_eq!(request.platforms(), ["Nintendo - Game Boy"]);
    assert_eq!(request.retention(), RetentionPolicy::KeepBestPerType);
    assert_eq!(request.limits(), &AcquisitionLimits::default());
    let all_games: AcquisitionRequestInput = serde_json::from_value(serde_json::json!({
        "sources": { "mode": "auto" },
        "platforms": ["Windows"],
        "games": { "mode": "all" },
        "regions": [],
        "languages": [],
        "asset_types": ["packaging"],
        "quality": null,
        "retention": "keep_everything",
        "limits": {}
    }))
    .unwrap();
    assert_eq!(all_games.games, GameSelection::All);
    assert_eq!(all_games.sources, SourceSelection::Auto);
}
