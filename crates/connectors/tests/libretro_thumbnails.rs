use std::{
    io::{Cursor, Read},
    sync::Mutex,
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
    requests: Mutex<Vec<String>>,
}

impl HttpTransport for FixtureTransport {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, PortError> {
        self.requests.lock().unwrap().push(url.to_owned());
        if url.ends_with("/.gitmodules") {
            Ok(GITMODULES_FIXTURE.as_bytes().to_vec())
        } else {
            Ok(b"libretro png fixture".to_vec())
        }
    }

    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        self.requests.lock().unwrap().push(url.to_owned());
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

const NES_BOX_ARTS_LISTING_URL: &str = "https://thumbnails.libretro.com/Nintendo%20-%20Nintendo%20Entertainment%20System/Named_Boxarts/";

/// The Apache directory listing `thumbnails.libretro.com` serves for the NES box arts folder,
/// linking to each of `hrefs`.
fn nes_box_arts_listing(hrefs: &[&str]) -> String {
    let rows: String = hrefs
        .iter()
        .map(|href| {
            format!(
                "<tr><td valign=\"top\"><img src=\"/icons/image2.gif\" alt=\"[IMG]\"></td><td><a href=\"{href}\">{href}</a></td><td align=\"right\">2025-04-09 11:54  </td><td align=\"right\">547K</td><td>&nbsp;</td></tr>\n"
            )
        })
        .collect();
    format!(
        r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 3.2 Final//EN">
<html>
 <head>
  <title>Index of /Nintendo - Nintendo Entertainment System/Named_Boxarts</title>
 </head>
 <body>
<h1>Index of /Nintendo - Nintendo Entertainment System/Named_Boxarts</h1>
  <table>
   <tr><th valign="top"><img src="/icons/blank.gif" alt="[ICO]"></th><th><a href="?C=N;O=D">Name</a></th><th><a href="?C=M;O=A">Last modified</a></th><th><a href="?C=S;O=A">Size</a></th><th><a href="?C=D;O=A">Description</a></th></tr>
   <tr><th colspan="5"><hr></th></tr>
<tr><td valign="top"><img src="/icons/back.gif" alt="[PARENTDIR]"></td><td><a href="/Nintendo%20-%20Nintendo%20Entertainment%20System/">Parent Directory</a></td><td>&nbsp;</td><td align="right">  - </td><td>&nbsp;</td></tr>
{rows}   <tr><th colspan="5"><hr></th></tr>
</table>
</body></html>
"#
    )
}

/// Serves the repository list and the NES box arts folder's listing as `listing` answers it;
/// every other request finds nothing.
struct ListingTransport {
    listing: Result<String, String>,
    requests: Mutex<Vec<String>>,
}

impl ListingTransport {
    fn answering(listing: Result<String, String>) -> Self {
        Self {
            listing,
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl HttpTransport for ListingTransport {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, PortError> {
        self.requests.lock().unwrap().push(url.to_owned());
        if url.ends_with("/.gitmodules") {
            Ok(GITMODULES_FIXTURE.as_bytes().to_vec())
        } else if url == NES_BOX_ARTS_LISTING_URL {
            self.listing
                .clone()
                .map(String::into_bytes)
                .map_err(PortError::new)
        } else {
            Err(PortError::new(format!(
                "download returned HTTP 404 Not Found for {url} "
            )))
        }
    }

    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        Ok(Box::new(Cursor::new(self.get_bytes(url)?)))
    }
}

/// Lets a test keep the transport to count the listings a connector read.
impl HttpTransport for &ListingTransport {
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
fn declares_its_thumbnail_types_and_downloads_the_discovered_fixture() {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());

    let capabilities = connector.capabilities();
    assert_eq!(
        capabilities.asset_types,
        vec![
            AssetType::BoxFront,
            AssetType::Screenshot,
            AssetType::TitleScreen
        ]
    );
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
fn discovers_nothing_for_asset_types_libretro_does_not_offer() {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());

    let candidates = connector
        .discover(&request(vec![AssetTypeSelector::Manual]))
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
    transport.requests.lock().unwrap().clear();
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
    assert!(transport.requests.lock().unwrap().is_empty());
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

#[test]
fn discovers_only_the_thumbnails_the_folder_s_listing_names() {
    let connector = LibretroThumbnailsConnector::with_transport(ListingTransport::answering(Ok(
        nes_box_arts_listing(&[
            "Rock_n%20Roll%20Racing.png",
            "Super%20Mario%20Bros.%20(World).png",
        ]),
    )));
    let request = request_with(|draft| {
        draft.games = GameSelection::Explicit(vec![
            "Super Mario Bros. (World)".to_owned(),
            "Homebrew Collection".to_owned(),
        ]);
    });

    let candidates = connector.discover(&request).unwrap();

    let found: Vec<_> = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.original_filename.as_str(),
                candidate.source_url.as_str(),
            )
        })
        .collect();
    assert_eq!(
        found,
        vec![(
            "Super Mario Bros. (World).png",
            "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System/master/Named_Boxarts/Super%20Mario%20Bros.%20(World).png"
        )]
    );
}

#[test]
fn reads_the_names_a_listing_escapes() {
    // The server escapes brackets and percent signs in lowercase hexadecimal, and characters
    // beyond ASCII byte by byte in UTF-8.
    let connector = LibretroThumbnailsConnector::with_transport(ListingTransport::answering(Ok(
        nes_box_arts_listing(&[
            "Burning%20Heroes%20%5bEnglish%5d.png",
            "Cotton%20100%25%20(Japan).png",
            "Kirby%e2%80%99s%20Adventure%20(USA).png",
        ]),
    )));
    let request = request_with(|draft| {
        draft.games = GameSelection::Explicit(vec![
            "Burning Heroes [English]".to_owned(),
            "Cotton 100% (Japan)".to_owned(),
            "Kirby\u{2019}s Adventure (USA)".to_owned(),
        ]);
    });

    let candidates = connector.discover(&request).unwrap();

    let found: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.original_filename.as_str())
        .collect();
    assert_eq!(
        found,
        vec![
            "Burning Heroes [English].png",
            "Cotton 100% (Japan).png",
            "Kirby\u{2019}s Adventure (USA).png",
        ]
    );
}

