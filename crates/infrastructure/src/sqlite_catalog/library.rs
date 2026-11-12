use std::collections::HashMap;

use game_media_vault_application::PortError;
use game_media_vault_domain::{
    AssetCandidateMatch, AssetProvenance, DerivedAsset, LibraryAsset, LibraryEntry, LibrarySummary,
    MediaInfo, ReleaseAssertion, ReleaseAssertionField, SourceId,
};
use rusqlite::{Connection, Row, ToSql};

use super::{
    SqliteCatalog,
    assets::parse_asset_type,
    derived::derived_by_original,
    reference::{assertion_field_to_str, parse_release_assertion_field},
    sql_error,
};

/// The Release Editions a projection reads.
#[derive(Clone, Copy)]
pub(super) enum Projection<'a> {
    /// Every one.
    All,
    /// Those holding the `limit` Assets retained last, with those Assets alone.
    Latest(usize),
    /// Those a JSON array of Release Edition ids names.
    Releases(&'a str),
}

/// The columns a library row reads, in the order `releases_with_assets` reads them.
const LIBRARY_COLUMNS: &str = "g.id, g.title,
    r.id, r.platform, r.region, r.edition_name,
    a.id, a.asset_type, a.object_hash, a.byte_len, a.original_filename,
    p.source_kind, p.source_asset_label, p.source_location,
    p.match_decision_json,
    a.media_type, a.width, a.height, a.document_json";

impl SqliteCatalog {
    /// Projects every Release Edition with its Assets, provenance and source assertions, read
    /// in one snapshot while executions retain media.
    pub(super) fn library_entries(&self) -> Result<Vec<LibraryEntry>, PortError> {
        self.project(Projection::All)
    }

    /// Projects the Release Editions `release_edition_ids` names as `library_entries` does,
    /// reading only their rows.
    pub(super) fn library_entries_named(
        &self,
        release_edition_ids: &[i64],
    ) -> Result<Vec<LibraryEntry>, PortError> {
        if release_edition_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = serde_json::to_string(release_edition_ids)
            .map_err(|error| PortError::new(format!("cannot list release ids: {error}")))?;
        self.project(Projection::Releases(&ids))
    }

    /// Projects whole Release Editions, with their source assertions, in one snapshot.
    fn project(&self, projection: Projection<'_>) -> Result<Vec<LibraryEntry>, PortError> {
        let mut connection = self.connect()?;
        let snapshot = connection.transaction().map_err(sql_error)?;
        let mut entries = releases_with_assets(&snapshot, projection)?;
        attach_assertions(&snapshot, &mut entries, projection)?;
        attach_derived(&mut entries, derived_by_original(&snapshot, projection)?);
        Ok(entries)
    }

