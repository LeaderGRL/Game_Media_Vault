use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

use game_media_vault_application::{
    DerivedStorePort, ObjectArea, ObjectCheck, ObjectStorePort, PortError, VaultRepairStorePort,
    VaultStorePort,
};
use game_media_vault_domain::{MediaInfo, StoredObject};

use crate::media::MediaInspector;

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Staging files of the stores this process is running, which verification must not take for
/// interrupted ones.
static STAGING_IN_PROGRESS: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

fn staging_in_progress() -> MutexGuard<'static, BTreeSet<PathBuf>> {
    // The set stays consistent even if a holder panicked.
    STAGING_IN_PROGRESS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// BLAKE3-addressed store of immutable original bytes (ADR 0002).
pub struct ContentAddressedStore {
    root: PathBuf,
}

impl ContentAddressedStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn object_path(&self, hash: &str) -> PathBuf {
        self.address("objects", hash)
    }

    /// Where a Derived Asset with this hash lives, apart from the originals.
    pub fn derived_path(&self, hash: &str) -> PathBuf {
        self.address("derived", hash)
    }

    fn address(&self, area: &str, hash: &str) -> PathBuf {
        let first = hash.get(0..2).unwrap_or("__");
        let second = hash.get(2..4).unwrap_or("__");
        self.root.join(area).join(first).join(second).join(hash)
    }

    /// Streams the bytes into a private staging file while hashing them.
    fn stage(&self, input: &mut dyn Read) -> Result<StagedObject, PortError> {
        let staging_dir = self.root.join("staging");
        fs::create_dir_all(&staging_dir).map_err(io_error)?;
        let (staging_path, mut output) = create_staging_file(&staging_dir)?;
        // From here on, dropping `staged` removes the staging file on every error path.
        let mut staged = StagedObject {
            staging_path,
            stored: StoredObject {
                hash: String::new(),
                byte_len: 0,
                media: MediaInfo::unknown(),
            },
        };
        let mut hasher = blake3::Hasher::new();
        let mut media = MediaInspector::default();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = input.read(&mut buffer).map_err(io_error)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            output.write_all(&buffer[..read]).map_err(io_error)?;
            media.update(&buffer[..read]);
            staged.stored.byte_len += read as u64;
        }
        output.sync_all().map_err(io_error)?;
        staged.stored.hash = hasher.finalize().to_hex().to_string();
        staged.stored.media = media.finish();
        Ok(staged)
    }

    /// Moves the staged file to its content address in `area`, or verifies the object already
    /// there.
    fn publish(&self, staged: StagedObject, area: &str) -> Result<StoredObject, PortError> {
        let target = self.address(area, &staged.stored.hash);
        let parent = target
            .parent()
            .ok_or_else(|| PortError::new("object path has no parent directory".into()))?;
        fs::create_dir_all(parent).map_err(io_error)?;
        if !target.exists() {
            match publish_staged_object(&staged.staging_path, &target, parent) {
                Ok(()) => return Ok(staged.stored.clone()),
                Err(PublishError::Durability(error)) => return Err(io_error(error)),
                // Another import published the same bytes concurrently; verify them below.
                Err(PublishError::NotPublished(error)) if !target.exists() => {
                    return Err(io_error(error));
                }
                Err(PublishError::NotPublished(_)) => {}
            }
        }
        verify_existing_object(&target, &staged.stored.hash, staged.stored.byte_len)?;
        sync_object_parent(parent).map_err(io_error)?;
        Ok(staged.stored.clone())
    }
}

impl ObjectStorePort for ContentAddressedStore {
    fn store_original(&self, reader: &mut dyn Read) -> Result<StoredObject, PortError> {
        let staged = self.stage(reader)?;
        self.publish(staged, "objects")
    }
}

impl DerivedStorePort for ContentAddressedStore {
    fn open_original(&self, hash: &str) -> Result<Box<dyn Read + Send>, PortError> {
        // Only content addresses name objects, so no other path is ever opened.
        if !is_object_hash(hash) {
            return Err(PortError::new(format!("{hash:?} is not an object hash")));
        }
        let file = File::open(self.object_path(hash)).map_err(io_error)?;
        Ok(Box::new(file))
    }

    fn store_derived(&self, reader: &mut dyn Read) -> Result<StoredObject, PortError> {
        let staged = self.stage(reader)?;
        self.publish(staged, "derived")
    }
}

impl VaultStorePort for ContentAddressedStore {
    fn stored_objects(&self, area: ObjectArea) -> Result<Vec<String>, PortError> {
        let mut hashes = Vec::new();
        for first in subdirectories(&self.root.join(area_name(area)))? {
            for second in subdirectories(&first)? {
                // Only files named by a hash and stored at that hash's address are objects of
                // this store; removing any other would remove a different path.
                hashes.extend(file_names(&second)?.into_iter().filter(|name| {
                    is_object_hash(name) && self.address(area_name(area), name) == second.join(name)
                }));
            }
        }
        Ok(hashes)
    }

    fn check_object(&self, area: ObjectArea, hash: &str) -> Result<ObjectCheck, PortError> {
        // Only content addresses name objects, so no other path is ever read.
        if !is_object_hash(hash) {
            return Ok(ObjectCheck::Unreadable {
                reason: format!("{hash:?} is not an object hash"),
            });
        }
        match hash_file(&self.address(area_name(area), hash)) {
            Ok(actual_hash) if actual_hash == hash => Ok(ObjectCheck::Intact),
            Ok(actual_hash) => Ok(ObjectCheck::Corrupt { actual_hash }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ObjectCheck::Missing),
            // One damaged or inaccessible object is a finding, not a reason to stop.
            Err(error) => Ok(ObjectCheck::Unreadable {
                reason: error.to_string(),
            }),
        }
    }

