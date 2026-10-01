use game_media_vault_application::{ParkedReview, PortError, ReviewRepositoryPort};
use game_media_vault_domain::{
    ImportedAsset, NewReviewItem, PersistAsset, ReviewDecision, ReviewItem, ReviewMatchCandidate,
    ReviewStatus,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};

use super::{
    SqliteCatalog, detach_candidate_links, persist_asset_in_transaction, sql_error,
    tag_candidate_provenance,
};

const REVIEW_ITEM_COLUMNS: &str =
    "id, candidate_identity, candidate_json, competing_matches_json, decision_json, status";

impl ReviewRepositoryPort for SqliteCatalog {
    fn list_review_items(&self) -> Result<Vec<ReviewItem>, PortError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {REVIEW_ITEM_COLUMNS} FROM review_items ORDER BY id"
            ))
            .map_err(sql_error)?;
        let rows = statement
            .query_map([], review_item_row)
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)?;
        rows.into_iter().map(decode_review_item).collect()
    }

    fn get_review_item(&self, review_item_id: i64) -> Result<Option<ReviewItem>, PortError> {
        select_review_item(&self.connect()?, "id = ?1", params![review_item_id])
    }

    fn find_review_item(&self, candidate_identity: &str) -> Result<Option<ReviewItem>, PortError> {
        select_review_item(
            &self.connect()?,
            "candidate_identity = ?1",
            params![candidate_identity],
        )
    }

    fn park_work_for_review(
        &self,
        run_id: i64,
        work_key: &str,
        item: NewReviewItem,
    ) -> Result<ParkedReview, PortError> {
        let candidate_json = to_json(&item.candidate, "review candidate")?;
        let competing_matches_json = to_json(&item.competing_matches, "review matches")?;
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let existing = select_review_item(
            &transaction,
            "candidate_identity = ?1",
            params![item.candidate_identity],
        )?;
        let review_item_id = match existing {
            Some(existing)
                if matches!(
                    existing.status,
                    ReviewStatus::Accepted | ReviewStatus::Rejected
                ) =>
            {
                return Ok(ParkedReview::AlreadyDecided(existing));
            }
            Some(existing) => {
                // Refresh the evidence; an item closed by re-evaluation becomes pending again.
                transaction
                    .execute(
                        "UPDATE review_items
                         SET candidate_json = ?1,
                             competing_matches_json = ?2,
                             status = CASE WHEN status = 'deferred' THEN 'deferred' ELSE 'pending' END,
                             decision_json = CASE WHEN status = 'deferred' THEN decision_json END
                         WHERE id = ?3",
                        params![candidate_json, competing_matches_json, existing.id],
                    )
                    .map_err(sql_error)?;
                existing.id
            }
            None => {
                transaction
                    .execute(
                        "INSERT INTO review_items (
                             candidate_identity, candidate_json, competing_matches_json, status
                         ) VALUES (?1, ?2, ?3, 'pending')",
                        params![
                            item.candidate_identity,
                            candidate_json,
                            competing_matches_json
                        ],
                    )
                    .map_err(sql_error)?;
                transaction.last_insert_rowid()
            }
        };
        transaction
            .execute(
                "UPDATE acquisition_run_work SET state = 'parked', review_item_id = ?1
                 WHERE run_id = ?2 AND work_key = ?3 AND state = 'queued'",
                params![review_item_id, run_id, work_key],
            )
            .map_err(sql_error)?;
        // A concurrent execution of the same run may already have parked this work here.
        let parked_here: bool = transaction
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM acquisition_run_work
                     WHERE run_id = ?1 AND work_key = ?2 AND review_item_id = ?3
                 )",
                params![run_id, work_key, review_item_id],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if !parked_here {
            return Err(PortError(format!(
                "acquisition run #{run_id} has no queued work {work_key:?} to park"
            )));
        }
        let review_item = select_review_item(&transaction, "id = ?1", params![review_item_id])?
            .ok_or_else(|| PortError(format!("review item #{review_item_id} disappeared")))?;
        transaction.commit().map_err(sql_error)?;
        Ok(ParkedReview::Parked(review_item))
    }

    fn close_review_item(
        &self,
        review_item_id: i64,
        status: ReviewStatus,
    ) -> Result<bool, PortError> {
        if !matches!(
            status,
            ReviewStatus::AutoResolved | ReviewStatus::Superseded
        ) {
            return Err(PortError(format!(
                "review items cannot be closed automatically as {status:?}"
            )));
        }
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        if !set_status_if_undecided(&transaction, review_item_id, status, None)? {
            return Ok(false);
        }
        complete_parked_work(&transaction, review_item_id)?;
        transaction.commit().map_err(sql_error)?;
        Ok(true)
    }

    fn decide_review_item(
        &self,
        review_item_id: i64,
        decision: ReviewDecision,
    ) -> Result<Option<ReviewItem>, PortError> {
        let decision_json = to_json(&decision, "review decision")?;
        let status = match decision {
            ReviewDecision::Accept { .. } => ReviewStatus::Accepted,
            ReviewDecision::Reject => ReviewStatus::Rejected,
            ReviewDecision::Defer => ReviewStatus::Deferred,
        };
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        if !set_status_if_undecided(&transaction, review_item_id, status, Some(&decision_json))? {
            return Ok(None);
        }
        let decided = select_review_item(&transaction, "id = ?1", params![review_item_id])?
            .ok_or_else(|| PortError(format!("review item #{review_item_id} disappeared")))?;
        match decision {
            ReviewDecision::Accept { release_edition_id } => {
                requeue_parked_work(&transaction, review_item_id)?;
                detach_candidate_links(
                    &transaction,
                    &decided.candidate_identity,
                    Some(release_edition_id),
                )?;
            }
            ReviewDecision::Reject => {
                complete_parked_work(&transaction, review_item_id)?;
                detach_candidate_links(&transaction, &decided.candidate_identity, None)?;
            }
            ReviewDecision::Defer => {}
        }
        let decided = Some(decided);
        transaction.commit().map_err(sql_error)?;
        Ok(decided)
    }

    fn persist_candidate_asset(
        &self,
        candidate_identity: &str,
        record: PersistAsset,
    ) -> Result<Option<ImportedAsset>, PortError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let review_item = select_review_item(
            &transaction,
            "candidate_identity = ?1",
            params![candidate_identity],
        )?;
        if let Some(item) = review_item {
            match (item.status, item.decision) {
                (ReviewStatus::Rejected, _) => return Ok(None),
                (ReviewStatus::Accepted, Some(ReviewDecision::Accept { release_edition_id }))
                    if record.existing_release_edition_id != Some(release_edition_id) =>
                {
                    return Ok(None);
                }
                (ReviewStatus::Pending | ReviewStatus::Deferred, _) => {
                    set_status_if_undecided(
                        &transaction,
                        item.id,
                        ReviewStatus::AutoResolved,
                        None,
                    )?;
                    complete_parked_work(&transaction, item.id)?;
                }
                _ => {}
            }
        }
        let source_id = record.source_id.clone();
        let source_location = record.source_location.clone();
        let imported = persist_asset_in_transaction(&transaction, record)?;
        tag_candidate_provenance(
            &transaction,
            &imported,
            source_id.as_str(),
            &source_location,
            candidate_identity,
        )?;
        detach_candidate_links(
            &transaction,
            candidate_identity,
            Some(imported.release_edition_id),
        )?;
        transaction.commit().map_err(sql_error)?;
        Ok(Some(imported))
    }
}

