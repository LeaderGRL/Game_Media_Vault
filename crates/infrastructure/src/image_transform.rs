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

/// Originals larger than this are refused rather than read into memory.
pub const DEFAULT_MAX_ORIGINAL_BYTES: u64 = 256 * 1024 * 1024;

/// Applies recipes to raster images with the `image` crate. Outputs are deterministic: the
/// same original and recipe always give the same bytes. Decoding keeps to the allocation
/// limit of the `image` crate.
pub struct ImageTransformer {
    max_original_bytes: u64,
}

impl ImageTransformer {
    pub fn new() -> Self {
        Self::with_max_original_bytes(DEFAULT_MAX_ORIGINAL_BYTES)
    }

    pub fn with_max_original_bytes(max_original_bytes: u64) -> Self {
        Self { max_original_bytes }
    }
}

impl Default for ImageTransformer {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaTransformPort for ImageTransformer {
    fn can_transform(&self, media_type: &str, recipe: &DerivationRecipe) -> bool {
        match recipe {
            DerivationRecipe::Thumbnail { .. } => DECODABLE_MEDIA_TYPES.contains(&media_type),
            // A packaging model reads several originals, never one alone.
            DerivationRecipe::PackagingModel { .. } => false,
        }
    }

    fn transform(
        &self,
        original: &mut dyn Read,
        media_type: &str,
        recipe: &DerivationRecipe,
    ) -> Result<Vec<u8>, PortError> {
        let DerivationRecipe::Thumbnail { max_edge } = recipe else {
            return Err(PortError::new(
                "a packaging model is built from several originals".to_owned(),
            ));
        };
        let mut bytes = Vec::new();
        original
            .take(self.max_original_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| PortError::new(format!("failed to read the original: {error}")))?;
        if bytes.len() as u64 > self.max_original_bytes {
            return Err(PortError::new(format!(
                "the original is larger than {} bytes",
                self.max_original_bytes
            )));
        }
        let image = match ImageFormat::from_mime_type(media_type) {
            // Formats without a signature to guess from, such as TGA, need the recorded one.
            Some(format) => image::load_from_memory_with_format(&bytes, format),
            None => image::load_from_memory(&bytes),
        }
        .map_err(|error| PortError::new(format!("cannot decode the original: {error}")))?;
        let (width, height) = image.dimensions();
        let output = if width.max(height) > *max_edge {
            // Fits both edges within the bound while keeping the aspect ratio.
            image.resize(*max_edge, *max_edge, FilterType::Lanczos3)
        } else {
            image
        };
        let mut encoded = Vec::new();
        output
            .write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)
            .map_err(|error| PortError::new(format!("failed to encode the output: {error}")))?;
        Ok(encoded)
    }
}
