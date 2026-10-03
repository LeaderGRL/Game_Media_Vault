use std::collections::HashMap;

use game_media_vault_application::PortError;
use game_media_vault_domain::{
    AssetCandidateMatch, AssetProvenance, DerivedAsset, LibraryAsset, LibraryEntry, MediaInfo,
    ReleaseAssertion, SourceId,
};
use rusqlite::Connection;

use super::{
    SqliteCatalog, assets::parse_asset_type, derived::derived_by_original,
    reference::parse_release_assertion_field, sql_error,
};

/// The columns a library row reads, in the order `releases_with_assets` reads them.
const LIBRARY_COLUMNS: &str = "g.id, g.title,
    r.id, r.platform, r.region, r.edition_name,
    a.id, a.asset_type, a.object_hash, a.byte_len, a.original_filename,
    p.source_kind, p.source_asset_label, p.source_location,
    p.match_decision_json,
    a.media_type, a.width, a.height, a.document_json";

impl SqliteCatalog {
    /// Projects every Release Edition with its Assets, provenance and source assertions.
    pub(super) fn library_entries(&self) -> Result<Vec<LibraryEntry>, PortError> {
        let connection = self.connect()?;
        let mut entries = releases_with_assets(&connection, None)?;
        attach_assertions(&connection, &mut entries)?;
        attach_derived(&mut entries, derived_by_original(&connection, None)?);
        Ok(entries)
    }

    /// Projects the Release Editions holding the `limit` Assets retained last, with those Assets
    /// alone and their provenance and derivatives, reading no other row.
    pub(super) fn latest_library_assets(
        &self,
        limit: usize,
    ) -> Result<Vec<LibraryEntry>, PortError> {
        let connection = self.connect()?;
        let mut entries = releases_with_assets(&connection, Some(limit))?;
        attach_derived(&mut entries, derived_by_original(&connection, Some(limit))?);
        Ok(entries)
    }
}

/// Reads Release Editions with their Assets and provenance, without assertions or derivatives:
/// every one, or with `latest`, those holding that many Assets retained last.
fn releases_with_assets(
    connection: &Connection,
    latest: Option<usize>,
) -> Result<Vec<LibraryEntry>, PortError> {
    let mut entries: Vec<LibraryEntry> = Vec::new();
    {
        let sql = match latest {
            None => format!(
                "SELECT {LIBRARY_COLUMNS}
                 FROM release_editions r
                 JOIN games g ON g.id = r.game_id
                 LEFT JOIN assets a ON a.release_edition_id = r.id
                 LEFT JOIN asset_provenance p ON p.asset_id = a.id
                 ORDER BY g.title, r.id, a.id, p.id"
            ),
            Some(_) => format!(
                "SELECT {LIBRARY_COLUMNS}
                 FROM (SELECT * FROM assets ORDER BY id DESC LIMIT ?1) a
                 JOIN release_editions r ON r.id = a.release_edition_id
                 JOIN games g ON g.id = r.game_id
                 LEFT JOIN asset_provenance p ON p.asset_id = a.id
                 ORDER BY g.title, r.id, a.id, p.id"
            ),
        };
        let mut statement = connection.prepare(&sql).map_err(sql_error)?;
        let mut rows = match latest {
            None => statement.query([]),
            Some(limit) => statement.query([i64::try_from(limit).unwrap_or(i64::MAX)]),
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

/// Attaches the source assertions of each of `entries`, in observation order.
fn attach_assertions(
    connection: &Connection,
    entries: &mut [LibraryEntry],
) -> Result<(), PortError> {
    let positions = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.release_edition_id, index))
        .collect::<HashMap<_, _>>();
    let mut assertion_statement = connection
        .prepare(
            "SELECT release_edition_id, source_id, source_location, field, qualifier, value
                 FROM release_assertions
                 ORDER BY release_edition_id, id",
        )
        .map_err(sql_error)?;
    let mut assertion_rows = assertion_statement.query([]).map_err(sql_error)?;
    while let Some(row) = assertion_rows.next().map_err(sql_error)? {
        let release_edition_id: i64 = row.get(0).map_err(sql_error)?;
        let Some(index) = positions.get(&release_edition_id).copied() else {
            continue;
        };
        let qualifier: String = row.get(4).map_err(sql_error)?;
        entries[index].assertions.push(ReleaseAssertion {
            source_id: SourceId::from(row.get::<_, String>(1).map_err(sql_error)?),
            source_location: row.get(2).map_err(sql_error)?,
            field: parse_release_assertion_field(&row.get::<_, String>(3).map_err(sql_error)?)?,
            qualifier: (!qualifier.is_empty()).then_some(qualifier),
            value: row.get(5).map_err(sql_error)?,
        });
    }

    Ok(())
}

/// Attaches to each Asset of `entries` the derivatives of its original.
fn attach_derived(entries: &mut [LibraryEntry], derived: HashMap<String, Vec<DerivedAsset>>) {
    for asset in entries.iter_mut().flat_map(|entry| entry.assets.iter_mut()) {
        if let Some(outputs) = derived.get(&asset.object_hash) {
            asset.derived = outputs.clone();
        }
    }
}
