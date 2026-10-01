use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

use game_media_vault_application::{CatalogPort, PortError, ReferenceCatalogRepositoryPort};
use game_media_vault_domain::{
    AssetCandidateMatch, AssetProvenance, AssetType, ImportedAsset, ImportedReleaseEdition,
    LibraryAsset, LibraryEntry, PersistAsset, ReferenceReleaseRecord, ReleaseAssertion,
    ReleaseAssertionField, SourceId,
};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};

mod reviews;
mod runs;
mod schema;

pub struct SqliteCatalog {
    path: PathBuf,
    mode: CatalogOpenMode,
    #[cfg(test)]
    busy_handler: Option<fn(i32) -> bool>,
}

#[derive(Clone, Copy)]
enum CatalogOpenMode {
    ExistingOnly,
}

impl SqliteCatalog {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, PortError> {
        let path = path.into();
        if path.exists() {
            return Self::open_existing(path);
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        initialize_new_catalog(&path, schema::create)?;
        Ok(Self {
            path,
            mode: CatalogOpenMode::ExistingOnly,
            #[cfg(test)]
            busy_handler: None,
        })
    }

    pub fn open_existing(path: impl Into<PathBuf>) -> Result<Self, PortError> {
        let path = path.into();
        if !path.is_file() {
            return Err(PortError(format!(
                "catalog does not exist: {}",
                path.display()
            )));
        }

        let catalog = Self {
            path,
            mode: CatalogOpenMode::ExistingOnly,
            #[cfg(test)]
            busy_handler: None,
        };
        let mut connection = catalog.connect()?;
        schema::open(&mut connection, &catalog.path)?;

        Ok(catalog)
    }

    fn connect(&self) -> Result<Connection, PortError> {
        let flags = match self.mode {
            CatalogOpenMode::ExistingOnly => OpenFlags::SQLITE_OPEN_READ_WRITE,
        };
        let connection = Connection::open_with_flags(&self.path, flags).map_err(sql_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .map_err(sql_error)?;
        #[cfg(test)]
        if let Some(handler) = self.busy_handler {
            connection.busy_handler(Some(handler)).map_err(sql_error)?;
        }
        Ok(connection)
    }
}

fn initialize_new_catalog(
    path: &Path,
    initialize: impl FnOnce(&mut Connection) -> Result<(), PortError>,
) -> Result<(), PortError> {
    let reservation = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_error)?;
    drop(reservation);

