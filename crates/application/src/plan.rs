use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AssetType, AssetTypeSelector, QualityRequirements,
    SourceSelection,
};
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
    /// acquire it.
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

/// Plans `request` across the registered `connectors`, one per Source.
///
/// Requirements the engine cannot apply are refused first. An explicit selection contacts only
/// the selected Sources, each of which needs a connector; `Auto` considers every registered
/// one. A Source is planned for the requested Asset Types it acquires once its connector
/// accepts the request, which may consult the Source. Every requested selector must be
/// acquired by a planned Source: when capabilities alone leave one uncovered, no Source is
/// consulted.
pub fn plan_acquisition(
    request: &AcquisitionRequest,
    connectors: &[&dyn ConnectorPort],
) -> Result<AcquisitionPlan, ApplicationError> {
    let CapableSources {
        sources: capable,
        mut excluded,
    } = capable_sources(request, connectors)?;
    let mut sources = Vec::new();
    for (connector, asset_types) in capable {
        match connector.unsupported_request_reason(request)? {
            Some(reason) => excluded.push(excluded_source(connector, reason)),
            None => sources.push(PlannedSource {
                source_id: connector.source_id().to_owned(),
                asset_types,
            }),
        }
    }
    let planned_types: Vec<AssetType> = sources
        .iter()
        .flat_map(|source| source.asset_types.iter().copied())
        .collect();
    ensure_covered(request, &planned_types, &excluded)?;

    let coverage = request
        .asset_types()
        .iter()
        .map(|&selector| SelectorCoverage {
            selector,
            sources: sources
                .iter()
                .filter(|source| selector.is_covered_by(&source.asset_types))
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

/// The selected Sources whose capabilities serve `request`, each with the requested Asset Types
/// it acquires, before any of them is consulted.
pub(crate) struct CapableSources<'a> {
    pub(crate) sources: Vec<(&'a dyn ConnectorPort, Vec<AssetType>)>,
    pub(crate) excluded: Vec<ExcludedSource>,
}

/// Selects the Sources of `request` among `connectors` by their capabilities alone. Refuses
/// requirements the engine cannot apply, selected Sources without a connector, and requested
/// selectors no capable Source acquires.
pub(crate) fn capable_sources<'a>(
    request: &AcquisitionRequest,
    connectors: &[&'a dyn ConnectorPort],
) -> Result<CapableSources<'a>, ApplicationError> {
    if let Some(reason) = request
        .quality()
        .and_then(QualityRequirements::unsupported_requirement)
    {
        return Err(ApplicationError::UnsupportedRequest(reason));
    }
    if request.limits() != &AcquisitionLimits::default() {
        return Err(ApplicationError::UnsupportedRequest(
            "acquisition limits are not supported yet",
        ));
    }
    let selected: Vec<&dyn ConnectorPort> = match request.sources() {
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
        match capable_asset_types(request, connector) {
            Ok(asset_types) => sources.push((connector, asset_types)),
            Err(reason) => excluded.push(excluded_source(connector, reason)),
        }
    }
    let capable_types: Vec<AssetType> = sources
        .iter()
        .flat_map(|(_, asset_types)| asset_types.iter().copied())
        .collect();
    ensure_covered(request, &capable_types, &excluded)?;
    Ok(CapableSources { sources, excluded })
}

/// The requested Asset Types `connector` can acquire, or why its Source is left out without
/// being consulted.
fn capable_asset_types(
    request: &AcquisitionRequest,
    connector: &dyn ConnectorPort,
) -> Result<Vec<AssetType>, String> {
    let capabilities = connector.capabilities();
    if !capabilities.direct_media_download {
        return Err("cannot download media directly".to_owned());
    }
    let asset_types: Vec<AssetType> = capabilities
        .asset_types
        .into_iter()
        .filter(|&asset_type| request.requests_asset_type(asset_type))
        .collect();
    if asset_types.is_empty() {
        return Err("acquires none of the requested asset types".to_owned());
    }
    Ok(asset_types)
}

pub(crate) fn excluded_source(connector: &dyn ConnectorPort, reason: String) -> ExcludedSource {
    ExcludedSource {
        source_id: connector.source_id().to_owned(),
        reason,
    }
}

pub(crate) fn ensure_covered(
    request: &AcquisitionRequest,
    acquired: &[AssetType],
    excluded: &[ExcludedSource],
) -> Result<(), ApplicationError> {
    let uncovered: Vec<AssetTypeSelector> = request
        .asset_types()
        .iter()
        .copied()
        .filter(|selector| !selector.is_covered_by(acquired))
        .collect();
    if uncovered.is_empty() {
        Ok(())
    } else {
        Err(ApplicationError::UncoveredAssetTypes {
            uncovered,
            excluded: excluded.to_vec(),
        })
    }
}
