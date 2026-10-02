use game_media_vault_application::{PortError, ReferenceCatalogRepositoryPort};
use game_media_vault_domain::{
    ImportedReleaseEdition, ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField,
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
        return Ok(ImportedReleaseEdition {
            game_id,
            release_edition_id,
        });
    }

    // Another source may already describe this release: its assertions then join that edition,
    // with the evidence of the link.
    let edition = NormalizedEdition {
        title: &normalized_title,
        platform: &normalized_platform,
        region: &normalized_region,
        edition: &normalized_edition,
    };
    if let Some((game_id, release_edition_id, evidence)) =
        linked_release_edition(transaction, &source_id, &edition, dumps.as_deref())?
    {
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
/// already holding a record of the same source is never linked by dumps, since one catalog
/// listing two releases of the same dumps describes two releases, but it counts among the
/// editions they point at.
fn linked_release_edition(
    transaction: &Transaction<'_>,
    source_id: &str,
    edition: &NormalizedEdition<'_>,
    dumps: Option<&str>,
) -> Result<Option<(i64, i64, &'static str)>, PortError> {
    if let Some(dumps) = dumps {
        // Every edition the dumps point at counts, so that one already holding a record of the
        // importing source never makes another look like the only match.
        let editions: Vec<(i64, i64, bool)> = transaction
            .prepare(
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
            )
            .map_err(sql_error)?
            .query_map(
                params![
                    dumps,
                    edition.platform,
                    edition.region,
                    edition.edition,
                    source_id
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(sql_error)?
            .collect::<rusqlite::Result<_>>()
            .map_err(sql_error)?;
        match editions.as_slice() {
            // An edition already holding a record of the importing source is another release.
            [] | [(_, _, true)] => {}
            [(game_id, release_edition_id, false)] => {
                return Ok(Some((*game_id, *release_edition_id, "sha1")));
            }
            _ => return Ok(None),
        }
    }
    // Starting from the matching title assertions lets the assertion value index find them.
    let editions = editions_where(
        transaction,
        "SELECT DISTINCT r.game_id, r.id
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
    Ok(match editions.as_slice() {
        [(game_id, release_edition_id)] => Some((*game_id, *release_edition_id, "title")),
        _ => None,
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
/// them when the record no longer names them whole.
fn record_dump_set(
    transaction: &Transaction<'_>,
    source_id: &str,
    source_record: &str,
    release_edition_id: i64,
    dumps: Option<&str>,
) -> Result<(), PortError> {
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
    Ok(())
}

fn editions_where(
    transaction: &Transaction<'_>,
    query: &str,
    parameters: impl rusqlite::Params,
) -> Result<Vec<(i64, i64)>, PortError> {
    let mut statement = transaction.prepare(query).map_err(sql_error)?;
    statement
        .query_map(parameters, |row| Ok((row.get(0)?, row.get(1)?)))
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
