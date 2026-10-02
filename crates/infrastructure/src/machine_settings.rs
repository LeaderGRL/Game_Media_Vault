use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

use game_media_vault_application::{MachineSettingsPort, PortError};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

/// The settings of this machine, kept as JSON in one file every vault the machine opens shares.
/// Changes are made one process at a time, and settings a newer version wrote are kept.
pub struct MachineSettingsFile {
    path: PathBuf,
}

#[derive(Default, Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    disabled_sources: Vec<String>,
    #[serde(flatten)]
    others: serde_json::Map<String, serde_json::Value>,
}

impl MachineSettingsFile {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The settings in the OS standard configuration directory, when the OS has one.
    pub fn machine() -> Option<Self> {
        dirs::config_dir().map(|dir| Self::at(dir.join("game-media-vault").join("settings.json")))
    }

    /// The settings as last written; none before the first change.
    fn read(&self) -> Result<Settings, PortError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Settings::default());
            }
            Err(error) => return Err(self.failed("read", error)),
        };
        // Unreadable settings never silently enable a Source this machine disabled.
        serde_json::from_slice(&bytes).map_err(|error| {
            PortError::new(format!(
                "machine settings {} are unreadable: {error}",
                self.path.display()
            ))
        })
    }

    /// Changes the settings while no other process does.
    fn change(&self, change: impl FnOnce(&mut Settings)) -> Result<(), PortError> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir).map_err(|error| self.failed("write", error))?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path.with_extension("json.lock"))
            .map_err(|error| self.failed("lock", error))?;
        lock.lock().map_err(|error| self.failed("lock", error))?;
        let mut settings = self.read()?;
        change(&mut settings);
        let written = serde_json::to_vec_pretty(&settings).map_err(|error| {
            PortError::new(format!("failed to encode machine settings: {error}"))
        })?;
        let mut staged = NamedTempFile::new_in(dir).map_err(|error| self.failed("write", error))?;
        staged
            .write_all(&written)
            .map_err(|error| self.failed("write", error))?;
        staged
            .persist(&self.path)
            .map_err(|error| self.failed("write", error.error))?;
        Ok(())
    }

    fn failed(&self, action: &str, error: io::Error) -> PortError {
        PortError::new(format!(
            "failed to {action} machine settings {}: {error}",
            self.path.display()
        ))
    }
}

impl MachineSettingsPort for MachineSettingsFile {
    fn disabled_sources(&self) -> Result<Vec<String>, PortError> {
        Ok(self.read()?.disabled_sources)
    }

    fn set_source_enabled(&self, source_id: &str, enabled: bool) -> Result<(), PortError> {
        self.change(|settings| {
            settings
                .disabled_sources
                .retain(|disabled| disabled != source_id);
            if !enabled {
                settings.disabled_sources.push(source_id.to_owned());
            }
        })
    }
}

/// Settings of a machine without a configuration directory: every Source stays enabled, and no
/// setting can change.
pub struct NoMachineSettings;

impl MachineSettingsPort for NoMachineSettings {
    fn disabled_sources(&self) -> Result<Vec<String>, PortError> {
        Ok(Vec::new())
    }

    fn set_source_enabled(&self, _source_id: &str, _enabled: bool) -> Result<(), PortError> {
        Err(PortError::new(
            "this machine has no configuration directory to keep its settings in".to_owned(),
        ))
    }
}

/// The settings of this machine, in the OS standard configuration directory when it has one.
pub fn machine_settings() -> Box<dyn MachineSettingsPort> {
    match MachineSettingsFile::machine() {
        Some(file) => Box::new(file),
        None => Box::new(NoMachineSettings),
    }
}
