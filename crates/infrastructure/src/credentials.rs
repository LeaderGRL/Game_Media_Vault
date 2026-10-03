use game_media_vault_application::{ApiKey, CredentialStorePort, PortError};
use keyring::v1::{Entry, Error};

/// The API keys of this machine's Sources, kept in the OS secure credential store (Windows
/// Credential Manager, macOS Keychain, the Secret Service elsewhere), one entry per Source. Its
/// errors never show a key.
pub struct KeyringCredentialStore {
    service: String,
}

impl KeyringCredentialStore {
    /// The store every vault of this machine shares.
    pub fn machine() -> Self {
        Self::named("game-media-vault")
    }

    /// A store under its own service name, which keeps its entries apart from the machine's.
    pub fn named(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, source_id: &str) -> Result<Entry, PortError> {
        Entry::new(&self.service, source_id).map_err(|error| failed("open", source_id, &error))
    }
}

impl CredentialStorePort for KeyringCredentialStore {
    fn api_key(&self, source_id: &str) -> Result<Option<ApiKey>, PortError> {
        match self.entry(source_id)?.get_password() {
            // A value no key can be, as another tool may have written, reads as no key, which
            // storing a key again replaces.
            Ok(key) => Ok(ApiKey::new(key).ok()),
            Err(Error::NoEntry) => Ok(None),
            Err(error) => Err(failed("read", source_id, &error)),
        }
    }

    fn set_api_key(&self, source_id: &str, key: &ApiKey) -> Result<(), PortError> {
        self.entry(source_id)?
            .set_password(key.expose())
            .map_err(|error| failed("store", source_id, &error))
    }

    fn clear_api_key(&self, source_id: &str) -> Result<(), PortError> {
        match self.entry(source_id)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(error) => Err(failed("forget", source_id, &error)),
        }
    }
}

/// Credentials of a machine that keeps none: no Source has a key, and none can be stored.
pub struct NoCredentials;

impl CredentialStorePort for NoCredentials {
    fn api_key(&self, _source_id: &str) -> Result<Option<ApiKey>, PortError> {
        Ok(None)
    }

    fn set_api_key(&self, _source_id: &str, _key: &ApiKey) -> Result<(), PortError> {
        Err(PortError::new(
            "this machine keeps no credentials for these commands".to_owned(),
        ))
    }

    fn clear_api_key(&self, _source_id: &str) -> Result<(), PortError> {
        Ok(())
    }
}

/// A failure of the credential store, which names the Source but never its key.
fn failed(action: &str, source_id: &str, error: &Error) -> PortError {
    PortError::new(format!(
        "failed to {action} the API key of {source_id} in this machine's credential store: {error}"
    ))
}
