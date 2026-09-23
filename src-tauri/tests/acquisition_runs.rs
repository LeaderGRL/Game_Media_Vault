use std::{
    io::{Cursor, Read},
    sync::mpsc,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use game_media_vault_application::{
    AcquisitionRequestInput, ConnectorPort, PortError, ReferenceCatalogRepositoryPort,
    load_acquisition_run as load_acquisition_run_use_case,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRunStatus, AssetCandidate, AssetType,
    AssetTypeSelector, ConnectorCapabilities, GameSelection, MatchingPolicy, ReferenceReleaseRecord,
    ReleaseAssertion, ReleaseAssertionField, RetentionPolicy, SourceId, SourceSelection,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::tempdir;

fn matching_policy() -> MatchingPolicy {
    MatchingPolicy {
        high_confidence_threshold: 80,
        medium_confidence_threshold: 50,
    }
}

fn request_input() -> AcquisitionRequestInput {
    AcquisitionRequestInput {
        sources: SourceSelection::Explicit(vec!["screenscraper".to_owned()]),
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::All,
        regions: Vec::new(),
        languages: vec!["en".to_owned()],
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    }
}

struct FixtureConnector;

fn seed_matching_release(vault: &std::path::Path) {
    let catalog = SqliteCatalog::open_existing(vault.join("catalog.sqlite3")).unwrap();
    catalog
        .persist_reference_release(ReferenceReleaseRecord {
            game_title: "Super Mario Bros. (World)".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "World".to_owned(),
            revision: None,
            edition_name: "Unspecified".to_owned(),
            assertions: vec![ReleaseAssertion {
                source_id: SourceId::from("fixture-reference"),
                source_location: "fixture://reference".to_owned(),
                field: ReleaseAssertionField::Identifier,
                qualifier: Some("source_record".to_owned()),
                value: "fixture:super-mario-bros-world".to_owned(),
            }],
        })
        .unwrap();
}

impl ConnectorPort for FixtureConnector {
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
        Ok(vec![AssetCandidate {
            game_title: "Super Mario Bros. (World)".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "World".to_owned(),
            edition_name: "Unspecified".to_owned(),
            asset_type: AssetType::BoxFront,
            source_id: SourceId::from("libretro-thumbnails"),
            source_asset_label: Some("Named_Boxarts".to_owned()),
            source_url: "https://example.invalid/smb-box-front.png".to_owned(),
            original_filename: "Super Mario Bros. (World).png".to_owned(),
        }])
    }

    fn download(&self, _candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        Ok(Box::new(Cursor::new(b"tauri connector fixture".to_vec())))
    }
}

struct ThreadRecordingConnector {
    worker_thread: Arc<Mutex<Option<thread::ThreadId>>>,
}

impl ConnectorPort for ThreadRecordingConnector {
    fn source_id(&self) -> &'static str {
        "libretro-thumbnails"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        FixtureConnector.capabilities()
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        *self.worker_thread.lock().unwrap() = Some(thread::current().id());
        FixtureConnector.discover(request)
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        *self.worker_thread.lock().unwrap() = Some(thread::current().id());
        FixtureConnector.download(candidate)
    }
}

struct BlockingConnector {
    download_started: mpsc::Sender<()>,
    continue_download: mpsc::Receiver<()>,
}

impl ConnectorPort for BlockingConnector {
    fn source_id(&self) -> &'static str {
        "libretro-thumbnails"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        FixtureConnector.capabilities()
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        FixtureConnector.discover(request)
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        self.download_started
            .send(())
            .map_err(|error| PortError(error.to_string()))?;
        self.continue_download
            .recv()
            .map_err(|error| PortError(error.to_string()))?;
        FixtureConnector.download(candidate)
    }
}

