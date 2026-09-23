use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

use game_media_vault_application::{
    CatalogPort, PortError, ReferenceCatalogRepositoryPort, RunRepositoryPort,
};
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionRequestDraft, AcquisitionRun, AcquisitionRunStatus,
    AcquisitionWorkItem, AssetProvenance, AssetType, ImportedAsset, ImportedReleaseEdition,
    LibraryAsset, LibraryEntry, PersistAsset, ReferenceReleaseRecord, ReleaseAssertion,
    ReleaseAssertionField, SourceId,
};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};

const ACQUISITION_REQUEST_SCHEMA_VERSION: i64 = 1;

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
        initialize_new_catalog(&path, initialize_schema)?;
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
        let connection = catalog.connect()?;
        if !is_recognized_catalog_schema(&connection)? {
            return Err(PortError(format!(
                "catalog schema is missing or incomplete: {}",
                catalog.path.display()
            )));
        }
        initialize_schema(&connection)?;
        let current_catalog_tables = [
            "games",
            "release_editions",
            "release_assertions",
            "assets",
            "asset_provenance",
            "acquisition_runs",
            "acquisition_run_work",
        ];
        if !required_tables_exist(&connection, &current_catalog_tables)? {
            return Err(PortError(format!(
                "catalog schema is missing or incomplete: {}",
                catalog.path.display()
            )));
        }

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
    initialize: impl FnOnce(&Connection) -> Result<(), PortError>,
) -> Result<(), PortError> {
    let reservation = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_error)?;
    drop(reservation);

    let initialization = (|| -> Result<(), PortError> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(sql_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .map_err(sql_error)?;
        initialize(&connection)
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

impl RunRepositoryPort for SqliteCatalog {
    fn create_run(&self, request: AcquisitionRequest) -> Result<AcquisitionRun, PortError> {
        let connection = self.connect()?;
        let request_json = serde_json::to_string(&request).map_err(|error| {
            PortError(format!("failed to serialize acquisition request: {error}"))
        })?;
        connection
            .execute(
                "INSERT INTO acquisition_runs (
                    request_json, request_schema_version, status, queued_work, completed_work
                 ) VALUES (?1, ?2, 'running', 0, 0)",
                params![request_json, ACQUISITION_REQUEST_SCHEMA_VERSION],
            )
            .map_err(sql_error)?;

        Ok(AcquisitionRun {
            id: connection.last_insert_rowid(),
            request,
            status: AcquisitionRunStatus::Running,
            queued_work: 0,
            completed_work: 0,
        })
    }

    fn get_run(&self, run_id: i64) -> Result<Option<AcquisitionRun>, PortError> {
        let connection = self.connect()?;
        let row = connection
            .query_row(
                "SELECT request_json, request_schema_version, status, queued_work, completed_work
                 FROM acquisition_runs WHERE id = ?1",
                params![run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(sql_error)?;

        let Some((request_json, request_schema_version, status, queued_work, completed_work)) = row
        else {
            return Ok(None);
        };
        if request_schema_version != ACQUISITION_REQUEST_SCHEMA_VERSION {
            return Err(PortError(format!(
                "unsupported acquisition request schema version: {request_schema_version}"
            )));
        }
        let draft: AcquisitionRequestDraft =
            serde_json::from_str(&request_json).map_err(|error| {
                PortError(format!("invalid persisted acquisition request: {error}"))
            })?;
        let request = AcquisitionRequest::try_from_draft(draft).map_err(|error| {
            PortError(format!("invalid persisted acquisition request: {error}"))
        })?;

        Ok(Some(AcquisitionRun {
            id: run_id,
            request,
            status: parse_run_status(&status)?,
            queued_work: u64::try_from(queued_work)
                .map_err(|_| PortError("catalog contains a negative queued work count".into()))?,
            completed_work: u64::try_from(completed_work).map_err(|_| {
                PortError("catalog contains a negative completed work count".into())
            })?,
        }))
    }

    fn list_runs(&self) -> Result<Vec<AcquisitionRun>, PortError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare("SELECT id FROM acquisition_runs ORDER BY id")
            .map_err(sql_error)?;
        let run_ids = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)?;

        run_ids
            .into_iter()
            .map(|run_id| {
                self.get_run(run_id)?.ok_or_else(|| {
                    PortError(format!(
                        "acquisition run #{run_id} disappeared while listing"
                    ))
                })
            })
            .collect()
    }

    fn queue_work(&self, run_id: i64, work_key: String) -> Result<(), PortError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let status: Option<String> = transaction
            .query_row(
                "SELECT status FROM acquisition_runs WHERE id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let Some(status) = status else {
            return Err(PortError(format!(
                "acquisition run #{run_id} does not exist"
            )));
        };
        if !matches!(status.as_str(), "running" | "paused") {
            return Err(PortError(format!(
                "acquisition run #{run_id} cannot accept work while {status}"
            )));
        }
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO acquisition_run_work (run_id, work_key, completed)
                 VALUES (?1, ?2, 0)",
                params![run_id, work_key],
            )
            .map_err(sql_error)?;
        if inserted == 1 {
            transaction
                .execute(
                    "UPDATE acquisition_runs SET queued_work = queued_work + 1 WHERE id = ?1",
                    params![run_id],
                )
                .map_err(sql_error)?;
        }
        transaction.commit().map_err(sql_error)
    }

    fn next_queued_work(&self, run_id: i64) -> Result<Option<AcquisitionWorkItem>, PortError> {
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT work.work_key
                 FROM acquisition_run_work AS work
                 INNER JOIN acquisition_runs AS run ON run.id = work.run_id
                 WHERE work.run_id = ?1
                   AND work.completed = 0
                   AND run.status = 'running'
                 ORDER BY work.id LIMIT 1",
                params![run_id],
                |row| Ok(AcquisitionWorkItem { key: row.get(0)? }),
            )
            .optional()
            .map_err(sql_error)
    }

    fn complete_work(&self, run_id: i64, work_key: &str) -> Result<(), PortError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let completed = transaction
            .execute(
                "UPDATE acquisition_run_work SET completed = 1
                 WHERE run_id = ?1 AND work_key = ?2 AND completed = 0",
                params![run_id, work_key],
            )
            .map_err(sql_error)?;
        if completed == 1 {
            transaction
                .execute(
                    "UPDATE acquisition_runs
                     SET queued_work = queued_work - 1,
                         completed_work = completed_work + 1
                     WHERE id = ?1",
                    params![run_id],
                )
                .map_err(sql_error)?;
        }
        transaction.commit().map_err(sql_error)
    }

    fn compare_and_set_run_status(
        &self,
        run_id: i64,
        expected: AcquisitionRunStatus,
        target: AcquisitionRunStatus,
    ) -> Result<bool, PortError> {
        let connection = self.connect()?;
        let target_status = run_status_to_str(target);
        let updated = connection
            .execute(
                "UPDATE acquisition_runs
                 SET status = ?1
                 WHERE id = ?2
                   AND status = ?3
                   AND (?1 != 'completed' OR queued_work = 0)",
                params![target_status, run_id, run_status_to_str(expected)],
            )
            .map_err(sql_error)?;
        Ok(updated == 1)
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
        let normalized_title = normalize(&record.game_title);
        let normalized_platform = normalize(&record.platform);
        let normalized_region = normalize(&record.region);
        let normalized_edition = normalize(&record.edition_name);
        let asset_type = asset_type_to_str(record.asset_type);
        let source_id = record.source_id.as_str();
        let byte_len = i64::try_from(record.byte_len)
            .map_err(|_| PortError("asset byte length exceeds SQLite INTEGER range".into()))?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let explicit_target = resolve_existing_release_target(&transaction, &record)?;
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
        if let Some(existing) = find_existing_import(&transaction, &record, &lookup)? {
            normalize_existing_provenance(&transaction, &record, source_id, &existing)?;
            transaction.commit().map_err(sql_error)?;
            return Ok(existing.imported);
        }

        let game_id = match explicit_target {
            Some((game_id, _)) => game_id,
            None => resolve_game_id(&transaction, &record, &normalized_title)?,
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
        transaction.commit().map_err(sql_error)?;

        Ok(ImportedAsset {
            game_id,
            release_edition_id,
            asset_id,
            object_hash: record.object_hash,
            byte_len: record.byte_len,
        })
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
                        p.source_kind, p.source_asset_label, p.source_location
                     FROM release_editions r
                     JOIN games g ON g.id = r.game_id
                     LEFT JOIN assets a ON a.release_edition_id = r.id
                     LEFT JOIN asset_provenance p ON p.asset_id = a.id
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

fn initialize_schema(connection: &Connection) -> Result<(), PortError> {
    migrate_legacy_game_identity(connection)?;
    connection
        .execute_batch("BEGIN IMMEDIATE;")
        .map_err(sql_error)?;
    let migration = (|| -> Result<(), PortError> {
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS games (
                id INTEGER PRIMARY KEY,
                title TEXT NOT NULL,
                normalized_title TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS release_editions (
                id INTEGER PRIMARY KEY,
                game_id INTEGER NOT NULL REFERENCES games(id),
                platform TEXT NOT NULL,
                normalized_platform TEXT NOT NULL,
                region TEXT NOT NULL,
                normalized_region TEXT NOT NULL,
                edition_name TEXT NOT NULL,
                normalized_edition_name TEXT NOT NULL,
                UNIQUE(game_id, normalized_platform, normalized_region, normalized_edition_name)
            );
            CREATE TABLE IF NOT EXISTS release_assertions (
                id INTEGER PRIMARY KEY,
                release_edition_id INTEGER NOT NULL REFERENCES release_editions(id),
                source_id TEXT NOT NULL,
                source_location TEXT NOT NULL,
                field TEXT NOT NULL,
                qualifier TEXT NOT NULL,
                value TEXT NOT NULL,
                normalized_value TEXT NOT NULL,
                UNIQUE(
                    release_edition_id,
                    source_id,
                    source_location,
                    field,
                    qualifier,
                    value
                )
            );
            CREATE TABLE IF NOT EXISTS assets (
                id INTEGER PRIMARY KEY,
                release_edition_id INTEGER NOT NULL REFERENCES release_editions(id),
                asset_type TEXT NOT NULL,
                object_hash TEXT NOT NULL,
                byte_len INTEGER NOT NULL CHECK(byte_len >= 0),
                original_filename TEXT NOT NULL,
                UNIQUE(release_edition_id, asset_type, object_hash)
            );
            CREATE TABLE IF NOT EXISTS asset_provenance (
                id INTEGER PRIMARY KEY,
                asset_id INTEGER NOT NULL REFERENCES assets(id),
                source_kind TEXT NOT NULL,
                source_location TEXT NOT NULL,
                source_asset_label TEXT,
                UNIQUE(asset_id, source_kind, source_location)
            );
            CREATE TABLE IF NOT EXISTS acquisition_runs (
                id INTEGER PRIMARY KEY,
                request_json TEXT NOT NULL,
                request_schema_version INTEGER NOT NULL DEFAULT 1
                    CHECK(request_schema_version > 0),
                status TEXT NOT NULL,
                queued_work INTEGER NOT NULL CHECK(queued_work >= 0),
                completed_work INTEGER NOT NULL CHECK(completed_work >= 0)
            );
            CREATE TABLE IF NOT EXISTS acquisition_run_work (
                id INTEGER PRIMARY KEY,
                run_id INTEGER NOT NULL REFERENCES acquisition_runs(id),
                work_key TEXT NOT NULL,
                completed INTEGER NOT NULL DEFAULT 0 CHECK(completed IN (0, 1)),
                UNIQUE(run_id, work_key)
            );
            CREATE INDEX IF NOT EXISTS idx_release_game ON release_editions(game_id);
            CREATE INDEX IF NOT EXISTS idx_release_assertion_release
                ON release_assertions(release_edition_id);
            CREATE INDEX IF NOT EXISTS idx_release_assertion_title
                ON release_assertions(source_id, field, normalized_value);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_release_assertion_source_record
                ON release_assertions(source_id, qualifier, value)
                WHERE field = 'identifier' AND qualifier = 'source_record';
            CREATE INDEX IF NOT EXISTS idx_game_normalized_title ON games(normalized_title);
            CREATE INDEX IF NOT EXISTS idx_asset_release ON assets(release_edition_id);
            CREATE INDEX IF NOT EXISTS idx_provenance_asset ON asset_provenance(asset_id);
            CREATE INDEX IF NOT EXISTS idx_run_work_pending
                ON acquisition_run_work(run_id, completed, id);",
            )
            .map_err(sql_error)?;

        let version_column_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('acquisition_runs')
                 WHERE name = 'request_schema_version'",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if version_column_count == 0 {
            connection
                .execute_batch(
                    "ALTER TABLE acquisition_runs
                 ADD COLUMN request_schema_version INTEGER NOT NULL DEFAULT 1
                 CHECK(request_schema_version > 0);",
                )
                .map_err(sql_error)?;
        }
        let source_asset_label_column_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('asset_provenance')
                 WHERE name = 'source_asset_label'",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if source_asset_label_column_count == 0 {
            connection
                .execute_batch(
                    "ALTER TABLE asset_provenance
                     ADD COLUMN source_asset_label TEXT;",
                )
                .map_err(sql_error)?;
        }
        Ok(())
    })();

    match migration {
        Ok(()) => connection.execute_batch("COMMIT;").map_err(sql_error),
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK;");
            Err(error)
        }
    }
}

