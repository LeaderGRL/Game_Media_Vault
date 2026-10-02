use std::{collections::HashSet, io::Read};

use game_media_vault_domain::{
    AssetTypeSelector, LibraryRelease, PackagingModelBasis, PackagingModelScans, PackagingTemplate,
    packaging_model_basis,
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
        let release_edition_id = release.entry.release_edition_id;
        let scans = match packaging_model_basis(
            &release.entry.assets,
            &release.preferred_assets,
            release.coverage.as_ref(),
        ) {
            PackagingModelBasis::Ready(scans) => scans,
            PackagingModelBasis::Incomplete(missing) => {
                summary.incomplete.push(IncompletePackaging {
                    release_edition_id,
                    missing,
                });
                continue;
            }
            PackagingModelBasis::WithoutTemplate => {
                summary.without_template += 1;
                continue;
            }
        };
        let recipe = scans.recipe();
        let key = (scans.front.object_hash.clone(), recipe.key());
        if built.contains(&key) || scans.model().is_some() {
            summary.up_to_date += 1;
            continue;
        }
        match build_model(store, builder, &scans) {
            Ok(bytes) => {
                let stored = store.store_derived(&mut bytes.as_slice())?;
                derivatives.record_derivative(&scans.front.object_hash, &recipe, &stored)?;
                built.insert(key);
                summary.generated += 1;
            }
            Err(error) => summary.failed.push(PackagingModelFailure {
                release_edition_id,
                reason: error.message().to_owned(),
            }),
        }
    }
    Ok(summary)
}

fn build_model(
    store: &dyn DerivedStorePort,
    builder: &dyn PackagingModelPort,
    scans: &PackagingModelScans<'_>,
) -> Result<Vec<u8>, PortError> {
    let mut front = store.open_original(&scans.front.object_hash)?;
    let mut back = store.open_original(&scans.back.object_hash)?;
    let mut spine = store.open_original(&scans.spine.object_hash)?;
    builder.build(
        scans.template,
        PackagingScans {
            front: PackagingScan {
                bytes: front.as_mut(),
                media_type: &scans.front.media.media_type,
            },
            back: PackagingScan {
                bytes: back.as_mut(),
                media_type: &scans.back.media.media_type,
            },
            spine: PackagingScan {
                bytes: spine.as_mut(),
                media_type: &scans.spine.media.media_type,
            },
        },
    )
}
