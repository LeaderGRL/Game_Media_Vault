use serde::{Deserialize, Serialize};

use crate::{MediaInfo, PackagingFamily};

/// A reproducible transformation of an original. The same original and recipe always give the
/// same Derived Asset, which is generated once and reused.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "transform", rename_all = "snake_case")]
pub enum DerivationRecipe {
    /// A PNG image whose longest edge is at most `max_edge` pixels; smaller images keep their
    /// size.
    Thumbnail { max_edge: u32 },
    /// A 3D model of the packaging whose front scan is the original, textured with that scan
    /// and the exact back and spine scans named here.
    PackagingModel {
        template: PackagingTemplate,
        back_hash: String,
        spine_hash: String,
    },
}

impl DerivationRecipe {
    /// Stable identity of the recipe, under which its outputs are recorded.
    pub fn key(&self) -> String {
        match self {
            Self::Thumbnail { max_edge } => format!("thumbnail-png-{max_edge}"),
            Self::PackagingModel {
                template,
                back_hash,
                spine_hash,
            } => format!(
                "packaging-model-{}-{back_hash}-{spine_hash}",
                template.key()
            ),
        }
    }
}

/// The geometry and texture slots of generated 3D packaging for a packaging family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackagingTemplate {
    /// A box textured with its front, back and spine, the spine on both sides.
    CardboardBox,
}

impl PackagingTemplate {
    /// The template that generates packaging of `family`, when one exists yet.
    pub fn for_family(family: PackagingFamily) -> Option<Self> {
        match family {
            PackagingFamily::CardboardBox => Some(Self::CardboardBox),
            PackagingFamily::JewelCase
            | PackagingFamily::KeepCase
            | PackagingFamily::CartridgeCase
            | PackagingFamily::ArcadeBoard
            | PackagingFamily::DigitalOnly => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::CardboardBox => "cardboard-box",
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
