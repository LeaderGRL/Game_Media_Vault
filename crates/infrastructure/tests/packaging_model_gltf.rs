use std::io::Cursor;

use game_media_vault_application::{PackagingModelPort, PackagingScan, PackagingScans};
use game_media_vault_domain::PackagingTemplate;
use game_media_vault_infrastructure::GltfPackagingBuilder;
use image::{ImageFormat, Rgba, RgbaImage};
use serde_json::Value;

/// A PNG scan of `width` by `height` pixels, transparent when `alpha` is below 255.
fn png(width: u32, height: u32, alpha: u8) -> Vec<u8> {
    let image = RgbaImage::from_pixel(width, height, Rgba([200, 40, 40, alpha]));
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

fn build(
    builder: &GltfPackagingBuilder,
    front: &[u8],
    back: &[u8],
    spine: &[u8],
) -> Result<Vec<u8>, String> {
    let (mut front, mut back, mut spine) = (front, back, spine);
    builder
        .build(
            PackagingTemplate::CardboardBox,
            PackagingScans {
                front: PackagingScan {
                    bytes: &mut front,
                    media_type: "image/png",
                },
                back: PackagingScan {
                    bytes: &mut back,
                    media_type: "image/png",
                },
                spine: PackagingScan {
                    bytes: &mut spine,
                    media_type: "image/png",
                },
            },
        )
        .map_err(|error| error.message().to_owned())
}

/// The JSON and binary chunks of a GLB file, checking its header.
fn chunks(glb: &[u8]) -> (Value, &[u8]) {
    let word = |offset: usize| u32::from_le_bytes(glb[offset..offset + 4].try_into().unwrap());
    assert_eq!(&glb[0..4], b"glTF");
    assert_eq!(word(4), 2);
    assert_eq!(word(8) as usize, glb.len());
    let json_len = word(12) as usize;
    assert_eq!(&glb[16..20], b"JSON");
    let json = serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    let bin_start = 20 + json_len;
    let bin_len = word(bin_start) as usize;
    assert_eq!(&glb[bin_start + 4..bin_start + 8], b"BIN\0");
    (json, &glb[bin_start + 8..bin_start + 8 + bin_len])
}

/// The bytes of the bufferView `index`.
fn view<'a>(json: &Value, bin: &'a [u8], index: &Value) -> &'a [u8] {
    let view = &json["bufferViews"][index.as_u64().unwrap() as usize];
    let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
    &bin[offset..offset + view["byteLength"].as_u64().unwrap() as usize]
}

/// The extent of the box along each axis, from the bounds of its positions.
fn extent(json: &Value) -> [f64; 3] {
    let mut min = [f64::MAX; 3];
    let mut max = [f64::MIN; 3];
    for primitive in json["meshes"][0]["primitives"].as_array().unwrap() {
        let accessor =
            &json["accessors"][primitive["attributes"]["POSITION"].as_u64().unwrap() as usize];
        for axis in 0..3 {
            min[axis] = min[axis].min(accessor["min"][axis].as_f64().unwrap());
            max[axis] = max[axis].max(accessor["max"][axis].as_f64().unwrap());
        }
    }
    [0, 1, 2].map(|axis| max[axis] - min[axis])
}