#[test]
fn tauri_exposes_the_shared_persisted_acquisition_run_state() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");

    let started =
        game_media_vault_tauri::start_acquisition_run_in_vault(&vault, request_input()).unwrap();
    assert_eq!(started.status, AcquisitionRunStatus::Running);

    let catalog = SqliteCatalog::open_existing(vault.join("catalog.sqlite3")).unwrap();
    let application_view = load_acquisition_run_use_case(&catalog, started.id).unwrap();
    assert_eq!(
        serde_json::to_value(&started).unwrap(),
        serde_json::to_value(&application_view).unwrap()
    );

    let loaded =
        game_media_vault_tauri::load_acquisition_run_from_vault(&vault, started.id).unwrap();
    assert_eq!(loaded, application_view);

    let listed = game_media_vault_tauri::list_acquisition_runs_from_vault(&vault).unwrap();
    assert_eq!(listed, vec![application_view.clone()]);

    let paused =
        game_media_vault_tauri::pause_acquisition_run_in_vault(&vault, started.id).unwrap();
    assert_eq!(paused.status, AcquisitionRunStatus::Paused);

    let resumed =
        game_media_vault_tauri::resume_acquisition_run_in_vault(&vault, started.id).unwrap();
    assert_eq!(resumed.status, AcquisitionRunStatus::Running);

    let cancelled =
        game_media_vault_tauri::cancel_acquisition_run_in_vault(&vault, started.id).unwrap();
    assert_eq!(cancelled.status, AcquisitionRunStatus::Cancelled);

    let reloaded =
        game_media_vault_tauri::load_acquisition_run_from_vault(&vault, started.id).unwrap();
    assert_eq!(reloaded, cancelled);
}

#[test]
fn tauri_adapter_can_execute_a_persisted_run_through_a_connector() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let request = AcquisitionRequestInput {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };
    let started = game_media_vault_tauri::start_acquisition_run_in_vault(&vault, request).unwrap();
    seed_matching_release(&vault);

    let completed = game_media_vault_tauri::execute_acquisition_run_in_vault_with_connector(
        &vault,
        started.id,
        &FixtureConnector,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(completed.status, AcquisitionRunStatus::Completed);
    let library = game_media_vault_tauri::load_library(&vault).unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(
        library[0].assets[0].provenance[0].source_location,
        "https://example.invalid/smb-box-front.png"
    );
}

#[test]
fn tauri_async_adapter_runs_blocking_acquisition_off_the_calling_thread() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let request = AcquisitionRequestInput {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };
    let started = game_media_vault_tauri::start_acquisition_run_in_vault(&vault, request).unwrap();
    let calling_thread = thread::current().id();
    let worker_thread = Arc::new(Mutex::new(None));

    let completed = tauri::async_runtime::block_on(
        game_media_vault_tauri::execute_acquisition_run_in_vault_with_connector_async(
            vault,
            started.id,
            Box::new(ThreadRecordingConnector {
                worker_thread: Arc::clone(&worker_thread),
            }),
            matching_policy(),
        ),
    )
    .unwrap();

    assert_eq!(completed.status, AcquisitionRunStatus::Completed);
    assert_ne!(worker_thread.lock().unwrap().unwrap(), calling_thread);
}

#[test]
fn tauri_async_execution_preserves_pause_or_cancel_during_an_active_download() {
    for target_status in [
        AcquisitionRunStatus::Paused,
        AcquisitionRunStatus::Cancelled,
    ] {
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        let request = AcquisitionRequestInput {
            sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
            platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
            games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
            regions: Vec::new(),
            languages: Vec::new(),
            asset_types: vec![AssetTypeSelector::BoxFront],
            quality: None,
            retention: RetentionPolicy::KeepEverything,
            limits: AcquisitionLimits::default(),
        };
        let started =
            game_media_vault_tauri::start_acquisition_run_in_vault(&vault, request).unwrap();
        seed_matching_release(&vault);
        let (download_started_tx, download_started_rx) = mpsc::channel();
        let (continue_download_tx, continue_download_rx) = mpsc::channel();
        let execution_vault = vault.clone();

        let execution = thread::spawn(move || {
            tauri::async_runtime::block_on(
                game_media_vault_tauri::execute_acquisition_run_in_vault_with_connector_async(
                    execution_vault,
                    started.id,
                    Box::new(BlockingConnector {
                        download_started: download_started_tx,
                        continue_download: continue_download_rx,
                    }),
                    matching_policy(),
                ),
            )
        });

        download_started_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let stopped = match target_status {
            AcquisitionRunStatus::Paused => {
                game_media_vault_tauri::pause_acquisition_run_in_vault(&vault, started.id).unwrap()
            }
            AcquisitionRunStatus::Cancelled => {
                game_media_vault_tauri::cancel_acquisition_run_in_vault(&vault, started.id).unwrap()
            }
            _ => unreachable!(),
        };
        assert_eq!(stopped.status, target_status);

        continue_download_tx.send(()).unwrap();
        let execution_result = execution.join().unwrap().unwrap();

        assert_eq!(execution_result.status, target_status);
        assert_eq!(execution_result.queued_work, 0);
    }
}
