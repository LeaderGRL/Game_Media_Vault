use std::{
    io::{Cursor, Read},
    sync::{Arc, Mutex},
};

use game_media_vault_application::{
    CatalogPort, ConnectorPort, DownloadLimits, PortError, ReferenceCatalogRepositoryPort,
    ReviewRepositoryPort, RunRepositoryPort, acquire_run_with_connectors, resolve_review_item,
    start_acquisition_run_with_connectors,
};
use game_media_vault_connectors::{
    HttpTransport, LAUNCHBOX_METADATA_URL, LaunchBoxGamesDbConnector, LibretroThumbnailsConnector,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRunStatus,
    AssetCandidate, AssetType, AssetTypeSelector, ConnectorCapabilities, GameSelection,
    MatchConfidence, MatchingPolicy, ReferenceReleaseRecord, ReleaseAssertion,
    ReleaseAssertionField, RetentionPolicy, ReviewDecision, ReviewStatus, SourceId,
    SourceSelection,
};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use tempfile::tempdir;

const BOX_FRONT_BYTES: &[u8] = b"libretro end-to-end box front fixture";
const GITMODULES_FIXTURE: &[u8] = br#"
[submodule "Nintendo - Nintendo Entertainment System"]
    path = Nintendo - Nintendo Entertainment System
    url = https://github.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System.git
    branch = master
"#;

#[derive(Clone)]
struct FixtureTransport {
    requested_urls: Arc<Mutex<Vec<String>>>,
}

impl HttpTransport for FixtureTransport {
    fn get_stream(
        &self,
        url: &str,
    ) -> Result<Box<dyn Read + Send>, game_media_vault_application::PortError> {
        self.requested_urls.lock().unwrap().push(url.to_owned());
        if url.ends_with("/.gitmodules") {
            Ok(Box::new(Cursor::new(GITMODULES_FIXTURE.to_vec())))
        } else {
            Ok(Box::new(Cursor::new(BOX_FRONT_BYTES.to_vec())))
        }
    }
}

struct NoDiscoveryConnector;

impl ConnectorPort for NoDiscoveryConnector {
    fn source_id(&self) -> &'static str {
        "libretro-thumbnails"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::BoxFront],
            direct_media_download: true,
        }
    }

    fn discover(&self, _request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        Ok(Vec::new())
    }

    fn download(&self, _candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        Ok(Box::new(Cursor::new(BOX_FRONT_BYTES.to_vec())))
    }
}

fn request() -> AcquisitionRequest {
    AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap()
}