    let initialization = (|| -> Result<(), PortError> {
        let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(sql_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .map_err(sql_error)?;
        initialize(&mut connection)
    })();

    match initialization {
        Ok(()) => Ok(()),
        Err(error) => match fs::remove_file(path) {
            Ok(()) => Err(error),
            Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => {
                Err(error)
            }
            Err(cleanup_error) => Err(PortError(format!(
                "{error}; failed to remove incomplete catalog {}: {cleanup_error}",
                path.display()
            ))),
        },
    }
}

impl ReferenceCatalogRepositoryPort for SqliteCatalog {
    fn persist_reference_release(
        &self,
        record: ReferenceReleaseRecord,
    ) -> Result<ImportedReleaseEdition, PortError> {
        let mut imported = self.persist_reference_releases(vec![record])?;
        imported
            .pop()
            .ok_or_else(|| PortError("reference persistence returned no release".into()))
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

impl CatalogPort for SqliteCatalog {
    fn persist_asset(&self, record: PersistAsset) -> Result<ImportedAsset, PortError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let imported = persist_asset_in_transaction(&transaction, record)?;
        transaction.commit().map_err(sql_error)?;
        Ok(imported)
    }

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError> {
        let connection = self.connect()?;
        let mut entries: Vec<LibraryEntry> = Vec::new();

        {
            let mut statement = connection
                .prepare(
                    "SELECT
                        g.id, g.title,
                        r.id, r.platform, r.region, r.edition_name,
                        a.id, a.asset_type, a.object_hash, a.byte_len, a.original_filename,
                        p.source_kind, p.source_asset_label, p.source_location,
                        m.decision_json
                     FROM release_editions r
                     JOIN games g ON g.id = r.game_id
                     LEFT JOIN assets a ON a.release_edition_id = r.id
                     LEFT JOIN asset_provenance p ON p.asset_id = a.id
                     LEFT JOIN asset_match_decisions m
                       ON m.asset_id = p.asset_id
                      AND m.source_kind = p.source_kind
                      AND m.source_location = p.source_location
                     ORDER BY g.title, r.id, a.id, p.id",
                )
                .map_err(sql_error)?;
            let mut rows = statement.query([]).map_err(sql_error)?;

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
                        serde_json::from_str::<AssetCandidateMatch>(&decision_json).map_err(
                            |error| {
                                PortError(format!(
                                    "catalog contains invalid asset match decision: {error}"
                                ))
                            },
                        )
                    })
                    .transpose()?;
                let entry = entries
                    .last_mut()
                    .ok_or_else(|| PortError("library release aggregation failed".into()))?;

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
                    .ok_or_else(|| PortError("catalog asset is missing its type".into()))?;
                let object_hash = row
                    .get::<_, Option<String>>(8)
                    .map_err(sql_error)?
                    .ok_or_else(|| PortError("catalog asset is missing its object hash".into()))?;
                let byte_len = row
                    .get::<_, Option<i64>>(9)
                    .map_err(sql_error)?
                    .ok_or_else(|| PortError("catalog asset is missing its byte length".into()))?;
                let original_filename = row
                    .get::<_, Option<String>>(10)
                    .map_err(sql_error)?
                    .ok_or_else(|| {
                        PortError("catalog asset is missing its original filename".into())
                    })?;
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
                    byte_len: u64::try_from(byte_len)
                        .map_err(|_| PortError("catalog contains a negative byte length".into()))?,
                    original_filename,
                    provenance,
                });
            }
        }

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

        Ok(entries)
    }
}

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

fn persist_asset_in_transaction(
    transaction: &Transaction<'_>,
    record: PersistAsset,
) -> Result<ImportedAsset, PortError> {
    let normalized_title = normalize(&record.game_title);
    let normalized_platform = normalize(&record.platform);
    let normalized_region = normalize(&record.region);
    let normalized_edition = normalize(&record.edition_name);
    let asset_type = asset_type_to_str(record.asset_type);
    let source_id = record.source_id.as_str();
    let byte_len = i64::try_from(record.byte_len)
        .map_err(|_| PortError("asset byte length exceeds SQLite INTEGER range".into()))?;
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
        normalize_existing_provenance(transaction, &record, source_id, &existing)?;
        persist_asset_match_decision(
            transaction,
            existing.imported.asset_id,
            existing.imported.release_edition_id,
            source_id,
            &record.source_location,
            record.match_decision.as_ref(),
        )?;
        return Ok(existing.imported);
    }

    persist_new_asset_in_transaction(
        transaction,
        record,
        normalized_title,
        normalized_platform,
        normalized_region,
        normalized_edition,
        asset_type,
        byte_len,
        explicit_target,
    )
}

