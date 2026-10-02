use serde::{Deserialize, Serialize};

use crate::MediaInfo;

/// A reproducible transformation of an original. The same original and recipe always give the
/// same Derived Asset, which is generated once and reused.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "transform", rename_all = "snake_case")]
pub enum DerivationRecipe {
    /// A PNG image whose longest edge is at most `max_edge` pixels; smaller images keep their
    /// size.
    Thumbnail { max_edge: u32 },
}

impl DerivationRecipe {
    /// Stable identity of the recipe, under which its outputs are recorded.
    pub fn key(&self) -> String {
        match self {
            Self::Thumbnail { max_edge } => format!("thumbnail-png-{max_edge}"),
        }
    }
}

/// A file generated from an original by a recipe. It never replaces its original.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedAsset {
    pub recipe: DerivationRecipe,
    pub object_hash: String,
    pub byte_len: u64,
    #[serde(flatten)]
    pub media: MediaInfo,
}
