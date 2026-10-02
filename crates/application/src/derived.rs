use std::io::Read;

use game_media_vault_domain::{DerivationRecipe, StoredObject};
use serde::Serialize;

use crate::{ApplicationError, PortError};

/// A stored original a recipe can be applied to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalObject {
    pub hash: String,
    /// Every media type recorded for the Assets sharing these bytes, sorted: one may be
    /// unidentified while another names the format.
    pub media_types: Vec<String>,
}

/// Derived Assets: reproducible outputs of an original object and a recipe.
pub trait DerivativeRepositoryPort {
    /// The originals of retained Assets that have no output of `recipe` yet.
    fn originals_without(
        &self,
        recipe: &DerivationRecipe,
    ) -> Result<Vec<OriginalObject>, PortError>;

    /// Records `output` as the result of `recipe` on the original `original_hash`; recording
    /// it again changes nothing.
    fn record_derivative(
        &self,
        original_hash: &str,
        recipe: &DerivationRecipe,
        output: &StoredObject,
    ) -> Result<(), PortError>;
}

/// Reads stored originals and stores derived outputs apart from them.
pub trait DerivedStorePort {
    fn open_original(&self, hash: &str) -> Result<Box<dyn Read + Send>, PortError>;

    /// Stores a derived output, content-addressed like originals but never among them.
    fn store_derived(&self, reader: &mut dyn Read) -> Result<StoredObject, PortError>;
}

/// Applies recipes to original bytes.
pub trait MediaTransformPort {
    /// Whether originals of `media_type` can go through `recipe`.
    fn can_transform(&self, media_type: &str, recipe: &DerivationRecipe) -> bool;

    /// Applies `recipe` to `original`, whose bytes are of `media_type`.
    fn transform(
        &self,
        original: &mut dyn Read,
        media_type: &str,
        recipe: &DerivationRecipe,
    ) -> Result<Vec<u8>, PortError>;
}

/// An original whose transformation failed; the original itself is left untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DerivationFailure {
    pub original_hash: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DerivationSummary {
    /// Derived Assets generated and recorded.
    pub derived: usize,
    /// Originals the transformer cannot read for this recipe.
    pub skipped: usize,
    pub failed: Vec<DerivationFailure>,
}

/// Generates the output of `recipe` for every retained original that lacks it. Originals are
/// only read: a transformation that fails is reported and the others still run, while storage
/// failures stop the generation.
pub fn derive_assets(
    derivatives: &dyn DerivativeRepositoryPort,
    store: &dyn DerivedStorePort,
    transformer: &dyn MediaTransformPort,
    recipe: &DerivationRecipe,
) -> Result<DerivationSummary, ApplicationError> {
    if *recipe == (DerivationRecipe::Thumbnail { max_edge: 0 }) {
        return Err(ApplicationError::InvalidThumbnailEdge);
    }
    let mut summary = DerivationSummary {
        derived: 0,
        skipped: 0,
        failed: Vec::new(),
    };
    for original in derivatives.originals_without(recipe)? {
        let Some(media_type) = original
            .media_types
            .iter()
            .find(|media_type| transformer.can_transform(media_type, recipe))
        else {
            summary.skipped += 1;
            continue;
        };
        let output = store
            .open_original(&original.hash)
            .and_then(|mut bytes| transformer.transform(bytes.as_mut(), media_type, recipe));
        match output {
            Ok(bytes) => {
                let stored = store.store_derived(&mut bytes.as_slice())?;
                derivatives.record_derivative(&original.hash, recipe, &stored)?;
                summary.derived += 1;
            }
            Err(error) => summary.failed.push(DerivationFailure {
                original_hash: original.hash,
                reason: error.message().to_owned(),
            }),
        }
    }
    Ok(summary)
}
