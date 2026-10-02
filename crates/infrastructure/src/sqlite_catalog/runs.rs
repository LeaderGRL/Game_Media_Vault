use game_media_vault_application::{PortError, RunRepositoryPort};
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRun, AcquisitionRunStatus,
    AcquisitionWorkItem, SourceFailure, SourceFailureStage,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{SqliteCatalog, sql_error};

const ACQUISITION_REQUEST_SCHEMA_VERSION: i64 = 1;

/// The oldest queued work of one Source of a run, at most `?3` items, oldest first. Its Source
/// expression matches `idx_run_work_source`, so the lookup reads those items alone.
const QUEUED_WORK_OF_SOURCE: &str = "
    SELECT id, work_key, candidate_json
    FROM acquisition_run_work
    WHERE run_id = ?1 AND json_extract(candidate_json, '$.source_id') = ?2 AND state = 'queued'
    ORDER BY id
    LIMIT ?3";

impl RunRepositoryPort for SqliteCatalog {
    fn create_run(
        &self,
        request: AcquisitionRequest,
        planned_sources: Vec<String>,
    ) -> Result<AcquisitionRun, PortError> {
        let connection = self.connect()?;
        let request_json = serde_json::to_string(&request).map_err(|error| {
            PortError::new(format!("failed to serialize acquisition request: {error}"))
        })?;
        let planned_sources_json = serde_json::to_string(&planned_sources).map_err(|error| {
            PortError::new(format!("failed to serialize planned sources: {error}"))
        })?;
        connection
            .execute(
                "INSERT INTO acquisition_runs
                     (request_json, request_schema_version, status, planned_sources_json)
                 VALUES (?1, ?2, 'running', ?3)",
                params![
                    request_json,
                    ACQUISITION_REQUEST_SCHEMA_VERSION,
                    planned_sources_json
                ],
            )
            .map_err(sql_error)?;

        Ok(AcquisitionRun {
            id: connection.last_insert_rowid(),
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

    fn get_run(&self, run_id: i64) -> Result<Option<AcquisitionRun>, PortError> {
        load_run(&self.connect()?, run_id)
    }

    fn run_status(&self, run_id: i64) -> Result<Option<AcquisitionRunStatus>, PortError> {
        self.connect()?
            .query_row(
                "SELECT status FROM acquisition_runs WHERE id = ?1",
                params![run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sql_error)?
            .map(|status| parse_run_status(&status))
            .transpose()
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
            .ok_or_else(|| PortError::new(format!("acquisition run #{run_id} does not exist")))?;
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
                PortError::new(format!(
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

    fn next_queued_work(
        &self,
        run_id: i64,
        skipped_sources: &[String],
    ) -> Result<Option<AcquisitionWorkItem>, PortError> {
        // Each Source's oldest item is looked up through the run work Source index, oldest
        // first, so the first is the oldest of the queue outside `skipped_sources`.
        Ok(self
            .queued_work(run_id, skipped_sources, 1)?
            .into_iter()
            .next())
    }

    fn queued_work(
        &self,
        run_id: i64,
        skipped_sources: &[String],
        per_source: usize,
    ) -> Result<Vec<AcquisitionWorkItem>, PortError> {
        let per_source = i64::try_from(per_source).unwrap_or(i64::MAX);
        let mut connection = self.connect()?;
        // One snapshot for the run's status and every Source's lookup.
        let transaction = connection.transaction().map_err(sql_error)?;
        let running = transaction
            .query_row(
                "SELECT status = 'running' FROM acquisition_runs WHERE id = ?1",
                params![run_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(sql_error)?
            .unwrap_or(false);
        if !running {
            return Ok(Vec::new());
        }
        let sources: Vec<String> = transaction
            .prepare("SELECT source_id FROM acquisition_run_discoveries WHERE run_id = ?1")
            .map_err(sql_error)?
            .query_map(params![run_id], |row| row.get(0))
            .map_err(sql_error)?
            .collect::<rusqlite::Result<_>>()
            .map_err(sql_error)?;
        let mut statement = transaction
            .prepare(QUEUED_WORK_OF_SOURCE)
            .map_err(sql_error)?;
        let mut found = Vec::new();
        for source_id in sources
            .iter()
            .filter(|source_id| !skipped_sources.contains(source_id))
        {
            let rows = statement
                .query_map(params![run_id, source_id, per_source], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(sql_error)?;
            for (position, row) in rows.enumerate() {
                let (id, key, candidate_json) = row.map_err(sql_error)?;
                found.push((position, id, key, candidate_json));
            }
        }
        // Every Source's oldest item first, in queue order, then every Source's second.
        found.sort_by_key(|(position, id, _, _)| (*position, *id));
        found
            .into_iter()
            .map(|(_, _, key, candidate_json)| {
                let candidate = serde_json::from_str(&candidate_json).map_err(|error| {
                    PortError::new(format!(
                        "catalog contains an invalid work candidate: {error}"
                    ))
                })?;
                Ok(AcquisitionWorkItem { key, candidate })
            })
            .collect()
    }

    fn complete_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError> {
        complete_queued_work(&self.connect()?, run_id, work_key, None)
    }

    fn complete_unavailable_work(
        &self,
        run_id: i64,
        work_key: &str,
        reason: &str,
    ) -> Result<(), PortError> {
        complete_queued_work(&self.connect()?, run_id, work_key, Some(reason))
    }

    fn record_source_failure(
        &self,
        run_id: i64,
        source_id: &str,
        stage: SourceFailureStage,
        message: &str,
    ) -> Result<(), PortError> {
        self.connect()?
            .execute(
                "INSERT INTO acquisition_source_failures (run_id, source_id, stage, message)
                 VALUES (?1, ?2, ?3, ?4)",
                params![run_id, source_id, failure_stage_to_str(stage), message],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    fn source_failures(&self) -> Result<Vec<SourceFailure>, PortError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT id, source_id, run_id, stage, message, recorded_at
                 FROM acquisition_source_failures
                 ORDER BY id",
            )
            .map_err(sql_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(sql_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        rows.into_iter()
            .map(
                |(sequence, source_id, run_id, stage, message, recorded_at)| {
                    Ok(SourceFailure {
                        sequence,
                        source_id,
                        run_id,
                        stage: parse_failure_stage(&stage)?,
                        message,
                        recorded_at,
                    })
                },
            )
            .collect()
    }
}

fn failure_stage_to_str(stage: SourceFailureStage) -> &'static str {
    match stage {
        SourceFailureStage::Discovery => "discovery",
        SourceFailureStage::Download => "download",
    }
}

fn parse_failure_stage(value: &str) -> Result<SourceFailureStage, PortError> {
    match value {
        "discovery" => Ok(SourceFailureStage::Discovery),
        "download" => Ok(SourceFailureStage::Download),
        other => Err(PortError::new(format!(
            "unknown Source failure stage in catalog: {other}"
        ))),
    }
}

/// Completes queued `work_key`, recording why it was unavailable if it was; work another
/// execution completed meanwhile stays as it is.
fn complete_queued_work(
    connection: &Connection,
    run_id: i64,
    work_key: &str,
    unavailable_reason: Option<&str>,
) -> Result<(), PortError> {
    connection
        .execute(
            "UPDATE acquisition_run_work SET state = 'done', unavailable_reason = ?3
                 WHERE run_id = ?1 AND work_key = ?2 AND state = 'queued'",
            params![run_id, work_key, unavailable_reason],
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
        Err(PortError::new(format!(
            "acquisition run #{run_id} has no work {work_key:?}"
        )))
    }
}

fn load_run(connection: &Connection, run_id: i64) -> Result<Option<AcquisitionRun>, PortError> {
    let row = connection
        .query_row(
            "SELECT run.request_json, run.request_schema_version, run.status, run.planned_sources_json,
                    COUNT(work.id) FILTER (WHERE work.state = 'queued'),
                    COUNT(work.id) FILTER (WHERE work.state = 'parked'),
                    COUNT(work.id) FILTER (WHERE work.state = 'done'),
                    COUNT(work.id) FILTER (WHERE work.quality_shortfalls_json IS NOT NULL),
                    COUNT(work.id) FILTER (WHERE work.outranked_json IS NOT NULL),
                    COUNT(work.id) FILTER (WHERE work.unavailable_reason IS NOT NULL)
             FROM acquisition_runs AS run
             LEFT JOIN acquisition_run_work AS work ON work.run_id = run.id
             WHERE run.id = ?1
             GROUP BY run.id",
            params![run_id],
            |row| {
                // Queued, parked, done, below-quality and outranked work.
                let mut counts = [0_i64; 6];
                for (index, count) in counts.iter_mut().enumerate() {
                    *count = row.get(4 + index)?;
                }
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    counts,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;
    let Some((
        request_json,
        request_schema_version,
        status,
        planned_sources_json,
        [queued, parked, done, below_quality, outranked, unavailable],
    )) = row
    else {
        return Ok(None);
    };
    if request_schema_version != ACQUISITION_REQUEST_SCHEMA_VERSION {
        return Err(PortError::new(format!(
            "unsupported acquisition request schema version: {request_schema_version}"
        )));
    }
    let draft: AcquisitionRequestDraft = serde_json::from_str(&request_json).map_err(|error| {
        PortError::new(format!("invalid persisted acquisition request: {error}"))
    })?;
    let request = AcquisitionRequest::try_from_draft(draft).map_err(|error| {
        PortError::new(format!("invalid persisted acquisition request: {error}"))
    })?;
    let planned_sources = serde_json::from_str(&planned_sources_json)
        .map_err(|error| PortError::new(format!("invalid persisted planned sources: {error}")))?;
    Ok(Some(AcquisitionRun {
        id: run_id,
        request,
        planned_sources,
        status: parse_run_status(&status)?,
        queued_work: count(queued),
        awaiting_review_work: count(parked),
        completed_work: count(done),
        below_quality_work: count(below_quality),
        outranked_work: count(outranked),
        unavailable_work: count(unavailable),
    }))
}

pub(super) fn parse_run_status(value: &str) -> Result<AcquisitionRunStatus, PortError> {
    match value {
        "running" => Ok(AcquisitionRunStatus::Running),
        "paused" => Ok(AcquisitionRunStatus::Paused),
        "cancelled" => Ok(AcquisitionRunStatus::Cancelled),
        "completed" => Ok(AcquisitionRunStatus::Completed),
        other => Err(PortError::new(format!(
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

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn the_queued_work_of_a_source_is_looked_up_through_its_index() {
        let temp = tempdir().unwrap();
        let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
        let connection = catalog.connect().unwrap();

        let plan: Vec<String> = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {QUEUED_WORK_OF_SOURCE}"))
            .unwrap()
            .query_map(params![1, "a-source", 2], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();

        assert!(
            plan.iter()
                .any(|step| step.contains("USING INDEX idx_run_work_source")),
            "{plan:?}"
        );
        assert!(
            !plan.iter().any(|step| step.contains("TEMP B-TREE")),
            "{plan:?}"
        );
    }
}
