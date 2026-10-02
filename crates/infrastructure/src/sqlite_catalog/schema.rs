use std::path::Path;

use game_media_vault_application::PortError;
use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::sql_error;

/// Identifies SQLite files created by Game Media Vault ("GMVA").
const VAULT_APPLICATION_ID: i32 = 0x474D_5641;

/// Layout version of the catalog tables. Bump it together with a new entry in `MIGRATIONS`.
const VAULT_SCHEMA_VERSION: i32 = 11;

/// Oldest layout that can still be upgraded. Version 1 was an unreleased pre-release layout.
const OLDEST_SUPPORTED_SCHEMA_VERSION: i32 = 2;

type Migration = fn(&Transaction<'_>) -> Result<(), PortError>;

/// Entry `i` upgrades a catalog from `OLDEST_SUPPORTED_SCHEMA_VERSION + i` to the next version.
const MIGRATIONS: &[Migration] = &[
    add_asset_media,
    add_work_quality_shortfalls,
    add_work_outranked,
    add_derived_objects,
    add_run_planned_sources,
    add_work_unavailable_reason,
    add_release_assertion_value_index,
    add_source_failures,
    add_reference_dump_sets,
];

/// Version 11 records the dumps each reference record asserted the last time it was imported,
/// which links the records of several sources by their dumps. Records imported before have
/// none until imported again. The table matches `SCHEMA`.
fn add_reference_dump_sets(transaction: &Transaction<'_>) -> Result<(), PortError> {
    transaction
        .execute_batch(REFERENCE_DUMP_SETS_TABLE)
        .map_err(sql_error)
}

/// One row per reference record whose every dump has a SHA-1: the sorted lower-case SHA-1s of
/// its dumps, replaced whenever the record is imported again.
const REFERENCE_DUMP_SETS_TABLE: &str = "
    CREATE TABLE reference_dump_sets (
        source_id TEXT NOT NULL,
        source_record TEXT NOT NULL,
        release_edition_id INTEGER NOT NULL REFERENCES release_editions(id),
        dump_set TEXT NOT NULL,
        PRIMARY KEY(source_id, source_record)
    );
    CREATE INDEX idx_reference_dump_set ON reference_dump_sets(dump_set);
";

/// Version 10 records the failures of Sources that executions met, as history. The table
/// matches `SCHEMA`.
fn add_source_failures(transaction: &Transaction<'_>) -> Result<(), PortError> {
    transaction
        .execute_batch(SOURCE_FAILURES_TABLE)
        .map_err(sql_error)
}

/// Failures are only appended; their id is their recording order.
const SOURCE_FAILURES_TABLE: &str = "
    CREATE TABLE acquisition_source_failures (
        id INTEGER PRIMARY KEY,
        run_id INTEGER NOT NULL REFERENCES acquisition_runs(id),
        source_id TEXT NOT NULL,
        stage TEXT NOT NULL CHECK(stage IN ('discovery', 'download')),
        message TEXT NOT NULL,
        recorded_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER))
    );
";

/// Version 9 indexes assertions by the values reference imports look up (the titles other
/// sources assert), which otherwise scan every assertion for each imported record.
/// The index matches `SCHEMA`.
fn add_release_assertion_value_index(transaction: &Transaction<'_>) -> Result<(), PortError> {
    transaction
        .execute_batch(RELEASE_ASSERTION_VALUE_INDEX)
        .map_err(sql_error)
}

const RELEASE_ASSERTION_VALUE_INDEX: &str = "
    CREATE INDEX idx_release_assertion_value
        ON release_assertions(field, qualifier, normalized_value);
";

/// Version 8 records why completed work could not be acquired because its Source no longer serves
/// the media. The column matches the `acquisition_run_work` layout of `SCHEMA`.
fn add_work_unavailable_reason(transaction: &Transaction<'_>) -> Result<(), PortError> {
    transaction
        .execute_batch("ALTER TABLE acquisition_run_work ADD COLUMN unavailable_reason TEXT;")
        .map_err(sql_error)
}

/// Version 7 records the Sources each run's Acquisition Plan kept. Runs before it never planned
/// an `Auto` selection, so they keep the Sources their request selects explicitly. The column
/// matches the `acquisition_runs` layout of `SCHEMA`.
fn add_run_planned_sources(transaction: &Transaction<'_>) -> Result<(), PortError> {
    transaction
        .execute_batch(
            "ALTER TABLE acquisition_runs
                 ADD COLUMN planned_sources_json TEXT NOT NULL DEFAULT '[]';
             UPDATE acquisition_runs
                 SET planned_sources_json =
                     COALESCE(json_extract(request_json, '$.sources.values'), '[]');",
        )
        .map_err(sql_error)
}