    /// Staging files of the stores this process is running are left out; any other is listed.
    fn staging_files(&self) -> Result<Vec<String>, PortError> {
        let staging_dir = self.root.join("staging");
        let in_progress = staging_in_progress();
        Ok(file_names(&staging_dir)?
            .into_iter()
            .filter(|name| !in_progress.contains(&staging_dir.join(name)))
            .collect())
    }
}

impl VaultRepairStorePort for ContentAddressedStore {
    fn remove_object(&self, area: ObjectArea, hash: &str) -> Result<(), PortError> {
        if !is_object_hash(hash) {
            return Err(PortError::new(format!("{hash:?} is not an object hash")));
        }
        remove_if_present(&self.address(area_name(area), hash))
    }

    fn remove_staging_file(&self, name: &str) -> Result<(), PortError> {
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
            return Err(PortError::new(format!(
                "{name:?} is not a staging file name"
            )));
        }
        remove_if_present(&self.root.join("staging").join(name))
    }
}

fn remove_if_present(path: &Path) -> Result<(), PortError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}

/// Whether `name` is a BLAKE3 hash as the store writes it: 64 lower-case hex digits. Upper case
/// is refused since case-insensitive file systems would alias it to another object.
fn is_object_hash(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn area_name(area: ObjectArea) -> &'static str {
    match area {
        ObjectArea::Original => "objects",
        ObjectArea::Derived => "derived",
    }
}

struct StagedObject {
    staging_path: PathBuf,
    stored: StoredObject,
}

impl Drop for StagedObject {
    fn drop(&mut self) {
        // Already moved when publication succeeded.
        let _ = fs::remove_file(&self.staging_path);
        staging_in_progress().remove(&self.staging_path);
    }
}

/// The BLAKE3 hash of the bytes stored at `path`.
fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut input = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// The names of the files directly under `directory`, sorted; none when it does not exist.
fn file_names(directory: &Path) -> Result<Vec<String>, PortError> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error(error)),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io_error)?;
        if entry.file_type().map_err(io_error)?.is_file() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// The directories directly under `directory`, sorted; none when it does not exist.
fn subdirectories(directory: &Path) -> Result<Vec<PathBuf>, PortError> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error(error)),
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io_error)?;
        if entry.file_type().map_err(io_error)?.is_dir() {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

fn verify_existing_object(
    path: &Path,
    expected_hash: &str,
    expected_len: u64,
) -> Result<(), PortError> {
    let metadata = fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() {
        return Err(PortError::new(format!(
            "object integrity check failed: {} is not a file",
            path.display()
        )));
    }
    if metadata.len() != expected_len {
        return Err(PortError::new(format!(
            "object integrity check failed: {} has length {}, expected {expected_len}",
            path.display(),
            metadata.len()
        )));
    }

    let actual_hash = hash_file(path).map_err(io_error)?;
    if actual_hash != expected_hash {
        return Err(PortError::new(format!(
            "object integrity check failed: {} hashes to {actual_hash}, expected {expected_hash}",
            path.display()
        )));
    }

    Ok(())
}

fn create_staging_file(staging_dir: &Path) -> Result<(PathBuf, File), PortError> {
    loop {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let staging_path = staging_dir.join(format!("{}-{sequence}.tmp", std::process::id()));

        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staging_path)
        {
            Ok(file) => {
                staging_in_progress().insert(staging_path.clone());
                return Ok((staging_path, file));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error(error)),
        }
    }
}

#[derive(Debug)]
enum PublishError {
    NotPublished(std::io::Error),
    Durability(std::io::Error),
}

fn publish_staged_object(staging: &Path, target: &Path, parent: &Path) -> Result<(), PublishError> {
    publish_staged_object_with(staging, target, parent, move_object, sync_object_parent)
}

fn publish_staged_object_with<M, S>(
    staging: &Path,
    target: &Path,
    parent: &Path,
    move_file: M,
    sync_parent: S,
) -> Result<(), PublishError>
where
    M: FnOnce(&Path, &Path) -> std::io::Result<()>,
    S: FnOnce(&Path) -> std::io::Result<()>,
{
    move_file(staging, target).map_err(PublishError::NotPublished)?;
    sync_parent(parent).map_err(PublishError::Durability)
}

#[cfg(unix)]
fn move_object(staging: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(staging, target)
}

#[cfg(windows)]
fn move_object(staging: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    let staging_wide: Vec<u16> = staging.as_os_str().encode_wide().chain(Some(0)).collect();
    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let moved = unsafe {
        MoveFileExW(
            staging_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn io_error(error: std::io::Error) -> PortError {
    PortError::new(error.to_string())
}

#[cfg(unix)]
fn sync_object_parent(parent: &Path) -> std::io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(windows)]
fn sync_object_parent(_parent: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn post_move_sync_failure_is_not_treated_as_a_publish_race() {
        let temp = tempdir().unwrap();
        let staging = temp.path().join("staging.tmp");
        let target = temp.path().join("object");
        fs::write(&staging, b"original bytes").unwrap();

        let error = publish_staged_object_with(
            &staging,
            &target,
            temp.path(),
            |from, to| fs::rename(from, to),
            |_| Err(io::Error::other("forced parent sync failure")),
        )
        .unwrap_err();

        assert!(matches!(error, PublishError::Durability(_)));
        assert!(!staging.exists());
        assert_eq!(fs::read(target).unwrap(), b"original bytes");
    }
}
