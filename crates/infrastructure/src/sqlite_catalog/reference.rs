use game_media_vault_application::{
    PortError, ReferenceCatalogRepositoryPort, ReferenceReviewOutcome,
    ReferenceReviewRepositoryPort,
};
use game_media_vault_domain::{
    ImportedReleaseEdition, ReferenceRecordSummary, ReferenceReleaseRecord, ReferenceReviewEdition,
    ReferenceReviewItem, ReleaseAssertion, ReleaseAssertionField, SourceId,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::{SqliteCatalog, normalize, sql_error};

impl ReferenceCatalogRepositoryPort for SqliteCatalog {
    fn persist_reference_release(
        &self,
        record: ReferenceReleaseRecord,
    ) -> Result<ImportedReleaseEdition, PortError> {
        let mut imported = self.persist_reference_releases(vec![record])?;
        imported
            .pop()
            .ok_or_else(|| PortError::new("reference persistence returned no release".into()))
    }

    fn persist_reference_releases(
        &self,
        records: Vec<ReferenceReleaseRecord>,
    ) -> Result<Vec<ImportedReleaseEdition>, PortError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let mut imported = Vec::with_capacity(records.len());
        for record in records {
            imported.push(persist_reference_release_in_transaction(
                &transaction,
                record,
            )?);
        }
        transaction.commit().map_err(sql_error)?;
        Ok(imported)
    }
}

