use std::{
    fs,
    path::{Path, PathBuf},
};

use game_media_vault_application::{CatalogPort, PortError};
use game_media_vault_domain::{
    AssetProvenance, AssetType, ImportedAsset, LibraryEntry, PersistAsset, SourceKind,
};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};

pub struct SqliteCatalog {
    path: PathBuf,
    mode: CatalogOpenMode,
}

#[derive(Clone, Copy)]
enum CatalogOpenMode {
    CreateOrOpen,
    ExistingOnly,
}

impl SqliteCatalog {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, PortError> {
        let path = path.into();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let catalog = Self {
            path,
            mode: CatalogOpenMode::CreateOrOpen,
        };
        let connection = catalog.connect()?;
        initialize_schema(&connection)?;
        Ok(catalog)
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
        };
        let connection = catalog.connect()?;
        let table_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table'
                   AND name IN ('games', 'release_editions', 'assets', 'asset_provenance')",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if table_count != 4 {
            return Err(PortError(format!(
                "catalog schema is missing or incomplete: {}",
                catalog.path.display()
            )));
        }

        Ok(catalog)
    }

    fn connect(&self) -> Result<Connection, PortError> {
        let flags = match self.mode {
            CatalogOpenMode::CreateOrOpen => {
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
            }
            CatalogOpenMode::ExistingOnly => OpenFlags::SQLITE_OPEN_READ_WRITE,
        };
        let connection = Connection::open_with_flags(&self.path, flags).map_err(sql_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .map_err(sql_error)?;
        Ok(connection)
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
        let source_kind = source_kind_to_str(record.source_kind);
        let byte_len = i64::try_from(record.byte_len)
            .map_err(|_| PortError("asset byte length exceeds SQLite INTEGER range".into()))?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let lookup = ExistingImportLookup {
            normalized_title: &normalized_title,
            normalized_platform: &normalized_platform,
            normalized_region: &normalized_region,
            normalized_edition: &normalized_edition,
            asset_type,
            source_kind,
            byte_len,
        };
        if let Some(existing) = find_existing_import(&transaction, &record, &lookup)? {
            normalize_existing_provenance(&transaction, &record, source_kind, &existing)?;
            transaction.commit().map_err(sql_error)?;
            return Ok(existing.imported);
        }

        let game_id = resolve_game_id(&transaction, &record, &normalized_title)?;

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
        let release_edition_id: i64 = transaction
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
                "INSERT INTO asset_provenance (asset_id, source_kind, source_location)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(asset_id, source_kind, source_location) DO NOTHING",
                params![asset_id, source_kind, record.source_location,],
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
        let mut statement = connection
            .prepare(
                "SELECT
                    g.id, g.title,
                    r.id, r.platform, r.region, r.edition_name,
                    a.id, a.asset_type, a.object_hash, a.byte_len, a.original_filename,
                    p.source_kind, p.source_location
                 FROM assets a
                 JOIN release_editions r ON r.id = a.release_edition_id
                 JOIN games g ON g.id = r.game_id
                 LEFT JOIN asset_provenance p ON p.asset_id = a.id
                 ORDER BY g.title, r.id, a.id, p.id",
            )
            .map_err(sql_error)?;
        let mut rows = statement.query([]).map_err(sql_error)?;
        let mut entries: Vec<LibraryEntry> = Vec::new();

        while let Some(row) = rows.next().map_err(sql_error)? {
            let asset_id: i64 = row.get(6).map_err(sql_error)?;
            let source_kind: Option<String> = row.get(11).map_err(sql_error)?;
            let source_location: Option<String> = row.get(12).map_err(sql_error)?;

            if let Some(existing) = entries
                .last_mut()
                .filter(|entry| entry.asset_id == asset_id)
            {
                if let (Some(kind), Some(location)) = (source_kind, source_location) {
                    existing.provenance.push(AssetProvenance {
                        source_kind: parse_source_kind(&kind)?,
                        source_location: location,
                    });
                }
                continue;
            }

            let byte_len: i64 = row.get(9).map_err(sql_error)?;
            let mut provenance = Vec::new();
            if let (Some(kind), Some(location)) = (source_kind, source_location) {
                provenance.push(AssetProvenance {
                    source_kind: parse_source_kind(&kind)?,
                    source_location: location,
                });
            }
            entries.push(LibraryEntry {
                game_id: row.get(0).map_err(sql_error)?,
                game_title: row.get(1).map_err(sql_error)?,
                release_edition_id: row.get(2).map_err(sql_error)?,
                platform: row.get(3).map_err(sql_error)?,
                region: row.get(4).map_err(sql_error)?,
                edition_name: row.get(5).map_err(sql_error)?,
                asset_id,
                asset_type: parse_asset_type(&row.get::<_, String>(7).map_err(sql_error)?)?,
                object_hash: row.get(8).map_err(sql_error)?,
                byte_len: u64::try_from(byte_len)
                    .map_err(|_| PortError("catalog contains a negative byte length".into()))?,
                original_filename: row.get(10).map_err(sql_error)?,
                provenance,
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
    source_kind: &'a str,
    byte_len: i64,
}

struct ExistingImportMatch {
    imported: ImportedAsset,
    matched_source_location: String,
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
               AND (?9 IS NULL OR g.id = ?9)",
        )
        .map_err(sql_error)?;
    let mut rows = statement
        .query(params![
            lookup.source_kind,
            lookup.asset_type,
            record.object_hash,
            lookup.byte_len,
            lookup.normalized_title,
            lookup.normalized_platform,
            lookup.normalized_region,
            lookup.normalized_edition,
            record.existing_game_id,
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
            matched_source_location,
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
    fs::canonicalize(Path::new(location)).ok()
}

fn normalize_existing_provenance(
    transaction: &Transaction<'_>,
    record: &PersistAsset,
    source_kind: &str,
    existing: &ExistingImportMatch,
) -> Result<(), PortError> {
    if existing.matched_source_location == record.source_location {
        return Ok(());
    }

    transaction
        .execute(
            "INSERT INTO asset_provenance (asset_id, source_kind, source_location)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(asset_id, source_kind, source_location) DO NOTHING",
            params![
                existing.imported.asset_id,
                source_kind,
                record.source_location,
            ],
        )
        .map_err(sql_error)?;
    Ok(())
}

fn initialize_schema(connection: &Connection) -> Result<(), PortError> {
    migrate_legacy_game_identity(connection)?;
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
                UNIQUE(asset_id, source_kind, source_location)
            );
            CREATE INDEX IF NOT EXISTS idx_release_game ON release_editions(game_id);
            CREATE INDEX IF NOT EXISTS idx_game_normalized_title ON games(normalized_title);
            CREATE INDEX IF NOT EXISTS idx_asset_release ON assets(release_edition_id);
            CREATE INDEX IF NOT EXISTS idx_provenance_asset ON asset_provenance(asset_id);",
        )
        .map_err(sql_error)
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

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
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

fn source_kind_to_str(source_kind: SourceKind) -> &'static str {
    match source_kind {
        SourceKind::LocalImport => "local_import",
    }
}

fn parse_source_kind(value: &str) -> Result<SourceKind, PortError> {
    match value {
        "local_import" => Ok(SourceKind::LocalImport),
        other => Err(PortError(format!(
            "unknown source kind in catalog: {other}"
        ))),
    }
}

fn io_error(error: std::io::Error) -> PortError {
    PortError(error.to_string())
}

fn sql_error(error: rusqlite::Error) -> PortError {
    PortError(error.to_string())
}
