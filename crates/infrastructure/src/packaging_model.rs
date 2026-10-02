//! Generated 3D packaging, written as glTF 2.0 binary files.

use std::io::Cursor;

use game_media_vault_application::{PackagingModelPort, PackagingScan, PackagingScans, PortError};
use game_media_vault_domain::PackagingTemplate;
use image::{DynamicImage, GenericImageView, ImageFormat, codecs::jpeg::JpegEncoder};
use serde_json::{Value, json};

use crate::image_transform::{DEFAULT_MAX_ORIGINAL_BYTES, decode_original, fit_within};

/// Longest edge of a texture by default: enough to read a box's print on any GPU.
pub const DEFAULT_MAX_TEXTURE_EDGE: u32 = 2048;

/// Quality of the JPEG textures of opaque scans.
const JPEG_QUALITY: u8 = 90;

const FLOAT: u32 = 5126;
const UNSIGNED_SHORT: u32 = 5123;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;

/// Texture coordinates of a face's corners, from its bottom left counterclockwise; glTF puts
/// the top of an image at 0.
const FACE_UVS: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

/// Builds packaging models as glTF 2.0 binary (`.glb`) files that embed their textures, so a
/// model stands alone. Outputs are deterministic: the same scans always give the same bytes.
pub struct GltfPackagingBuilder {
    max_original_bytes: u64,
    max_texture_edge: u32,
}

impl GltfPackagingBuilder {
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_MAX_ORIGINAL_BYTES, DEFAULT_MAX_TEXTURE_EDGE)
    }

    /// Refuses scans larger than `max_original_bytes` and scales textures down to fit
    /// `max_texture_edge`.
    pub fn with_limits(max_original_bytes: u64, max_texture_edge: u32) -> Self {
        Self {
            max_original_bytes,
            max_texture_edge,
        }
    }

    /// The texture of a scan, with the size of the scan it shows. An upright scan is turned so
    /// that it is taller than wide.
    fn texture(
        &self,
        slot: &str,
        scan: PackagingScan<'_>,
        upright: bool,
    ) -> Result<Texture, PortError> {
        let named = |error: PortError| PortError::new(format!("{slot} scan: {}", error.message()));
        let mut image =
            decode_original(scan.bytes, scan.media_type, self.max_original_bytes).map_err(named)?;
        if upright && image.width() > image.height() {
            // A spine scanned lying down reads along the box once turned clockwise.
            image = image.rotate90();
        }
        let (width, height) = image.dimensions();
        let (mime_type, bytes) =
            encode_texture(&fit_within(image, self.max_texture_edge)).map_err(named)?;
        Ok(Texture {
            width,
            height,
            mime_type,
            bytes,
        })
    }
}

impl Default for GltfPackagingBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl PackagingModelPort for GltfPackagingBuilder {
    fn build(
        &self,
        template: PackagingTemplate,
        scans: PackagingScans<'_>,
    ) -> Result<Vec<u8>, PortError> {
        match template {
            PackagingTemplate::CardboardBox => {
                let front = self.texture("front", scans.front, false)?;
                let back = self.texture("back", scans.back, false)?;
                let spine = self.texture("spine", scans.spine, true)?;
                cardboard_box(&front, &back, &spine)
            }
        }
    }
}

struct Texture {
    width: u32,
    height: u32,
    mime_type: &'static str,
    bytes: Vec<u8>,
}

/// Encodes an opaque texture as JPEG, and one with transparency as PNG, which keeps it.
fn encode_texture(image: &DynamicImage) -> Result<(&'static str, Vec<u8>), PortError> {
    let failed =
        |error: image::ImageError| PortError::new(format!("failed to encode the texture: {error}"));
    let mut bytes = Vec::new();
    let transparent =
        image.color().has_alpha() && image.to_rgba8().pixels().any(|pixel| pixel.0[3] < u8::MAX);
    if transparent {
        image
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .map_err(failed)?;
        Ok(("image/png", bytes))
    } else {
        DynamicImage::ImageRgb8(image.to_rgb8())
            .write_with_encoder(JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY))
            .map_err(failed)?;
        Ok(("image/jpeg", bytes))
    }
}

