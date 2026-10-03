use game_media_vault_application::{
    CandidateAssetOutcome, ParkedReview, PortError, ReviewDecisionOutcome, ReviewRepositoryPort,
};
use game_media_vault_domain::{
    AssetType, LibraryAsset, MediaInfo, NewReviewItem, PersistAsset, QualityShortfall,
    RetentionPolicy, ReviewDecision, ReviewItem, ReviewMatchCandidate, ReviewStatus, StoredObject,
    outranked_by,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};

use super::{
    SqliteCatalog,
    assets::{asset_type_to_str, detach_candidate_links, persist_asset_in_transaction},
    sql_error,
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
            let settled: bool = transaction
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM acquisition_run_work
                         WHERE run_id = ?1 AND work_key = ?2 AND state = 'done'
                     )",
                    params![run_id, work_key],
                    |row| row.get(0),
                )
                .map_err(sql_error)?;
            // Returning without committing rolls back the item changes made above.
            if settled {
                return Ok(ParkedReview::Settled);
            }
            return Err(PortError::new(format!(
                "acquisition run #{run_id} has no queued work {work_key:?} to park"
            )));
        }
        // An uncertain match must not affect the library before a human confirms it, so an
        // earlier automatic link of the candidate goes until the decision.
        detach_candidate_links(&transaction, &item.candidate_identity, None)?;
        let review_item = select_review_item(&transaction, "id = ?1", params![review_item_id])?
            .ok_or_else(|| PortError::new(format!("review item #{review_item_id} disappeared")))?;
        transaction.commit().map_err(sql_error)?;
        Ok(ParkedReview::Parked(review_item))
    }

    fn supersede_candidate_review(
        &self,
        run_id: i64,
        candidate_identity: &str,
    ) -> Result<bool, PortError> {
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
            match item.status {
                ReviewStatus::Accepted | ReviewStatus::Rejected => return Ok(false),
                ReviewStatus::Pending | ReviewStatus::Deferred | ReviewStatus::AutoResolved => {
                    transaction
                        .execute(
                            "UPDATE review_items SET status = ?1, decision_json = NULL
                             WHERE id = ?2",
                            params![review_status_to_str(ReviewStatus::Superseded), item.id],
                        )
                        .map_err(sql_error)?;
                    complete_parked_work(&transaction, item.id)?;
                }
                ReviewStatus::Superseded => {}
            }
        }
        // The engine no longer believes in the candidate, so its automatic link goes too.
        detach_candidate_links(&transaction, candidate_identity, None)?;
        complete_run_work(
            &transaction,
            run_id,
            candidate_identity,
            WorkOutcome::Settled,
        )?;
        transaction.commit().map_err(sql_error)?;
        Ok(true)
    }

    fn decide_review_item(
        &self,
        review_item_id: i64,
        decision: ReviewDecision,
    ) -> Result<ReviewDecisionOutcome, PortError> {
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
        // The write lock is held from here on, so the checked state is the decided state.
        let Some(current) = select_review_item(&transaction, "id = ?1", params![review_item_id])?
        else {
            return Ok(ReviewDecisionOutcome::NotFound);
        };
        if !current.status.is_undecided() {
            return Ok(ReviewDecisionOutcome::NotUndecided(current.status));
        }
        if let ReviewDecision::Accept { release_edition_id } = decision
            && !current
                .competing_matches
                .iter()
                .any(|candidate| candidate.release_edition_id == release_edition_id)
        {
            return Ok(ReviewDecisionOutcome::NotCompeting);
        }
        set_status_if_undecided(&transaction, review_item_id, status, Some(&decision_json))?;
        let decided = select_review_item(&transaction, "id = ?1", params![review_item_id])?
            .ok_or_else(|| PortError::new(format!("review item #{review_item_id} disappeared")))?;
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
        transaction.commit().map_err(sql_error)?;
        Ok(ReviewDecisionOutcome::Recorded(Box::new(decided)))
    }

    fn persist_candidate_asset(
        &self,
        run_id: i64,
        candidate_identity: &str,
        record: PersistAsset,
        retention: RetentionPolicy,
    ) -> Result<CandidateAssetOutcome, PortError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        // The write lock is held from here on, so the compared Assets are the retained ones.
        if retention == RetentionPolicy::KeepBestPerType
            && let Some(release_edition_id) = record.existing_release_edition_id
        {
            let candidate = StoredObject {
                hash: record.object_hash.clone(),
                byte_len: record.byte_len,
                media: record.media.clone(),
            };
            let retained = retained_assets(&transaction, release_edition_id, record.asset_type)?;
            if let Some(outranked) = outranked_by(&candidate, &retained) {
                let outranked_json = to_json(&outranked, "outranked original")?;
                if !settle_unlinked(
                    &transaction,
                    run_id,
                    candidate_identity,
                    release_edition_id,
                    WorkOutcome::Outranked(&outranked_json),
                )? {
                    return Ok(CandidateAssetOutcome::HumanDecisionConflict);
                }
                transaction.commit().map_err(sql_error)?;
                return Ok(CandidateAssetOutcome::Outranked(outranked));
            }
        }
        if !auto_resolve_candidate_review(
            &transaction,
            candidate_identity,
            record.existing_release_edition_id,
            ParkedWork::Complete,
        )? {
            return Ok(CandidateAssetOutcome::HumanDecisionConflict);
        }
        let imported =
            persist_asset_in_transaction(&transaction, record, Some(candidate_identity))?;
        detach_candidate_links(
            &transaction,
            candidate_identity,
            Some(imported.release_edition_id),
        )?;
        complete_run_work(
            &transaction,
            run_id,
            candidate_identity,
            WorkOutcome::Settled,
        )?;
        transaction.commit().map_err(sql_error)?;
        Ok(CandidateAssetOutcome::Linked(imported))
    }

    fn complete_candidate_below_quality(
        &self,
        run_id: i64,
        candidate_identity: &str,
        release_edition_id: i64,
        shortfalls: &[QualityShortfall],
    ) -> Result<bool, PortError> {
        let shortfalls_json = to_json(&shortfalls, "quality shortfalls")?;
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        if !settle_unlinked(
            &transaction,
            run_id,
            candidate_identity,
            release_edition_id,
            WorkOutcome::BelowQuality(&shortfalls_json),
        )? {
            return Ok(false);
        }
        transaction.commit().map_err(sql_error)?;
        Ok(true)
    }
}

