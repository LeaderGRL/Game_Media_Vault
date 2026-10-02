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
        linked_release_edition(transaction, &source_id, &edition)?
    {
        let link = link_assertion(identity, evidence.to_owned());
        persist_release_assertions(transaction, release_edition_id, &record.assertions)?;
        persist_release_assertions(transaction, release_edition_id, &[link])?;
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
/// links them: the one edition of its platform, region and edition another source titled the
/// same. Several such editions link none of them. Dump checksums link nothing yet: the catalog
/// keeps every claim a source ever made, so it cannot tell the dumps one catalog edition
/// asserts together, nor how many share a checksum.
fn linked_release_edition(
    transaction: &Transaction<'_>,
    source_id: &str,
    edition: &NormalizedEdition<'_>,
) -> Result<Option<(i64, i64, &'static str)>, PortError> {
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
