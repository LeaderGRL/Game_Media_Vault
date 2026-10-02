use std::{
    fs,
    path::{Path, PathBuf},
};

use game_media_vault_application::PortError;
use game_media_vault_domain::{AssetType, ImportedAsset, PersistAsset};
use rusqlite::{OptionalExtension, Transaction, params};

use super::{normalize, sql_error};

struct ExistingImportLookup<'a> {
    normalized_title: &'a str,
    normalized_platform: &'a str,
    normalized_region: &'a str,
    normalized_edition: &'a str,
    asset_type: &'a str,
    source_id: &'a str,
    byte_len: i64,
    release_edition_id: Option<i64>,
}

struct ExistingImportMatch {
    imported: ImportedAsset,
}

/// Persists an asset and its provenance. `candidate_identity` names the acquisition candidate
/// the provenance belongs to; direct imports have none.
pub(super) fn persist_asset_in_transaction(
    transaction: &Transaction<'_>,
    record: PersistAsset,
    candidate_identity: Option<&str>,
) -> Result<ImportedAsset, PortError> {
    let normalized_title = normalize(&record.game_title);
    let normalized_platform = normalize(&record.platform);
    let normalized_region = normalize(&record.region);
    let normalized_edition = normalize(&record.edition_name);
    let asset_type = asset_type_to_str(record.asset_type);
    let source_id = record.source_id.as_str();
    let byte_len = i64::try_from(record.byte_len)
        .map_err(|_| PortError::new("asset byte length exceeds SQLite INTEGER range".into()))?;
    let explicit_target = resolve_existing_release_target(transaction, &record)?;
    let lookup = ExistingImportLookup {
        normalized_title: &normalized_title,
        normalized_platform: &normalized_platform,
        normalized_region: &normalized_region,
        normalized_edition: &normalized_edition,
        asset_type,
        source_id,
        byte_len,
        release_edition_id: explicit_target.map(|(_, release_edition_id)| release_edition_id),
    };
    if let Some(existing) = find_existing_import(transaction, &record, &lookup)? {
        record_provenance(transaction, &existing.imported, &record, candidate_identity)?;
        return Ok(existing.imported);
    }

    let imported = persist_new_asset_in_transaction(
        transaction,
        &record,
        normalized_title,
        normalized_platform,
        normalized_region,
        normalized_edition,
        asset_type,
        byte_len,
        explicit_target,
    )?;
    record_provenance(transaction, &imported, &record, candidate_identity)?;
    Ok(imported)
}

