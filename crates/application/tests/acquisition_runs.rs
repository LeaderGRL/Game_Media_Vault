use std::cell::RefCell;

use game_media_vault_application::{
    AcquisitionRequestInput, PortError, RunRepositoryPort, start_acquisition_run,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRun, AcquisitionRunStatus, AssetTypeSelector,
    GameSelection, RetentionPolicy, SourceSelection,
};
use serde_json::json;

struct RecordingRunRepository {
    persisted_requests: RefCell<Vec<serde_json::Value>>,
}

impl RecordingRunRepository {
    fn new() -> Self {
        Self {
            persisted_requests: RefCell::new(Vec::new()),
        }
    }
}

impl RunRepositoryPort for RecordingRunRepository {
    fn create_run(&self, request: AcquisitionRequest) -> Result<AcquisitionRun, PortError> {
        self.persisted_requests
            .borrow_mut()
            .push(serde_json::to_value(&request).unwrap());

        Ok(AcquisitionRun {
            id: 7,
            request,
            status: AcquisitionRunStatus::Running,
            queued_work: 0,
            completed_work: 0,
        })
    }
}

#[test]
fn starting_an_acquisition_run_persists_the_validated_request() {
    let runs = RecordingRunRepository::new();

    let run = start_acquisition_run(
        &runs,
        AcquisitionRequestInput {
            sources: SourceSelection::Auto,
            platforms: vec!["Windows".to_owned()],
            games: GameSelection::All,
            regions: Vec::new(),
            languages: Vec::new(),
            asset_types: vec![AssetTypeSelector::BoxFront],
            quality: None,
            retention: RetentionPolicy::KeepEverything,
            limits: AcquisitionLimits::default(),
        },
    )
    .unwrap();

    assert_eq!(run.id, 7);
    assert_eq!(run.status, AcquisitionRunStatus::Running);
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 0);
    assert_eq!(
        runs.persisted_requests.borrow().as_slice(),
        &[json!({
            "sources": { "mode": "auto" },
            "platforms": ["Windows"],
            "games": { "mode": "all" },
            "regions": [],
            "languages": [],
            "asset_types": ["box_front"],
            "quality": null,
            "retention": "keep_everything",
            "limits": {
                "max_games": null,
                "max_downloads": null,
                "max_concurrent_downloads": null,
                "max_bytes": null
            }
        })]
    );
}
