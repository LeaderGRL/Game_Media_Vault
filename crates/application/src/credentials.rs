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

/// One credential a Source asks for, such as an API key, or the identifier and password of an
/// account at the Source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CredentialField {
    /// Its stable name, as `source key set --field` takes it.
    pub id: &'static str,
    /// What the user is asked for.
    pub label: &'static str,
    /// Whether the Source works without it, perhaps less well.
    pub optional: bool,
}

/// The one credential of a Source that needs just an API key.
pub const API_KEY_FIELD: CredentialField = CredentialField {
    id: "api-key",
    label: "API key",
    optional: false,
};

impl CredentialField {
    /// The name this machine's credential store keeps it under for `source_id`: the Source's own
    /// for an API key, so a key stored before Sources had several credentials still counts, and
    /// `<source>/<field>` for any other.
    pub fn stored_as(&self, source_id: &str) -> String {
        if self.id == API_KEY_FIELD.id {
            source_id.to_owned()
        } else {
            format!("{source_id}/{}", self.id)
        }
    }
}

/// Whether this machine stores one credential a Source asks for, never the credential itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialFieldState {
    pub id: String,
    pub label: String,
    pub optional: bool,
    pub state: CredentialState,
}

/// Whether a Source has the credentials it needs on this machine, never the credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialState {
    /// The Source needs no credential.
    NotNeeded,
    /// The Source needs a credential this machine does not store.
    Missing,
    /// This machine stores every credential the Source needs.
    Stored,
    /// This machine's credential store could not be read, so whether it stores the key is
    /// unknown.
    Unreadable,
}