impl ReferenceReviewRepositoryPort for SqliteCatalog {
    fn list_reference_review_items(&self) -> Result<Vec<ReferenceReviewItem>, PortError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT id, source_id, source_record, release_edition_id, evidence, candidates_json
                 FROM reference_review_items
                 WHERE status = 'pending'
                 ORDER BY id",
            )
            .map_err(sql_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(sql_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        rows.into_iter()
            .map(
                |(id, source_id, source_record, release_edition_id, evidence, candidates_json)| {
                    let candidates = serde_json::from_str(&candidates_json).map_err(|error| {
                        PortError::new(format!(
                            "catalog contains invalid review candidates: {error}"
                        ))
                    })?;
                    Ok(ReferenceReviewItem {
                        id,
                        source_id: SourceId::from(source_id),
                        source_record,
                        release_edition_id,
                        evidence,
                        candidates,
                    })
                },
            )
            .collect()
    }

    fn describe_reference_review_editions(
        &self,
        release_edition_ids: &[i64],
    ) -> Result<Vec<ReferenceReviewEdition>, PortError> {
        let connection = self.connect()?;
        let mut editions = Vec::with_capacity(release_edition_ids.len());
        for &release_edition_id in release_edition_ids {
            let Some((game_title, platform, region, edition_name)) = connection
                .query_row(
                    "SELECT g.title, r.platform, r.region, r.edition_name
                     FROM release_editions r
                     JOIN games g ON g.id = r.game_id
                     WHERE r.id = ?1",
                    params![release_edition_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()
                .map_err(sql_error)?
            else {
                continue;
            };
            let records = connection
                .prepare(
                    "SELECT s.source_id, s.value, COALESCE((
                         SELECT t.value FROM release_assertions t
                         WHERE t.release_edition_id = s.release_edition_id
                           AND t.source_id = s.source_id
                           AND t.field = 'title' AND t.qualifier = ''
                         ORDER BY t.id DESC LIMIT 1
                     ), '')
                     FROM release_assertions s
                     WHERE s.release_edition_id = ?1
                       AND s.field = 'identifier' AND s.qualifier = 'source_record'
                     ORDER BY s.id",
                )
                .map_err(sql_error)?
                .query_map(params![release_edition_id], |row| {
                    Ok(ReferenceRecordSummary {
                        source_id: SourceId::from(row.get::<_, String>(0)?),
                        source_record: row.get(1)?,
                        title: row.get(2)?,
                    })
                })
                .map_err(sql_error)?
                .collect::<rusqlite::Result<_>>()
                .map_err(sql_error)?;
            editions.push(ReferenceReviewEdition {
                release_edition_id,
                game_title,
                platform,
                region,
                edition_name,
                records,
            });
        }
        Ok(editions)
    }

    fn link_reference_review_item(
        &self,
        item_id: i64,
        release_edition_id: i64,
    ) -> Result<ReferenceReviewOutcome, PortError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let Some(item) = pending_review_item(&transaction, item_id)? else {
            return Ok(ReferenceReviewOutcome::ItemNotPending);
        };
        let source_id = item.source_id.as_str();
        let merged = item.release_edition_id;
        if !item.candidates.contains(&release_edition_id) {
            return Ok(ReferenceReviewOutcome::NotACandidate);
        }
        // A record its source linked into another source's edition leaves it alone.
        if holds_link(&transaction, merged, source_id)? {
            let outcome = move_linked_record(&transaction, &item, release_edition_id)?;
            if outcome == ReferenceReviewOutcome::Decided {
                transaction.commit().map_err(sql_error)?;
            }
            return Ok(outcome);
        }
        // Two editions holding records of one source, the item's or any linked since, are two
        // releases of it.
        if share_a_source_of_record(&transaction, merged, release_edition_id)? {
            return Ok(ReferenceReviewOutcome::NotACandidate);
        }
        let holds_assets: bool = transaction
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM assets WHERE release_edition_id = ?1)",
                params![merged],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if holds_assets {
            return Ok(ReferenceReviewOutcome::EditionHoldsAssets);
        }
        let identity = transaction
            .query_row(
                "SELECT source_location FROM release_assertions
                 WHERE release_edition_id = ?1 AND source_id = ?2
                   AND field = 'identifier' AND qualifier = 'source_record' AND value = ?3",
                params![merged, source_id, item.source_record],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sql_error)?;
        // The claims made of the merged edition, the record's and any linked since, join the
        // chosen one, with the evidence of the decision.
        transaction
            .execute(
                "UPDATE OR IGNORE release_assertions SET release_edition_id = ?2
                 WHERE release_edition_id = ?1",
                params![merged, release_edition_id],
            )
            .map_err(sql_error)?;
        // What stays behind repeats a claim the chosen edition already carries.
        transaction
            .execute(
                "DELETE FROM release_assertions WHERE release_edition_id = ?1",
                params![merged],
            )
            .map_err(sql_error)?;
        transaction
            .execute(
                "UPDATE reference_dump_sets SET release_edition_id = ?2
                 WHERE release_edition_id = ?1",
                params![merged, release_edition_id],
            )
            .map_err(sql_error)?;
        if let Some(source_location) = identity {
            let link = ReleaseAssertion {
                source_id: item.source_id.clone(),
                source_location,
                field: ReleaseAssertionField::Identifier,
                qualifier: Some("linked_by".to_owned()),
                value: "review".to_owned(),
            };
            persist_release_assertions(&transaction, release_edition_id, &[link])?;
        }
        // Other records that may describe the merged edition may describe the chosen one now.
        let others: Vec<(i64, String)> = transaction
            .prepare(
                "SELECT id, candidates_json FROM reference_review_items
                 WHERE status = 'pending' AND id != ?1",
            )
            .map_err(sql_error)?
            .query_map(params![item_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(sql_error)?
            .collect::<rusqlite::Result<_>>()
            .map_err(sql_error)?;
        for (other_id, candidates_json) in others {
            let mut candidates: Vec<i64> =
                serde_json::from_str(&candidates_json).map_err(|error| {
                    PortError::new(format!(
                        "catalog contains invalid review candidates: {error}"
                    ))
                })?;
            if !candidates.contains(&merged) {
                continue;
            }
            candidates.retain(|candidate| *candidate != merged);
            if !candidates.contains(&release_edition_id) {
                candidates.push(release_edition_id);
            }
            let candidates_json = serde_json::to_string(&candidates).map_err(|error| {
                PortError::new(format!("failed to serialize review candidates: {error}"))
            })?;
            transaction
                .execute(
                    "UPDATE reference_review_items SET candidates_json = ?2 WHERE id = ?1",
                    params![other_id, candidates_json],
                )
                .map_err(sql_error)?;
        }
        transaction
            .execute(
                "UPDATE reference_review_items SET status = 'linked' WHERE id = ?1",
                params![item_id],
            )
            .map_err(sql_error)?;
        // The emptied edition goes, and its Game when it has no other edition.
        let game_id: i64 = transaction
            .query_row(
                "SELECT game_id FROM release_editions WHERE id = ?1",
                params![merged],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        transaction
            .execute(
                "UPDATE reference_review_items SET release_edition_id = ?2
                 WHERE release_edition_id = ?1",
                params![merged, release_edition_id],
            )
            .map_err(sql_error)?;
        transaction
            .execute(
                "DELETE FROM release_editions WHERE id = ?1",
                params![merged],
            )
            .map_err(sql_error)?;
        transaction
            .execute(
                "DELETE FROM games WHERE id = ?1
                 AND NOT EXISTS (SELECT 1 FROM release_editions WHERE game_id = ?1)",
                params![game_id],
            )
            .map_err(sql_error)?;
        transaction.commit().map_err(sql_error)?;
        Ok(ReferenceReviewOutcome::Decided)
    }

    fn keep_reference_review_item_apart(
        &self,
        item_id: i64,
    ) -> Result<ReferenceReviewOutcome, PortError> {
        let decided = self
            .connect()?
            .execute(
                "UPDATE reference_review_items SET status = 'kept_apart'
                 WHERE id = ?1 AND status = 'pending'",
                params![item_id],
            )
            .map_err(sql_error)?;
        Ok(if decided == 0 {
            ReferenceReviewOutcome::ItemNotPending
        } else {
            ReferenceReviewOutcome::Decided
        })
    }
}

fn persist_reference_release_in_transaction(
    transaction: &Transaction<'_>,
    record: ReferenceReleaseRecord,
) -> Result<ImportedReleaseEdition, PortError> {
    let identity = record
        .assertions
        .iter()
        .find(|assertion| {
            assertion.field == ReleaseAssertionField::Identifier
                && assertion.qualifier.as_deref() == Some("source_record")
        })
        .ok_or_else(|| {
            PortError::new(
                "reference release is missing its source_record identifier assertion".into(),
            )
        })?;
    let source_id = identity.source_id.as_str().to_owned();
    let source_record = identity.value.clone();
    let normalized_title = normalize(&record.game_title);
    let normalized_platform = normalize(&record.platform);
    let normalized_region = normalize(&record.region);
    let normalized_edition = normalize(&record.edition_name);
    let dumps = dump_set(&record.assertions);
    let edition = NormalizedEdition {
        title: &normalized_title,
        platform: &normalized_platform,
        region: &normalized_region,
        edition: &normalized_edition,
    };

    if let Some((game_id, release_edition_id)) = transaction
        .query_row(
            "SELECT r.game_id, r.id
             FROM release_assertions a
             JOIN release_editions r ON r.id = a.release_edition_id
             WHERE a.source_id = ?1
               AND a.field = 'identifier'
               AND a.qualifier = 'source_record'
               AND a.value = ?2
             LIMIT 1",
            params![source_id, source_record],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(sql_error)?
    {
        persist_release_assertions(transaction, release_edition_id, &record.assertions)?;
        // The evidence of a link moves with the catalog that made it, as its other claims do.
        let links: Vec<String> = {
            let mut statement = transaction
                .prepare(
                    "SELECT value FROM release_assertions
                     WHERE release_edition_id = ?1 AND source_id = ?2
                       AND field = 'identifier' AND qualifier = 'linked_by'",
                )
                .map_err(sql_error)?;
            statement
                .query_map(params![release_edition_id, source_id], |row| row.get(0))
                .map_err(sql_error)?
                .collect::<rusqlite::Result<_>>()
                .map_err(sql_error)?
        };
        let links: Vec<ReleaseAssertion> = links
            .into_iter()
            .map(|evidence| link_assertion(identity, evidence))
            .collect();
        persist_release_assertions(transaction, release_edition_id, &links)?;
        record_dump_set(
            transaction,
            &source_id,
            &source_record,
            release_edition_id,
            dumps.as_deref(),
        )?;
        revalidate_review_item(
            transaction,
            &source_id,
            &source_record,
            release_edition_id,
            &edition,
            dumps.as_deref(),
        )?;
        return Ok(ImportedReleaseEdition {
            game_id,
            release_edition_id,
        });
    }

    // Another source may already describe this release: its assertions then join that edition,
    // with the evidence of the link.
    let evidence = linked_release_edition(transaction, &source_id, &edition, dumps.as_deref())?;
    if let Evidence::Links(game_id, release_edition_id, evidence) = evidence {
        let link = link_assertion(identity, evidence.to_owned());
        persist_release_assertions(transaction, release_edition_id, &record.assertions)?;
        persist_release_assertions(transaction, release_edition_id, &[link])?;
        record_dump_set(
            transaction,
            &source_id,
            &source_record,
            release_edition_id,
            dumps.as_deref(),
        )?;
        return Ok(ImportedReleaseEdition {
            game_id,
            release_edition_id,
        });
    }

    // The Game a source titled so on this platform, this source's own first. Never one holding
    // this edition with another source's claims: the record was not linked to that edition, so
    // joining it would merge them without evidence.
    let game_id = match transaction
        .query_row(
            "SELECT r.game_id
             FROM release_assertions t
             JOIN release_editions r ON r.id = t.release_edition_id
             WHERE t.field = 'title'
               AND t.qualifier = ''
               AND t.normalized_value = ?1
               AND r.normalized_platform = ?2
               AND NOT EXISTS (
                   SELECT 1
                   FROM release_editions e
                   JOIN release_assertions x ON x.release_edition_id = e.id
                   WHERE e.game_id = r.game_id
                     AND e.normalized_platform = ?2
                     AND e.normalized_region = ?3
                     AND e.normalized_edition_name = ?4
                     AND x.source_id != ?5
               )
             ORDER BY t.source_id = ?5 DESC, r.id
             LIMIT 1",
            params![
                normalized_title,
                normalized_platform,
                normalized_region,
                normalized_edition,
                source_id
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?
    {
        Some(game_id) => game_id,
        None => {
            transaction
                .execute(
                    "INSERT INTO games (title, normalized_title) VALUES (?1, ?2)",
                    params![record.game_title, normalized_title],
                )
                .map_err(sql_error)?;
            transaction.last_insert_rowid()
        }
    };

    transaction
        .execute(
            "INSERT INTO release_editions (
                game_id, platform, normalized_platform, region, normalized_region,
                edition_name, normalized_edition_name
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(game_id, normalized_platform, normalized_region, normalized_edition_name)
             DO NOTHING",
            params![
                game_id,
                record.platform,
                normalized_platform,
                record.region,
                normalized_region,
                record.edition_name,
                normalized_edition,
            ],
        )
        .map_err(sql_error)?;
    let release_edition_id = transaction
        .query_row(
            "SELECT id FROM release_editions
             WHERE game_id = ?1
               AND normalized_platform = ?2
               AND normalized_region = ?3
               AND normalized_edition_name = ?4",
            params![
                game_id,
                normalized_platform,
                normalized_region,
                normalized_edition
            ],
            |row| row.get(0),
        )
        .map_err(sql_error)?;

    persist_release_assertions(transaction, release_edition_id, &record.assertions)?;
    record_dump_set(
        transaction,
        &source_id,
        &source_record,
        release_edition_id,
        dumps.as_deref(),
    )?;
    // The record keeps its own edition until a human tells which candidate, if any, it describes.
    if let Evidence::Uncertain(candidates, evidence) = evidence {
        raise_review_item(
            transaction,
            &source_id,
            &source_record,
            release_edition_id,
            &candidates,
            evidence,
        )?;
    }

    Ok(ImportedReleaseEdition {
        game_id,
        release_edition_id,
    })
}

/// The claim of the source of `identity` that it linked a record to an edition by `evidence`.
fn link_assertion(identity: &ReleaseAssertion, evidence: String) -> ReleaseAssertion {
    ReleaseAssertion {
        source_id: identity.source_id.clone(),
        source_location: identity.source_location.clone(),
        field: ReleaseAssertionField::Identifier,
        qualifier: Some("linked_by".to_owned()),
        value: evidence,
    }
}

/// What the evidence of a record says about the editions of other sources.
enum Evidence {
    /// The one edition the record describes too: its Game, its id and the evidence.
    Links(i64, i64, &'static str),
    /// Several editions the record may describe, which a human tells apart, with the evidence.
    Uncertain(Vec<i64>, &'static str),
    /// No edition of another source.
    Unlinked,
}

/// The normalized identity of the release edition a record describes.
struct NormalizedEdition<'a> {
    title: &'a str,
    platform: &'a str,
    region: &'a str,
    edition: &'a str,
}

/// The release edition another source asserts the record describes too, with the evidence that
/// links them. Dumps come first: the one edition of the record's platform, region and edition
/// whose record of another source last asserted exactly the record's dumps. Otherwise the one
/// such edition another source titled the same. Several editions, by dumps or by title, link
/// none of them, and dump evidence pointing at several editions forbids a title link. An edition
/// already holding a record of the same source is never linked, since one catalog listing two
/// records describes two releases, but it counts among the editions the evidence points at, and
/// dumps pointing at it alone forbid a title link.
fn linked_release_edition(
    transaction: &Transaction<'_>,
    source_id: &str,
    edition: &NormalizedEdition<'_>,
    dumps: Option<&str>,
) -> Result<Evidence, PortError> {
    let by_dumps = match dumps {
        // Every edition the dumps point at counts, so that one already holding a record of the
        // importing source never makes another look like the only match.
        Some(dumps) => editions_where(
            transaction,
            "SELECT DISTINCT r.game_id, r.id, EXISTS (
                 SELECT 1 FROM release_assertions own
                 WHERE own.release_edition_id = r.id
                   AND own.source_id = ?5
                   AND own.field = 'identifier'
                   AND own.qualifier = 'source_record'
             )
             FROM reference_dump_sets d
             JOIN release_editions r ON r.id = d.release_edition_id
             WHERE d.dump_set = ?1
               AND d.source_id != ?5
               AND r.normalized_platform = ?2
               AND r.normalized_region = ?3
               AND r.normalized_edition_name = ?4",
            params![
                dumps,
                edition.platform,
                edition.region,
                edition.edition,
                source_id
            ],
        )?,
        None => Vec::new(),
    };
    let (editions, evidence) = if by_dumps.is_empty() {
        // Starting from the matching title assertions lets the assertion value index find them.
        let by_title = editions_where(
            transaction,
            "SELECT DISTINCT r.game_id, r.id, EXISTS (
                 SELECT 1 FROM release_assertions own
                 WHERE own.release_edition_id = r.id
                   AND own.source_id = ?5
                   AND own.field = 'identifier'
                   AND own.qualifier = 'source_record'
             )
             FROM release_assertions t
             JOIN release_editions r ON r.id = t.release_edition_id
             WHERE t.field = 'title'
               AND t.qualifier = ''
               AND t.normalized_value = ?1
               AND t.source_id != ?5
               AND r.normalized_platform = ?2
               AND r.normalized_region = ?3
               AND r.normalized_edition_name = ?4",
            params![
                edition.title,
                edition.platform,
                edition.region,
                edition.edition,
                source_id
            ],
        )?;
        (by_title, "title")
    } else {
        (by_dumps, "sha1")
    };
    // Only the editions holding no record of the importing source may be the record's.
    let candidates: Vec<(i64, i64)> = editions
        .iter()
        .filter(|(_, _, occupied)| !occupied)
        .map(|(game_id, release_edition_id, _)| (*game_id, *release_edition_id))
        .collect();
    Ok(match (editions.as_slice(), candidates.as_slice()) {
        ([_], [(game_id, release_edition_id)]) => {
            Evidence::Links(*game_id, *release_edition_id, evidence)
        }
        (_, []) => Evidence::Unlinked,
        (_, several) => Evidence::Uncertain(
            several
                .iter()
                .map(|(_, release_edition_id)| *release_edition_id)
                .collect(),
            evidence,
        ),
    })
}

/// The dumps `assertions` name, as their sorted lower-case SHA-1s, when every dump (each ROM,
/// track or disk name) has one, forty hexadecimal digits long; otherwise the set of dumps is not
/// known whole.
fn dump_set(assertions: &[ReleaseAssertion]) -> Option<String> {
    let identifiers = |qualifier: &'static str| {
        assertions.iter().filter(move |assertion| {
            assertion.field == ReleaseAssertionField::Identifier
                && assertion.qualifier.as_deref() == Some(qualifier)
        })
    };
    let dumps = identifiers("rom_name").count() + identifiers("disk_name").count();
    let mut sha1: Vec<String> = identifiers("sha1")
        .map(|assertion| assertion.value.trim().to_ascii_lowercase())
        .collect();
    // A placeholder such as `none` names no dump, so it never links two records.
    let is_sha1 =
        |value: &String| value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit());
    if sha1.is_empty() || sha1.len() != dumps || !sha1.iter().all(is_sha1) {
        return None;
    }
    sha1.sort();
    Some(sha1.join(","))
}

/// Records the dumps the record last asserted, replacing what it asserted before, or forgets
/// them when the record no longer names them whole. The records of other sources sharing the
/// dumps it asserted before or now see other editions, so their items are brought up to date.
fn record_dump_set(
    transaction: &Transaction<'_>,
    source_id: &str,
    source_record: &str,
    release_edition_id: i64,
    dumps: Option<&str>,
) -> Result<(), PortError> {
    let before: Option<String> = transaction
        .query_row(
            "SELECT dump_set FROM reference_dump_sets WHERE source_id = ?1 AND source_record = ?2",
            params![source_id, source_record],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    match dumps {
        Some(dumps) => transaction.execute(
            "INSERT INTO reference_dump_sets (source_id, source_record, release_edition_id, dump_set)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(source_id, source_record)
             DO UPDATE SET release_edition_id = ?3, dump_set = ?4",
            params![source_id, source_record, release_edition_id, dumps],
        ),
        None => transaction.execute(
            "DELETE FROM reference_dump_sets WHERE source_id = ?1 AND source_record = ?2",
            params![source_id, source_record],
        ),
    }
    .map_err(sql_error)?;
    if let Some(before) = before.as_deref().filter(|before| Some(*before) != dumps) {
        revalidate_dump_peers(transaction, source_id, release_edition_id, before)?;
    }
    if let Some(dumps) = dumps {
        revalidate_dump_peers(transaction, source_id, release_edition_id, dumps)?;
    }
    Ok(())
}

/// Brings the Reference Review Item of a record already in the catalog, which keeps
/// `release_edition_id`, up to date with the editions its evidence points at now. A pending item
/// names them, or goes when none is left. A decided item is asked again only once an edition
/// the human did not see appears.
fn revalidate_review_item(
    transaction: &Transaction<'_>,
    source_id: &str,
    source_record: &str,
    release_edition_id: i64,
    edition: &NormalizedEdition<'_>,
    dumps: Option<&str>,
) -> Result<(), PortError> {
    let item = transaction
        .query_row(
            "SELECT release_edition_id, status, candidates_json FROM reference_review_items
             WHERE source_id = ?1 AND source_record = ?2",
            params![source_id, source_record],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;
    let (seen_edition, status, seen_candidates) = match item {
        Some(item) => item,
        // A record its dumps alone linked is asked about once they point at another edition too.
        None if linked_by_dumps(transaction, source_id, release_edition_id)? => {
            (release_edition_id, "linked".to_owned(), "[]".to_owned())
        }
        None => return Ok(()),
    };
    // The record's own edition holds its record, so the evidence never offers it.
    let (candidates, evidence) =
        match linked_release_edition(transaction, source_id, edition, dumps)? {
            Evidence::Links(_, candidate, evidence) => (vec![candidate], evidence),
            Evidence::Uncertain(candidates, evidence) => (candidates, evidence),
            Evidence::Unlinked => (Vec::new(), ""),
        };
    if status != "pending" {
        let seen: Vec<i64> = serde_json::from_str(&seen_candidates).map_err(|error| {
            PortError::new(format!(
                "catalog contains invalid review candidates: {error}"
            ))
        })?;
        let unseen = candidates
            .iter()
            .any(|candidate| *candidate != seen_edition && !seen.contains(candidate));
        if !unseen {
            return Ok(());
        }
    }
    if candidates.is_empty() {
        transaction
            .execute(
                "DELETE FROM reference_review_items WHERE source_id = ?1 AND source_record = ?2",
                params![source_id, source_record],
            )
            .map_err(sql_error)?;
        return Ok(());
    }
    raise_review_item(
        transaction,
        source_id,
        source_record,
        release_edition_id,
        &candidates,
        evidence,
    )
}

/// Whether the source holds its record on the edition only because its dumps linked them.
fn linked_by_dumps(
    transaction: &Transaction<'_>,
    source_id: &str,
    release_edition_id: i64,
) -> Result<bool, PortError> {
    transaction
        .query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM release_assertions
                 WHERE release_edition_id = ?1 AND source_id = ?2
                   AND field = 'identifier' AND qualifier = 'linked_by' AND value = 'sha1'
             )",
            params![release_edition_id, source_id],
            |row| row.get(0),
        )
        .map_err(sql_error)
}

/// Brings up to date the items of the records of other sources than `source_id` that last
/// asserted `dumps` on the platform, region and edition of `release_edition_id`, since an import
/// of `source_id` changed the editions those dumps point at.
fn revalidate_dump_peers(
    transaction: &Transaction<'_>,
    source_id: &str,
    release_edition_id: i64,
    dumps: &str,
) -> Result<(), PortError> {
    type Peer = (String, String, i64, Option<String>, String, String, String);
    let peers: Vec<Peer> = transaction
        .prepare(
            "SELECT d.source_id, d.source_record, d.release_edition_id, (
                 SELECT t.normalized_value FROM release_assertions t
                 WHERE t.release_edition_id = d.release_edition_id
                   AND t.source_id = d.source_id
                   AND t.field = 'title' AND t.qualifier = ''
                 ORDER BY t.id DESC LIMIT 1
             ), r.normalized_platform, r.normalized_region, r.normalized_edition_name
             FROM reference_dump_sets d
             JOIN release_editions r ON r.id = d.release_edition_id
             JOIN release_editions own ON own.id = ?3
             WHERE d.dump_set = ?1
               AND d.source_id != ?2
               AND r.normalized_platform = own.normalized_platform
               AND r.normalized_region = own.normalized_region
               AND r.normalized_edition_name = own.normalized_edition_name",
        )
        .map_err(sql_error)?
        .query_map(params![dumps, source_id, release_edition_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })
        .map_err(sql_error)?
        .collect::<rusqlite::Result<_>>()
        .map_err(sql_error)?;
    for (peer_source, peer_record, peer_edition, title, platform, region, edition) in peers {
        // The record's latest title only counts once no other source shares its dumps.
        let peer = NormalizedEdition {
            title: title.as_deref().unwrap_or_default(),
            platform: &platform,
            region: &region,
            edition: &edition,
        };
        revalidate_review_item(
            transaction,
            &peer_source,
            &peer_record,
            peer_edition,
            &peer,
            Some(dumps),
        )?;
    }
    Ok(())
}

