use std::{
    cell::RefCell,
    io::{Cursor, Read},
};

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_connectors::{HttpTransport, LibretroThumbnailsConnector};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetType, AssetTypeSelector,
    GameSelection, RetentionPolicy, SourceKind, SourceSelection,
};

const GITMODULES_FIXTURE: &str = r#"
[submodule "Nintendo - Nintendo Entertainment System"]
    path = Nintendo - Nintendo Entertainment System
    url = https://github.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System.git
    branch = master
[submodule "Philips - Videopac+"]
    path = Philips - Videopac+
    url = https://github.com/libretro-thumbnails/Philips_-_Videopac.git
    branch = master
[submodule "Sega - Naomi"]
    path = Sega - Naomi
    url = https://github.com/libretro-thumbnails/Sega_-_Naomi.git
    branch = main
"#;

#[derive(Default)]
struct FixtureTransport {
    requests: RefCell<Vec<String>>,
}

impl HttpTransport for FixtureTransport {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, PortError> {
        self.requests.borrow_mut().push(url.to_owned());
        if url.ends_with("/.gitmodules") {
            Ok(GITMODULES_FIXTURE.as_bytes().to_vec())
        } else {
            Ok(b"libretro png fixture".to_vec())
        }
    }

    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        self.requests.borrow_mut().push(url.to_owned());
        Ok(Box::new(Cursor::new(b"libretro png fixture".to_vec())))
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

    let mut stream = connector.download(candidate).unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
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

#[test]
fn resolves_a_libretro_repository_slug_that_differs_from_the_platform_name() {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Philips - Videopac+".to_owned()],
        games: GameSelection::Explicit(vec!["Pickaxe Pete".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    let candidate = connector.discover(&request).unwrap().remove(0);

    assert_eq!(
        candidate.source_url,
        "https://raw.githubusercontent.com/libretro-thumbnails/Philips_-_Videopac/master/Named_Boxarts/Pickaxe%20Pete.png"
    );
}

#[test]
fn resolves_the_branch_declared_by_libretro_metadata() {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Sega - Naomi".to_owned()],
        games: GameSelection::Explicit(vec!["Crazy Taxi".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    let candidate = connector.discover(&request).unwrap().remove(0);

    assert_eq!(
        candidate.source_url,
        "https://raw.githubusercontent.com/libretro-thumbnails/Sega_-_Naomi/main/Named_Boxarts/Crazy%20Taxi.png"
    );
}

#[test]
fn rejects_region_filters_without_source_region_evidence() {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: vec!["World".to_owned()],
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    let error = connector.discover(&request).unwrap_err();

    assert!(error.0.contains("region"));
}

#[test]
fn rejects_language_filters_without_source_language_evidence() {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: vec!["fr".to_owned()],
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    let error = connector.discover(&request).unwrap_err();

    assert!(error.0.contains("language"));
}