fn required_tables_exist(connection: &Connection, table_names: &[&str]) -> Result<bool, PortError> {
    for table_name in table_names {
        let exists: i64 = connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
                 )",
                params![table_name],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if exists != 1 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn is_recognized_catalog_schema(connection: &Connection) -> Result<bool, PortError> {
    if !table_matches_columns(
        connection,
        "games",
        &[
            ("id", "INTEGER", false, true),
            ("title", "TEXT", true, false),
            ("normalized_title", "TEXT", true, false),
        ],
    )? || !table_matches_columns(
        connection,
        "release_editions",
        &[
            ("id", "INTEGER", false, true),
            ("game_id", "INTEGER", true, false),
            ("platform", "TEXT", true, false),
            ("normalized_platform", "TEXT", true, false),
            ("region", "TEXT", true, false),
            ("normalized_region", "TEXT", true, false),
            ("edition_name", "TEXT", true, false),
            ("normalized_edition_name", "TEXT", true, false),
        ],
    )? || !has_foreign_key(connection, "release_editions", "game_id", "games", "id")?
        || !has_unique_index(
            connection,
            "release_editions",
            &[
                "game_id",
                "normalized_platform",
                "normalized_region",
                "normalized_edition_name",
            ],
        )?
    {
        return Ok(false);
    }

    if table_exists(connection, "release_assertions")?
        && (!table_matches_columns(
            connection,
            "release_assertions",
            &[
                ("id", "INTEGER", false, true),
                ("release_edition_id", "INTEGER", true, false),
                ("source_id", "TEXT", true, false),
                ("source_location", "TEXT", true, false),
                ("field", "TEXT", true, false),
                ("qualifier", "TEXT", true, false),
                ("value", "TEXT", true, false),
                ("normalized_value", "TEXT", true, false),
            ],
        )? || !has_foreign_key(
            connection,
            "release_assertions",
            "release_edition_id",
            "release_editions",
            "id",
        )? || !has_unique_index(
            connection,
            "release_assertions",
            &[
                "release_edition_id",
                "source_id",
                "source_location",
                "field",
                "qualifier",
                "value",
            ],
        )?)
    {
        return Ok(false);
    }

    let has_assets = table_exists(connection, "assets")?;
    let has_provenance = table_exists(connection, "asset_provenance")?;
    if has_assets != has_provenance {
        return Ok(false);
    }
    let legacy_provenance = has_provenance
        && table_matches_columns(
            connection,
            "asset_provenance",
            &[
                ("id", "INTEGER", false, true),
                ("asset_id", "INTEGER", true, false),
                ("source_kind", "TEXT", true, false),
                ("source_location", "TEXT", true, false),
            ],
        )?;
    let labeled_provenance = has_provenance
        && table_matches_columns(
            connection,
            "asset_provenance",
            &[
                ("id", "INTEGER", false, true),
                ("asset_id", "INTEGER", true, false),
                ("source_kind", "TEXT", true, false),
                ("source_location", "TEXT", true, false),
                ("source_asset_label", "TEXT", false, false),
            ],
        )?;
    if has_assets
        && (!table_matches_columns(
            connection,
            "assets",
            &[
                ("id", "INTEGER", false, true),
                ("release_edition_id", "INTEGER", true, false),
                ("asset_type", "TEXT", true, false),
                ("object_hash", "TEXT", true, false),
                ("byte_len", "INTEGER", true, false),
                ("original_filename", "TEXT", true, false),
            ],
        )? || !has_foreign_key(
            connection,
            "assets",
            "release_edition_id",
            "release_editions",
            "id",
        )? || !has_unique_index(
            connection,
            "assets",
            &["release_edition_id", "asset_type", "object_hash"],
        )? || (!legacy_provenance && !labeled_provenance)
            || !has_foreign_key(connection, "asset_provenance", "asset_id", "assets", "id")?
            || !has_unique_index(
                connection,
                "asset_provenance",
                &["asset_id", "source_kind", "source_location"],
            )?)
    {
        return Ok(false);
    }

    let has_runs = table_exists(connection, "acquisition_runs")?;
    let has_run_work = table_exists(connection, "acquisition_run_work")?;
    if has_runs != has_run_work {
        return Ok(false);
    }
    if has_runs {
        let versioned = table_matches_columns(
            connection,
            "acquisition_runs",
            &[
                ("id", "INTEGER", false, true),
                ("request_json", "TEXT", true, false),
                ("request_schema_version", "INTEGER", true, false),
                ("status", "TEXT", true, false),
                ("queued_work", "INTEGER", true, false),
                ("completed_work", "INTEGER", true, false),
            ],
        )?;
        let versioned_after_alter = table_matches_columns(
            connection,
            "acquisition_runs",
            &[
                ("id", "INTEGER", false, true),
                ("request_json", "TEXT", true, false),
                ("status", "TEXT", true, false),
                ("queued_work", "INTEGER", true, false),
                ("completed_work", "INTEGER", true, false),
                ("request_schema_version", "INTEGER", true, false),
            ],
        )?;
        let unversioned = table_matches_columns(
            connection,
            "acquisition_runs",
            &[
                ("id", "INTEGER", false, true),
                ("request_json", "TEXT", true, false),
                ("status", "TEXT", true, false),
                ("queued_work", "INTEGER", true, false),
                ("completed_work", "INTEGER", true, false),
            ],
        )?;
        if (!versioned && !versioned_after_alter && !unversioned)
            || !table_matches_columns(
                connection,
                "acquisition_run_work",
                &[
                    ("id", "INTEGER", false, true),
                    ("run_id", "INTEGER", true, false),
                    ("work_key", "TEXT", true, false),
                    ("completed", "INTEGER", true, false),
                ],
            )?
            || !has_foreign_key(
                connection,
                "acquisition_run_work",
                "run_id",
                "acquisition_runs",
                "id",
            )?
            || !has_unique_index(connection, "acquisition_run_work", &["run_id", "work_key"])?
        {
            return Ok(false);
        }
    }

    Ok(true)
}

fn table_exists(connection: &Connection, table_name: &str) -> Result<bool, PortError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            params![table_name],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists == 1)
        .map_err(sql_error)
}

