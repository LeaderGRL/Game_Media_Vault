use game_media_vault_application::{PortError, RunRepositoryPort};
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRun, AcquisitionRunStatus,
    AcquisitionWorkItem,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{SqliteCatalog, sql_error};

const ACQUISITION_REQUEST_SCHEMA_VERSION: i64 = 1;

impl RunRepositoryPort for SqliteCatalog {
    fn create_run(&self, request: AcquisitionRequest) -> Result<AcquisitionRun, PortError> {
        let connection = self.connect()?;
        let request_json = serde_json::to_string(&request).map_err(|error| {
            PortError(format!("failed to serialize acquisition request: {error}"))
        })?;
        connection
            .execute(
                "INSERT INTO acquisition_runs (request_json, request_schema_version, status)
                 VALUES (?1, ?2, 'running')",
                params![request_json, ACQUISITION_REQUEST_SCHEMA_VERSION],
            )
            .map_err(sql_error)?;

        Ok(AcquisitionRun {
            id: connection.last_insert_rowid(),
            request,
            status: AcquisitionRunStatus::Running,
            queued_work: 0,
            awaiting_review_work: 0,
            completed_work: 0,
            below_quality_work: 0,
        })
    }

    fn get_run(&self, run_id: i64) -> Result<Option<AcquisitionRun>, PortError> {
        load_run(&self.connect()?, run_id)
    }

    fn list_runs(&self) -> Result<Vec<AcquisitionRun>, PortError> {
        let connection = self.connect()?;
        let run_ids = connection
            .prepare("SELECT id FROM acquisition_runs ORDER BY id")
            .map_err(sql_error)?
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)?;
        run_ids
            .into_iter()
            .filter_map(|run_id| load_run(&connection, run_id).transpose())
            .collect()
    }

    fn compare_and_set_run_status(
        &self,
        run_id: i64,
        expected: AcquisitionRunStatus,
        target: AcquisitionRunStatus,
    ) -> Result<bool, PortError> {
        let updated = self
            .connect()?
            .execute(
                "UPDATE acquisition_runs
                 SET status = ?1
                 WHERE id = ?2
                   AND status = ?3
                   AND (?1 != 'completed' OR NOT EXISTS (
                       SELECT 1 FROM acquisition_run_work
                       WHERE run_id = ?2 AND state = 'queued'
                   ))",
                params![
                    run_status_to_str(target),
                    run_id,
                    run_status_to_str(expected)
                ],
            )
            .map_err(sql_error)?;
        Ok(updated == 1)
    }

    fn has_discovered(&self, run_id: i64, source_id: &str) -> Result<bool, PortError> {
        self.connect()?
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM acquisition_run_discoveries
                     WHERE run_id = ?1 AND source_id = ?2
                 )",
                params![run_id, source_id],
                |row| row.get(0),
            )
            .map_err(sql_error)
    }

    fn record_discovery(
        &self,
        run_id: i64,
        source_id: &str,
        work: &[AcquisitionWorkItem],
    ) -> Result<bool, PortError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let status = transaction
            .query_row(
                "SELECT status FROM acquisition_runs WHERE id = ?1",
                params![run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sql_error)?
            .ok_or_else(|| PortError(format!("acquisition run #{run_id} does not exist")))?;
        if !matches!(status.as_str(), "running" | "paused") {
            return Ok(false);
        }
        // Claiming the marker first under the write lock keeps exactly one discovery snapshot
        // per source when executions of the same run race.
        let claimed = transaction
            .execute(
                "INSERT OR IGNORE INTO acquisition_run_discoveries (run_id, source_id)
                 VALUES (?1, ?2)",
                params![run_id, source_id],
            )
            .map_err(sql_error)?;
        if claimed == 0 {
            return Ok(true);
        }
        for item in work {
            let candidate_json = serde_json::to_string(&item.candidate).map_err(|error| {
                PortError(format!(
                    "failed to serialize acquisition candidate: {error}"
                ))
            })?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO acquisition_run_work (
                         run_id, work_key, candidate_json, state
                     ) VALUES (?1, ?2, ?3, 'queued')",
                    params![run_id, item.key, candidate_json],
                )
                .map_err(sql_error)?;
        }
        transaction.commit().map_err(sql_error)?;
        Ok(true)
    }

    fn next_queued_work(&self, run_id: i64) -> Result<Option<AcquisitionWorkItem>, PortError> {
        let row = self
            .connect()?
            .query_row(
                "SELECT work.work_key, work.candidate_json
                 FROM acquisition_run_work AS work
                 INNER JOIN acquisition_runs AS run ON run.id = work.run_id
                 WHERE work.run_id = ?1 AND work.state = 'queued' AND run.status = 'running'
                 ORDER BY work.id
                 LIMIT 1",
                params![run_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(sql_error)?;
        row.map(|(key, candidate_json)| {
            let candidate = serde_json::from_str(&candidate_json).map_err(|error| {
                PortError(format!(
                    "catalog contains an invalid work candidate: {error}"
                ))
            })?;
            Ok(AcquisitionWorkItem { key, candidate })
        })
        .transpose()
    }

    fn complete_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError> {
        let connection = self.connect()?;
        connection
            .execute(
                "UPDATE acquisition_run_work SET state = 'done'
                 WHERE run_id = ?1 AND work_key = ?2 AND state = 'queued'",
                params![run_id, work_key],
            )
            .map_err(sql_error)?;
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM acquisition_run_work WHERE run_id = ?1 AND work_key = ?2
                 )",
                params![run_id, work_key],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if exists {
            Ok(())
        } else {
            Err(PortError(format!(
                "acquisition run #{run_id} has no work {work_key:?}"
            )))
        }
    }
}

