use std::{cell::RefCell, collections::HashMap};

use game_media_vault_application::{
    CatalogPort, ConnectorPort, ObjectStorePort, PortError, RunRepositoryPort,
    acquire_run_with_connector,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRun,
    AcquisitionRunStatus, AcquisitionWorkItem, AssetCandidate, AssetType, AssetTypeSelector,
    ConnectorCapabilities, GameSelection, ImportedAsset, LibraryEntry, PersistAsset,
    RetentionPolicy, SourceKind, SourceSelection, StoredObject,
};

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
}

impl FakeRuns {
    fn new(run: AcquisitionRun) -> Self {
        Self {
            run: RefCell::new(run),
            work: RefCell::new(HashMap::new()),
        }
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
        Ok(vec![AssetCandidate {
            game_title: "Super Mario Bros. (World)".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "Unknown".to_owned(),
            edition_name: "Unspecified".to_owned(),
            asset_type: AssetType::BoxFront,
            source_kind: SourceKind::LibretroThumbnails,
            source_url: "https://example.invalid/Named_Boxarts/Super%20Mario%20Bros.%20(World).png"
                .to_owned(),
            original_filename: "Super Mario Bros. (World).png".to_owned(),
        }])
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Vec<u8>, PortError> {
        self.downloads
            .borrow_mut()
            .push(candidate.source_url.clone());
        Ok(b"fixture box front".to_vec())
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

    fn store_original_bytes(&self, bytes: &[u8]) -> Result<StoredObject, PortError> {
        self.bytes.borrow_mut().push(bytes.to_vec());
        Ok(StoredObject {
            hash: "fixture-hash".to_owned(),
            byte_len: bytes.len() as u64,
        })
    }
}

#[derive(Default)]
struct FakeCatalog {
    records: RefCell<Vec<PersistAsset>>,
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
        Ok(Vec::new())
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
    };
    let store = FakeStore::default();
    let catalog = FakeCatalog::default();

    let imported = acquire_run_with_connector(&runs, &catalog, &store, &connector, 7).unwrap();

    assert_eq!(imported.len(), 1);
    assert_eq!(connector.downloads.borrow().len(), 1);
    assert_eq!(
        store.bytes.borrow().as_slice(),
        &[b"fixture box front".to_vec()]
    );

    let records = catalog.records.borrow();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].asset_type, AssetType::BoxFront);
    assert_eq!(records[0].source_kind, SourceKind::LibretroThumbnails);
    assert_eq!(
        records[0].source_location,
        "https://example.invalid/Named_Boxarts/Super%20Mario%20Bros.%20(World).png"
    );

    let final_run = runs.run.borrow();
    assert_eq!(final_run.status, AcquisitionRunStatus::Completed);
    assert_eq!(final_run.queued_work, 0);
    assert_eq!(final_run.completed_work, 1);
}
