use game_media_vault_domain::{AssetCandidate, AssetType};

/// Stable, source-scoped identity of an Asset Candidate. It keys run work and Review Items, so
/// human decisions follow the candidate across runs.
///
/// A provider candidate ID identifies the candidate when the Source offers one; otherwise the
/// normalized descriptive fields, label (or filename) and locator do. Parts are length-prefixed
/// so field values cannot collide through delimiters.
pub fn candidate_identity(source_id: &str, candidate: &AssetCandidate) -> String {
    let mut identity = "candidate".to_owned();
    if let Some(provider_candidate_id) = candidate.provider_candidate_id.as_deref() {
        for part in [
            source_id,
            provider_candidate_id,
            asset_type_part(candidate.asset_type),
        ] {
            push_part(&mut identity, part);
        }
        return identity;
    }
    for part in [
        source_id,
        normalize(&candidate.platform).as_str(),
        normalize(&candidate.game_title).as_str(),
        normalize(&candidate.region).as_str(),
        normalize(&candidate.edition_name).as_str(),
        asset_type_part(candidate.asset_type),
        candidate
            .source_asset_label
            .as_deref()
            .unwrap_or(&candidate.original_filename),
        &candidate.source_url,
    ] {
        push_part(&mut identity, part);
    }
    identity
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

fn push_part(identity: &mut String, value: &str) {
    identity.push(':');
    identity.push_str(&value.len().to_string());
    identity.push(':');
    identity.push_str(value);
}

fn asset_type_part(asset_type: AssetType) -> &'static str {
    match asset_type {
        AssetType::BoxFront => "box_front",
    }
}
