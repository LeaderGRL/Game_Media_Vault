//! SQLite catalog adapter. Each port lives in its own module: `runs`, `reviews`, `assets`,
//! `library`, `reference`, `derived` and `verify`; `schema` owns the table layout and its
//! versioning.

use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::Mutex,
};

use game_media_vault_application::{CatalogPort, PortError};
use game_media_vault_domain::{ImportedAsset, LibraryEntry, PersistAsset};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};

mod assets;
mod derived;
mod library;
mod reference;
mod reviews;
mod runs;
mod schema;
mod verify;

pub struct SqliteCatalog {
    path: PathBuf,
    mode: CatalogOpenMode,
    /// The locks of the runs this catalog executes, by run, held until released.
    executions: Mutex<HashMap<i64, File>>,
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
            executions: Mutex::new(HashMap::new()),
            #[cfg(test)]
            busy_handler: None,
        })
    }

    pub fn open_existing(path: impl Into<PathBuf>) -> Result<Self, PortError> {
        let path = path.into();
        if !path.is_file() {
            return Err(PortError::new(format!(
                "catalog does not exist: {}",
                path.display()
            )));
        }

        let catalog = Self {
            path,
            mode: CatalogOpenMode::ExistingOnly,
            executions: Mutex::new(HashMap::new()),
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
            Err(cleanup_error) => Err(PortError::new(format!(
                "{error}; failed to remove incomplete catalog {}: {cleanup_error}",
                path.display()
            ))),
        },
    }
}

impl CatalogPort for SqliteCatalog {
    fn persist_asset(&self, record: PersistAsset) -> Result<ImportedAsset, PortError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let imported = assets::persist_asset_in_transaction(&transaction, record, None)?;
        transaction.commit().map_err(sql_error)?;
        Ok(imported)
    }

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError> {
        self.library_entries()
    }

    fn list_latest_assets(&self, limit: usize) -> Result<Vec<LibraryEntry>, PortError> {
        self.latest_library_assets(limit)
    }
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

fn io_error(error: std::io::Error) -> PortError {
    PortError::new(error.to_string())
}

fn sql_error(error: rusqlite::Error) -> PortError {
    PortError::new(error.to_string())
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
    use game_media_vault_domain::{AssetType, MediaInfo, PersistAsset, SourceId};
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
            Err(PortError::new("forced initialization failure".to_owned()))
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
                        executions: Mutex::new(HashMap::new()),
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
                            media: MediaInfo::unknown(),
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
