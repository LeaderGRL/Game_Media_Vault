use game_media_vault_domain::MediaInfo;
use imagesize::{Compression, ImageType};

mod boxes;
mod heif;
mod jpeg;
mod jxl;
mod pdf;
mod pnm;
mod tiff;

use heif::HeifPropertyScanner;
use jpeg::JpegFrameScanner;
use jxl::JxlCodestreamScanner;
pub(crate) use pdf::document_metadata;
use pnm::PnmHeaderScanner;
use tiff::TiffDirectoryScanner;

/// Bytes read from the start of an original to identify it. JPEG frame headers beyond them are
/// found by `JpegFrameScanner` while the bytes stream past.
const MEDIA_HEADER_BYTES: usize = 256 * 1024;

/// Identifies an original from its bytes: its media type and, for images, their pixel size.
/// It reads them as storing does, so both always agree.
pub fn inspect_media(bytes: &[u8]) -> MediaInfo {
    let mut inspector = MediaInspector::default();
    inspector.update(bytes);
    inspector.finish()
}

/// Identifies an original from its first bytes: its media type and, for images whose header
/// fits in `header`, their pixel size.
fn inspect_header(header: &[u8]) -> MediaInfo {
    for (signature, media_type) in [
        (&b"%PDF-"[..], "application/pdf"),
        // glTF 2.0 binary models, such as generated packaging.
        (&b"glTF"[..], "model/gltf-binary"),
    ] {
        if header.starts_with(signature) {
            return MediaInfo {
                media_type: media_type.to_owned(),
                width: None,
                height: None,
                document: None,
            };
        }
    }
    if let Some(media_type) = video_media_type(header) {
        return MediaInfo {
            media_type: media_type.to_owned(),
            width: None,
            height: None,
            document: None,
        };
    }
    let Ok(image_type) = imagesize::image_type(header) else {
        return MediaInfo::unknown();
    };
    let Some(media_type) = image_media_type(image_type) else {
        return MediaInfo::unknown();
    };
    let size = imagesize::blob_size(header).ok();
    MediaInfo {
        media_type: media_type.to_owned(),
        width: size.and_then(|size| u32::try_from(size.width).ok()),
        height: size.and_then(|size| u32::try_from(size.height).ok()),
        document: None,
    }
}

/// The media type of a video container the header opens: an ISO base media file whose `ftyp`
/// box names no image brand, or a Matroska or WebM file.
fn video_media_type(header: &[u8]) -> Option<&'static str> {
    if header.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        // Players read WebM, the Matroska profile videos are published in, by this name.
        return Some("video/webm");
    }
    let brand = header.get(8..12)?;
    // HEIF and AVIF images share the container, and their brands say so.
    let image_brands: [&[u8]; 8] = [
        b"heic", b"heix", b"heim", b"heis", b"hevc", b"mif1", b"msf1", b"avif",
    ];
    if header.get(4..8) == Some(&b"ftyp"[..]) && !image_brands.contains(&brand) {
        return Some(if brand == b"qt  " {
            "video/quicktime"
        } else {
            "video/mp4"
        });
    }
    None
}

fn image_media_type(image_type: ImageType) -> Option<&'static str> {
    match image_type {
        ImageType::Png => Some("image/png"),
        ImageType::Jpeg => Some("image/jpeg"),
        ImageType::Gif => Some("image/gif"),
        ImageType::Webp => Some("image/webp"),
        ImageType::Bmp => Some("image/bmp"),
        ImageType::Tiff => Some("image/tiff"),
        ImageType::Jxl => Some("image/jxl"),
        ImageType::Ico => Some("image/x-icon"),
        ImageType::Heif(Compression::Av1) => Some("image/avif"),
        ImageType::Heif(Compression::Hevc) => Some("image/heic"),
        ImageType::Heif(_) => Some("image/heif"),
        ImageType::Pnm => Some(PORTABLE_ANYMAP),
        ImageType::Qoi => Some("image/qoi"),
        ImageType::Tga => Some("image/x-tga"),
        ImageType::Farbfeld => Some("image/x-farbfeld"),
        ImageType::Ilbm => Some("image/x-ilbm"),
        ImageType::Exr => Some("image/x-exr"),
        ImageType::Hdr => Some("image/vnd.radiance"),
        // GPU texture containers and layered editing documents are kept as opaque originals.
        _ => None,
    }
}

const PORTABLE_ANYMAP: &str = "image/x-portable-anymap";

/// Inspects an original while it streams: it keeps a bounded prefix for format detection and
/// follows JPEG segments, the first TIFF image file directory, HEIF and JPEG XL container
/// boxes and portable anymap headers past it.
#[derive(Default)]
pub(crate) struct MediaInspector {
    header: Vec<u8>,
    jpeg: JpegFrameScanner,
    tiff: TiffDirectoryScanner,
    heif: HeifPropertyScanner,
    jxl: JxlCodestreamScanner,
    pnm: PnmHeaderScanner,
}

impl MediaInspector {
    pub(crate) fn update(&mut self, chunk: &[u8]) {
        let room = MEDIA_HEADER_BYTES.saturating_sub(self.header.len());
        self.header
            .extend_from_slice(&chunk[..chunk.len().min(room)]);
        self.jpeg.update(chunk);
        self.tiff.update(chunk);
        self.heif.update(chunk);
        self.jxl.update(chunk);
        self.pnm.update(chunk);
    }

    pub(crate) fn finish(self) -> MediaInfo {
        let mut media = inspect_header(&self.header);
        let streamed_size = match media.media_type.as_str() {
            "image/jpeg" => self.jpeg.size(),
            "image/tiff" if media.width.is_none() => self.tiff.size(),
            "image/avif" | "image/heic" | "image/heif" if media.width.is_none() => self.heif.size(),
            "image/jxl" if media.width.is_none() => self.jxl.size(),
            // The portable anymap signature is two letters, so only a header read to its
            // dimensions identifies one.
            PORTABLE_ANYMAP => match self.pnm.size() {
                Some(size) => Some(size),
                None => return MediaInfo::unknown(),
            },
            _ => None,
        };
        if let Some((width, height)) = streamed_size {
            media.width = Some(width);
            media.height = Some(height);
        }
        media
    }
}