/// Version 6 records the Derived Assets generated from originals. The table matches `SCHEMA`.
fn add_derived_objects(transaction: &Transaction<'_>) -> Result<(), PortError> {
    transaction
        .execute_batch(DERIVED_OBJECTS_TABLE)
        .map_err(sql_error)
}

/// One output per original and recipe; recipes are reproducible, so it is generated once.
const DERIVED_OBJECTS_TABLE: &str = "
    CREATE TABLE derived_objects (
        original_hash TEXT NOT NULL,
        recipe_key TEXT NOT NULL,
        recipe_json TEXT NOT NULL,
        object_hash TEXT NOT NULL,
        byte_len INTEGER NOT NULL CHECK(byte_len >= 0),
        media_type TEXT NOT NULL,
        width INTEGER CHECK(width IS NULL OR width > 0),
        height INTEGER CHECK(height IS NULL OR height > 0),
        PRIMARY KEY(original_hash, recipe_key)
    );
";

/// Version 5 records why Keep Best Per Type retained no new Asset for completed work. The
/// column matches the `acquisition_run_work` layout of `SCHEMA`.
fn add_work_outranked(transaction: &Transaction<'_>) -> Result<(), PortError> {
    transaction
        .execute_batch("ALTER TABLE acquisition_run_work ADD COLUMN outranked_json TEXT;")
        .map_err(sql_error)
}

/// Version 4 records how completed work fell short of its run's quality requirements. The
/// column matches the `acquisition_run_work` layout of `SCHEMA`.
fn add_work_quality_shortfalls(transaction: &Transaction<'_>) -> Result<(), PortError> {
    transaction
        .execute_batch("ALTER TABLE acquisition_run_work ADD COLUMN quality_shortfalls_json TEXT;")
        .map_err(sql_error)
}

/// Version 3 records what each original is. Assets imported before keep unknown media until
/// vault verification inspects their objects again. The columns match the `assets` layout of
/// `SCHEMA`.
fn add_asset_media(transaction: &Transaction<'_>) -> Result<(), PortError> {
    transaction
        .execute_batch(
            "ALTER TABLE assets
                 ADD COLUMN media_type TEXT NOT NULL DEFAULT 'application/octet-stream';
             ALTER TABLE assets
                 ADD COLUMN width INTEGER CHECK(width IS NULL OR width > 0);
             ALTER TABLE assets
                 ADD COLUMN height INTEGER CHECK(height IS NULL OR height > 0);",
        )
        .map_err(sql_error)
}

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
        media_type TEXT NOT NULL DEFAULT 'application/octet-stream',
        width INTEGER CHECK(width IS NULL OR width > 0),
        height INTEGER CHECK(height IS NULL OR height > 0),
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
            CHECK(status IN ('running', 'paused', 'cancelled', 'completed')),
        planned_sources_json TEXT NOT NULL DEFAULT '[]'
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
        quality_shortfalls_json TEXT,
        outranked_json TEXT,
        unavailable_reason TEXT,
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
        return Err(PortError::new(
            "refusing to create a vault catalog in a non-empty database".to_owned(),
        ));
    }
    transaction.execute_batch(SCHEMA).map_err(sql_error)?;
    transaction
        .execute_batch(DERIVED_OBJECTS_TABLE)
        .map_err(sql_error)?;
    transaction
        .execute_batch(RELEASE_ASSERTION_VALUE_INDEX)
        .map_err(sql_error)?;
    transaction
        .execute_batch(SOURCE_FAILURES_TABLE)
        .map_err(sql_error)?;
    transaction
        .execute_batch(REFERENCE_DUMP_SETS_TABLE)
        .map_err(sql_error)?;
    stamp(&transaction, VAULT_SCHEMA_VERSION)?;
    transaction.commit().map_err(sql_error)
}

