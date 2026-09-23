use std::{
    cell::RefCell,
    collections::HashMap,
    io::{Cursor, Read},
};

use game_media_vault_application::{
    CatalogPort, ConnectorPort, ObjectStorePort, PortError, RunRepositoryPort,
    acquire_run_with_connector,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRun,
    AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate, AssetType, AssetTypeSelector,
    ConnectorCapabilities, GameSelection, ImportedAsset, LibraryEntry, MatchConfidence,
    MatchingPolicy, PersistAsset, QualityRequirements, RetentionPolicy, SourceId, SourceSelection,
    StoredObject,
};

fn matching_policy() -> MatchingPolicy {
    MatchingPolicy {
        high_confidence_threshold: 80,
        medium_confidence_threshold: 50,
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

struct FakeRuns {
    run: RefCell<AcquisitionRun>,
    work: RefCell<HashMap<String, bool>>,
    status_after_complete: Option<AcquisitionRunStatus>,
    status_before_complete_cas: Option<AcquisitionRunStatus>,
}

impl FakeRuns {
    fn new(run: AcquisitionRun) -> Self {
        Self {
            run: RefCell::new(run),
            work: RefCell::new(HashMap::new()),
            status_after_complete: None,
            status_before_complete_cas: None,
        }
    }

    fn with_status_after_complete(mut self, status: AcquisitionRunStatus) -> Self {
        self.status_after_complete = Some(status);
        self
    }

    fn with_status_before_complete_cas(mut self, status: AcquisitionRunStatus) -> Self {
        self.status_before_complete_cas = Some(status);
        self
    }
}

impl RunRepositoryPort for FakeRuns {
    fn create_run(&self, _request: AcquisitionRequest) -> Result<AcquisitionRun, PortError> {
        unreachable!()
    }

    fn get_run(&self, run_id: i64) -> Result<Option<AcquisitionRun>, PortError> {
        Ok((self.run.borrow().id == run_id).then(|| self.run.borrow().clone()))
    }

    fn list_runs(&self) -> Result<Vec<AcquisitionRun>, PortError> {
        Ok(vec![self.run.borrow().clone()])
    }

    fn queue_work(&self, run_id: i64, work_key: String) -> Result<(), PortError> {
        assert_eq!(run_id, self.run.borrow().id);
        let mut work = self.work.borrow_mut();
        if work.insert(work_key, false).is_none() {
            self.run.borrow_mut().queued_work += 1;
        }
        Ok(())
    }

    fn next_queued_work(&self, run_id: i64) -> Result<Option<AcquisitionWorkItem>, PortError> {
        assert_eq!(run_id, self.run.borrow().id);
        Ok(self
            .work
            .borrow()
            .iter()
            .find(|(_, completed)| !**completed)
            .map(|(key, _)| AcquisitionWorkItem { key: key.clone() }))
    }

    fn complete_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError> {
        assert_eq!(run_id, self.run.borrow().id);
        let mut work = self.work.borrow_mut();
        if let Some(completed) = work.get_mut(work_key)
            && !*completed
        {
            *completed = true;
            let mut run = self.run.borrow_mut();
            run.queued_work -= 1;
            run.completed_work += 1;
            if let Some(status) = self.status_after_complete {
                run.status = status;
            }
        }
        Ok(())
    }

    fn compare_and_set_run_status(
        &self,
        run_id: i64,
        expected: AcquisitionRunStatus,
        target: AcquisitionRunStatus,
    ) -> Result<bool, PortError> {
        assert_eq!(run_id, self.run.borrow().id);
        let mut run = self.run.borrow_mut();
        if target == AcquisitionRunStatus::Completed
            && let Some(status) = self.status_before_complete_cas
        {
            run.status = status;
        }
        if run.status != expected
            || (target == AcquisitionRunStatus::Completed && run.queued_work != 0)
        {
            return Ok(false);
        }
        run.status = target;
        Ok(true)
    }
}

struct FakeConnector {
    downloads: RefCell<Vec<String>>,
    candidates: Vec<AssetCandidate>,
}

impl ConnectorPort for FakeConnector {
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
        if !self.candidates.is_empty() {
            return Ok(self.candidates.clone());
        }
        Ok(vec![AssetCandidate {
            game_title: "Super Mario Bros. (World)".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "Unknown".to_owned(),
            edition_name: "Unspecified".to_owned(),
            asset_type: AssetType::BoxFront,
            source_id: SourceId::from("libretro-thumbnails"),
            source_asset_label: Some("Named_Boxarts".to_owned()),
            source_url: "https://example.invalid/Named_Boxarts/Super%20Mario%20Bros.%20(World).png"
                .to_owned(),
            original_filename: "Super Mario Bros. (World).png".to_owned(),
        }])
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        self.downloads
            .borrow_mut()
            .push(candidate.source_url.clone());
        Ok(Box::new(Cursor::new(b"fixture box front".to_vec())))
    }
}

#[derive(Default)]
struct FakeStore {
    bytes: RefCell<Vec<Vec<u8>>>,
}

impl ObjectStorePort for FakeStore {
    fn store_original(&self, _source: &std::path::Path) -> Result<StoredObject, PortError> {
        unreachable!()
    }

    fn store_original_reader(&self, reader: &mut dyn Read) -> Result<StoredObject, PortError> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|error| PortError(error.to_string()))?;
        self.bytes.borrow_mut().push(bytes.clone());
        Ok(StoredObject {
            hash: "fixture-hash".to_owned(),
            byte_len: bytes.len() as u64,
        })
    }
}

