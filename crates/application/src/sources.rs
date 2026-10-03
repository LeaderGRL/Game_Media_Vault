use std::io::Read;

use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, SourceFailure,
};
use serde::Serialize;

use crate::{
    ApiKey, ApplicationError, ConnectorPort, CredentialField, CredentialFieldState,
    CredentialState, CredentialStorePort, MachineSettingsPort, PortError, RunRepositoryPort,
};

/// What this machine keeps for every vault it opens: its settings and its credentials.
#[derive(Clone, Copy)]
pub struct Machine<'a> {
    pub settings: &'a dyn MachineSettingsPort,
    pub credentials: &'a dyn CredentialStorePort,
}

/// What planning knows about a registered Source: the Asset Types it acquires, whether it
/// downloads media directly, whether it takes part in acquisitions on this machine, and whether
/// this machine stores the credential it needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceDescription {
    pub source_id: String,
    pub asset_types: Vec<AssetType>,
    pub direct_media_download: bool,
    pub enabled: bool,
    pub credential: CredentialState,
    /// Each credential the Source asks for, and whether this machine stores it.
    pub credential_fields: Vec<CredentialFieldState>,
    /// What the Source is known to limit, in words, when anything is known.
    pub rate_limits: Option<String>,
}

/// Describes the registered `connectors` in registry order, from the capabilities planning
/// uses and what this `machine` keeps; no Source is consulted, and no credential is shown.
pub fn describe_sources(
    connectors: &[&dyn ConnectorPort],
    machine: Machine<'_>,
) -> Result<Vec<SourceDescription>, ApplicationError> {
    let disabled_sources = machine.settings.disabled_sources()?;
    Ok(connectors
        .iter()
        .map(|connector| {
            let capabilities = connector.capabilities();
            // A store that cannot be read leaves every other Source described.
            let credential_fields: Vec<CredentialFieldState> = connector
                .credential_fields()
                .iter()
                .map(|field| CredentialFieldState {
                    id: field.id.to_owned(),
                    label: field.label.to_owned(),
                    optional: field.optional,
                    state: match machine
                        .credentials
                        .api_key(&field.stored_as(connector.source_id()))
                    {
                        Ok(Some(_)) => CredentialState::Stored,
                        Ok(None) => CredentialState::Missing,
                        Err(_) => CredentialState::Unreadable,
                    },
                })
                .collect();
            let credential = if credential_fields.is_empty() {
                CredentialState::NotNeeded
            } else if credential_fields
                .iter()
                .any(|field| field.state == CredentialState::Unreadable)
            {
                CredentialState::Unreadable
            } else if credential_fields
                .iter()
                .any(|field| !field.optional && field.state == CredentialState::Missing)
            {
                CredentialState::Missing
            } else {
                CredentialState::Stored
            };
            SourceDescription {
                source_id: connector.source_id().to_owned(),
                asset_types: capabilities.asset_types,
                direct_media_download: capabilities.direct_media_download,
                enabled: !disabled_sources
                    .iter()
                    .any(|disabled| disabled == connector.source_id()),
                credential,
                credential_fields,
                rate_limits: connector.rate_limits(),
            }
        })
        .collect())
}

/// The registered connector of `source_id`.
fn registered<'a>(
    connectors: &[&'a dyn ConnectorPort],
    source_id: &str,
) -> Result<&'a dyn ConnectorPort, ApplicationError> {
    connectors
        .iter()
        .copied()
        .find(|connector| connector.source_id() == source_id)
        .ok_or_else(|| ApplicationError::SourceNotRegistered(source_id.to_owned()))
}

/// Enables or disables the registered Source `source_id` on this machine, for every vault it
/// opens, and describes the registered `connectors` as they are now.
pub fn set_source_enabled(
    machine: Machine<'_>,
    connectors: &[&dyn ConnectorPort],
    source_id: &str,
    enabled: bool,
) -> Result<Vec<SourceDescription>, ApplicationError> {
    registered(connectors, source_id)?;
    machine.settings.set_source_enabled(source_id, enabled)?;
    describe_sources(connectors, machine)
}

