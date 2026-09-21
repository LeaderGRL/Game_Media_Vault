use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use game_media_vault_application::{ObjectStorePort, PortError};
use game_media_vault_domain::StoredObject;

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
}

impl ObjectStorePort for ContentAddressedStore {
    fn store_original(&self, source: &Path) -> Result<StoredObject, PortError> {
        let staging_dir = self.root.join("staging");
        fs::create_dir_all(&staging_dir).map_err(io_error)?;

        let (staging_path, mut output) = create_staging_file(&staging_dir)?;
        let mut input = File::open(source).map_err(io_error)?;
        let mut hasher = blake3::Hasher::new();
        let mut byte_len = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];

        loop {
            let read = input.read(&mut buffer).map_err(io_error)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            output.write_all(&buffer[..read]).map_err(io_error)?;
            byte_len += read as u64;
        }
        output.sync_all().map_err(io_error)?;
        drop(output);

        let hash = hasher.finalize().to_hex().to_string();
        let target = self.object_path(&hash);
        let parent = target
            .parent()
            .ok_or_else(|| PortError("object path has no parent directory".into()))?;
        fs::create_dir_all(parent).map_err(io_error)?;

        if target.exists() {
            if let Err(error) = verify_existing_object(&target, &hash, byte_len) {
                let _ = fs::remove_file(&staging_path);
                return Err(error);
            }
            fs::remove_file(&staging_path).map_err(io_error)?;
        } else if let Err(error) = fs::rename(&staging_path, &target) {
            if target.exists() {
                if let Err(error) = verify_existing_object(&target, &hash, byte_len) {
                    let _ = fs::remove_file(&staging_path);
                    return Err(error);
                }
                fs::remove_file(&staging_path).map_err(io_error)?;
            } else {
                return Err(io_error(error));
            }
        }

        Ok(StoredObject { hash, byte_len })
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
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&staging_path);
            }
            Err(error) => return Err(io_error(error)),
        }
    }
}

fn io_error(error: std::io::Error) -> PortError {
    PortError(error.to_string())
}
