use game_media_vault_domain::{AcquisitionRequest, AssetType, AssetTypeSelector, SourceSelection};
use serde::Serialize;

use crate::{ApplicationError, ConnectorPort};

/// The Sources an Acquisition Request contacts and what each of them acquires for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AcquisitionPlan {
    /// Sources that serve the request, in selection order (registration order for `Auto`).
    pub sources: Vec<PlannedSource>,
    /// Selected Sources left out, each with the reason.
    pub excluded: Vec<ExcludedSource>,
    /// For each requested Asset Type selector, in request order, the planned Sources that
    /// acquire types it selects; none means no selected Source covers it.
    pub coverage: Vec<SelectorCoverage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedSource {
    pub source_id: String,
    /// The requested Asset Types this Source acquires.
    pub asset_types: Vec<AssetType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExcludedSource {
    pub source_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectorCoverage {
    pub selector: AssetTypeSelector,
    pub sources: Vec<String>,
}

/// Plans `request` across the registered `connectors`.
///
/// An explicit selection contacts only the selected Sources, each of which needs a connector;
/// `Auto` considers every registered one. A Source is planned for the requested Asset Types it
/// acquires, once its connector accepts the request, which may consult the Source; Sources
/// whose capabilities already rule them out are not consulted. A plan no Source serves is
/// refused.
pub fn plan_acquisition(
    request: &AcquisitionRequest,
    connectors: &[&dyn ConnectorPort],
) -> Result<AcquisitionPlan, ApplicationError> {
    let selected = match request.sources() {
        SourceSelection::Auto => connectors.to_vec(),
        SourceSelection::Explicit(source_ids) => source_ids
            .iter()
            .map(|source_id| {
                connectors
                    .iter()
                    .copied()
                    .find(|connector| connector.source_id() == source_id)
                    .ok_or_else(|| ApplicationError::UnsupportedConnectorPlan {
                        source_id: source_id.clone(),
                        reason: "no connector is registered for this source".to_owned(),
                    })
            })
            .collect::<Result<_, _>>()?,
    };

    let mut sources = Vec::new();
    let mut excluded = Vec::new();
    for connector in selected {
        let source_id = connector.source_id().to_owned();
        match plan_source(request, connector)? {
            Ok(asset_types) => sources.push(PlannedSource {
                source_id,
                asset_types,
            }),
            Err(reason) => excluded.push(ExcludedSource { source_id, reason }),
        }
    }
    if sources.is_empty() {
        return Err(ApplicationError::NoSourceServesPlan { excluded });
    }

    let coverage = request
        .asset_types()
        .iter()
        .map(|&selector| SelectorCoverage {
            selector,
            sources: sources
                .iter()
                .filter(|source| {
                    source.asset_types.iter().any(|asset_type| {
                        asset_type.selector() == selector || asset_type.family() == selector
                    })
                })
                .map(|source| source.source_id.clone())
                .collect(),
        })
        .collect();
    Ok(AcquisitionPlan {
        sources,
        excluded,
        coverage,
    })
}

/// The requested Asset Types `connector` acquires, or why its Source is left out.
fn plan_source(
    request: &AcquisitionRequest,
    connector: &dyn ConnectorPort,
) -> Result<Result<Vec<AssetType>, String>, ApplicationError> {
    let capabilities = connector.capabilities();
    if !capabilities.direct_media_download {
        return Ok(Err("cannot download media directly".to_owned()));
    }
    let asset_types: Vec<AssetType> = capabilities
        .asset_types
        .into_iter()
        .filter(|&asset_type| request.requests_asset_type(asset_type))
        .collect();
    if asset_types.is_empty() {
        return Ok(Err("acquires none of the requested asset types".to_owned()));
    }
    Ok(match connector.unsupported_request_reason(request)? {
        Some(reason) => Err(reason),
        None => Ok(asset_types),
    })
}