struct FakeCatalog {
    records: RefCell<Vec<PersistAsset>>,
    library: Vec<LibraryEntry>,
}

impl Default for FakeCatalog {
    fn default() -> Self {
        Self {
            records: RefCell::new(Vec::new()),
            library: vec![matching_release()],
        }
    }
}

impl CatalogPort for FakeCatalog {
    fn persist_asset(&self, record: PersistAsset) -> Result<ImportedAsset, PortError> {
        self.records.borrow_mut().push(record.clone());
        Ok(ImportedAsset {
            game_id: 1,
            release_edition_id: 1,
            asset_id: 1,
            object_hash: record.object_hash,
            byte_len: record.byte_len,
        })
    }

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError> {
        Ok(self.library.clone())
    }
}

fn matching_release() -> LibraryEntry {
    LibraryEntry {
        game_id: 41,
        game_title: "Super Mario Bros. (World)".to_owned(),
        release_edition_id: 73,
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        assertions: Vec::new(),
        assets: Vec::new(),
    }
}

fn release_for_candidate(candidate: &AssetCandidate, release_edition_id: i64) -> LibraryEntry {
    LibraryEntry {
        game_id: release_edition_id + 1_000,
        game_title: candidate.game_title.clone(),
        release_edition_id,
        platform: candidate.platform.clone(),
        region: candidate.region.clone(),
        edition_name: candidate.edition_name.clone(),
        assertions: Vec::new(),
        assets: Vec::new(),
    }
}

#[test]
fn acquires_a_requested_box_front_through_the_connector_pipeline() {
    let run = AcquisitionRun {
        id: 7,
        request: request(),
        status: AcquisitionRunStatus::Running,
        queued_work: 0,
        completed_work: 0,
    };
    let runs = FakeRuns::new(run);
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: Vec::new(),
    };
    let store = FakeStore::default();
    let catalog = FakeCatalog::default();

    let imported =
        acquire_run_with_connector(&runs, &catalog, &store, &connector, 7, matching_policy())
            .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(connector.downloads.borrow().len(), 1);
    assert_eq!(
        store.bytes.borrow().as_slice(),
        &[b"fixture box front".to_vec()]
    );

    let records = catalog.records.borrow();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].asset_type, AssetType::BoxFront);
    assert_eq!(records[0].existing_game_id, Some(41));
    assert_eq!(records[0].existing_release_edition_id, Some(73));
    let match_decision = records[0].match_decision.as_ref().unwrap();
    assert_eq!(match_decision.release_edition_id, Some(73));
    assert_eq!(match_decision.score, 80);
    assert_eq!(match_decision.confidence, MatchConfidence::High);
    assert_eq!(match_decision.evidence.len(), 4);
    assert_eq!(records[0].source_id, SourceId::from("libretro-thumbnails"));
    assert_eq!(
        records[0].source_location,
        "https://example.invalid/Named_Boxarts/Super%20Mario%20Bros.%20(World).png"
    );

    let final_run = runs.run.borrow();
    assert_eq!(final_run.status, AcquisitionRunStatus::Completed);
    assert_eq!(final_run.queued_work, 0);
    assert_eq!(final_run.completed_work, 1);
}