/// What becomes of the work parked on a Review Item closed by a high-confidence match.
enum ParkedWork {
    /// The candidate was linked, so the parked work has nothing left to do.
    Complete,
    /// Nothing was linked, so every run evaluates the candidate again.
    Requeue,
}

/// Closes the candidate's Review Item after a high-confidence match to `release_edition_id`:
/// an undecided or superseded item becomes `AutoResolved`. Returns `false`, changing nothing,
/// when a human rejected the candidate or accepted another Release Edition.
fn auto_resolve_candidate_review(
    transaction: &Transaction<'_>,
    candidate_identity: &str,
    release_edition_id: Option<i64>,
    parked_work: ParkedWork,
) -> Result<bool, PortError> {
    let Some(item) = select_review_item(
        transaction,
        "candidate_identity = ?1",
        params![candidate_identity],
    )?
    else {
        return Ok(true);
    };
    match (item.status, item.decision) {
        (ReviewStatus::Rejected, _) => return Ok(false),
        (
            ReviewStatus::Accepted,
            Some(ReviewDecision::Accept {
                release_edition_id: accepted,
            }),
        ) if release_edition_id != Some(accepted) => return Ok(false),
        (ReviewStatus::Pending | ReviewStatus::Deferred, _) => {
            set_status_if_undecided(transaction, item.id, ReviewStatus::AutoResolved, None)?;
            match parked_work {
                ParkedWork::Complete => complete_parked_work(transaction, item.id)?,
                ParkedWork::Requeue => requeue_parked_work(transaction, item.id)?,
            }
        }
        // The candidate became high confidence after a run had dismissed it.
        (ReviewStatus::Superseded, _) => {
            transaction
                .execute(
                    "UPDATE review_items SET status = ?1 WHERE id = ?2",
                    params![review_status_to_str(ReviewStatus::AutoResolved), item.id],
                )
                .map_err(sql_error)?;
        }
        _ => {}
    }
    Ok(true)
}

