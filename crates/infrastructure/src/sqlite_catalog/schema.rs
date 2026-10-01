use std::path::Path;

use game_media_vault_application::PortError;
use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::sql_error;

/// Identifies SQLite files created by Game Media Vault ("GMVA").
const VAULT_APPLICATION_ID: i32 = 0x474D_5641;

/// Layout version of the catalog tables. Bump it together with a new entry in `MIGRATIONS`.
const VAULT_SCHEMA_VERSION: i32 = 2;

/// Oldest layout that can still be upgraded. Version 1 was an unreleased pre-release layout.
const OLDEST_SUPPORTED_SCHEMA_VERSION: i32 = 2;

type Migration = fn(&Transaction<'_>) -> Result<(), PortError>;

/// Entry `i` upgrades a catalog from `OLDEST_SUPPORTED_SCHEMA_VERSION + i` to the next version.
const MIGRATIONS: &[Migration] = &[];

const _: () = assert!(
    MIGRATIONS.len() as i32 == VAULT_SCHEMA_VERSION - OLDEST_SUPPORTED_SCHEMA_VERSION,
    "every supported schema version needs a migration to the current layout"
);

const SCHEMA: &str = "
    CREATE TABLE games (
        id INTEGER PRIMARY KEY,
        title TEXT NOT NULL,
        normalized_title TEXT NOT NULL
    );
    CREATE TABLE release_editions (
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
    CREATE TABLE release_assertions (
        id INTEGER PRIMARY KEY,
        release_edition_id INTEGER NOT NULL REFERENCES release_editions(id),
        source_id TEXT NOT NULL,
        source_location TEXT NOT NULL,
        field TEXT NOT NULL,
        qualifier TEXT NOT NULL,
        value TEXT NOT NULL,
        normalized_value TEXT NOT NULL,
        UNIQUE(release_edition_id, source_id, source_location, field, qualifier, value)
    );
    CREATE TABLE assets (
        id INTEGER PRIMARY KEY,
        release_edition_id INTEGER NOT NULL REFERENCES release_editions(id),
        asset_type TEXT NOT NULL,
        object_hash TEXT NOT NULL,
        byte_len INTEGER NOT NULL CHECK(byte_len >= 0),
        original_filename TEXT NOT NULL,
        UNIQUE(release_edition_id, asset_type, object_hash)
    );
    CREATE TABLE asset_provenance (
        id INTEGER PRIMARY KEY,
        asset_id INTEGER NOT NULL REFERENCES assets(id),
        source_kind TEXT NOT NULL,
        source_location TEXT NOT NULL,
        source_asset_label TEXT,
        -- Acquisition candidate this row records, or '' for a direct import: candidates sharing
        -- a locator keep their own label and match decision.
        candidate_identity TEXT NOT NULL DEFAULT '',
        match_decision_json TEXT,
        UNIQUE(asset_id, source_kind, source_location, candidate_identity)
    );
    CREATE TABLE acquisition_runs (
        id INTEGER PRIMARY KEY,
        request_json TEXT NOT NULL,
        request_schema_version INTEGER NOT NULL CHECK(request_schema_version > 0),
        status TEXT NOT NULL
            CHECK(status IN ('running', 'paused', 'cancelled', 'completed'))
    );
    CREATE TABLE acquisition_run_discoveries (
        run_id INTEGER NOT NULL REFERENCES acquisition_runs(id),
        source_id TEXT NOT NULL,
        PRIMARY KEY(run_id, source_id)
    );
    CREATE TABLE review_items (
        id INTEGER PRIMARY KEY,
        candidate_identity TEXT NOT NULL UNIQUE,
        candidate_json TEXT NOT NULL,
        competing_matches_json TEXT NOT NULL,
        decision_json TEXT,
        status TEXT NOT NULL CHECK(
            status IN ('pending', 'deferred', 'accepted', 'rejected', 'auto_resolved', 'superseded')
        )
    );
    CREATE TABLE acquisition_run_work (
        id INTEGER PRIMARY KEY,
        run_id INTEGER NOT NULL REFERENCES acquisition_runs(id),
        work_key TEXT NOT NULL,
        candidate_json TEXT NOT NULL,
        state TEXT NOT NULL CHECK(state IN ('queued', 'parked', 'done')),
        review_item_id INTEGER REFERENCES review_items(id),
        UNIQUE(run_id, work_key),
        CHECK((state = 'parked') = (review_item_id IS NOT NULL))
    );
    CREATE INDEX idx_release_game ON release_editions(game_id);
    CREATE INDEX idx_release_assertion_release ON release_assertions(release_edition_id);
    CREATE INDEX idx_release_assertion_title
        ON release_assertions(source_id, field, normalized_value);
    CREATE UNIQUE INDEX idx_release_assertion_source_record
        ON release_assertions(source_id, qualifier, value)
        WHERE field = 'identifier' AND qualifier = 'source_record';
    CREATE INDEX idx_game_normalized_title ON games(normalized_title);
    CREATE INDEX idx_asset_release ON assets(release_edition_id);
    CREATE INDEX idx_provenance_asset ON asset_provenance(asset_id);
    CREATE INDEX idx_provenance_candidate ON asset_provenance(candidate_identity);
    CREATE INDEX idx_run_work_state ON acquisition_run_work(run_id, state, id);
    CREATE INDEX idx_run_work_review ON acquisition_run_work(review_item_id)
        WHERE review_item_id IS NOT NULL;
";

/// Creates the current schema in an empty database and stamps it as a vault catalog.
pub(super) fn create(connection: &mut Connection) -> Result<(), PortError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql_error)?;
    let existing_objects: i64 = transaction
        .query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))
        .map_err(sql_error)?;
    if existing_objects != 0 {
        return Err(PortError(
            "refusing to create a vault catalog in a non-empty database".to_owned(),
        ));
    }
    transaction.execute_batch(SCHEMA).map_err(sql_error)?;
    stamp(&transaction, VAULT_SCHEMA_VERSION)?;
    transaction.commit().map_err(sql_error)
}