/// Verifies that an existing database is a supported vault catalog and upgrades older layouts.
pub(super) fn open(connection: &mut Connection, path: &Path) -> Result<(), PortError> {
    let application_id = read_pragma(connection, "application_id")?;
    if application_id != VAULT_APPLICATION_ID {
        return Err(PortError::new(format!(
            "{} is not a Game Media Vault catalog",
            path.display()
        )));
    }
    let version = read_pragma(connection, "user_version")?;
    if version > VAULT_SCHEMA_VERSION {
        return Err(newer_schema(path, version));
    }
    if version < OLDEST_SUPPORTED_SCHEMA_VERSION {
        return Err(PortError::new(format!(
            "catalog {} uses unsupported schema version {version}; recreate the vault with this version of Game Media Vault",
            path.display()
        )));
    }
    // A current catalog needs no write lock, which another process may be holding.
    if version == VAULT_SCHEMA_VERSION {
        return Ok(());
    }
    loop {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        // Another opener may have upgraded the catalog while this one waited for the lock.
        let version = read_pragma(&transaction, "user_version")?;
        if version > VAULT_SCHEMA_VERSION {
            return Err(newer_schema(path, version));
        }
        if version == VAULT_SCHEMA_VERSION {
            return Ok(());
        }
        let migrate = MIGRATIONS[(version - OLDEST_SUPPORTED_SCHEMA_VERSION) as usize];
        migrate(&transaction)?;
        stamp(&transaction, version + 1)?;
        transaction.commit().map_err(sql_error)?;
    }
}

fn newer_schema(path: &Path, version: i32) -> PortError {
    PortError::new(format!(
        "catalog {} uses schema version {version}, which is newer than the supported version {VAULT_SCHEMA_VERSION}; upgrade Game Media Vault",
        path.display()
    ))
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

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Condvar, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Duration,
    };

    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::*;

    /// Serializes the racing tests, which share the busy-handler signal.
    static RACE: Mutex<()> = Mutex::new(());
    static WAITING_FOR_WRITE_LOCK: AtomicBool = AtomicBool::new(false);
    static WAITING_SIGNAL: (Mutex<()>, Condvar) = (Mutex::new(()), Condvar::new());

    fn signal_waiting(_: i32) -> bool {
        WAITING_FOR_WRITE_LOCK.store(true, Ordering::SeqCst);
        WAITING_SIGNAL.1.notify_all();
        thread::sleep(Duration::from_millis(1));
        true
    }

    /// Opens a catalog at the oldest supported version while a first opener holds the write
    /// lock, then lets the first opener stamp `version_after_first_opener` and release it.
    fn open_racing_a_first_opener(version_after_first_opener: i32) -> Result<(), PortError> {
        let _race = RACE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        WAITING_FOR_WRITE_LOCK.store(false, Ordering::SeqCst);
        let temp = tempdir().unwrap();
        let path = temp.path().join("catalog.sqlite3");
        let mut first_opener = Connection::open(&path).unwrap();
        create(&mut first_opener).unwrap();
        first_opener
            .execute_batch(&format!(
                "PRAGMA user_version = {OLDEST_SUPPORTED_SCHEMA_VERSION}; BEGIN IMMEDIATE;"
            ))
            .unwrap();
        let second_path = path.clone();
        let second_opener = thread::spawn(move || {
            let mut connection = Connection::open(&second_path).unwrap();
            connection.busy_handler(Some(signal_waiting)).unwrap();
            open(&mut connection, &second_path)
        });
        let guard = WAITING_SIGNAL.0.lock().unwrap();
        let (_guard, timeout) = WAITING_SIGNAL
            .1
            .wait_timeout_while(guard, Duration::from_secs(5), |_| {
                !WAITING_FOR_WRITE_LOCK.load(Ordering::SeqCst)
            })
            .unwrap();
        assert!(
            !timeout.timed_out(),
            "the second opener should wait for the lock"
        );

        first_opener
            .execute_batch(&format!(
                "PRAGMA user_version = {version_after_first_opener}; COMMIT;"
            ))
            .unwrap();
        second_opener.join().unwrap()
    }

    #[test]
    fn a_catalog_upgraded_by_a_concurrent_opener_is_not_upgraded_again() {
        assert!(open_racing_a_first_opener(VAULT_SCHEMA_VERSION).is_ok());
    }

    #[test]
    fn a_catalog_upgraded_past_this_version_meanwhile_is_refused() {
        let error = open_racing_a_first_opener(VAULT_SCHEMA_VERSION + 1).unwrap_err();

        assert!(error.message().contains("newer"), "{error}");
    }

    #[test]
    fn a_current_catalog_opens_while_another_process_writes() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("catalog.sqlite3");
        let mut writer = Connection::open(&path).unwrap();
        create(&mut writer).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE;").unwrap();

        // Without a busy timeout, any write lock request would fail at once.
        let mut reader = Connection::open(&path).unwrap();

        assert!(open(&mut reader, &path).is_ok());
    }
}
