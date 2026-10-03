use std::io::Read;

use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, SourceFailure,
};
use serde::Serialize;

use crate::{
    ApiKey, ApplicationError, ConnectorPort, CredentialState, CredentialStorePort,
    MachineSettingsPort, PortError, RunRepositoryPort,
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
            let credential = if !connector.needs_api_key() {
                CredentialState::NotNeeded
            } else {
                match machine.credentials.api_key(connector.source_id()) {
                    Ok(Some(_)) => CredentialState::Stored,
                    Ok(None) => CredentialState::Missing,
                    Err(_) => CredentialState::Unreadable,
                }
            };
            SourceDescription {
                source_id: connector.source_id().to_owned(),
                asset_types: capabilities.asset_types,
                direct_media_download: capabilities.direct_media_download,
                enabled: !disabled_sources
                    .iter()
                    .any(|disabled| disabled == connector.source_id()),
                credential,
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
    if !registered(connectors, source_id)?.needs_api_key() {
        return Err(ApplicationError::SourceNeedsNoApiKey(source_id.to_owned()));
    }
    machine.credentials.set_api_key(source_id, key)?;
    describe_sources(connectors, machine)
}

/// Forgets the API key this machine stores for the registered Source `source_id`, and
/// describes the registered `connectors` as they are now.
pub fn clear_source_api_key(
    machine: Machine<'_>,
    connectors: &[&dyn ConnectorPort],
    source_id: &str,
) -> Result<Vec<SourceDescription>, ApplicationError> {
    registered(connectors, source_id)?;
    machine.credentials.clear_api_key(source_id)?;
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
