use std::{collections::HashSet, io::Read};

use game_media_vault_domain::{
    AssetType, AssetTypeSelector, CoverageProfile, DerivationRecipe, LibraryAsset, LibraryRelease,
    PackagingTemplate,
};
use serde::Serialize;

use crate::{ApplicationError, CatalogPort, DerivativeRepositoryPort, DerivedStorePort, PortError};

/// The bytes of one scan a packaging model is textured with, and the media type to read them
/// as.
pub struct PackagingScan<'a> {
    pub bytes: &'a mut dyn Read,
    pub media_type: &'a str,
}

/// The scans of the texture slots of a packaging template.
pub struct PackagingScans<'a> {
    pub front: PackagingScan<'a>,
    pub back: PackagingScan<'a>,
    pub spine: PackagingScan<'a>,
}

/// Builds 3D packaging models from scans.
pub trait PackagingModelPort {
    /// Builds the model of `template` textured with `scans`; the same scans always give the
    /// same bytes.
    fn build(
        &self,
        template: PackagingTemplate,
        scans: PackagingScans<'_>,
    ) -> Result<Vec<u8>, PortError>;
}

/// A release a template fits whose Packaging Coverage Profile still misses Asset Types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IncompletePackaging {
    pub release_edition_id: i64,
    pub missing: Vec<AssetTypeSelector>,
}

/// A release whose model could not be built; its scans are left untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackagingModelFailure {
    pub release_edition_id: i64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackagingModelSummary {
    /// Models built and recorded.
    pub generated: usize,
    /// Releases whose model from their Preferred Assets already exists.
    pub up_to_date: usize,
    pub incomplete: Vec<IncompletePackaging>,
    /// Releases whose packaging family is unknown or has no template yet.
    pub without_template: usize,
    pub failed: Vec<PackagingModelFailure>,
}

/// The Preferred Assets a release's model is textured with, and its recipe.
struct ModelBasis<'a> {
    front: &'a LibraryAsset,
    back: &'a LibraryAsset,
    spine: &'a LibraryAsset,
    template: PackagingTemplate,
    recipe: DerivationRecipe,
}

/// Builds the 3D packaging model of every release whose Packaging Coverage Profile is complete
/// and whose packaging family has a template, from the Preferred Asset of each texture slot.
/// A model is a Derived Asset of the front scan whose recipe names the other scans exactly, so
/// it is built once per set of scans. Scans are only read: a build that fails is reported and
/// the other releases still build, while storage failures stop the generation.
pub fn derive_packaging_models(
    catalog: &dyn CatalogPort,
    derivatives: &dyn DerivativeRepositoryPort,
    store: &dyn DerivedStorePort,
    builder: &dyn PackagingModelPort,
) -> Result<PackagingModelSummary, ApplicationError> {
    let mut summary = PackagingModelSummary {
        generated: 0,
        up_to_date: 0,
        incomplete: Vec::new(),
        without_template: 0,
        failed: Vec::new(),
    };
    // Releases sharing their scans share their model.
    let mut built: HashSet<(String, String)> = HashSet::new();
    for entry in catalog.list_library()? {
        let release = LibraryRelease::from(entry);
        let basis = match model_basis(&release) {
            Basis::Ready(basis) => basis,
            Basis::Incomplete(missing) => {
                summary.incomplete.push(IncompletePackaging {
                    release_edition_id: release.entry.release_edition_id,
                    missing,
                });
                continue;
            }
            Basis::WithoutTemplate => {
                summary.without_template += 1;
                continue;
            }
        };
        let key = (basis.front.object_hash.clone(), basis.recipe.key());
        if built.contains(&key)
            || basis
                .front
                .derived
                .iter()
                .any(|derived| derived.recipe == basis.recipe)
        {
            summary.up_to_date += 1;
            continue;
        }
        match build_model(store, builder, &basis) {
            Ok(bytes) => {
                let stored = store.store_derived(&mut bytes.as_slice())?;
                derivatives.record_derivative(&basis.front.object_hash, &basis.recipe, &stored)?;
                built.insert(key);
                summary.generated += 1;
            }
            Err(error) => summary.failed.push(PackagingModelFailure {
                release_edition_id: release.entry.release_edition_id,
                reason: error.message().to_owned(),
            }),
        }
    }
    Ok(summary)
}

enum Basis<'a> {
    Ready(ModelBasis<'a>),
    Incomplete(Vec<AssetTypeSelector>),
    WithoutTemplate,
}

fn model_basis(release: &LibraryRelease) -> Basis<'_> {
    let Some(coverage) = &release.coverage else {
        return Basis::WithoutTemplate;
    };
    let Some(template) = PackagingTemplate::for_family(coverage.packaging_family) else {
        return Basis::WithoutTemplate;
    };
    if let Some(packaging) = coverage
        .profiles
        .iter()
        .find(|profile| profile.profile == CoverageProfile::Packaging)
        && !packaging.missing.is_empty()
    {
        return Basis::Incomplete(packaging.missing.clone());
    }
    let preferred = |asset_type: AssetType| {
        let asset_id = release
            .preferred_assets
            .iter()
            .find(|preferred| preferred.asset_type == asset_type)?
            .asset_id;
        release
            .entry
            .assets
            .iter()
            .find(|asset| asset.asset_id == asset_id)
    };
    let slots = [
        (AssetTypeSelector::BoxFront, preferred(AssetType::BoxFront)),
        (AssetTypeSelector::BoxBack, preferred(AssetType::BoxBack)),
        (AssetTypeSelector::Spine, preferred(AssetType::Spine)),
    ];
    // A complete Packaging profile has every slot; this keeps the requirements explained if
    // the profile and the template ever disagree.
    let [(_, Some(front)), (_, Some(back)), (_, Some(spine))] = slots else {
        return Basis::Incomplete(
            slots
                .iter()
                .filter(|(_, asset)| asset.is_none())
                .map(|(selector, _)| *selector)
                .collect(),
        );
    };
    Basis::Ready(ModelBasis {
        front,
        back,
        spine,
        template,
        recipe: DerivationRecipe::PackagingModel {
            template,
            back_hash: back.object_hash.clone(),
            spine_hash: spine.object_hash.clone(),
        },
    })
}

fn build_model(
    store: &dyn DerivedStorePort,
    builder: &dyn PackagingModelPort,
    basis: &ModelBasis<'_>,
) -> Result<Vec<u8>, PortError> {
    let mut front = store.open_original(&basis.front.object_hash)?;
    let mut back = store.open_original(&basis.back.object_hash)?;
    let mut spine = store.open_original(&basis.spine.object_hash)?;
    builder.build(
        basis.template,
        PackagingScans {
            front: PackagingScan {
                bytes: front.as_mut(),
                media_type: &basis.front.media.media_type,
            },
            back: PackagingScan {
                bytes: back.as_mut(),
                media_type: &basis.back.media.media_type,
            },
            spine: PackagingScan {
                bytes: spine.as_mut(),
                media_type: &basis.spine.media.media_type,
            },
        },
    )
}
