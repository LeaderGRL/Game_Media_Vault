use game_media_vault_application::{
    AcquisitionRequestInput, AcquisitionRequestValidationError, build_acquisition_request,
};
use game_media_vault_domain::{
    AcquisitionLimits, AssetTypeSelector, GameSelection, RetentionPolicy, SourceSelection,
};

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