/// Stores on this machine the API key the registered Source `source_id` needs, for every vault
/// it opens, and describes the registered `connectors` as they are now.
pub fn set_source_api_key(
    machine: Machine<'_>,
    connectors: &[&dyn ConnectorPort],
    source_id: &str,
    key: &ApiKey,
) -> Result<Vec<SourceDescription>, ApplicationError> {
    set_source_credential(machine, connectors, source_id, None, key)
}

/// Stores, in this machine's credential store, the credential `field` of the registered Source
/// `source_id`, or its one credential when no field is named, and describes the registered
/// `connectors` as they are now.
pub fn set_source_credential(
    machine: Machine<'_>,
    connectors: &[&dyn ConnectorPort],
    source_id: &str,
    field: Option<&str>,
    value: &ApiKey,
) -> Result<Vec<SourceDescription>, ApplicationError> {
    let fields = registered(connectors, source_id)?.credential_fields();
    if fields.is_empty() {
        return Err(ApplicationError::SourceNeedsNoApiKey(source_id.to_owned()));
    }
    let field = match field {
        Some(name) => named_field(source_id, fields, name)?,
        None => match fields {
            [only] => only,
            several => {
                return Err(ApplicationError::CredentialFieldRequired {
                    source_id: source_id.to_owned(),
                    fields: several.iter().map(|field| field.id.to_owned()).collect(),
                });
            }
        },
    };
    if field.single_word && value.expose().contains(char::is_whitespace) {
        return Err(ApplicationError::CredentialNotOneWord(
            field.label.to_owned(),
        ));
    }
    machine
        .credentials
        .set_api_key(&field.stored_as(source_id), value)?;
    describe_sources(connectors, machine)
}

/// The credential named `name` among the `fields` of the Source `source_id`.
fn named_field<'a>(
    source_id: &str,
    fields: &'a [CredentialField],
    name: &str,
) -> Result<&'a CredentialField, ApplicationError> {
    fields.iter().find(|field| field.id == name).ok_or_else(|| {
        ApplicationError::UnknownCredentialField {
            source_id: source_id.to_owned(),
            field: name.to_owned(),
        }
    })
}

/// Forgets the API key this machine stores for the registered Source `source_id`, and
/// describes the registered `connectors` as they are now.
pub fn clear_source_api_key(
    machine: Machine<'_>,
    connectors: &[&dyn ConnectorPort],
    source_id: &str,
) -> Result<Vec<SourceDescription>, ApplicationError> {
    clear_source_credential(machine, connectors, source_id, None)
}

/// Forgets the credential `field` this machine stores for the registered Source `source_id`, or
/// every credential of it when no field is named, and describes the registered `connectors` as
/// they are now.
pub fn clear_source_credential(
    machine: Machine<'_>,
    connectors: &[&dyn ConnectorPort],
    source_id: &str,
    field: Option<&str>,
) -> Result<Vec<SourceDescription>, ApplicationError> {
    let fields = registered(connectors, source_id)?.credential_fields();
    match field {
        Some(name) => machine
            .credentials
            .clear_api_key(&named_field(source_id, fields, name)?.stored_as(source_id))?,
        None => {
            for field in fields {
                machine
                    .credentials
                    .clear_api_key(&field.stored_as(source_id))?;
            }
            // A key an earlier version stored under the Source's own name, when the Source
            // asked for one, goes too.
            if !fields
                .iter()
                .any(|field| field.stored_as(source_id) == source_id)
            {
                machine.credentials.clear_api_key(source_id)?;
            }
        }
    }
    describe_sources(connectors, machine)
}

/// The failures executions recorded for one Source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceFailureSummary {
    pub source_id: String,
    /// Failures recorded in all.
    pub failures: usize,
    /// The latest failures, newest first.
    pub latest: Vec<SourceFailure>,
}