#[allow(clippy::too_many_arguments)]
fn persist_new_asset_in_transaction(
    transaction: &Transaction<'_>,
    record: PersistAsset,
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
        None => resolve_game_id(transaction, &record, &normalized_title)?,
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
    record: PersistAsset,
    game_id: i64,
    release_edition_id: i64,
    asset_type: &str,
    byte_len: i64,
) -> Result<ImportedAsset, PortError> {
    transaction
        .execute(
            "INSERT INTO assets (
                release_edition_id, asset_type, object_hash, byte_len, original_filename
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(release_edition_id, asset_type, object_hash) DO NOTHING",
            params![
                release_edition_id,
                asset_type,
                record.object_hash,
                byte_len,
                record.original_filename,
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

    let source_id = record.source_id.as_str();
    transaction
        .execute(
            "INSERT INTO asset_provenance (
                asset_id, source_kind, source_asset_label, source_location
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(asset_id, source_kind, source_location) DO UPDATE SET
                source_asset_label = COALESCE(
                    excluded.source_asset_label,
                    asset_provenance.source_asset_label
                )",
            params![
                asset_id,
                source_id,
                record.source_asset_label,
                record.source_location,
            ],
        )
        .map_err(sql_error)?;
    persist_asset_match_decision(
        transaction,
        asset_id,
        release_edition_id,
        source_id,
        &record.source_location,
        record.match_decision.as_ref(),
    )?;

    Ok(ImportedAsset {
        game_id,
        release_edition_id,
        asset_id,
        object_hash: record.object_hash,
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
        None => Err(PortError(format!("game #{game_id} does not exist"))),
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
            PortError(format!(
                "release edition #{release_edition_id} does not exist"
            ))
        })?;

    if let Some(existing_game_id) = record.existing_game_id
        && existing_game_id != game_id
    {
        return Err(PortError(format!(
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

fn normalize_existing_provenance(
    transaction: &Transaction<'_>,
    record: &PersistAsset,
    source_id: &str,
    existing: &ExistingImportMatch,
) -> Result<(), PortError> {
    transaction
        .execute(
            "INSERT INTO asset_provenance (
                asset_id, source_kind, source_asset_label, source_location
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(asset_id, source_kind, source_location) DO UPDATE SET
                source_asset_label = COALESCE(
                    excluded.source_asset_label,
                    asset_provenance.source_asset_label
                )",
            params![
                existing.imported.asset_id,
                source_id,
                record.source_asset_label,
                record.source_location,
            ],
        )
        .map_err(sql_error)?;
    Ok(())
}

fn persist_asset_match_decision(
    transaction: &Transaction<'_>,
    asset_id: i64,
    release_edition_id: i64,
    source_id: &str,
    source_location: &str,
    match_decision: Option<&AssetCandidateMatch>,
) -> Result<(), PortError> {
    let Some(match_decision) = match_decision else {
        return Ok(());
    };
    if match_decision.release_edition_id != Some(release_edition_id) {
        return Err(PortError(format!(
            "asset match decision targets release {:?}, expected #{release_edition_id}",
            match_decision.release_edition_id
        )));
    }
    let decision_json = serde_json::to_string(match_decision)
        .map_err(|error| PortError(format!("failed to serialize asset match decision: {error}")))?;
    transaction
        .execute(
            "INSERT INTO asset_match_decisions (
                asset_id, source_kind, source_location, decision_json
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(asset_id, source_kind, source_location)
             DO UPDATE SET decision_json = excluded.decision_json",
            params![asset_id, source_id, source_location, decision_json],
        )
        .map_err(sql_error)?;
    Ok(())
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
            PortError("reference release is missing its source_record identifier assertion".into())
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
        return Ok(ImportedReleaseEdition {
            game_id,
            release_edition_id,
        });
    }

    let game_id = match transaction
        .query_row(
            "SELECT r.game_id
             FROM release_assertions a
             JOIN release_editions r ON r.id = a.release_edition_id
             WHERE a.source_id = ?1
               AND a.field = 'title'
               AND a.normalized_value = ?2
               AND r.normalized_platform = ?3
             ORDER BY r.id
             LIMIT 1",
            params![source_id, normalized_title, normalized_platform],
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

fn persist_release_assertions(
    transaction: &Transaction<'_>,
    release_edition_id: i64,
    assertions: &[ReleaseAssertion],
) -> Result<(), PortError> {
    for assertion in assertions {
        if assertion.source_id.as_str().trim().is_empty() || assertion.value.trim().is_empty() {
            return Err(PortError(
                "release assertions require non-blank source ids and values".into(),
            ));
        }
        let qualifier = assertion.qualifier.as_deref().unwrap_or("");
        transaction
            .execute(
                "INSERT OR IGNORE INTO release_assertions (
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

/// Records that an acquisition candidate supports a provenance row. Distinct candidates can
/// share one row when they point at the same locator and bytes.
fn tag_candidate_provenance(
    transaction: &Transaction<'_>,
    imported: &ImportedAsset,
    record_source_id: &str,
    record_source_location: &str,
    candidate_identity: &str,
) -> Result<(), PortError> {
    transaction
        .execute(
            "INSERT OR IGNORE INTO asset_provenance_candidates (provenance_id, candidate_identity)
             SELECT id, ?1 FROM asset_provenance
             WHERE asset_id = ?2 AND source_kind = ?3 AND source_location = ?4",
            params![
                candidate_identity,
                imported.asset_id,
                record_source_id,
                record_source_location
            ],
        )
        .map_err(sql_error)?;
    Ok(())
}

/// Removes the candidate's acquisition links, except to `keep_release_edition_id`. Provenance
/// still supported by another candidate stays; assets left without provenance leave the
/// library while their original objects stay in the store until vault verification collects
/// them.
fn detach_candidate_links(
    transaction: &Transaction<'_>,
    candidate_identity: &str,
    keep_release_edition_id: Option<i64>,
) -> Result<(), PortError> {
    let detached = transaction
        .prepare(
            "SELECT link.provenance_id, provenance.asset_id
             FROM asset_provenance_candidates AS link
             JOIN asset_provenance AS provenance ON provenance.id = link.provenance_id
             JOIN assets AS asset ON asset.id = provenance.asset_id
             WHERE link.candidate_identity = ?1
               AND (?2 IS NULL OR asset.release_edition_id != ?2)",
        )
        .map_err(sql_error)?
        .query_map(
            params![candidate_identity, keep_release_edition_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    for (provenance_id, asset_id) in detached {
        transaction
            .execute(
                "DELETE FROM asset_provenance_candidates
                 WHERE provenance_id = ?1 AND candidate_identity = ?2",
                params![provenance_id, candidate_identity],
            )
            .map_err(sql_error)?;
        let still_supported: bool = transaction
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM asset_provenance_candidates WHERE provenance_id = ?1
                 )",
                params![provenance_id],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if still_supported {
            continue;
        }
        transaction
            .execute(
                "DELETE FROM asset_match_decisions
                 WHERE (asset_id, source_kind, source_location) IN (
                     SELECT asset_id, source_kind, source_location
                     FROM asset_provenance WHERE id = ?1
                 )",
                params![provenance_id],
            )
            .map_err(sql_error)?;
        transaction
            .execute(
                "DELETE FROM asset_provenance WHERE id = ?1",
                params![provenance_id],
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

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

fn assertion_field_to_str(field: ReleaseAssertionField) -> &'static str {
    match field {
        ReleaseAssertionField::Title => "title",
        ReleaseAssertionField::Region => "region",
        ReleaseAssertionField::Revision => "revision",
        ReleaseAssertionField::Identifier => "identifier",
    }
}

fn parse_release_assertion_field(value: &str) -> Result<ReleaseAssertionField, PortError> {
    match value {
        "title" => Ok(ReleaseAssertionField::Title),
        "region" => Ok(ReleaseAssertionField::Region),
        "revision" => Ok(ReleaseAssertionField::Revision),
        "identifier" => Ok(ReleaseAssertionField::Identifier),
        other => Err(PortError(format!(
            "unknown release assertion field in catalog: {other}"
        ))),
    }
}

fn asset_type_to_str(asset_type: AssetType) -> &'static str {
    match asset_type {
        AssetType::BoxFront => "box_front",
    }
}

fn parse_asset_type(value: &str) -> Result<AssetType, PortError> {
    match value {
        "box_front" => Ok(AssetType::BoxFront),
        other => Err(PortError(format!("unknown asset type in catalog: {other}"))),
    }
}

fn io_error(error: std::io::Error) -> PortError {
    PortError(error.to_string())
}

fn sql_error(error: rusqlite::Error) -> PortError {
    PortError(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        sync::{Condvar, Mutex, OnceLock},
        thread,
        time::Duration,
    };

    use game_media_vault_application::CatalogPort;
    use game_media_vault_domain::{AssetType, PersistAsset, SourceId};
    use tempfile::tempdir;

    use super::*;

    static BUSY_THREADS: OnceLock<(Mutex<HashSet<thread::ThreadId>>, Condvar)> = OnceLock::new();

    fn record_busy_thread(_: i32) -> bool {
        let (threads, ready) =
            BUSY_THREADS.get_or_init(|| (Mutex::new(HashSet::new()), Condvar::new()));
        threads.lock().unwrap().insert(thread::current().id());
        ready.notify_all();
        true
    }

    #[test]
    fn failed_new_catalog_initialization_removes_the_reserved_file() {
        let temp = tempdir().unwrap();
        let catalog_path = temp.path().join("catalog.sqlite3");

        let error = initialize_new_catalog(&catalog_path, |_| {
            Err(PortError("forced initialization failure".to_owned()))
        })
        .unwrap_err();

        assert_eq!(error.to_string(), "forced initialization failure");
        assert!(!catalog_path.exists());
    }

    #[test]
    fn duplicate_lookup_is_serialized_with_concurrent_writes() {
        let temp = tempdir().unwrap();
        let catalog_path = temp.path().join("catalog.sqlite3");
        SqliteCatalog::open(&catalog_path).unwrap();
        let blocker = Connection::open(&catalog_path).unwrap();
        blocker
            .execute_batch("PRAGMA busy_timeout = 5000; BEGIN IMMEDIATE;")
            .unwrap();

        let (busy_threads, ready) =
            BUSY_THREADS.get_or_init(|| (Mutex::new(HashSet::new()), Condvar::new()));
        busy_threads.lock().unwrap().clear();

        let workers: Vec<_> = (0..2)
            .map(|_| {
                let catalog_path = catalog_path.clone();
                thread::spawn(move || {
                    let catalog = SqliteCatalog {
                        path: catalog_path,
                        mode: CatalogOpenMode::ExistingOnly,
                        busy_handler: Some(record_busy_thread),
                    };
                    catalog
                        .persist_asset(PersistAsset {
                            existing_game_id: None,
                            existing_release_edition_id: None,
                            match_decision: None,
                            game_title: "Concurrent Game".to_owned(),
                            platform: "Windows".to_owned(),
                            region: "Worldwide".to_owned(),
                            edition_name: "Standard".to_owned(),
                            asset_type: AssetType::BoxFront,
                            object_hash: "shared-object-hash".to_owned(),
                            byte_len: 42,
                            original_filename: "front.png".to_owned(),
                            source_id: SourceId::from("local_import"),
                            source_asset_label: None,
                            source_location: "C:/collection/front.png".to_owned(),
                        })
                        .unwrap()
                })
            })
            .collect();

        let observed = busy_threads.lock().unwrap();
        let (observed, timeout) = ready
            .wait_timeout_while(observed, Duration::from_secs(5), |threads| {
                threads.len() < 2
            })
            .unwrap();
        assert!(
            !timeout.timed_out(),
            "both workers should contend on the SQLite write lock"
        );
        assert_eq!(observed.len(), 2);
        drop(observed);

        blocker.execute_batch("COMMIT;").unwrap();

        let imported: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(imported[0].asset_id, imported[1].asset_id);

        let catalog = SqliteCatalog::open_existing(&catalog_path).unwrap();
        assert_eq!(catalog.list_library().unwrap().len(), 1);
    }
}
