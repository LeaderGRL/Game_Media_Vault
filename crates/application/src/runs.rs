use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRequestValidationError, AcquisitionRun,
    AcquisitionRunStatus,
};

use crate::{ApplicationError, RunRepositoryPort};

pub fn build_acquisition_request(
    input: AcquisitionRequestDraft,
) -> Result<AcquisitionRequest, AcquisitionRequestValidationError> {
    AcquisitionRequest::try_from_draft(input)
}

pub fn start_acquisition_run(
    runs: &dyn RunRepositoryPort,
    input: AcquisitionRequestDraft,
) -> Result<AcquisitionRun, ApplicationError> {
    let request = build_acquisition_request(input)?;
    Ok(runs.create_run(request)?)
}

pub fn load_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    runs.get_run(run_id)?
        .ok_or(ApplicationError::RunNotFound(run_id))
}

pub fn list_acquisition_runs(
    runs: &dyn RunRepositoryPort,
) -> Result<Vec<AcquisitionRun>, ApplicationError> {
    Ok(runs.list_runs()?)
}

pub fn pause_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    transition_acquisition_run(runs, run_id, AcquisitionRunStatus::Paused)
}

pub fn resume_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    transition_acquisition_run(runs, run_id, AcquisitionRunStatus::Running)
}

pub fn cancel_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    transition_acquisition_run(runs, run_id, AcquisitionRunStatus::Cancelled)
}

pub fn complete_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
) -> Result<AcquisitionRun, ApplicationError> {
    transition_acquisition_run(runs, run_id, AcquisitionRunStatus::Completed)
}

fn transition_acquisition_run(
    runs: &dyn RunRepositoryPort,
    run_id: i64,
    target: AcquisitionRunStatus,
) -> Result<AcquisitionRun, ApplicationError> {
    let mut current = load_acquisition_run(runs, run_id)?;
    loop {
        if current.status == target {
            return Ok(current);
        }
        if target == AcquisitionRunStatus::Completed && current.queued_work != 0 {
            return Err(ApplicationError::RunHasQueuedWork {
                queued_work: current.queued_work,
            });
        }

        let allowed = matches!(
            (current.status, target),
            (AcquisitionRunStatus::Running, AcquisitionRunStatus::Paused)
                | (AcquisitionRunStatus::Paused, AcquisitionRunStatus::Running)
                | (
                    AcquisitionRunStatus::Running,
                    AcquisitionRunStatus::Cancelled
                )
                | (
                    AcquisitionRunStatus::Paused,
                    AcquisitionRunStatus::Cancelled
                )
                | (
                    AcquisitionRunStatus::Running,
                    AcquisitionRunStatus::Completed
                )
        );
        if !allowed {
            return Err(ApplicationError::InvalidRunTransition {
                from: current.status,
                to: target,
            });
        }

        if runs.compare_and_set_run_status(run_id, current.status, target)? {
            return load_acquisition_run(runs, run_id);
        }
        current = load_acquisition_run(runs, run_id)?;
    }
}
