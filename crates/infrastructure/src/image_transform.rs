use std::io::{Cursor, Read};

use game_media_vault_application::{MediaTransformPort, PortError};
use game_media_vault_domain::DerivationRecipe;
use image::{GenericImageView, ImageFormat, imageops::FilterType};

/// Media types of the originals this transformer decodes.
const DECODABLE_MEDIA_TYPES: &[&str] = &[
    "image/bmp",
    "image/gif",
    "image/jpeg",
    "image/png",
    "image/qoi",
    "image/tiff",
    "image/webp",
    "image/x-icon",
    "image/x-portable-anymap",
    "image/x-tga",
];

/// Applies recipes to raster images with the `image` crate. Outputs are deterministic: the
/// same original and recipe always give the same bytes.
pub struct ImageTransformer;

impl MediaTransformPort for ImageTransformer {
    fn can_transform(&self, media_type: &str, recipe: &DerivationRecipe) -> bool {
        match recipe {
            DerivationRecipe::Thumbnail { .. } => DECODABLE_MEDIA_TYPES.contains(&media_type),
        }
    }

    fn transform(
        &self,
        original: &mut dyn Read,
        _media_type: &str,
        recipe: &DerivationRecipe,
    ) -> Result<Vec<u8>, PortError> {
        let mut bytes = Vec::new();
        original
            .read_to_end(&mut bytes)
            .map_err(|error| PortError::new(format!("failed to read the original: {error}")))?;
        let image = image::load_from_memory(&bytes)
            .map_err(|error| PortError::new(format!("cannot decode the original: {error}")))?;
        let output = match recipe {
            DerivationRecipe::Thumbnail { max_edge } => {
                let (width, height) = image.dimensions();
                if width.max(height) > *max_edge {
                    // Fits both edges within the bound while keeping the aspect ratio.
                    image.resize(*max_edge, *max_edge, FilterType::Lanczos3)
                } else {
                    image
                }
            }
        };
        let mut encoded = Vec::new();
        output
            .write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)
            .map_err(|error| PortError::new(format!("failed to encode the output: {error}")))?;
        Ok(encoded)
    }
}
