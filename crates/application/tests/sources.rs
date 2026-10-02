mod support;

use game_media_vault_application::{ConnectorPort, SourceDescription, describe_sources};
use game_media_vault_domain::AssetType;
use support::FakeConnector;

#[test]
fn sources_are_described_with_the_capabilities_planning_uses() {
    let boxes = FakeConnector::new(Vec::new());
    let snaps = FakeConnector {
        source_id: "snap-source",
        asset_types: vec![AssetType::Screenshot, AssetType::TitleScreen],
        ..FakeConnector::new(Vec::new())
    };
    let connectors: Vec<&dyn ConnectorPort> = vec![&boxes, &snaps];

    let sources = describe_sources(&connectors);

    assert_eq!(
        sources,
        [
            SourceDescription {
                source_id: boxes.source_id().to_owned(),
                asset_types: vec![AssetType::BoxFront],
                direct_media_download: true,
            },
            SourceDescription {
                source_id: "snap-source".to_owned(),
                asset_types: vec![AssetType::Screenshot, AssetType::TitleScreen],
                direct_media_download: true,
            },
        ]
    );
    // Describing a Source never consults it.
    assert_eq!(*boxes.discover_calls.borrow(), 0);
}
