use std::io::Cursor;

use game_media_vault_application::MediaTransformPort;
use game_media_vault_domain::DerivationRecipe;
use game_media_vault_infrastructure::ImageTransformer;
use image::{GenericImageView, ImageFormat, RgbImage};

const THUMBNAIL: DerivationRecipe = DerivationRecipe::Thumbnail { max_edge: 100 };

fn png(width: u32, height: u32) -> Vec<u8> {
    encoded(width, height, ImageFormat::Png)
}

fn encoded(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
    let mut bytes = Vec::new();
    RgbImage::from_fn(width, height, |x, y| image::Rgb([x as u8, y as u8, 128]))
        .write_to(&mut Cursor::new(&mut bytes), format)
        .unwrap();
    bytes
}

fn thumbnail(original: &[u8]) -> image::DynamicImage {
    thumbnail_of(original, "image/png")
}

fn thumbnail_of(original: &[u8], media_type: &str) -> image::DynamicImage {
    let output = ImageTransformer::new()
        .transform(&mut &original[..], media_type, &THUMBNAIL)
        .unwrap();
    image::load_from_memory_with_format(&output, ImageFormat::Png).unwrap()
}

#[test]
fn thumbnails_fit_their_longest_edge_and_keep_the_aspect_ratio() {
    assert_eq!(thumbnail(&png(400, 300)).dimensions(), (100, 75));
    assert_eq!(thumbnail(&png(150, 600)).dimensions(), (25, 100));
}

#[test]
fn smaller_images_keep_their_size() {
    assert_eq!(thumbnail(&png(64, 48)).dimensions(), (64, 48));
}

#[test]
fn the_same_original_always_gives_the_same_thumbnail() {
    let original = png(400, 300);
    let first = ImageTransformer::new()
        .transform(&mut &original[..], "image/png", &THUMBNAIL)
        .unwrap();
    let second = ImageTransformer::new()
        .transform(&mut &original[..], "image/png", &THUMBNAIL)
        .unwrap();

    assert_eq!(first, second);
}

#[test]
fn only_decodable_images_are_transformed() {
    assert!(ImageTransformer::new().can_transform("image/png", &THUMBNAIL));
    assert!(ImageTransformer::new().can_transform("image/jpeg", &THUMBNAIL));
    assert!(!ImageTransformer::new().can_transform("application/pdf", &THUMBNAIL));
    assert!(!ImageTransformer::new().can_transform("image/jxl", &THUMBNAIL));
    assert!(
        ImageTransformer::new()
            .transform(&mut &b"not an image"[..], "image/png", &THUMBNAIL)
            .is_err()
    );
}

#[test]
fn originals_without_a_signature_decode_with_their_recorded_format() {
    // A TGA file need not start with any signature to guess its format from.
    let original = encoded(400, 300, ImageFormat::Tga);

    assert_eq!(
        thumbnail_of(&original, "image/x-tga").dimensions(),
        (100, 75)
    );
}

#[test]
fn originals_beyond_the_size_bound_are_refused_before_being_read_whole() {
    let original = png(400, 300);
    let transformer = ImageTransformer::with_max_original_bytes(original.len() as u64 - 1);

    let error = transformer
        .transform(&mut &original[..], "image/png", &THUMBNAIL)
        .unwrap_err();

    assert!(
        error.message().contains("larger than"),
        "{}",
        error.message()
    );
}