#[test]
fn acquires_and_persists_a_libretro_box_front_end_to_end_without_live_network() {
    let temp = tempdir().unwrap();
    let catalog_path = temp.path().join("catalog.sqlite3");
    let object_root = temp.path().join("objects");
    let catalog = SqliteCatalog::open(&catalog_path).unwrap();
    catalog
        .persist_reference_release(ReferenceReleaseRecord {
            game_title: "Super Mario Bros.".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "World".to_owned(),
            revision: None,
            edition_name: "Standard".to_owned(),
            assertions: vec![ReleaseAssertion {
                source_id: SourceId::from("fixture-reference"),
                source_location: "fixture://reference".to_owned(),
                field: ReleaseAssertionField::Identifier,
                qualifier: Some("source_record".to_owned()),
                value: "fixture:super-mario-bros-world".to_owned(),
            }],
        })
        .unwrap();
    let object_store = ContentAddressedStore::new(&object_root);
    let requested_urls = Arc::new(Mutex::new(Vec::new()));
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport {
        requested_urls: requested_urls.clone(),
    });

    let run = catalog
        .create_run(request(), vec!["libretro-thumbnails".to_owned()])
        .unwrap();
    let imported = acquire_run_with_connectors(
        &catalog,
        &catalog,
        &catalog,
        &object_store,
        &[&connector],
        run.id,
        MatchingPolicy {
            high_confidence_threshold: 80,
            medium_confidence_threshold: 50,
        },
        DownloadLimits::default(),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    let final_run = catalog.get_run(run.id).unwrap().unwrap();
    assert_eq!(final_run.status, AcquisitionRunStatus::Completed);
    assert_eq!(final_run.queued_work, 0);
    assert_eq!(final_run.completed_work, 1);

    let library = catalog.list_library().unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].assets.len(), 1);
    let asset = &library[0].assets[0];
    assert_eq!(asset.asset_type, AssetType::BoxFront);
    assert_eq!(asset.original_filename, "Super Mario Bros. (World).png");
    assert_eq!(asset.provenance.len(), 1);
    assert_eq!(
        asset.provenance[0].source_id,
        SourceId::from("libretro-thumbnails")
    );
    assert_eq!(
        asset.provenance[0].source_asset_label.as_deref(),
        Some("Named_Boxarts")
    );
    assert_eq!(
        asset.provenance[0].source_location,
        "https://raw.githubusercontent.com/libretro-thumbnails/Nintendo_-_Nintendo_Entertainment_System/master/Named_Boxarts/Super%20Mario%20Bros.%20(World).png"
    );

    assert_eq!(
        std::fs::read(object_store.object_path(&asset.object_hash)).unwrap(),
        BOX_FRONT_BYTES
    );
    assert_eq!(
        requested_urls.lock().unwrap().as_slice(),
        &[
            "https://raw.githubusercontent.com/libretro-thumbnails/libretro-thumbnails/master/.gitmodules".to_owned(),
            asset.provenance[0].source_location.clone(),
        ]
    );

    drop(catalog);
    let reopened = SqliteCatalog::open_existing(&catalog_path).unwrap();
    let reopened_library = reopened.list_library().unwrap();
    assert_eq!(reopened_library, library);
}

#[test]
fn accepted_review_is_applied_after_reopening_without_rediscovery() {
    let temp = tempdir().unwrap();
    let catalog_path = temp.path().join("catalog.sqlite3");
    let object_root = temp.path().join("objects");
    let catalog = SqliteCatalog::open(&catalog_path).unwrap();
    let release = catalog
        .persist_reference_release(ReferenceReleaseRecord {
            game_title: "Super Mario Bros.".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "World".to_owned(),
            revision: None,
            edition_name: "Rev 1".to_owned(),
            assertions: vec![ReleaseAssertion {
                source_id: SourceId::from("fixture-reference"),
                source_location: "fixture://reference".to_owned(),
                field: ReleaseAssertionField::Identifier,
                qualifier: Some("source_record".to_owned()),
                value: "fixture:super-mario-bros-world".to_owned(),
            }],
        })
        .unwrap();
    let run = catalog
        .create_run(request(), vec!["libretro-thumbnails".to_owned()])
        .unwrap();
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport {
        requested_urls: Arc::new(Mutex::new(Vec::new())),
    });
    // The release is a different revision, so the edition conflict makes the match uncertain.
    let policy = MatchingPolicy {
        high_confidence_threshold: 80,
        medium_confidence_threshold: 50,
    };

    let imported = acquire_run_with_connectors(
        &catalog,
        &catalog,
        &catalog,
        &ContentAddressedStore::new(&object_root),
        &[&connector],
        run.id,
        policy,
        DownloadLimits::default(),
    )
    .unwrap();
    assert!(imported.is_empty());
    assert_eq!(
        catalog
            .get_run(run.id)
            .unwrap()
            .unwrap()
            .awaiting_review_work,
        1
    );
    drop(catalog);

    let reopened = SqliteCatalog::open_existing(&catalog_path).unwrap();
    let review_item_id = reopened.list_review_items().unwrap()[0].id;
    resolve_review_item(
        &reopened,
        review_item_id,
        ReviewDecision::Accept {
            release_edition_id: release.release_edition_id,
        },
    )
    .unwrap();
    let imported = acquire_run_with_connectors(
        &reopened,
        &reopened,
        &reopened,
        &ContentAddressedStore::new(&object_root),
        &[&NoDiscoveryConnector],
        run.id,
        policy,
        DownloadLimits::default(),
    )
    .unwrap();

    assert_eq!(imported.len(), 1);
    let final_run = reopened.get_run(run.id).unwrap().unwrap();
    assert_eq!(final_run.status, AcquisitionRunStatus::Completed);
    assert_eq!(final_run.completed_work, 1);
    assert_eq!(
        reopened.list_review_items().unwrap()[0].status,
        ReviewStatus::Accepted
    );
    let library = reopened.list_library().unwrap();
    let provenance = &library[0].assets[0].provenance[0];
    assert_eq!(
        provenance.match_decision.as_ref().unwrap().confidence,
        MatchConfidence::Confirmed
    );
}