fn load_run(connection: &Connection, run_id: i64) -> Result<Option<AcquisitionRun>, PortError> {
    let row = connection
        .query_row(
            "SELECT run.request_json, run.request_schema_version, run.status,
                    COUNT(work.id) FILTER (WHERE work.state = 'queued'),
                    COUNT(work.id) FILTER (WHERE work.state = 'parked'),
                    COUNT(work.id) FILTER (WHERE work.state = 'done'),
                    COUNT(work.id) FILTER (WHERE work.quality_shortfalls_json IS NOT NULL)
             FROM acquisition_runs AS run
             LEFT JOIN acquisition_run_work AS work ON work.run_id = run.id
             WHERE run.id = ?1
             GROUP BY run.id",
            params![run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;
    let Some((request_json, request_schema_version, status, queued, parked, done, below_quality)) =
        row
    else {
        return Ok(None);
    };
    if request_schema_version != ACQUISITION_REQUEST_SCHEMA_VERSION {
        return Err(PortError(format!(
            "unsupported acquisition request schema version: {request_schema_version}"
        )));
    }
    let draft: AcquisitionRequestDraft = serde_json::from_str(&request_json)
        .map_err(|error| PortError(format!("invalid persisted acquisition request: {error}")))?;
    let request = AcquisitionRequest::try_from_draft(draft)
        .map_err(|error| PortError(format!("invalid persisted acquisition request: {error}")))?;
    Ok(Some(AcquisitionRun {
        id: run_id,
        request,
        status: parse_run_status(&status)?,
        queued_work: count(queued),
        awaiting_review_work: count(parked),
        completed_work: count(done),
        below_quality_work: count(below_quality),
    }))
}

fn parse_run_status(value: &str) -> Result<AcquisitionRunStatus, PortError> {
    match value {
        "running" => Ok(AcquisitionRunStatus::Running),
        "paused" => Ok(AcquisitionRunStatus::Paused),
        "cancelled" => Ok(AcquisitionRunStatus::Cancelled),
        "completed" => Ok(AcquisitionRunStatus::Completed),
        other => Err(PortError(format!(
            "unknown acquisition run status: {other}"
        ))),
    }
}

fn run_status_to_str(status: AcquisitionRunStatus) -> &'static str {
    match status {
        AcquisitionRunStatus::Running => "running",
        AcquisitionRunStatus::Paused => "paused",
        AcquisitionRunStatus::Cancelled => "cancelled",
        AcquisitionRunStatus::Completed => "completed",
    }
}

/// Converts a SQL `COUNT`, which is never negative.
fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}