#[test]
fn low_confidence_candidate_is_left_unattached_without_downloading() {
    let runs = FakeRuns::new(run_with_request(request()));
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![AssetCandidate {
            game_title: "Completely Different Game".to_owned(),
            platform: "Different Platform".to_owned(),
            region: "Europe".to_owned(),
            edition_name: "Collector".to_owned(),
            asset_type: AssetType::BoxFront,
            source_id: SourceId::from("libretro-thumbnails"),
            source_asset_label: Some("Named_Boxarts".to_owned()),
            source_url: "https://example.invalid/unmatched.png".to_owned(),
            original_filename: "unmatched.png".to_owned(),
        }],
    };
    let store = FakeStore::default();
    let catalog = FakeCatalog::default();

    let imported =
        acquire_run_with_connector(&runs, &catalog, &store, &connector, 7, matching_policy())
            .unwrap();

    assert!(imported.is_empty());
    assert!(connector.downloads.borrow().is_empty());
    assert!(store.bytes.borrow().is_empty());
    assert!(catalog.records.borrow().is_empty());
    assert_eq!(runs.run.borrow().completed_work, 1);
    assert_eq!(runs.run.borrow().status, AcquisitionRunStatus::Completed);
}

#[test]
fn packaging_selector_acquires_the_supported_box_front() {
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::Packaging],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();
    let runs = FakeRuns::new(run_with_request(request));
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: Vec::new(),
    };
    let store = FakeStore::default();
    let catalog = FakeCatalog::default();

    let imported =
        acquire_run_with_connector(&runs, &catalog, &store, &connector, 7, matching_policy())
            .unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(connector.downloads.borrow().len(), 1);
    assert_eq!(catalog.records.borrow()[0].asset_type, AssetType::BoxFront);
}

#[test]
fn pause_or_cancel_during_the_last_download_preserves_the_requested_run_status() {
    for status in [
        AcquisitionRunStatus::Paused,
        AcquisitionRunStatus::Cancelled,
    ] {
        let runs = FakeRuns::new(run_with_request(request())).with_status_after_complete(status);
        let connector = FakeConnector {
            downloads: RefCell::new(Vec::new()),
            candidates: Vec::new(),
        };

        let imported = acquire_run_with_connector(
            &runs,
            &FakeCatalog::default(),
            &FakeStore::default(),
            &connector,
            7,
            matching_policy(),
        )
        .unwrap();

        assert_eq!(imported.len(), 1);
        assert_eq!(runs.run.borrow().status, status);
        assert_eq!(runs.run.borrow().queued_work, 0);
    }
}

#[test]
fn pause_or_cancel_winning_the_final_completion_race_does_not_fail_execution() {
    for status in [
        AcquisitionRunStatus::Paused,
        AcquisitionRunStatus::Cancelled,
    ] {
        let runs =
            FakeRuns::new(run_with_request(request())).with_status_before_complete_cas(status);
        let connector = FakeConnector {
            downloads: RefCell::new(Vec::new()),
            candidates: Vec::new(),
        };

        let imported = acquire_run_with_connector(
            &runs,
            &FakeCatalog::default(),
            &FakeStore::default(),
            &connector,
            7,
            matching_policy(),
        )
        .unwrap();

        assert_eq!(imported.len(), 1);
        assert_eq!(runs.run.borrow().status, status);
        assert_eq!(runs.run.borrow().queued_work, 0);
    }
}

fn run_with_request(request: AcquisitionRequest) -> AcquisitionRun {
    AcquisitionRun {
        id: 7,
        request,
        status: AcquisitionRunStatus::Running,
        queued_work: 0,
        completed_work: 0,
    }
}