#[test]
fn reads_each_folder_s_listing_once_whatever_the_number_of_discoveries() {
    let transport = ListingTransport::answering(Ok(nes_box_arts_listing(&[
        "Super%20Mario%20Bros.%20(World).png",
    ])));
    let connector = LibretroThumbnailsConnector::with_transport(&transport);
    let request = request(vec![AssetTypeSelector::BoxFront]);

    let first = connector.discover(&request).unwrap();
    let second = connector.discover(&request).unwrap();

    assert_eq!(first.len(), 1);
    assert_eq!(second, first);
    let listing_reads = transport
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|url| url.as_str() == NES_BOX_ARTS_LISTING_URL)
        .count();
    assert_eq!(listing_reads, 1);
}

#[test]
fn offers_every_name_when_a_folder_s_listing_cannot_be_read() {
    for listing in [
        Err("download returned HTTP 503 Service Unavailable".to_owned()),
        Ok("<html><head><title>Maintenance</title></head><body><a href=\"Other%20Game.png\">Other Game.png</a></body></html>".to_owned()),
    ] {
        let connector =
            LibretroThumbnailsConnector::with_transport(ListingTransport::answering(listing));
        let request = request_with(|draft| {
            draft.games = GameSelection::Explicit(vec![
                "Super Mario Bros. (World)".to_owned(),
                "Homebrew Collection".to_owned(),
            ]);
        });

        let candidates = connector.discover(&request).unwrap();

        let found: Vec<_> = candidates
            .iter()
            .map(|candidate| candidate.original_filename.as_str())
            .collect();
        assert_eq!(
            found,
            vec!["Super Mario Bros. (World).png", "Homebrew Collection.png"]
        );
    }
}

#[test]
fn discovers_screenshots_and_title_screens_from_their_own_thumbnail_folders() {
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport::default());

    let candidates = connector
        .discover(&request(vec![
            AssetTypeSelector::Screenshot,
            AssetTypeSelector::TitleScreen,
        ]))
        .unwrap();

    let found: Vec<_> = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.asset_type,
                candidate.source_asset_label.as_deref().unwrap(),
                candidate.source_url.as_str(),
            )
        })
        .collect();
    assert_eq!(
        found,
        vec![
            (
                AssetType::Screenshot,
                "Named_Snaps",
                "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System/master/Named_Snaps/Super%20Mario%20Bros.%20(World).png"
            ),
            (
                AssetType::TitleScreen,
                "Named_Titles",
                "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System/master/Named_Titles/Super%20Mario%20Bros.%20(World).png"
            ),
        ]
    );
    assert_eq!(
        candidates[0].provider_candidate_id.as_deref(),
        Some("Nintendo_-_Nintendo_Entertainment_System/Named_Snaps/Super Mario Bros. (World)")
    );
}