/// A face of the model: its corners from the bottom left counterclockwise, seen from outside,
/// and the direction it faces.
struct Face {
    corners: [[f32; 3]; 4],
    normal: [f32; 3],
}

/// A box one unit tall, as wide as the front scan and as deep as the spine scan for that
/// height, centred on the origin with its front facing +Z. The spine is on both sides; the top
/// and bottom are plain.
fn cardboard_box(front: &Texture, back: &Texture, spine: &Texture) -> Result<Vec<u8>, PortError> {
    let x = front.width as f32 / front.height as f32 / 2.0;
    let y = 0.5;
    let z = spine.width as f32 / spine.height as f32 / 2.0;
    let face = |corners, normal| Face { corners, normal };
    let parts = [
        (
            "front",
            vec![face(
                [[-x, -y, z], [x, -y, z], [x, y, z], [-x, y, z]],
                [0.0, 0.0, 1.0],
            )],
        ),
        (
            "back",
            vec![face(
                [[x, -y, -z], [-x, -y, -z], [-x, y, -z], [x, y, -z]],
                [0.0, 0.0, -1.0],
            )],
        ),
        (
            "spine",
            vec![
                face(
                    [[-x, -y, -z], [-x, -y, z], [-x, y, z], [-x, y, -z]],
                    [-1.0, 0.0, 0.0],
                ),
                face(
                    [[x, -y, z], [x, -y, -z], [x, y, -z], [x, y, z]],
                    [1.0, 0.0, 0.0],
                ),
            ],
        ),
        (
            "edges",
            vec![
                face(
                    [[-x, y, z], [x, y, z], [x, y, -z], [-x, y, -z]],
                    [0.0, 1.0, 0.0],
                ),
                face(
                    [[-x, -y, -z], [x, -y, -z], [x, -y, z], [-x, -y, z]],
                    [0.0, -1.0, 0.0],
                ),
            ],
        ),
    ];
    let textures = [front, back, spine];

    let mut positions: Vec<f32> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut uvs: Vec<f32> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    let mut accessors = Vec::new();
    let mut primitives = Vec::new();
    let mut materials = Vec::new();
    for (material, (name, faces)) in parts.iter().enumerate() {
        let first_vertex = positions.len() / 3;
        let first_index = indices.len();
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for (number, face) in faces.iter().enumerate() {
            for (corner, uv) in face.corners.iter().zip(FACE_UVS) {
                positions.extend(corner);
                normals.extend(face.normal);
                uvs.extend(uv);
                for axis in 0..3 {
                    min[axis] = min[axis].min(corner[axis]);
                    max[axis] = max[axis].max(corner[axis]);
                }
            }
            let base = u16::try_from(number * 4).expect("a primitive has a few faces");
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        let vertices = faces.len() * 4;
        let first_accessor = accessors.len();
        accessors.extend([
            json!({"bufferView": 0, "byteOffset": first_vertex * 12, "componentType": FLOAT,
                   "count": vertices, "type": "VEC3", "min": min, "max": max}),
            json!({"bufferView": 1, "byteOffset": first_vertex * 12, "componentType": FLOAT,
                   "count": vertices, "type": "VEC3"}),
            json!({"bufferView": 2, "byteOffset": first_vertex * 8, "componentType": FLOAT,
                   "count": vertices, "type": "VEC2"}),
            json!({"bufferView": 3, "byteOffset": first_index * 2,
                   "componentType": UNSIGNED_SHORT, "count": faces.len() * 6,
                   "type": "SCALAR"}),
        ]);
        primitives.push(json!({
            "attributes": {
                "POSITION": first_accessor,
                "NORMAL": first_accessor + 1,
                "TEXCOORD_0": first_accessor + 2,
            },
            "indices": first_accessor + 3,
            "material": material,
        }));
        let color = match textures.get(material) {
            Some(_) => json!({"baseColorTexture": {"index": material}}),
            None => json!({"baseColorFactor": [0.9, 0.9, 0.9, 1.0]}),
        };
        let mut pbr = color;
        pbr["metallicFactor"] = json!(0.0);
        pbr["roughnessFactor"] = json!(0.8);
        materials.push(json!({"name": name, "pbrMetallicRoughness": pbr}));
    }

    let mut buffer = BinaryBuffer::default();
    buffer.push(&floats(&positions), Some(ARRAY_BUFFER));
    buffer.push(&floats(&normals), Some(ARRAY_BUFFER));
    buffer.push(&floats(&uvs), Some(ARRAY_BUFFER));
    let index_bytes: Vec<u8> = indices
        .iter()
        .flat_map(|index| index.to_le_bytes())
        .collect();
    buffer.push(&index_bytes, Some(ELEMENT_ARRAY_BUFFER));
    let images: Vec<Value> = textures
        .iter()
        .map(|texture| {
            let view = buffer.push(&texture.bytes, None);
            json!({"bufferView": view, "mimeType": texture.mime_type})
        })
        .collect();

    let document = json!({
        "asset": {"version": "2.0", "generator": "Game Media Vault"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "packaging", "mesh": 0}],
        "meshes": [{"name": "packaging", "primitives": primitives}],
        "materials": materials,
        "textures": (0..textures.len())
            .map(|source| json!({"sampler": 0, "source": source}))
            .collect::<Vec<_>>(),
        // Linear filtering with mipmaps, clamped so a face never samples the opposite edge.
        "samplers": [{"magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071}],
        "images": images,
        "accessors": accessors,
        "bufferViews": buffer.views,
        "buffers": [{"byteLength": buffer.bytes.len()}],
    });
    glb(&document, &buffer.bytes)
}

fn floats(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// The binary chunk of a GLB file and the views into it, each aligned to four bytes.
#[derive(Default)]
struct BinaryBuffer {
    bytes: Vec<u8>,
    views: Vec<Value>,
}

impl BinaryBuffer {
    /// Appends `data` as a new view and returns its index.
    fn push(&mut self, data: &[u8], target: Option<u32>) -> usize {
        let mut view =
            json!({"buffer": 0, "byteOffset": self.bytes.len(), "byteLength": data.len()});
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        self.bytes.extend_from_slice(data);
        self.bytes.resize(self.bytes.len().next_multiple_of(4), 0);
        self.views.push(view);
        self.views.len() - 1
    }
}

/// Assembles a GLB file from its JSON document and its binary chunk.
fn glb(document: &Value, binary: &[u8]) -> Result<Vec<u8>, PortError> {
    let mut json = serde_json::to_vec(document)
        .map_err(|error| PortError::new(format!("failed to write the model: {error}")))?;
    // The JSON chunk is padded with spaces to keep the binary chunk aligned.
    json.resize(json.len().next_multiple_of(4), b' ');
    let length = |bytes: usize| {
        u32::try_from(bytes).map_err(|_| PortError::new("the model is larger than 4 GiB".into()))
    };
    let total = length(12 + 8 + json.len() + 8 + binary.len())?;
    let mut file = Vec::with_capacity(total as usize);
    file.extend_from_slice(b"glTF");
    file.extend_from_slice(&2_u32.to_le_bytes());
    file.extend_from_slice(&total.to_le_bytes());
    file.extend_from_slice(&length(json.len())?.to_le_bytes());
    file.extend_from_slice(b"JSON");
    file.extend_from_slice(&json);
    file.extend_from_slice(&length(binary.len())?.to_le_bytes());
    file.extend_from_slice(b"BIN\0");
    file.extend_from_slice(binary);
    Ok(file)
}