fn table_matches_columns(
    connection: &Connection,
    table_name: &str,
    expected: &[(&str, &str, bool, bool)],
) -> Result<bool, PortError> {
    let sql = format!(
        "SELECT name, type, \"notnull\", pk FROM pragma_table_info('{}') ORDER BY cid",
        table_name.replace('\'', "''")
    );
    let mut statement = connection.prepare(&sql).map_err(sql_error)?;
    let actual = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get::<_, i64>(3)? != 0,
            ))
        })
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;

    Ok(actual.len() == expected.len()
        && actual.iter().zip(expected).all(
            |((actual_name, actual_type, actual_not_null, actual_primary_key), expected)| {
                actual_name == expected.0
                    && actual_type.eq_ignore_ascii_case(expected.1)
                    && *actual_not_null == expected.2
                    && *actual_primary_key == expected.3
            },
        ))
}

fn has_foreign_key(
    connection: &Connection,
    table_name: &str,
    from_column: &str,
    referenced_table: &str,
    referenced_column: &str,
) -> Result<bool, PortError> {
    let sql = format!(
        "SELECT COUNT(*) FROM pragma_foreign_key_list('{}')
         WHERE \"from\" = ?1 AND \"table\" = ?2 AND \"to\" = ?3",
        table_name.replace('\'', "''")
    );
    connection
        .query_row(
            &sql,
            params![from_column, referenced_table, referenced_column],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count == 1)
        .map_err(sql_error)
}