fn set_status_if_undecided(
    transaction: &Transaction<'_>,
    review_item_id: i64,
    status: ReviewStatus,
    decision_json: Option<&str>,
) -> Result<bool, PortError> {
    let updated = transaction
        .execute(
            "UPDATE review_items SET status = ?1, decision_json = ?2
             WHERE id = ?3 AND status IN ('pending', 'deferred')",
            params![review_status_to_str(status), decision_json, review_item_id],
        )
        .map_err(sql_error)?;
    Ok(updated == 1)
}

fn complete_parked_work(
    transaction: &Transaction<'_>,
    review_item_id: i64,
) -> Result<(), PortError> {
    transaction
        .execute(
            "UPDATE acquisition_run_work SET state = 'done', review_item_id = NULL
             WHERE review_item_id = ?1",
            params![review_item_id],
        )
        .map_err(sql_error)?;
    Ok(())
}

/// Requeues parked work in runs that can still execute it, reopening completed runs.
fn requeue_parked_work(
    transaction: &Transaction<'_>,
    review_item_id: i64,
) -> Result<(), PortError> {
    transaction
        .execute(
            "UPDATE acquisition_runs SET status = 'running'
             WHERE status = 'completed' AND id IN (
                 SELECT run_id FROM acquisition_run_work WHERE review_item_id = ?1
             )",
            params![review_item_id],
        )
        .map_err(sql_error)?;
    transaction
        .execute(
            "UPDATE acquisition_run_work SET state = 'queued', review_item_id = NULL
             WHERE review_item_id = ?1 AND run_id IN (
                 SELECT id FROM acquisition_runs WHERE status != 'cancelled'
             )",
            params![review_item_id],
        )
        .map_err(sql_error)?;
    Ok(())
}

