use std::sync::Mutex;

use game_media_vault_application::{
    AcquisitionRequestInput, AcquisitionRequestValidationError, ApplicationError, PortError,
    RunRepositoryPort, pause_acquisition_run, start_acquisition_run,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRun, AcquisitionRunStatus,
    AcquisitionWorkItem, AssetTypeSelector, GameSelection, RetentionPolicy, SourceFailure,
    SourceFailureStage, SourceSelection,
};
use serde_json::json;

struct RecordingRunRepository {
    persisted_requests: Mutex<Vec<serde_json::Value>>,
}

impl RecordingRunRepository {
    fn new() -> Self {
        Self {
            persisted_requests: Mutex::new(Vec::new()),
        }
    }
}

impl RunRepositoryPort for RecordingRunRepository {
    fn create_run(
        &self,
        request: AcquisitionRequest,
        planned_sources: Vec<String>,
    ) -> Result<AcquisitionRun, PortError> {
        self.persisted_requests
            .lock()
            .unwrap()
            .push(serde_json::to_value(&request).unwrap());

        Ok(AcquisitionRun {
            id: 7,
            request,
            planned_sources,
            status: AcquisitionRunStatus::Running,
            queued_work: 0,
            awaiting_review_work: 0,
            completed_work: 0,
            below_quality_work: 0,
            outranked_work: 0,
            unavailable_work: 0,
        })
    }

    fn get_run(&self, _run_id: i64) -> Result<Option<AcquisitionRun>, PortError> {
        Ok(None)
    }

    fn claim_execution(&self, _run_id: i64) -> Result<bool, PortError> {
        Ok(true)
    }

    fn release_execution(&self, _run_id: i64) -> Result<(), PortError> {
        Ok(())
    }

    fn run_status(&self, _run_id: i64) -> Result<Option<AcquisitionRunStatus>, PortError> {
        Ok(None)
    }

    fn list_runs(&self) -> Result<Vec<AcquisitionRun>, PortError> {
        Ok(Vec::new())
    }

    fn has_discovered(&self, _run_id: i64, _source_id: &str) -> Result<bool, PortError> {
        Ok(false)
    }

    fn record_discovery(
        &self,
        _run_id: i64,
        _source_id: &str,
        _work: &[AcquisitionWorkItem],
    ) -> Result<bool, PortError> {
        Ok(true)
    }
    fn next_queued_work(
        &self,
        _run_id: i64,
        _skipped_sources: &[String],
    ) -> Result<Option<AcquisitionWorkItem>, PortError> {
        Ok(None)
    }

    fn complete_work(&self, _run_id: i64, _work_key: &str) -> Result<(), PortError> {
        Ok(())
    }

    fn complete_unavailable_work(
        &self,
        _run_id: i64,
        _work_key: &str,
        _reason: &str,
    ) -> Result<(), PortError> {
        Ok(())
    }

    fn queued_work(
        &self,
        _run_id: i64,
        _skipped_sources: &[String],
        _per_source: usize,
    ) -> Result<Vec<AcquisitionWorkItem>, PortError> {
        Ok(Vec::new())
    }

    fn record_source_failure(
        &self,
        _run_id: i64,
        _source_id: &str,
        _stage: SourceFailureStage,
        _message: &str,
    ) -> Result<(), PortError> {
        Ok(())
    }

    fn source_failures(&self) -> Result<Vec<SourceFailure>, PortError> {
        Ok(Vec::new())
    }

    fn compare_and_set_run_status(
        &self,
        _run_id: i64,
        _expected: AcquisitionRunStatus,
        _target: AcquisitionRunStatus,
    ) -> Result<bool, PortError> {
        Err(PortError::new(
            "not implemented by this test double".to_owned(),
        ))
    }
}