fn has_unique_index(
    connection: &Connection,
    table_name: &str,
    expected_columns: &[&str],
) -> Result<bool, PortError> {
    let sql = format!(
        "SELECT name FROM pragma_index_list('{}') WHERE \"unique\" = 1",
        table_name.replace('\'', "''")
    );
    let mut statement = connection.prepare(&sql).map_err(sql_error)?;
    let indexes = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;

    for index_name in indexes {
        let index_sql = format!(
            "SELECT name FROM pragma_index_info('{}') ORDER BY seqno",
            index_name.replace('\'', "''")
        );
        let mut index_statement = connection.prepare(&index_sql).map_err(sql_error)?;
        let columns = index_statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)?;
        if columns
            .iter()
            .map(String::as_str)
            .eq(expected_columns.iter().copied())
        {
            return Ok(true);
        }
    }

    Ok(false)
}

fn parse_run_status(value: &str) -> Result<AcquisitionRunStatus, PortError> {
    match value {
        "running" => Ok(AcquisitionRunStatus::Running),
        "paused" => Ok(AcquisitionRunStatus::Paused),
        "cancelled" => Ok(AcquisitionRunStatus::Cancelled),
        "completed" => Ok(AcquisitionRunStatus::Completed),
        other => Err(PortError(format!(
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

fn migrate_legacy_game_identity(connection: &Connection) -> Result<(), PortError> {
    let games_sql: Option<String> = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'games'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    let Some(games_sql) = games_sql else {
        return Ok(());
    };
    let normalized_sql = games_sql
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if !normalized_sql.contains("normalized_title text not null unique") {
        return Ok(());
    }

    connection
        .execute_batch("PRAGMA foreign_keys = OFF; PRAGMA legacy_alter_table = ON;")
        .map_err(sql_error)?;
    let migration = connection.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE games RENAME TO games_legacy;
         CREATE TABLE games (
             id INTEGER PRIMARY KEY,
             title TEXT NOT NULL,
             normalized_title TEXT NOT NULL
         );
         INSERT INTO games (id, title, normalized_title)
         SELECT id, title, normalized_title FROM games_legacy;
         DROP TABLE games_legacy;
         COMMIT;",
    );
    if migration.is_err() {
        let _ = connection.execute_batch("ROLLBACK;");
    }
    let restore =
        connection.execute_batch("PRAGMA legacy_alter_table = OFF; PRAGMA foreign_keys = ON;");

    migration.map_err(sql_error)?;
    restore.map_err(sql_error)
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
