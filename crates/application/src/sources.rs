use std::io::Read;

use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, SourceFailure,
};
use serde::Serialize;

use crate::{ApplicationError, ConnectorPort, MachineSettingsPort, PortError, RunRepositoryPort};

/// What planning knows about a registered Source: the Asset Types it acquires, whether it
/// downloads media directly, and whether it takes part in acquisitions on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceDescription {
    pub source_id: String,
    pub asset_types: Vec<AssetType>,
    pub direct_media_download: bool,
    pub enabled: bool,
}

/// Describes the registered `connectors` in registry order, from the capabilities planning
/// uses and the `disabled_sources` of this machine; no Source is consulted.
pub fn describe_sources(
    connectors: &[&dyn ConnectorPort],
    disabled_sources: &[String],
) -> Vec<SourceDescription> {
    connectors
        .iter()
        .map(|connector| {
            let capabilities = connector.capabilities();
            SourceDescription {
                source_id: connector.source_id().to_owned(),
                asset_types: capabilities.asset_types,
                direct_media_download: capabilities.direct_media_download,
                enabled: !disabled_sources
                    .iter()
                    .any(|disabled| disabled == connector.source_id()),
            }
        })
        .collect()
}

/// Enables or disables the registered Source `source_id` on this machine, for every vault it
/// opens, and describes the registered `connectors` as they are now.
pub fn set_source_enabled(
    settings: &dyn MachineSettingsPort,
    connectors: &[&dyn ConnectorPort],
    source_id: &str,
    enabled: bool,
) -> Result<Vec<SourceDescription>, ApplicationError> {
    if !connectors
        .iter()
        .any(|connector| connector.source_id() == source_id)
    {
        return Err(ApplicationError::SourceNotRegistered(source_id.to_owned()));
    }
    settings.set_source_enabled(source_id, enabled)?;
    Ok(describe_sources(connectors, &settings.disabled_sources()?))
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