/// The decoded texture of the material named `name`.
fn texture(json: &Value, bin: &[u8], name: &str) -> image::DynamicImage {
    let material = json["materials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|material| material["name"] == name)
        .unwrap();
    let texture = &json["textures"][material["pbrMetallicRoughness"]["baseColorTexture"]["index"]
        .as_u64()
        .unwrap() as usize];
    let image = &json["images"][texture["source"].as_u64().unwrap() as usize];
    image::load_from_memory(view(json, bin, &image["bufferView"])).unwrap()
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-6
}

#[test]
fn a_box_is_a_gltf_binary_textured_with_its_three_scans() {
    let glb = build(
        &GltfPackagingBuilder::new(),
        &png(60, 80, 255),
        &png(60, 80, 255),
        &png(10, 80, 255),
    )
    .unwrap();

    let (json, bin) = chunks(&glb);
    assert_eq!(json["asset"]["version"], "2.0");
    assert_eq!(
        json["buffers"][0]["byteLength"].as_u64().unwrap() as usize,
        bin.len()
    );
    // The front, the back, the spine on both sides, and plain top and bottom edges.
    assert_eq!(json["meshes"][0]["primitives"].as_array().unwrap().len(), 4);
    for name in ["front", "back", "spine"] {
        let scan = texture(&json, bin, name);
        assert!(scan.width() > 0, "{name}");
    }
    assert_eq!(json["images"].as_array().unwrap().len(), 3);
}

#[test]
fn the_box_takes_its_proportions_from_the_front_and_spine_scans() {
    let glb = build(
        &GltfPackagingBuilder::new(),
        &png(60, 80, 255),
        &png(61, 79, 255),
        &png(10, 80, 255),
    )
    .unwrap();

    let [width, height, depth] = extent(&chunks(&glb).0);

    // One unit tall, as wide as the front and as deep as the spine for that height.
    assert!(close(height, 1.0), "{height}");
    assert!(close(width, 0.75), "{width}");
    assert!(close(depth, 0.125), "{depth}");
}

#[test]
fn the_same_scans_always_give_the_same_model() {
    let builder = GltfPackagingBuilder::new();
    let scans = (png(60, 80, 255), png(60, 80, 255), png(10, 80, 255));

    let first = build(&builder, &scans.0, &scans.1, &scans.2).unwrap();
    let again = build(&builder, &scans.0, &scans.1, &scans.2).unwrap();

    assert_eq!(first, again);
}

#[test]
fn a_spine_scanned_lying_down_is_turned_upright() {
    let glb = build(
        &GltfPackagingBuilder::new(),
        &png(60, 80, 255),
        &png(60, 80, 255),
        &png(80, 10, 255),
    )
    .unwrap();

    let (json, bin) = chunks(&glb);
    let spine = texture(&json, bin, "spine");
    assert_eq!((spine.width(), spine.height()), (10, 80));
    assert!(close(extent(&json)[2], 0.125));
}

#[test]
fn textures_fit_within_the_largest_edge_and_keep_transparency_lossless() {
    let glb = build(
        &GltfPackagingBuilder::with_limits(1024 * 1024, 32),
        &png(60, 80, 255),
        &png(60, 80, 128),
        &png(10, 80, 255),
    )
    .unwrap();

    let (json, bin) = chunks(&glb);
    let front = texture(&json, bin, "front");
    assert_eq!((front.width(), front.height()), (24, 32));
    // Opaque scans are compressed as JPEG; a scan with transparency stays PNG.
    let mime_of = |name: &str| {
        let material = json["materials"]
            .as_array()
            .unwrap()
            .iter()
            .find(|material| material["name"] == name)
            .unwrap();
        let texture = &json["textures"]
            [material["pbrMetallicRoughness"]["baseColorTexture"]["index"]
                .as_u64()
                .unwrap() as usize];
        json["images"][texture["source"].as_u64().unwrap() as usize]["mimeType"].clone()
    };
    assert_eq!(mime_of("front"), "image/jpeg");
    assert_eq!(mime_of("back"), "image/png");
}

#[test]
fn a_scan_that_cannot_be_read_names_its_slot() {
    let error = build(
        &GltfPackagingBuilder::new(),
        &png(60, 80, 255),
        b"not an image",
        &png(10, 80, 255),
    )
    .unwrap_err();

    assert!(error.starts_with("back scan: "), "{error}");
}

#[test]
fn a_scan_larger_than_the_bound_is_refused_before_decoding() {
    let error = build(
        &GltfPackagingBuilder::with_limits(64, 2048),
        &png(60, 80, 255),
        &png(60, 80, 255),
        &png(10, 80, 255),
    )
    .unwrap_err();

    assert!(error.starts_with("front scan: "), "{error}");
    assert!(error.contains("larger than 64 bytes"), "{error}");
}
