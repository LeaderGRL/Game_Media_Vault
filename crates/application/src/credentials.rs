use std::fmt;

use serde::Serialize;

use crate::ApplicationError;

/// An API key a Source requires, which only this machine's secure credential store keeps: never
/// the vault, logs, exports or provenance. Its `Debug` output never shows it, and it cannot be
/// serialized.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// The key as the user gave it, without surrounding whitespace. A blank key, or one with
    /// whitespace or control characters inside, is refused.
    pub fn new(value: impl AsRef<str>) -> Result<Self, ApplicationError> {
        let value = value.as_ref().trim();
        if value.is_empty()
            || value
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(ApplicationError::InvalidApiKey);
        }
        Ok(Self(value.to_owned()))
    }

    /// The key itself, for the connector that sends it to its Source.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey(<redacted>)")
    }
}

/// Whether a Source has the credential it needs on this machine, never the credential itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialState {
    /// The Source needs no credential.
    NotNeeded,
    /// The Source needs an API key this machine does not store.
    Missing,
    /// This machine stores the API key the Source needs.
    Stored,
}
