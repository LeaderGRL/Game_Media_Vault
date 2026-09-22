use game_media_vault_application::{
    AcquisitionRequestInput, AcquisitionRequestValidationError, build_acquisition_request,
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
    assert_eq!(actual, expected);
}