fn execute_error(request: AcquisitionRequest) -> game_media_vault_application::ApplicationError {
    let runs = FakeRuns::new(run_with_request(request));
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: Vec::new(),
    };
    acquire_run_with_connector(
        &runs,
        &FakeCatalog::default(),
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap_err()
}

#[test]
fn rejects_multi_source_execution_until_a_multi_source_plan_exists() {
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec![
            "libretro-thumbnails".to_owned(),
            "screenscraper".to_owned(),
        ]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    let error = execute_error(request);

    assert!(matches!(
        error,
        game_media_vault_application::ApplicationError::UnsupportedConnectorPlan { .. }
    ));
}

#[test]
fn rejects_quality_and_retention_behavior_reserved_for_the_quality_slice() {
    let quality_request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: Some(QualityRequirements {
            accepted_mime_types: vec!["image/jpeg".to_owned()],
            ..QualityRequirements::default()
        }),
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();
    let retention_request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepBestPerType,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    assert!(matches!(
        execute_error(quality_request),
        game_media_vault_application::ApplicationError::UnsupportedConnectorPlan { .. }
    ));
    assert!(matches!(
        execute_error(retention_request),
        game_media_vault_application::ApplicationError::UnsupportedConnectorPlan { .. }
    ));
}

#[test]
fn rejects_acquisition_limits_until_the_scheduler_slice_can_enforce_them() {
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits {
            max_downloads: Some(1),
            ..AcquisitionLimits::default()
        },
    })
    .unwrap();

    assert!(matches!(
        execute_error(request),
        game_media_vault_application::ApplicationError::UnsupportedConnectorPlan { .. }
    ));
}

#[test]
fn rejects_asset_types_not_declared_by_the_connector() {
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Nintendo Entertainment System".to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::Screenshot],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();

    assert!(matches!(
        execute_error(request),
        game_media_vault_application::ApplicationError::UnsupportedConnectorPlan { .. }
    ));
}

#[test]
fn distinct_candidates_that_share_a_source_url_keep_distinct_work_items() {
    let first = AssetCandidate {
        game_title: "A:B".to_owned(),
        platform: "Nintendo - Nintendo Entertainment System".to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("libretro-thumbnails"),
        source_asset_label: Some("Named_Boxarts".to_owned()),
        source_url: "https://example.invalid/Named_Boxarts/A_B.png".to_owned(),
        original_filename: "A_B.png".to_owned(),
    };
    let second = AssetCandidate {
        game_title: "A?B".to_owned(),
        ..first.clone()
    };
    let runs = FakeRuns::new(run_with_request(request()));
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        library: vec![
            release_for_candidate(&first, 81),
            release_for_candidate(&second, 82),
        ],
    };
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![first, second],
    };

    let imported = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 2);
    assert_eq!(catalog.records.borrow().len(), 2);
    assert_eq!(runs.run.borrow().completed_work, 2);
}

#[test]
fn candidate_identity_fields_cannot_collide_through_work_key_delimiters() {
    let first = AssetCandidate {
        game_title: "C".to_owned(),
        platform: "A:B".to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("libretro-thumbnails"),
        source_asset_label: Some("Named_Boxarts".to_owned()),
        source_url: "https://example.invalid/shared.png".to_owned(),
        original_filename: "shared.png".to_owned(),
    };
    let second = AssetCandidate {
        game_title: "B:C".to_owned(),
        platform: "A".to_owned(),
        ..first.clone()
    };
    let runs = FakeRuns::new(run_with_request(request()));
    let catalog = FakeCatalog {
        records: RefCell::new(Vec::new()),
        library: vec![
            release_for_candidate(&first, 91),
            release_for_candidate(&second, 92),
        ],
    };
    let connector = FakeConnector {
        downloads: RefCell::new(Vec::new()),
        candidates: vec![first, second],
    };

    let imported = acquire_run_with_connector(
        &runs,
        &catalog,
        &FakeStore::default(),
        &connector,
        7,
        matching_policy(),
    )
    .unwrap();

    assert_eq!(imported.len(), 2);
    assert_eq!(catalog.records.borrow().len(), 2);
    assert_eq!(runs.run.borrow().completed_work, 2);
}