/// Asks a human which of the `candidates` the record keeping `release_edition_id` describes,
/// replacing what was asked before about that record.
fn raise_review_item(
    transaction: &Transaction<'_>,
    source_id: &str,
    source_record: &str,
    release_edition_id: i64,
    candidates: &[i64],
    evidence: &str,
) -> Result<(), PortError> {
    let candidates_json = serde_json::to_string(candidates).map_err(|error| {
        PortError::new(format!("failed to serialize review candidates: {error}"))
    })?;
    transaction
        .execute(
            "INSERT INTO reference_review_items (
                source_id, source_record, release_edition_id, evidence, candidates_json, status
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending')
             ON CONFLICT(source_id, source_record) DO UPDATE SET
                release_edition_id = ?3, evidence = ?4, candidates_json = ?5, status = 'pending'",
            params![
                source_id,
                source_record,
                release_edition_id,
                evidence,
                candidates_json
            ],
        )
        .map_err(sql_error)?;
    Ok(())
}

/// The pending Reference Review Item `item_id`, if there is one.
fn pending_review_item(
    transaction: &Transaction<'_>,
    item_id: i64,
) -> Result<Option<ReferenceReviewItem>, PortError> {
    let row = transaction
        .query_row(
            "SELECT source_id, source_record, release_edition_id, evidence, candidates_json
             FROM reference_review_items
             WHERE id = ?1 AND status = 'pending'",
            params![item_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;
    row.map(
        |(source_id, source_record, release_edition_id, evidence, candidates_json)| {
            let candidates = serde_json::from_str(&candidates_json).map_err(|error| {
                PortError::new(format!(
                    "catalog contains invalid review candidates: {error}"
                ))
            })?;
            Ok(ReferenceReviewItem {
                id: item_id,
                source_id: SourceId::from(source_id),
                source_record,
                release_edition_id,
                evidence,
                candidates,
            })
        },
    )
    .transpose()
}

/// Moves the record of `item`, which its source linked into another source's edition, alone to
/// the candidate `chosen`, with the evidence of the decision. The edition it leaves keeps the
/// claims of the other sources and its Assets.
fn move_linked_record(
    transaction: &Transaction<'_>,
    item: &ReferenceReviewItem,
    chosen: i64,
) -> Result<ReferenceReviewOutcome, PortError> {
    let source_id = item.source_id.as_str();
    let left = item.release_edition_id;
    // An edition holding a record of the item's source is another release of it.
    if holds_record_of(transaction, chosen, source_id)? {
        return Ok(ReferenceReviewOutcome::NotACandidate);
    }
    let identity = transaction
        .query_row(
            "SELECT source_location FROM release_assertions
             WHERE release_edition_id = ?1 AND source_id = ?2
               AND field = 'identifier' AND qualifier = 'source_record' AND value = ?3",
            params![left, source_id, item.source_record],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sql_error)?;
    // The evidence that linked it there does not follow it.
    transaction
        .execute(
            "DELETE FROM release_assertions
             WHERE release_edition_id = ?1 AND source_id = ?2
               AND field = 'identifier' AND qualifier = 'linked_by'",
            params![left, source_id],
        )
        .map_err(sql_error)?;
    transaction
        .execute(
            "UPDATE OR IGNORE release_assertions SET release_edition_id = ?2
             WHERE release_edition_id = ?1 AND source_id = ?3",
            params![left, chosen, source_id],
        )
        .map_err(sql_error)?;
    // What stays behind repeats a claim the chosen edition already carries.
    transaction
        .execute(
            "DELETE FROM release_assertions WHERE release_edition_id = ?1 AND source_id = ?2",
            params![left, source_id],
        )
        .map_err(sql_error)?;
    transaction
        .execute(
            "UPDATE reference_dump_sets SET release_edition_id = ?3
             WHERE source_id = ?1 AND source_record = ?2",
            params![source_id, item.source_record, chosen],
        )
        .map_err(sql_error)?;
    if let Some(source_location) = identity {
        let link = ReleaseAssertion {
            source_id: item.source_id.clone(),
            source_location,
            field: ReleaseAssertionField::Identifier,
            qualifier: Some("linked_by".to_owned()),
            value: "review".to_owned(),
        };
        persist_release_assertions(transaction, chosen, &[link])?;
    }
    // The edition it leaves counts among those the human considered.
    let mut considered = item.candidates.clone();
    considered.push(left);
    let considered = serde_json::to_string(&considered).map_err(|error| {
        PortError::new(format!("failed to serialize review candidates: {error}"))
    })?;
    transaction
        .execute(
            "UPDATE reference_review_items
             SET status = 'linked', release_edition_id = ?2, candidates_json = ?3
             WHERE id = ?1",
            params![item.id, chosen, considered],
        )
        .map_err(sql_error)?;
    // The records of other sources sharing its dumps see them point at other editions now.
    let dumps: Option<String> = transaction
        .query_row(
            "SELECT dump_set FROM reference_dump_sets WHERE source_id = ?1 AND source_record = ?2",
            params![source_id, item.source_record],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    if let Some(dumps) = dumps {
        revalidate_dump_peers(transaction, source_id, chosen, &dumps)?;
    }
    Ok(ReferenceReviewOutcome::Decided)
}

/// Whether the source linked its record into the edition, which another source founded.
fn holds_link(
    transaction: &Transaction<'_>,
    release_edition_id: i64,
    source_id: &str,
) -> Result<bool, PortError> {
    transaction
        .query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM release_assertions
                 WHERE release_edition_id = ?1 AND source_id = ?2
                   AND field = 'identifier' AND qualifier = 'linked_by'
             )",
            params![release_edition_id, source_id],
            |row| row.get(0),
        )
        .map_err(sql_error)
}

/// Whether the edition holds a record of the source.
fn holds_record_of(
    transaction: &Transaction<'_>,
    release_edition_id: i64,
    source_id: &str,
) -> Result<bool, PortError> {
    transaction
        .query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM release_assertions
                 WHERE release_edition_id = ?1 AND source_id = ?2
                   AND field = 'identifier' AND qualifier = 'source_record'
             )",
            params![release_edition_id, source_id],
            |row| row.get(0),
        )
        .map_err(sql_error)
}

