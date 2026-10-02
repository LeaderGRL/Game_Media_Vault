use std::{
    cell::RefCell,
    io::{Cursor, Read},
};

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_connectors::{HttpTransport, LibretroThumbnailsConnector};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetType, AssetTypeSelector,
    GameSelection, PlatformBoundGameSelector, RetentionPolicy, SourceId, SourceSelection,
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

/// Lets a test keep the transport to inspect the requests a connector made.
impl HttpTransport for &FixtureTransport {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, PortError> {
        (**self).get_bytes(url)
    }

    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        (**self).get_stream(url)
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
    assert_eq!(candidate.source_id, SourceId::from("libretro-thumbnails"));
    assert_eq!(
        candidate.provider_candidate_id.as_deref(),
        Some("Nintendo_-_Nintendo_Entertainment_System/Named_Boxarts/Super Mario Bros. (World)")
    );
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
fn replaces_backticks_in_libretro_thumbnail_filenames() {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Rock`n Roll Racing".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    let candidate = connector.discover(&request).unwrap().remove(0);

    assert_eq!(candidate.original_filename, "Rock_n Roll Racing.png");
    assert_eq!(
        candidate.source_url,
        "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System/master/Named_Boxarts/Rock_n%20Roll%20Racing.png"
    );
}

#[test]
fn platform_bound_targets_respect_explicit_platform_filters() {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::PlatformBound(vec![
            PlatformBoundGameSelector {
                platform: "Nintendo - Nintendo Entertainment System".to_owned(),
                game: "Super Mario Bros. (World)".to_owned(),
            },
            PlatformBoundGameSelector {
                platform: "Sega - Naomi".to_owned(),
                game: "Crazy Taxi".to_owned(),
            },
        ]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    let candidates = connector.discover(&request).unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].platform,
        "Nintendo - Nintendo Entertainment System"
    );
    assert_eq!(candidates[0].game_title, "Super Mario Bros.");
    assert_eq!(candidates[0].region, "World");
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

    assert!(error.message().contains("region"));
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

    assert!(error.message().contains("language"));
}

fn discover_one(game: &str) -> game_media_vault_domain::AssetCandidate {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        games: GameSelection::Explicit(vec![game.to_owned()]),
        ..request_draft()
    })
    .unwrap();
    connector.discover(&request).unwrap().remove(0)
}

fn request_draft() -> AcquisitionRequestDraft {
    AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    }
}

fn request_with(change: fn(&mut AcquisitionRequestDraft)) -> AcquisitionRequest {
    let mut draft = request_draft();
    change(&mut draft);
    AcquisitionRequest::try_from_draft(draft).unwrap()
}

#[test]
fn candidates_carry_the_release_fields_encoded_in_the_no_intro_thumbnail_name() {
    let candidate = discover_one("Tetris (World) (Rev 1)");

    assert_eq!(candidate.game_title, "Tetris");
    assert_eq!(candidate.region, "World");
    assert_eq!(candidate.edition_name, "Rev 1");
    assert_eq!(candidate.original_filename, "Tetris (World) (Rev 1).png");
    assert_eq!(
        candidate.provider_candidate_id.as_deref(),
        Some("Nintendo_-_Nintendo_Entertainment_System/Named_Boxarts/Tetris (World) (Rev 1)")
    );
}

#[test]
fn untagged_thumbnail_names_keep_an_unknown_region_and_standard_edition() {
    let candidate = discover_one("Homebrew Collection");

    assert_eq!(candidate.game_title, "Homebrew Collection");
    assert_eq!(candidate.region, "Unknown");
    assert_eq!(candidate.edition_name, "Standard");
}

#[test]
fn explains_the_requests_it_cannot_execute_before_any_discovery() {
    let transport = FixtureTransport::default();
    let connector = LibretroThumbnailsConnector::with_transport(&transport);
    let base = request(vec![AssetTypeSelector::BoxFront]);

    assert_eq!(connector.unsupported_request_reason(&base).unwrap(), None);
    transport.requests.borrow_mut().clear();
    for (request, topic) in [
        (
            request_with(|draft| draft.games = GameSelection::All),
            "game selection",
        ),
        (
            request_with(|draft| draft.regions = vec!["World".to_owned()]),
            "region",
        ),
        (
            request_with(|draft| draft.languages = vec!["fr".to_owned()]),
            "language",
        ),
    ] {
        let reason = connector
            .unsupported_request_reason(&request)
            .unwrap()
            .unwrap();
        assert!(reason.contains(topic), "{reason}");
    }
    // Selections Libretro can never satisfy are refused without reaching the source.
    assert!(transport.requests.borrow().is_empty());
}

#[test]
fn refuses_platforms_without_a_libretro_repository_before_a_run_starts() {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());
    let request = request_with(|draft| {
        draft.platforms = vec![
            "Nintendo - Nintendo Entertainment System".to_owned(),
            "Nintendo - Famicom Disk Sytem".to_owned(),
        ];
    });

    let reason = connector
        .unsupported_request_reason(&request)
        .unwrap()
        .unwrap();

    assert!(reason.contains("Nintendo - Famicom Disk Sytem"), "{reason}");
}

/// Answers every request with bytes that are not UTF-8.
struct MalformedMetadataTransport;

impl HttpTransport for MalformedMetadataTransport {
    fn get_stream(&self, _url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        Ok(Box::new(Cursor::new(vec![0xff, 0xfe, 0xfd])))
    }
}

#[test]
fn malformed_repository_metadata_is_invalid_source_data() {
    let connector = LibretroThumbnailsConnector::with_transport(MalformedMetadataTransport);

    let error = connector
        .discover(&request(vec![AssetTypeSelector::BoxFront]))
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{error}");
}