const LAUNCHBOX_BOX_FRONT_BYTES: &[u8] = b"launchbox end-to-end box front fixture";

/// Serves a LaunchBox dataset holding one worldwide Box Front of Super Mario Bros.
struct LaunchBoxFixtureTransport;

impl HttpTransport for LaunchBoxFixtureTransport {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        if url != LAUNCHBOX_METADATA_URL {
            return Ok(Box::new(Cursor::new(LAUNCHBOX_BOX_FRONT_BYTES.to_vec())));
        }
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive
            .start_file("Metadata.xml", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(
            &mut archive,
            br#"<LaunchBox>
  <Game>
    <Name>Super Mario Bros.</Name>
    <DatabaseID>140</DatabaseID>
    <Platform>Nintendo Entertainment System</Platform>
  </Game>
  <GameImage>
    <DatabaseID>140</DatabaseID>
    <FileName>smb-front-world.jpg</FileName>
    <Type>Box - Front</Type>
    <Region>World</Region>
  </GameImage>
</LaunchBox>"#,
        )
        .unwrap();
        Ok(Box::new(Cursor::new(
            archive.finish().unwrap().into_inner(),
        )))
    }
}

#[test]
fn one_auto_run_acquires_from_both_sources_with_distinct_provenance() {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    catalog
        .persist_reference_release(ReferenceReleaseRecord {
            game_title: "Super Mario Bros.".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "World".to_owned(),
            revision: None,
            edition_name: "Standard".to_owned(),
            assertions: vec![ReleaseAssertion {
                source_id: SourceId::from("fixture-reference"),
                source_location: "fixture://reference".to_owned(),
                field: ReleaseAssertionField::Identifier,
                qualifier: Some("source_record".to_owned()),
                value: "fixture:super-mario-bros-world".to_owned(),
            }],
        })
        .unwrap();
    let object_store = ContentAddressedStore::new(temp.path().join("objects"));
    let libretro = LibretroThumbnailsConnector::with_transport(FixtureTransport {
        requested_urls: Arc::new(Mutex::new(Vec::new())),
    });
    let launchbox = LaunchBoxGamesDbConnector::with_transport(LaunchBoxFixtureTransport);
    let connectors: [&dyn ConnectorPort; 2] = [&libretro, &launchbox];
    let draft = AcquisitionRequestDraft {
        sources: SourceSelection::Auto,
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };

    let run = start_acquisition_run_with_connectors(&catalog, draft, &connectors).unwrap();
    let imported = acquire_run_with_connectors(
        &catalog,
        &catalog,
        &catalog,
        &object_store,
        &connectors,
        run.id,
        MatchingPolicy {
            high_confidence_threshold: 80,
            medium_confidence_threshold: 50,
        },
        DownloadLimits::default(),
    )
    .unwrap();

    assert_eq!(
        run.planned_sources,
        ["libretro-thumbnails", "launchbox-games-db"]
    );
    assert_eq!(imported.len(), 2);
    let library = catalog.list_library().unwrap();
    let sources: Vec<SourceId> = library[0]
        .assets
        .iter()
        .map(|asset| asset.provenance[0].source_id.clone())
        .collect();
    assert_eq!(
        sources,
        [
            SourceId::from("libretro-thumbnails"),
            SourceId::from("launchbox-games-db")
        ]
    );
    assert_eq!(
        library[0].assets[1].provenance[0].source_location,
        "https://images.launchbox-app.com/smb-front-world.jpg"
    );
    assert_eq!(
        catalog.get_run(run.id).unwrap().unwrap().status,
        AcquisitionRunStatus::Completed
    );
}
