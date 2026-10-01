use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::{AssetType, LibraryAsset};

/// The Asset Game Media Vault currently prefers for one Asset Type of a Release Edition, with
/// why it outranks each other Asset of that type. The other Assets stay retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreferredAsset {
    pub asset_type: AssetType,
    pub asset_id: i64,
    /// The other Assets of the type, best first, each with the reason it ranks lower.
    pub outranks: Vec<AssetPreference>,
}

/// Why the Preferred Asset outranks another Asset of its type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetPreference {
    pub asset_id: i64,
    pub reason: PreferenceReason,
}

/// What decides between two Assets of a type, checked in declaration order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum PreferenceReason {
    /// More pixels; an unknown pixel size ranks below every known one.
    MorePixels { preferred: u64, other: Option<u64> },
    /// The same pixel count in more bytes, which usually means less compression.
    MoreBytes { preferred: u64, other: u64 },
    /// Nothing else differs, so the Asset acquired first stays preferred.
    AcquiredFirst,
}

/// Derives the Preferred Asset of every Asset Type present in `assets`, in the order the types
/// first appear.
pub fn preferred_assets(assets: &[LibraryAsset]) -> Vec<PreferredAsset> {
    let mut asset_types: Vec<AssetType> = Vec::new();
    for asset in assets {
        if !asset_types.contains(&asset.asset_type) {
            asset_types.push(asset.asset_type);
        }
    }
    asset_types
        .into_iter()
        .filter_map(|asset_type| {
            let mut ranked: Vec<&LibraryAsset> = assets
                .iter()
                .filter(|asset| asset.asset_type == asset_type)
                .collect();
            ranked.sort_by(|a, b| rank(a, b));
            let (preferred, others) = ranked.split_first()?;
            Some(PreferredAsset {
                asset_type,
                asset_id: preferred.asset_id,
                outranks: others
                    .iter()
                    .map(|other| AssetPreference {
                        asset_id: other.asset_id,
                        reason: reason(preferred, other),
                    })
                    .collect(),
            })
        })
        .collect()
}

/// Orders two Assets of a type, the preferred one first.
fn rank(a: &LibraryAsset, b: &LibraryAsset) -> Ordering {
    pixel_count(b)
        .cmp(&pixel_count(a))
        .then(b.byte_len.cmp(&a.byte_len))
        .then(a.asset_id.cmp(&b.asset_id))
}

/// Why `preferred`, which ranks first, outranks `other`.
fn reason(preferred: &LibraryAsset, other: &LibraryAsset) -> PreferenceReason {
    match (pixel_count(preferred), pixel_count(other)) {
        (Some(preferred), other) if other != Some(preferred) => {
            PreferenceReason::MorePixels { preferred, other }
        }
        _ if preferred.byte_len != other.byte_len => PreferenceReason::MoreBytes {
            preferred: preferred.byte_len,
            other: other.byte_len,
        },
        _ => PreferenceReason::AcquiredFirst,
    }
}

fn pixel_count(asset: &LibraryAsset) -> Option<u64> {
    asset
        .media
        .width
        .zip(asset.media.height)
        .map(|(width, height)| u64::from(width) * u64::from(height))
}