/// Summarizes the failures executions recorded, by Source in source id order, keeping the
/// latest `per_source` of each. Sources that never failed are left out.
pub fn summarize_source_failures(
    runs: &dyn RunRepositoryPort,
    per_source: usize,
) -> Result<Vec<SourceFailureSummary>, ApplicationError> {
    let mut summaries: Vec<SourceFailureSummary> = Vec::new();
    // Newest first, so each Source keeps its latest failures.
    for failure in runs.source_failures()?.into_iter().rev() {
        let index = match summaries
            .iter()
            .position(|summary| summary.source_id == failure.source_id)
        {
            Some(index) => index,
            None => {
                summaries.push(SourceFailureSummary {
                    source_id: failure.source_id.clone(),
                    failures: 0,
                    latest: Vec::new(),
                });
                summaries.len() - 1
            }
        };
        let summary = &mut summaries[index];
        summary.failures += 1;
        if summary.latest.len() < per_source {
            summary.latest.push(failure);
        }
    }
    summaries.sort_by(|left, right| left.source_id.cmp(&right.source_id));
    Ok(summaries)
}

/// Why a Source disabled on this machine takes no part in acquisitions.
const DISABLED_ON_THIS_MACHINE: &str = "disabled on this machine";

/// A registered connector as this machine's settings leave it. A Source disabled on this
/// machine keeps its capabilities, so plans and descriptions still list it, but refuses every
/// request, discovery and download without being contacted; its queued work waits until it is
/// enabled again.
pub struct MachineConnector<C> {
    connector: C,
    enabled: bool,
}

impl<C> MachineConnector<C> {
    fn of(connector: C, disabled_sources: &[String]) -> Self
    where
        C: ConnectorPort,
    {
        let enabled = !disabled_sources
            .iter()
            .any(|disabled| disabled == connector.source_id());
        Self { connector, enabled }
    }
}

impl<C: ConnectorPort> ConnectorPort for MachineConnector<C> {
    fn source_id(&self) -> &'static str {
        self.connector.source_id()
    }

    fn needs_api_key(&self) -> bool {
        self.connector.needs_api_key()
    }

    fn credential_fields(&self) -> &'static [CredentialField] {
        self.connector.credential_fields()
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        self.connector.capabilities()
    }

    fn unsupported_request_reason(
        &self,
        request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        if !self.enabled {
            return Ok(Some(DISABLED_ON_THIS_MACHINE.to_owned()));
        }
        self.connector.unsupported_request_reason(request)
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        if !self.enabled {
            return Err(PortError::new(DISABLED_ON_THIS_MACHINE.to_owned()));
        }
        self.connector.discover(request)
    }

    fn discovery_batch_size(&self) -> Option<usize> {
        self.connector.discovery_batch_size()
    }

    fn download(&self, candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        if !self.enabled {
            return Err(PortError::new(DISABLED_ON_THIS_MACHINE.to_owned()));
        }
        self.connector.download(candidate)
    }

    fn disabled_reason(&self) -> Option<String> {
        if !self.enabled {
            return Some(DISABLED_ON_THIS_MACHINE.to_owned());
        }
        self.connector.disabled_reason()
    }

    fn rate_limits(&self) -> Option<String> {
        self.connector.rate_limits()
    }
}

/// The registered connectors as this machine's settings leave them, in registry order.
pub struct MachineConnectors<'a>(Vec<MachineConnector<&'a dyn ConnectorPort>>);

impl MachineConnectors<'_> {
    /// The connectors, to plan and execute acquisitions with.
    pub fn refs(&self) -> Vec<&dyn ConnectorPort> {
        self.0
            .iter()
            .map(|connector| connector as &dyn ConnectorPort)
            .collect()
    }
}

/// Wraps the registered `connectors` so that the `disabled_sources` of this machine take no
/// part in acquisitions.
pub fn machine_connectors<'a>(
    connectors: &[&'a dyn ConnectorPort],
    disabled_sources: &[String],
) -> MachineConnectors<'a> {
    MachineConnectors(
        connectors
            .iter()
            .map(|&connector| MachineConnector::of(connector, disabled_sources))
            .collect(),
    )
}

/// Wraps the connectors of an owned `registry`, as [`machine_connectors`] does.
pub fn machine_registry(
    registry: Vec<Box<dyn ConnectorPort>>,
    disabled_sources: &[String],
) -> Vec<Box<dyn ConnectorPort>> {
    registry
        .into_iter()
        .map(|connector| {
            Box::new(MachineConnector::of(connector, disabled_sources)) as Box<dyn ConnectorPort>
        })
        .collect()
}