#[allow(clippy::too_many_arguments)]
fn persist_new_asset_in_transaction(
    transaction: &Transaction<'_>,
    record: &PersistAsset,
    normalized_title: String,
    normalized_platform: String,
    normalized_region: String,
    normalized_edition: String,
    asset_type: &str,
    byte_len: i64,
    explicit_target: Option<(i64, i64)>,
) -> Result<ImportedAsset, PortError> {
    let game_id = match explicit_target {
        Some((game_id, _)) => game_id,
        None => resolve_game_id(transaction, record, &normalized_title)?,
    };
    let release_edition_id = if let Some((_, release_edition_id)) = explicit_target {
        release_edition_id
    } else {
        transaction
            .execute(
                "INSERT INTO release_editions (
                    game_id, platform, normalized_platform, region, normalized_region,
                    edition_name, normalized_edition_name
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(game_id, normalized_platform, normalized_region, normalized_edition_name) DO NOTHING",
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
        transaction
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
            .map_err(sql_error)?
    };

    persist_new_asset_row(
        transaction,
        record,
        game_id,
        release_edition_id,
        asset_type,
        byte_len,
    )
}

fn persist_new_asset_row(
    transaction: &Transaction<'_>,
    record: &PersistAsset,
    game_id: i64,
    release_edition_id: i64,
    asset_type: &str,
    byte_len: i64,
) -> Result<ImportedAsset, PortError> {
    transaction
        .execute(
            "INSERT INTO assets (
                release_edition_id, asset_type, object_hash, byte_len, original_filename,
                media_type, width, height
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(release_edition_id, asset_type, object_hash) DO NOTHING",
            params![
                release_edition_id,
                asset_type,
                record.object_hash,
                byte_len,
                record.original_filename,
                record.media.media_type,
                record.media.width,
                record.media.height,
            ],
        )
        .map_err(sql_error)?;
    let asset_id: i64 = transaction
        .query_row(
            "SELECT id FROM assets
             WHERE release_edition_id = ?1
               AND asset_type = ?2
               AND object_hash = ?3",
            params![release_edition_id, asset_type, record.object_hash],
            |row| row.get(0),
        )
        .map_err(sql_error)?;

    Ok(ImportedAsset {
        game_id,
        release_edition_id,
        asset_id,
        object_hash: record.object_hash.clone(),
        byte_len: record.byte_len,
    })
}

fn resolve_game_id(
    transaction: &Transaction<'_>,
    record: &PersistAsset,
    normalized_title: &str,
) -> Result<i64, PortError> {
    let Some(game_id) = record.existing_game_id else {
        transaction
            .execute(
                "INSERT INTO games (title, normalized_title) VALUES (?1, ?2)",
                params![record.game_title, normalized_title],
            )
            .map_err(sql_error)?;
        return Ok(transaction.last_insert_rowid());
    };

    let exists: Option<i64> = transaction
        .query_row(
            "SELECT id FROM games WHERE id = ?1",
            params![game_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?;

    match exists {
        None => Err(PortError::new(format!("game #{game_id} does not exist"))),
        Some(_) => Ok(game_id),
    }
}

fn resolve_existing_release_target(
    transaction: &Transaction<'_>,
    record: &PersistAsset,
) -> Result<Option<(i64, i64)>, PortError> {
    let Some(release_edition_id) = record.existing_release_edition_id else {
        return Ok(None);
    };

    let game_id = transaction
        .query_row(
            "SELECT game_id FROM release_editions WHERE id = ?1",
            params![release_edition_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(sql_error)?
        .ok_or_else(|| {
            PortError::new(format!(
                "release edition #{release_edition_id} does not exist"
            ))
        })?;

    if let Some(existing_game_id) = record.existing_game_id
        && existing_game_id != game_id
    {
        return Err(PortError::new(format!(
            "release edition #{release_edition_id} does not belong to game #{existing_game_id}"
        )));
    }

    Ok(Some((game_id, release_edition_id)))
}

fn find_existing_import(
    transaction: &Transaction<'_>,
    record: &PersistAsset,
    lookup: &ExistingImportLookup<'_>,
) -> Result<Option<ExistingImportMatch>, PortError> {
    let mut statement = transaction
        .prepare(
            "SELECT g.id, r.id, a.id, p.source_location
             FROM asset_provenance p
             JOIN assets a ON a.id = p.asset_id
             JOIN release_editions r ON r.id = a.release_edition_id
             JOIN games g ON g.id = r.game_id
             WHERE p.source_kind = ?1
               AND a.asset_type = ?2
               AND a.object_hash = ?3
               AND a.byte_len = ?4
               AND g.normalized_title = ?5
               AND r.normalized_platform = ?6
               AND r.normalized_region = ?7
               AND r.normalized_edition_name = ?8
               AND (?9 IS NULL OR g.id = ?9)
               AND (?10 IS NULL OR r.id = ?10)",
        )
        .map_err(sql_error)?;
    let mut rows = statement
        .query(params![
            lookup.source_id,
            lookup.asset_type,
            record.object_hash,
            lookup.byte_len,
            lookup.normalized_title,
            lookup.normalized_platform,
            lookup.normalized_region,
            lookup.normalized_edition,
            record.existing_game_id,
            lookup.release_edition_id,
        ])
        .map_err(sql_error)?;

    while let Some(row) = rows.next().map_err(sql_error)? {
        let matched_source_location: String = row.get(3).map_err(sql_error)?;
        if !equivalent_source_location(&matched_source_location, &record.source_location) {
            continue;
        }

        return Ok(Some(ExistingImportMatch {
            imported: ImportedAsset {
                game_id: row.get(0).map_err(sql_error)?,
                release_edition_id: row.get(1).map_err(sql_error)?,
                asset_id: row.get(2).map_err(sql_error)?,
                object_hash: record.object_hash.clone(),
                byte_len: record.byte_len,
            },
        }));
    }

    Ok(None)
}

fn equivalent_source_location(stored: &str, current: &str) -> bool {
    if stored == current {
        return true;
    }

    canonicalize_location(stored)
        .zip(canonicalize_location(current))
        .is_some_and(|(stored, current)| stored == current)
}

fn canonicalize_location(location: &str) -> Option<PathBuf> {
    let path = Path::new(location);
    if !path.is_absolute() {
        return None;
    }
    fs::canonicalize(path).ok()
}

/// Records where the asset came from, for one candidate (or the direct import), with the
/// candidate's own label and match decision. Re-imports keep earlier details they do not carry.
fn record_provenance(
    transaction: &Transaction<'_>,
    imported: &ImportedAsset,
    record: &PersistAsset,
    candidate_identity: Option<&str>,
) -> Result<(), PortError> {
    let decision_json = record
        .match_decision
        .as_ref()
        .map(|decision| {
            if decision.release_edition_id != Some(imported.release_edition_id) {
                return Err(PortError::new(format!(
                    "asset match decision targets release {:?}, expected #{}",
                    decision.release_edition_id, imported.release_edition_id
                )));
            }
            serde_json::to_string(decision).map_err(|error| {
                PortError::new(format!("failed to serialize asset match decision: {error}"))
            })
        })
        .transpose()?;
    transaction
        .execute(
            "INSERT INTO asset_provenance (
                asset_id, source_kind, source_asset_label, source_location,
                candidate_identity, match_decision_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(asset_id, source_kind, source_location, candidate_identity)
             DO UPDATE SET
                source_asset_label = COALESCE(
                    excluded.source_asset_label,
                    asset_provenance.source_asset_label
                ),
                match_decision_json = COALESCE(
                    excluded.match_decision_json,
                    asset_provenance.match_decision_json
                )",
            params![
                imported.asset_id,
                record.source_id.as_str(),
                record.source_asset_label,
                record.source_location,
                candidate_identity.unwrap_or(""),
                decision_json,
            ],
        )
        .map_err(sql_error)?;
    Ok(())
}

pub(super) fn asset_type_to_str(asset_type: AssetType) -> &'static str {
    match asset_type {
        AssetType::BoxFront => "box_front",
        AssetType::Screenshot => "screenshot",
        AssetType::TitleScreen => "title_screen",
    }
}

pub(super) fn parse_asset_type(value: &str) -> Result<AssetType, PortError> {
    match value {
        "box_front" => Ok(AssetType::BoxFront),
        "screenshot" => Ok(AssetType::Screenshot),
        "title_screen" => Ok(AssetType::TitleScreen),
        other => Err(PortError::new(format!(
            "unknown asset type in catalog: {other}"
        ))),
    }
}

/// Removes the candidate's provenance, except on `keep_release_edition_id`. Assets left without
/// provenance leave the library while their original objects stay in the store until vault
/// verification collects them.
pub(super) fn detach_candidate_links(
    transaction: &Transaction<'_>,
    candidate_identity: &str,
    keep_release_edition_id: Option<i64>,
) -> Result<(), PortError> {
    let detached_assets = transaction
        .prepare(
            "SELECT DISTINCT provenance.asset_id
             FROM asset_provenance AS provenance
             JOIN assets AS asset ON asset.id = provenance.asset_id
             WHERE provenance.candidate_identity = ?1
               AND (?2 IS NULL OR asset.release_edition_id != ?2)",
        )
        .map_err(sql_error)?
        .query_map(
            params![candidate_identity, keep_release_edition_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    for asset_id in detached_assets {
        transaction
            .execute(
                "DELETE FROM asset_provenance WHERE asset_id = ?1 AND candidate_identity = ?2",
                params![asset_id, candidate_identity],
            )
            .map_err(sql_error)?;
        transaction
            .execute(
                "DELETE FROM assets WHERE id = ?1
                 AND NOT EXISTS (SELECT 1 FROM asset_provenance WHERE asset_id = ?1)",
                params![asset_id],
            )
            .map_err(sql_error)?;
    }
    Ok(())
}