    /// Summarizes every Release Edition, in library order, from its own row and the types,
    /// Sources and titles of its Assets and assertions alone, in one snapshot.
    pub(super) fn library_summaries(&self) -> Result<Vec<LibrarySummary>, PortError> {
        let mut connection = self.connect()?;
        let snapshot = connection.transaction().map_err(sql_error)?;
        let mut summaries: Vec<LibrarySummary> = Vec::new();
        {
            let mut statement = snapshot
                .prepare(
                    "SELECT g.id, g.title, r.id, r.platform, r.region, r.edition_name
                     FROM release_editions r
                     JOIN games g ON g.id = r.game_id
                     ORDER BY g.title, r.id",
                )
                .map_err(sql_error)?;
            let mut rows = statement.query([]).map_err(sql_error)?;
            while let Some(row) = rows.next().map_err(sql_error)? {
                summaries.push(LibrarySummary {
                    game_id: row.get(0).map_err(sql_error)?,
                    game_title: row.get(1).map_err(sql_error)?,
                    release_edition_id: row.get(2).map_err(sql_error)?,
                    platform: row.get(3).map_err(sql_error)?,
                    region: row.get(4).map_err(sql_error)?,
                    edition_name: row.get(5).map_err(sql_error)?,
                    asset_types: Vec::new(),
                    source_ids: Vec::new(),
                    title_assertions: Vec::new(),
                });
            }
        }
        let positions: HashMap<i64, usize> = summaries
            .iter()
            .enumerate()
            .map(|(index, summary)| (summary.release_edition_id, index))
            .collect();
        for_each_row(
            &snapshot,
            "SELECT release_edition_id, asset_type FROM assets ORDER BY release_edition_id, id",
            &[],
            |row| {
                let asset_type = parse_asset_type(&row.get::<_, String>(1).map_err(sql_error)?)?;
                if let Some(summary) = summary_of(&mut summaries, &positions, row)?
                    && !summary.asset_types.contains(&asset_type)
                {
                    summary.asset_types.push(asset_type);
                }
                Ok(())
            },
        )?;
        for_each_row(
            &snapshot,
            "SELECT a.release_edition_id, p.source_kind
             FROM asset_provenance p JOIN assets a ON a.id = p.asset_id
             UNION
             SELECT release_edition_id, source_id FROM release_assertions",
            &[],
            |row| {
                if let Some(summary) = summary_of(&mut summaries, &positions, row)? {
                    summary
                        .source_ids
                        .push(SourceId::from(row.get::<_, String>(1).map_err(sql_error)?));
                }
                Ok(())
            },
        )?;
        for_each_row(
            &snapshot,
            "SELECT release_edition_id, source_id, source_location, field, qualifier, value
             FROM release_assertions
             WHERE field = ?1
             ORDER BY release_edition_id, id",
            &[&assertion_field_to_str(ReleaseAssertionField::Title)],
            |row| {
                let assertion = assertion_of(row)?;
                if let Some(summary) = summary_of(&mut summaries, &positions, row)? {
                    summary.title_assertions.push(assertion);
                }
                Ok(())
            },
        )?;
        for summary in &mut summaries {
            summary
                .source_ids
                .sort_by(|left, right| left.as_str().cmp(right.as_str()));
            summary.source_ids.dedup();
        }
        Ok(summaries)
    }

    /// Projects the Release Editions holding the `limit` Assets retained last, with those Assets
    /// alone and their provenance and derivatives, reading no other row. One snapshot reads both,
    /// so an Asset retained meanwhile never shifts the derivatives' window off the Assets read.
    pub(super) fn latest_library_assets(
        &self,
        limit: usize,
    ) -> Result<Vec<LibraryEntry>, PortError> {
        let mut connection = self.connect()?;
        let snapshot = connection.transaction().map_err(sql_error)?;
        let projection = Projection::Latest(limit);
        let mut entries = releases_with_assets(&snapshot, projection)?;
        attach_derived(&mut entries, derived_by_original(&snapshot, projection)?);
        Ok(entries)
    }
}

/// The summary of the Release Edition a row names in its first column, at its place in
/// `positions`.
fn summary_of<'a>(
    summaries: &'a mut [LibrarySummary],
    positions: &HashMap<i64, usize>,
    row: &Row<'_>,
) -> Result<Option<&'a mut LibrarySummary>, PortError> {
    let release_edition_id: i64 = row.get(0).map_err(sql_error)?;
    Ok(positions
        .get(&release_edition_id)
        .map(|&index| &mut summaries[index]))
}

