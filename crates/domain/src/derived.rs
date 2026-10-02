use serde::{Deserialize, Serialize};

use crate::{
    AssetType, AssetTypeSelector, CoverageProfile, LibraryAsset, MediaInfo, PackagingFamily,
    PreferredAsset, ReleaseCoverage,
};

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

    /// The originals the recipe reads besides the one its output is recorded under.
    pub fn other_originals(&self) -> Vec<String> {
        match self {
            Self::Thumbnail { .. } => Vec::new(),
            Self::PackagingModel {
                back_hash,
                spine_hash,
                ..
            } => vec![back_hash.clone(), spine_hash.clone()],
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

/// What the packaging model of a release rests on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackagingModelBasis<'a> {
    Ready(PackagingModelScans<'a>),
    /// Its Packaging Coverage Profile still misses these Asset Types.
    Incomplete(Vec<AssetTypeSelector>),
    /// Its packaging family is unknown or has no template yet.
    WithoutTemplate,
}

/// The Preferred Asset of each texture slot of a packaging template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackagingModelScans<'a> {
    pub template: PackagingTemplate,
    pub front: &'a LibraryAsset,
    pub back: &'a LibraryAsset,
    pub spine: &'a LibraryAsset,
}

impl<'a> PackagingModelScans<'a> {
    /// The recipe of the model of these exact scans, recorded under the front scan.
    pub fn recipe(&self) -> DerivationRecipe {
        DerivationRecipe::PackagingModel {
            template: self.template,
            back_hash: self.back.object_hash.clone(),
            spine_hash: self.spine.object_hash.clone(),
        }
    }

    /// The model of these scans, once it has been built.
    pub fn model(&self) -> Option<&'a DerivedAsset> {
        let recipe = self.recipe();
        self.front
            .derived
            .iter()
            .find(|derived| derived.recipe == recipe)
    }
}

/// The basis of the packaging model of a release with these `assets`, Preferred Assets and
/// coverage: the template of its packaging family, textured with the Preferred Asset of each
/// slot once its Packaging Coverage Profile is complete.
pub fn packaging_model_basis<'a>(
    assets: &'a [LibraryAsset],
    preferred: &[PreferredAsset],
    coverage: Option<&ReleaseCoverage>,
) -> PackagingModelBasis<'a> {
    let Some(coverage) = coverage else {
        return PackagingModelBasis::WithoutTemplate;
    };
    let Some(template) = PackagingTemplate::for_family(coverage.packaging_family) else {
        return PackagingModelBasis::WithoutTemplate;
    };
    if let Some(packaging) = coverage
        .profiles
        .iter()
        .find(|profile| profile.profile == CoverageProfile::Packaging)
        && !packaging.missing.is_empty()
    {
        return PackagingModelBasis::Incomplete(packaging.missing.clone());
    }
    let preferred_asset = |asset_type: AssetType| {
        let asset_id = preferred
            .iter()
            .find(|preferred| preferred.asset_type == asset_type)?
            .asset_id;
        assets.iter().find(|asset| asset.asset_id == asset_id)
    };
    let slots = [
        (
            AssetTypeSelector::BoxFront,
            preferred_asset(AssetType::BoxFront),
        ),
        (
            AssetTypeSelector::BoxBack,
            preferred_asset(AssetType::BoxBack),
        ),
        (AssetTypeSelector::Spine, preferred_asset(AssetType::Spine)),
    ];
    // A complete Packaging profile has every slot; this keeps the requirements explained if
    // the profile and the template ever disagree.
    let [(_, Some(front)), (_, Some(back)), (_, Some(spine))] = slots else {
        return PackagingModelBasis::Incomplete(
            slots
                .iter()
                .filter(|(_, asset)| asset.is_none())
                .map(|(selector, _)| *selector)
                .collect(),
        );
    };
    PackagingModelBasis::Ready(PackagingModelScans {
        template,
        front,
        back,
        spine,
    })
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
