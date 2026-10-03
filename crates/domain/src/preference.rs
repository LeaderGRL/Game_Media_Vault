use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::{AssetType, LibraryAsset, MediaInfo, StoredObject};

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
            let mut ranked: Vec<Standing> = assets
                .iter()
                .filter(|asset| asset.asset_type == asset_type)
                .map(Standing::of)
                .collect();
            ranked.sort_by(rank);
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

/// Why a retained Asset stays preferred over a new original of its type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outranked {
    pub preferred_asset_id: i64,
    pub reason: PreferenceReason,
}

/// The retained Asset that keeps `candidate`, a new original of the same type, out of the
/// `kept` preferred originals of its type, with why: the last of the `kept` retained Assets that
/// rank above it, the Preferred Asset when one is kept. `None` when fewer retained Assets rank
/// above it. A candidate whose bytes a retained Asset already holds is that Asset, so it is
/// never outranked.
pub fn outranked_by(
    candidate: &StoredObject,
    retained: &[LibraryAsset],
    kept: usize,
) -> Option<Outranked> {
    if retained
        .iter()
        .any(|asset| asset.object_hash == candidate.hash)
    {
        return None;
    }
    // A candidate is acquired after every retained Asset.
    let candidate = Standing {
        asset_id: i64::MAX,
        byte_len: candidate.byte_len,
        pixel_count: pixel_count(&candidate.media),
    };
    let mut above: Vec<Standing> = retained
        .iter()
        .map(Standing::of)
        .filter(|standing| rank(standing, &candidate) == Ordering::Less)
        .collect();
    above.sort_by(rank);
    let last_kept = above.get(kept.max(1) - 1)?;
    Some(Outranked {
        preferred_asset_id: last_kept.asset_id,
        reason: reason(last_kept, &candidate),
    })
}

/// What preference compares about an original.
struct Standing {
    asset_id: i64,
    byte_len: u64,
    pixel_count: Option<u64>,
}

impl Standing {
    fn of(asset: &LibraryAsset) -> Self {
        Self {
            asset_id: asset.asset_id,
            byte_len: asset.byte_len,
            pixel_count: pixel_count(&asset.media),
        }
    }
}

/// Orders two originals of a type, the preferred one first.
fn rank(a: &Standing, b: &Standing) -> Ordering {
    b.pixel_count
        .cmp(&a.pixel_count)
        .then(b.byte_len.cmp(&a.byte_len))
        .then(a.asset_id.cmp(&b.asset_id))
}

/// Why `preferred`, which ranks first, outranks `other`.
fn reason(preferred: &Standing, other: &Standing) -> PreferenceReason {
    match (preferred.pixel_count, other.pixel_count) {
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

fn pixel_count(media: &MediaInfo) -> Option<u64> {
    media
        .width
        .zip(media.height)
        .map(|(width, height)| u64::from(width) * u64::from(height))
}