/// Whether some source holds a record on both editions.
fn share_a_source_of_record(
    transaction: &Transaction<'_>,
    first: i64,
    second: i64,
) -> Result<bool, PortError> {
    transaction
        .query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM release_assertions AS one
                 JOIN release_assertions AS other ON other.source_id = one.source_id
                 WHERE one.release_edition_id = ?1 AND other.release_edition_id = ?2
                   AND one.field = 'identifier' AND one.qualifier = 'source_record'
                   AND other.field = 'identifier' AND other.qualifier = 'source_record'
             )",
            params![first, second],
            |row| row.get(0),
        )
        .map_err(sql_error)
}

/// The editions `query` selects: each one's Game, its id and whether it holds a record of the
/// importing source.
fn editions_where(
    transaction: &Transaction<'_>,
    query: &str,
    parameters: impl rusqlite::Params,
) -> Result<Vec<(i64, i64, bool)>, PortError> {
    let mut statement = transaction.prepare(query).map_err(sql_error)?;
    statement
        .query_map(parameters, |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(sql_error)?
        .collect::<rusqlite::Result<_>>()
        .map_err(sql_error)
}

fn persist_release_assertions(
    transaction: &Transaction<'_>,
    release_edition_id: i64,
    assertions: &[ReleaseAssertion],
) -> Result<(), PortError> {
    for assertion in assertions {
        if assertion.source_id.as_str().trim().is_empty() || assertion.value.trim().is_empty() {
            return Err(PortError::new(
                "release assertions require non-blank source ids and values".into(),
            ));
        }
        let qualifier = assertion.qualifier.as_deref().unwrap_or("");
        // Recording order is observation order: a claim observed again, possibly from a moved
        // or renamed catalog, moves after the claims it may have replaced meanwhile, so a
        // source reverting a correction is its latest word.
        transaction
            .execute(
                "DELETE FROM release_assertions
                 WHERE release_edition_id = ?1 AND source_id = ?2
                   AND field = ?3 AND qualifier = ?4 AND value = ?5",
                params![
                    release_edition_id,
                    assertion.source_id.as_str(),
                    assertion_field_to_str(assertion.field),
                    qualifier,
                    assertion.value,
                ],
            )
            .map_err(sql_error)?;
        transaction
            .execute(
                "INSERT INTO release_assertions (
                    release_edition_id,
                    source_id,
                    source_location,
                    field,
                    qualifier,
                    value,
                    normalized_value
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    release_edition_id,
                    assertion.source_id.as_str(),
                    assertion.source_location,
                    assertion_field_to_str(assertion.field),
                    qualifier,
                    assertion.value,
                    normalize(&assertion.value),
                ],
            )
            .map_err(sql_error)?;
    }
    Ok(())
}

fn assertion_field_to_str(field: ReleaseAssertionField) -> &'static str {
    match field {
        ReleaseAssertionField::Title => "title",
        ReleaseAssertionField::Region => "region",
        ReleaseAssertionField::Revision => "revision",
        ReleaseAssertionField::Identifier => "identifier",
    }
}

pub(super) fn parse_release_assertion_field(
    value: &str,
) -> Result<ReleaseAssertionField, PortError> {
    match value {
        "title" => Ok(ReleaseAssertionField::Title),
        "region" => Ok(ReleaseAssertionField::Region),
        "revision" => Ok(ReleaseAssertionField::Revision),
        "identifier" => Ok(ReleaseAssertionField::Identifier),
        other => Err(PortError::new(format!(
            "unknown release assertion field in catalog: {other}"
        ))),
    }
}
