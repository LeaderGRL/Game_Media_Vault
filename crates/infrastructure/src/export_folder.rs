//! The folder on disk an export copies originals to.

use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

use game_media_vault_application::{ExportTargetPort, PortError};

/// Writes each copy beside its final name first, then renames it into place, so that a copy
/// interrupted midway never stands for an original.
pub struct ExportFolder {
    root: PathBuf,
}

impl ExportFolder {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
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
        let file_name = path
            .file_name()
            .ok_or_else(|| PortError::new(format!("{} names no file", path.display())))?;
        let mut partial_name = file_name.to_os_string();
        partial_name.push(".partial");
        let partial = parent.join(partial_name);
        let copied = File::create(&partial)
            .and_then(|mut file| io::copy(reader, &mut file).and_then(|_| file.sync_all()));
        if let Err(error) = copied {
            let _ = fs::remove_file(&partial);
            return Err(io_error(&path, error));
        }
        fs::rename(&partial, &path).map_err(|error| {
            let _ = fs::remove_file(&partial);
            io_error(&path, error)
        })
    }
}

fn io_error(path: &Path, error: io::Error) -> PortError {
    PortError::new(format!("failed to write {}: {error}", path.display()))
}
