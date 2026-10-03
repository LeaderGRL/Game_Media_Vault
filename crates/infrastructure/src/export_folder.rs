//! The folder on disk an export copies originals to.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use game_media_vault_application::{ApplicationError, ExportTargetPort, PortError};

/// Writes each copy beside its final name first, in a file of its own, then renames it into
/// place, so that a copy interrupted midway, or written by another export at the same time,
/// never stands for an original.
pub struct ExportFolder {
    root: PathBuf,
}

impl ExportFolder {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The folder `destination` names, resolved against the working directory when relative,
    /// unless it lies within the vault at `vault_root`, which an export leaves unchanged.
    pub fn outside_vault(destination: &Path, vault_root: &Path) -> Result<Self, ApplicationError> {
        let resolve = |path: &Path, error: io::Error| {
            PortError::new(format!("failed to resolve {}: {error}", path.display()))
        };
        let destination =
            std::path::absolute(destination).map_err(|error| resolve(destination, error))?;
        let vault = fs::canonicalize(vault_root).map_err(|error| resolve(vault_root, error))?;
        if lies_within(&destination, &vault) {
            return Err(ApplicationError::ExportWithinVault);
        }
        Ok(Self::new(destination))
    }
}

/// Whether `path`, which may not exist yet, lies within the canonical `root` once the part of it
/// that exists is made canonical.
fn lies_within(path: &Path, root: &Path) -> bool {
    let mut existing = path.to_path_buf();
    let mut missing = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name().map(std::ffi::OsStr::to_os_string) else {
            break;
        };
        missing.push(name);
        existing.pop();
    }
    let Ok(mut canonical) = fs::canonicalize(&existing) else {
        return false;
    };
    for name in missing.iter().rev() {
        canonical.push(name);
    }
    canonical.starts_with(root)
}

impl ExportTargetPort for ExportFolder {
    fn holds(&self, relative_path: &Path, byte_len: u64) -> Result<bool, PortError> {
        match fs::metadata(self.root.join(relative_path)) {
            Ok(metadata) => Ok(metadata.is_file() && metadata.len() == byte_len),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(io_error(&self.root.join(relative_path), error)),
        }
    }

    fn write(&self, relative_path: &Path, reader: &mut dyn Read) -> Result<(), PortError> {
        let path = self.root.join(relative_path);
        let parent = path
            .parent()
            .ok_or_else(|| PortError::new(format!("{} has no folder", path.display())))?;
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
        if path.file_name().is_none() {
            return Err(PortError::new(format!("{} names no file", path.display())));
        }
        let (partial, mut file) = create_partial(parent).map_err(|error| io_error(&path, error))?;
        let copied = io::copy(reader, &mut file).and_then(|_| file.sync_all());
        drop(file);
        // The rename replaces a stale copy.
        if let Err(error) = copied.and_then(|()| fs::rename(&partial, &path)) {
            let _ = fs::remove_file(&partial);
            return Err(io_error(&path, error));
        }
        Ok(())
    }
}

/// Copies this process has begun, which tell its partial files apart.
static PARTIAL_COPIES: AtomicU64 = AtomicU64::new(0);

/// A partial file in `parent` that no other write, of this export or another, takes: its name
/// holds this process and a count of its copies, and it is created only if no file has it. The
/// name is short whatever the final name, which may already be as long as file systems allow.
fn create_partial(parent: &Path) -> io::Result<(PathBuf, File)> {
    loop {
        let copy = PARTIAL_COPIES.fetch_add(1, Ordering::Relaxed);
        let partial = parent.join(format!(".export-{}-{copy}.partial", process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
        {
            Ok(file) => return Ok((partial, file)),
            // Left by an export that stopped midway in an earlier process of the same id.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}

fn io_error(path: &Path, error: io::Error) -> PortError {
    PortError::new(format!("failed to write {}: {error}", path.display()))
}
