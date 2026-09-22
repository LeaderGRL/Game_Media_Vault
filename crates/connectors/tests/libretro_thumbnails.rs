use std::cell::RefCell;

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_connectors::{HttpTransport, LibretroThumbnailsConnector};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetType, AssetTypeSelector,
    GameSelection, RetentionPolicy, SourceKind, SourceSelection,
};

#[derive(Default)]
struct FixtureTransport {
    requests: RefCell<Vec<String>>,
}

impl HttpTransport for FixtureTransport {
    fn get(&self, url: &str) -> Result<Vec<u8>, PortError> {
        self.requests.borrow_mut().push(url.to_owned());
        Ok(b"libretro png fixture".to_vec())
    }
}

fn request(asset_types: Vec<AssetTypeSelector>) -> AcquisitionRequest {
    AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types,
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap()
}

#[test]
fn declares_box_front_capability_and_downloads_the_discovered_fixture() {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());

    let capabilities = connector.capabilities();
    assert_eq!(capabilities.asset_types, vec![AssetType::BoxFront]);
    assert!(capabilities.direct_media_download);

    let candidates = connector
        .discover(&request(vec![AssetTypeSelector::BoxFront]))
        .unwrap();
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate.asset_type, AssetType::BoxFront);
    assert_eq!(candidate.source_kind, SourceKind::LibretroThumbnails);
    assert_eq!(candidate.original_filename, "Super Mario Bros. (World).png");
    assert_eq!(
        candidate.source_url,
        "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System/master/Named_Boxarts/Super%20Mario%20Bros.%20(World).png"
    );

    let bytes = connector.download(candidate).unwrap();
    assert_eq!(bytes, b"libretro png fixture");
}

#[test]
fn does_not_discover_box_art_when_box_front_is_not_requested() {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());

    let candidates = connector
        .discover(&request(vec![AssetTypeSelector::Screenshot]))
        .unwrap();

    assert!(candidates.is_empty());
}
