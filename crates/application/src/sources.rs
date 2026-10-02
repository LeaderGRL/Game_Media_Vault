use game_media_vault_domain::AssetType;
use serde::Serialize;

use crate::ConnectorPort;

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
