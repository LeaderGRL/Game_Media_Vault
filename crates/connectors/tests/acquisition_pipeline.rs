use std::{
    io::{Cursor, Read},
    sync::{Arc, Mutex},
};

use game_media_vault_application::{CatalogPort, RunRepositoryPort, acquire_run_with_connector};
use game_media_vault_connectors::{HttpTransport, LibretroThumbnailsConnector};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRunStatus,
    AssetType, AssetTypeSelector, GameSelection, RetentionPolicy, SourceId, SourceSelection,
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
    let object_store = ContentAddressedStore::new(&object_root);
    let requested_urls = Arc::new(Mutex::new(Vec::new()));
    let connector = LibretroThumbnailsConnector::with_transport(FixtureTransport {
        requested_urls: requested_urls.clone(),
    });

    let run = catalog.create_run(request()).unwrap();
    let imported =
        acquire_run_with_connector(&catalog, &catalog, &object_store, &connector, run.id).unwrap();

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