/// Runs `sql` with `params`, handing each row to `each`.
fn for_each_row(
    connection: &Connection,
    sql: &str,
    params: &[&dyn ToSql],
    mut each: impl FnMut(&Row<'_>) -> Result<(), PortError>,
) -> Result<(), PortError> {
    let mut statement = connection.prepare(sql).map_err(sql_error)?;
    let mut rows = statement.query(params).map_err(sql_error)?;
    while let Some(row) = rows.next().map_err(sql_error)? {
        each(row)?;
    }
    Ok(())
}

/// The assertion a row reads as `release_edition_id, source_id, source_location, field,
/// qualifier, value`.
fn assertion_of(row: &Row<'_>) -> Result<ReleaseAssertion, PortError> {
    let qualifier: String = row.get(4).map_err(sql_error)?;
    Ok(ReleaseAssertion {
        source_id: SourceId::from(row.get::<_, String>(1).map_err(sql_error)?),
        source_location: row.get(2).map_err(sql_error)?,
        field: parse_release_assertion_field(&row.get::<_, String>(3).map_err(sql_error)?)?,
        qualifier: (!qualifier.is_empty()).then_some(qualifier),
        value: row.get(5).map_err(sql_error)?,
    })
}

/// Reads the Release Editions of `projection` with their Assets and provenance, without
/// assertions or derivatives.
fn releases_with_assets(
    connection: &Connection,
    projection: Projection<'_>,
) -> Result<Vec<LibraryEntry>, PortError> {
    let mut entries: Vec<LibraryEntry> = Vec::new();
    {
        let sql = match projection {
            Projection::All => format!(
                "SELECT {LIBRARY_COLUMNS}
                 FROM release_editions r
                 JOIN games g ON g.id = r.game_id
                 LEFT JOIN assets a ON a.release_edition_id = r.id
                 LEFT JOIN asset_provenance p ON p.asset_id = a.id
                 ORDER BY g.title, r.id, a.id, p.id"
            ),
            Projection::Latest(_) => format!(
                "SELECT {LIBRARY_COLUMNS}
                 FROM (SELECT * FROM assets ORDER BY id DESC LIMIT ?1) a
                 JOIN release_editions r ON r.id = a.release_edition_id
                 JOIN games g ON g.id = r.game_id
                 LEFT JOIN asset_provenance p ON p.asset_id = a.id
                 ORDER BY g.title, r.id, a.id, p.id"
            ),
            Projection::Releases(_) => format!(
                "SELECT {LIBRARY_COLUMNS}
                 FROM release_editions r
                 JOIN games g ON g.id = r.game_id
                 LEFT JOIN assets a ON a.release_edition_id = r.id
                 LEFT JOIN asset_provenance p ON p.asset_id = a.id
                 WHERE r.id IN (SELECT value FROM json_each(?1))
                 ORDER BY g.title, r.id, a.id, p.id"
            ),
        };
        let mut statement = connection.prepare(&sql).map_err(sql_error)?;
        let mut rows = match projection {
            Projection::All => statement.query([]),
            Projection::Latest(limit) => {
                statement.query([i64::try_from(limit).unwrap_or(i64::MAX)])
            }
            Projection::Releases(ids) => statement.query([ids]),
        }
        .map_err(sql_error)?;

        while let Some(row) = rows.next().map_err(sql_error)? {
            let release_edition_id: i64 = row.get(2).map_err(sql_error)?;
            if entries
                .last()
                .is_none_or(|entry| entry.release_edition_id != release_edition_id)
            {
                entries.push(LibraryEntry {
                    game_id: row.get(0).map_err(sql_error)?,
                    game_title: row.get(1).map_err(sql_error)?,
                    release_edition_id,
                    platform: row.get(3).map_err(sql_error)?,
                    region: row.get(4).map_err(sql_error)?,
                    edition_name: row.get(5).map_err(sql_error)?,
                    assertions: Vec::new(),
                    assets: Vec::new(),
                });
            }

            let Some(asset_id) = row.get::<_, Option<i64>>(6).map_err(sql_error)? else {
                continue;
            };
            let source_id: Option<String> = row.get(11).map_err(sql_error)?;
            let source_asset_label: Option<String> = row.get(12).map_err(sql_error)?;
            let source_location: Option<String> = row.get(13).map_err(sql_error)?;
            let match_decision = row
                .get::<_, Option<String>>(14)
                .map_err(sql_error)?
                .map(|decision_json| {
                    serde_json::from_str::<AssetCandidateMatch>(&decision_json).map_err(|error| {
                        PortError::new(format!(
                            "catalog contains invalid asset match decision: {error}"
                        ))
                    })
                })
                .transpose()?;
            let entry = entries
                .last_mut()
                .ok_or_else(|| PortError::new("library release aggregation failed".into()))?;

            if let Some(asset) = entry
                .assets
                .last_mut()
                .filter(|asset| asset.asset_id == asset_id)
            {
                if let (Some(id), Some(location)) = (source_id, source_location) {
                    asset.provenance.push(AssetProvenance {
                        source_id: SourceId::from(id),
                        source_asset_label,
                        source_location: location,
                        match_decision,
                    });
                }
                continue;
            }

            let asset_type = row
                .get::<_, Option<String>>(7)
                .map_err(sql_error)?
                .ok_or_else(|| PortError::new("catalog asset is missing its type".into()))?;
            let object_hash = row
                .get::<_, Option<String>>(8)
                .map_err(sql_error)?
                .ok_or_else(|| PortError::new("catalog asset is missing its object hash".into()))?;
            let byte_len = row
                .get::<_, Option<i64>>(9)
                .map_err(sql_error)?
                .ok_or_else(|| PortError::new("catalog asset is missing its byte length".into()))?;
            let original_filename = row
                .get::<_, Option<String>>(10)
                .map_err(sql_error)?
                .ok_or_else(|| {
                    PortError::new("catalog asset is missing its original filename".into())
                })?;
            let media = MediaInfo {
                media_type: row
                    .get::<_, Option<String>>(15)
                    .map_err(sql_error)?
                    .ok_or_else(|| {
                        PortError::new("catalog asset is missing its media type".into())
                    })?,
                width: row.get(16).map_err(sql_error)?,
                height: row.get(17).map_err(sql_error)?,
                document: row
                    .get::<_, Option<String>>(18)
                    .map_err(sql_error)?
                    .map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(|error| {
                        PortError::new(format!(
                            "catalog contains invalid document metadata: {error}"
                        ))
                    })?,
            };
            let mut provenance = Vec::new();
            if let (Some(id), Some(location)) = (source_id, source_location) {
                provenance.push(AssetProvenance {
                    source_id: SourceId::from(id),
                    source_asset_label,
                    source_location: location,
                    match_decision,
                });
            }
            entry.assets.push(LibraryAsset {
                asset_id,
                asset_type: parse_asset_type(&asset_type)?,
                object_hash,
                byte_len: u64::try_from(byte_len).map_err(|_| {
                    PortError::new("catalog contains a negative byte length".into())
                })?,
                media,
                original_filename,
                provenance,
                derived: Vec::new(),
            });
        }
    }
    Ok(entries)
}

/// Attaches the source assertions of each of `entries`, read from `projection`, in observation
/// order.
fn attach_assertions(
    connection: &Connection,
    entries: &mut [LibraryEntry],
    projection: Projection<'_>,
) -> Result<(), PortError> {
    let positions = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.release_edition_id, index))
        .collect::<HashMap<_, _>>();
    let columns = "SELECT release_edition_id, source_id, source_location, field, qualifier, value
                   FROM release_assertions";
    let (sql, ids) = match projection {
        Projection::Releases(ids) => (
            format!(
                "{columns}
                 WHERE release_edition_id IN (SELECT value FROM json_each(?1))
                 ORDER BY release_edition_id, id"
            ),
            Some(ids),
        ),
        Projection::All | Projection::Latest(_) => {
            (format!("{columns} ORDER BY release_edition_id, id"), None)
        }
    };
    let params: Vec<&dyn ToSql> = ids.iter().map(|ids| ids as &dyn ToSql).collect();
    for_each_row(connection, &sql, &params, |row| {
        let release_edition_id: i64 = row.get(0).map_err(sql_error)?;
        if let Some(&index) = positions.get(&release_edition_id) {
            entries[index].assertions.push(assertion_of(row)?);
        }
        Ok(())
    })
}

/// Attaches to each Asset of `entries` the derivatives of its original.
fn attach_derived(entries: &mut [LibraryEntry], derived: HashMap<String, Vec<DerivedAsset>>) {
    for asset in entries.iter_mut().flat_map(|entry| entry.assets.iter_mut()) {
        if let Some(outputs) = derived.get(&asset.object_hash) {
            asset.derived = outputs.clone();
        }
    }
}