fn select_review_item(
    connection: &Connection,
    predicate: &str,
    parameters: impl rusqlite::Params,
) -> Result<Option<ReviewItem>, PortError> {
    connection
        .query_row(
            &format!("SELECT {REVIEW_ITEM_COLUMNS} FROM review_items WHERE {predicate}"),
            parameters,
            review_item_row,
        )
        .optional()
        .map_err(sql_error)?
        .map(decode_review_item)
        .transpose()
}

type ReviewItemRow = (i64, String, String, String, Option<String>, String);

fn review_item_row(row: &Row<'_>) -> rusqlite::Result<ReviewItemRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn decode_review_item(
    (id, candidate_identity, candidate_json, competing_matches_json, decision_json, status): ReviewItemRow,
) -> Result<ReviewItem, PortError> {
    let candidate = from_json(&candidate_json, "review candidate")?;
    let competing_matches: Vec<ReviewMatchCandidate> =
        from_json(&competing_matches_json, "review matches")?;
    let decision = decision_json
        .map(|json| from_json(&json, "review decision"))
        .transpose()?;
    Ok(ReviewItem {
        id,
        candidate_identity,
        candidate,
        competing_matches,
        decision,
        status: parse_review_status(&status)?,
    })
}

fn to_json(value: &impl serde::Serialize, what: &str) -> Result<String, PortError> {
    serde_json::to_string(value)
        .map_err(|error| PortError(format!("failed to serialize {what}: {error}")))
}

fn from_json<T: serde::de::DeserializeOwned>(json: &str, what: &str) -> Result<T, PortError> {
    serde_json::from_str(json)
        .map_err(|error| PortError(format!("catalog contains an invalid {what}: {error}")))
}

fn review_status_to_str(status: ReviewStatus) -> &'static str {
    match status {
        ReviewStatus::Pending => "pending",
        ReviewStatus::Deferred => "deferred",
        ReviewStatus::Accepted => "accepted",
        ReviewStatus::Rejected => "rejected",
        ReviewStatus::AutoResolved => "auto_resolved",
        ReviewStatus::Superseded => "superseded",
    }
}

fn parse_review_status(value: &str) -> Result<ReviewStatus, PortError> {
    match value {
        "pending" => Ok(ReviewStatus::Pending),
        "deferred" => Ok(ReviewStatus::Deferred),
        "accepted" => Ok(ReviewStatus::Accepted),
        "rejected" => Ok(ReviewStatus::Rejected),
        "auto_resolved" => Ok(ReviewStatus::AutoResolved),
        "superseded" => Ok(ReviewStatus::Superseded),
        _ => Err(PortError(format!(
            "catalog contains invalid review status {value:?}"
        ))),
    }
}