/// Verifies that an existing database is a supported vault catalog and upgrades older layouts.
pub(super) fn open(connection: &mut Connection, path: &Path) -> Result<(), PortError> {
    let application_id = read_pragma(connection, "application_id")?;
    if application_id != VAULT_APPLICATION_ID {
        return Err(PortError(format!(
            "{} is not a Game Media Vault catalog",
            path.display()
        )));
    }
    let version = read_pragma(connection, "user_version")?;
    if version > VAULT_SCHEMA_VERSION {
        return Err(PortError(format!(
            "catalog {} uses schema version {version}, which is newer than the supported version {VAULT_SCHEMA_VERSION}; upgrade Game Media Vault",
            path.display()
        )));
    }
    if version < OLDEST_SUPPORTED_SCHEMA_VERSION {
        return Err(PortError(format!(
            "catalog {} uses unsupported schema version {version}; recreate the vault with this version of Game Media Vault",
            path.display()
        )));
    }
    for target in version + 1..=VAULT_SCHEMA_VERSION {
        let migrate = MIGRATIONS[(target - OLDEST_SUPPORTED_SCHEMA_VERSION - 1) as usize];
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        migrate(&transaction)?;
        stamp(&transaction, target)?;
        transaction.commit().map_err(sql_error)?;
    }
    Ok(())
}

fn stamp(transaction: &Transaction<'_>, version: i32) -> Result<(), PortError> {
    transaction
        .execute_batch(&format!(
            "PRAGMA application_id = {VAULT_APPLICATION_ID}; PRAGMA user_version = {version};"
        ))
        .map_err(sql_error)
}

fn read_pragma(connection: &Connection, name: &str) -> Result<i32, PortError> {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .map_err(sql_error)
}