/// How completed work ended, recorded so its candidate stays explainable.
enum WorkOutcome<'a> {
    /// Linked, dismissed or settled by a decision: nothing more to record.
    Settled,
    /// The original fell short of the quality requirements, as these JSON shortfalls say.
    BelowQuality(&'a str),
    /// A retained Asset outranks the original under Keep Best Per Type, as this JSON says.
    Outranked(&'a str),
}

/// Settles a candidate matched to `release_edition_id` without linking its original: its
/// Review Item is closed automatically (requeueing the work parked on it so every run applies
/// its own requirements), its links to other editions are removed and its work in `run_id`
/// records `outcome`. Returns `false`, changing nothing, when a human decision conflicts.
fn settle_unlinked(
    transaction: &Transaction<'_>,
    run_id: i64,
    candidate_identity: &str,
    release_edition_id: i64,
    outcome: WorkOutcome<'_>,
) -> Result<bool, PortError> {
    if !auto_resolve_candidate_review(
        transaction,
        candidate_identity,
        Some(release_edition_id),
        ParkedWork::Requeue,
    )? {
        return Ok(false);
    }
    detach_candidate_links(transaction, candidate_identity, Some(release_edition_id))?;
    complete_run_work(transaction, run_id, candidate_identity, outcome)?;
    Ok(true)
}

/// Assets of the edition and type that some provenance still retains.
fn retained_assets(
    transaction: &Transaction<'_>,
    release_edition_id: i64,
    asset_type: AssetType,
) -> Result<Vec<LibraryAsset>, PortError> {
    let mut statement = transaction
        .prepare(
            "SELECT id, object_hash, byte_len, original_filename, media_type, width, height
             FROM assets
             WHERE release_edition_id = ?1 AND asset_type = ?2
               AND EXISTS (SELECT 1 FROM asset_provenance WHERE asset_id = assets.id)
             ORDER BY id",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(
            params![release_edition_id, asset_type_to_str(asset_type)],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    MediaInfo {
                        media_type: row.get(4)?,
                        width: row.get(5)?,
                        height: row.get(6)?,
                        // Ranking weighs no document metadata.
                        document: None,
                    },
                ))
            },
        )
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    rows.into_iter()
        .map(
            |(asset_id, object_hash, byte_len, original_filename, media)| {
                Ok(LibraryAsset {
                    asset_id,
                    asset_type,
                    object_hash,
                    byte_len: u64::try_from(byte_len).map_err(|_| {
                        PortError::new("catalog contains a negative byte length".into())
                    })?,
                    media,
                    original_filename,
                    provenance: Vec::new(),
                    derived: Vec::new(),
                })
            },
        )
        .collect()
}

/// Completes the candidate's work in `run_id`, whether queued or parked, so the outcome and the
/// work settle together.
fn complete_run_work(
    transaction: &Transaction<'_>,
    run_id: i64,
    candidate_identity: &str,
    outcome: WorkOutcome<'_>,
) -> Result<(), PortError> {
    let (quality_shortfalls_json, outranked_json) = match outcome {
        WorkOutcome::Settled => (None, None),
        WorkOutcome::BelowQuality(shortfalls) => (Some(shortfalls), None),
        WorkOutcome::Outranked(outranked) => (None, Some(outranked)),
    };
    transaction
        .execute(
            "UPDATE acquisition_run_work
             SET state = 'done', review_item_id = NULL,
                 quality_shortfalls_json = ?3, outranked_json = ?4
             WHERE run_id = ?1 AND work_key = ?2",
            params![
                run_id,
                candidate_identity,
                quality_shortfalls_json,
                outranked_json
            ],
        )
        .map_err(sql_error)?;
    Ok(())
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
        .map_err(|error| PortError::new(format!("failed to serialize {what}: {error}")))
}

fn from_json<T: serde::de::DeserializeOwned>(json: &str, what: &str) -> Result<T, PortError> {
    serde_json::from_str(json)
        .map_err(|error| PortError::new(format!("catalog contains an invalid {what}: {error}")))
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

pub(super) fn parse_review_status(value: &str) -> Result<ReviewStatus, PortError> {
    match value {
        "pending" => Ok(ReviewStatus::Pending),
        "deferred" => Ok(ReviewStatus::Deferred),
        "accepted" => Ok(ReviewStatus::Accepted),
        "rejected" => Ok(ReviewStatus::Rejected),
        "auto_resolved" => Ok(ReviewStatus::AutoResolved),
        "superseded" => Ok(ReviewStatus::Superseded),
        _ => Err(PortError::new(format!(
            "catalog contains invalid review status {value:?}"
        ))),
    }
}
