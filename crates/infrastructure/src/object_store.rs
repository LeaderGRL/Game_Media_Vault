use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use game_media_vault_application::{ObjectStorePort, PortError};
use game_media_vault_domain::{MediaInfo, StoredObject};

use crate::media::{MEDIA_HEADER_BYTES, inspect_media};

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// BLAKE3-addressed store of immutable original bytes (ADR 0002).
pub struct ContentAddressedStore {
    root: PathBuf,
}

impl ContentAddressedStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn object_path(&self, hash: &str) -> PathBuf {
        let first = hash.get(0..2).unwrap_or("__");
        let second = hash.get(2..4).unwrap_or("__");
        self.root
            .join("objects")
            .join(first)
            .join(second)
            .join(hash)
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
        let mut header = Vec::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = input.read(&mut buffer).map_err(io_error)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            output.write_all(&buffer[..read]).map_err(io_error)?;
            let header_room = MEDIA_HEADER_BYTES.saturating_sub(header.len());
            header.extend_from_slice(&buffer[..read.min(header_room)]);
            staged.stored.byte_len += read as u64;
        }
        output.sync_all().map_err(io_error)?;
        staged.stored.hash = hasher.finalize().to_hex().to_string();
        staged.stored.media = inspect_media(&header);
        Ok(staged)
    }

    /// Moves the staged file to its content address, or verifies the object already there.
    fn publish(&self, staged: StagedObject) -> Result<StoredObject, PortError> {
        let target = self.object_path(&staged.stored.hash);
        let parent = target
            .parent()
            .ok_or_else(|| PortError("object path has no parent directory".into()))?;
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
        self.publish(staged)
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
    }
}

fn verify_existing_object(
    path: &Path,
    expected_hash: &str,
    expected_len: u64,
) -> Result<(), PortError> {
    let metadata = fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() {
        return Err(PortError(format!(
            "object integrity check failed: {} is not a file",
            path.display()
        )));
    }
    if metadata.len() != expected_len {
        return Err(PortError(format!(
            "object integrity check failed: {} has length {}, expected {expected_len}",
            path.display(),
            metadata.len()
        )));
    }

    let mut input = File::open(path).map_err(io_error)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let actual_hash = hasher.finalize().to_hex().to_string();
    if actual_hash != expected_hash {
        return Err(PortError(format!(
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
            Ok(file) => return Ok((staging_path, file)),
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
    PortError(error.to_string())
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