#[test]
fn starting_an_acquisition_run_persists_the_validated_request() {
    let runs = RecordingRunRepository::new();

    let run = start_acquisition_run(
        &runs,
        AcquisitionRequestInput {
            sources: SourceSelection::Explicit(vec!["fixture-provider".to_owned()]),
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
    assert_eq!(run.planned_sources, ["fixture-provider"]);
    assert_eq!(run.status, AcquisitionRunStatus::Running);
    assert_eq!(run.queued_work, 0);
    assert_eq!(run.completed_work, 0);
    assert_eq!(
        runs.persisted_requests.lock().unwrap().as_slice(),
        &[json!({
            "sources": { "mode": "explicit", "values": ["fixture-provider"] },
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

#[test]
fn invalid_acquisition_request_is_rejected_before_persistence() {
    let runs = RecordingRunRepository::new();

    let error = start_acquisition_run(
        &runs,
        AcquisitionRequestInput {
            sources: SourceSelection::Explicit(Vec::new()),
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
    .unwrap_err();

    assert_eq!(
        error,
        game_media_vault_application::ApplicationError::Validation(
            AcquisitionRequestValidationError::MissingSources
        )
    );
    assert!(runs.persisted_requests.lock().unwrap().is_empty());
}

struct RacingRunRepository {
    status: Mutex<AcquisitionRunStatus>,
    request: AcquisitionRequest,
}

impl RacingRunRepository {
    fn new(request: AcquisitionRequest) -> Self {
        Self {
            status: Mutex::new(AcquisitionRunStatus::Running),
            request,
        }
    }

    fn run(&self) -> AcquisitionRun {
        AcquisitionRun {
            id: 11,
            request: self.request.clone(),
            planned_sources: Vec::new(),
            status: *self.status.lock().unwrap(),
            queued_work: 0,
            awaiting_review_work: 0,
            completed_work: 0,
            below_quality_work: 0,
            outranked_work: 0,
            unavailable_work: 0,
        }
    }
}

impl RunRepositoryPort for RacingRunRepository {
    fn create_run(
        &self,
        _request: AcquisitionRequest,
        _planned_sources: Vec<String>,
    ) -> Result<AcquisitionRun, PortError> {
        Ok(self.run())
    }

    fn get_run(&self, run_id: i64) -> Result<Option<AcquisitionRun>, PortError> {
        Ok((run_id == 11).then(|| self.run()))
    }

    fn claim_execution(&self, _run_id: i64) -> Result<bool, PortError> {
        Ok(true)
    }

    fn release_execution(&self, _run_id: i64) -> Result<(), PortError> {
        Ok(())
    }

    fn run_status(&self, _run_id: i64) -> Result<Option<AcquisitionRunStatus>, PortError> {
        Ok(Some(*self.status.lock().unwrap()))
    }

    fn list_runs(&self) -> Result<Vec<AcquisitionRun>, PortError> {
        Ok(vec![self.run()])
    }

    fn has_discovered(&self, _run_id: i64, _source_id: &str) -> Result<bool, PortError> {
        Ok(false)
    }

    fn record_discovery(
        &self,
        _run_id: i64,
        _source_id: &str,
        _work: &[AcquisitionWorkItem],
    ) -> Result<bool, PortError> {
        Ok(true)
    }
    fn next_queued_work(
        &self,
        _run_id: i64,
        _skipped_sources: &[String],
    ) -> Result<Option<AcquisitionWorkItem>, PortError> {
        Ok(None)
    }

    fn complete_work(&self, _run_id: i64, _work_key: &str) -> Result<(), PortError> {
        Ok(())
    }

    fn complete_unavailable_work(
        &self,
        _run_id: i64,
        _work_key: &str,
        _reason: &str,
    ) -> Result<(), PortError> {
        Ok(())
    }

    fn queued_work(
        &self,
        _run_id: i64,
        _skipped_sources: &[String],
        _per_source: usize,
    ) -> Result<Vec<AcquisitionWorkItem>, PortError> {
        Ok(Vec::new())
    }

    fn record_source_failure(
        &self,
        _run_id: i64,
        _source_id: &str,
        _stage: SourceFailureStage,
        _message: &str,
    ) -> Result<(), PortError> {
        Ok(())
    }

    fn source_failures(&self) -> Result<Vec<SourceFailure>, PortError> {
        Ok(Vec::new())
    }

    fn compare_and_set_run_status(
        &self,
        _run_id: i64,
        expected: AcquisitionRunStatus,
        target: AcquisitionRunStatus,
    ) -> Result<bool, PortError> {
        // Simulate a concurrent cancellation after the application validated Running.
        *self.status.lock().unwrap() = AcquisitionRunStatus::Cancelled;
        if *self.status.lock().unwrap() != expected {
            return Ok(false);
        }
        *self.status.lock().unwrap() = target;
        Ok(true)
    }
}

#[test]
fn a_stale_pause_cannot_overwrite_a_concurrent_cancellation() {
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestInput {
        sources: SourceSelection::Auto,
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::All,
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();
    let runs = RacingRunRepository::new(request);

    let error = pause_acquisition_run(&runs, 11).unwrap_err();

    assert_eq!(
        error,
        ApplicationError::InvalidRunTransition {
            from: AcquisitionRunStatus::Cancelled,
            to: AcquisitionRunStatus::Paused,
        }
    );
    assert_eq!(
        *runs.status.lock().unwrap(),
        AcquisitionRunStatus::Cancelled
    );
}
