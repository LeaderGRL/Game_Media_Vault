use game_media_vault_domain::{AssetType, SourceFailure};
use serde::Serialize;

use crate::{ApplicationError, ConnectorPort, RunRepositoryPort};

/// What planning knows about a registered Source: the Asset Types it acquires and whether it
/// downloads media directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceDescription {
    pub source_id: String,
    pub asset_types: Vec<AssetType>,
    pub direct_media_download: bool,
}

/// Describes the registered `connectors` in registry order, from the capabilities planning
/// uses; no Source is consulted.
pub fn describe_sources(connectors: &[&dyn ConnectorPort]) -> Vec<SourceDescription> {
    connectors
        .iter()
        .map(|connector| {
            let capabilities = connector.capabilities();
            SourceDescription {
                source_id: connector.source_id().to_owned(),
                asset_types: capabilities.asset_types,
                direct_media_download: capabilities.direct_media_download,
            }
        })
        .collect()
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
