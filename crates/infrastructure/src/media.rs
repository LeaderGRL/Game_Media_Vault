use game_media_vault_domain::MediaInfo;
use imagesize::{Compression, ImageType};

/// Bytes read from the start of an original to identify it. JPEG frame headers can follow
/// large metadata segments, so this covers several of them.
pub(crate) const MEDIA_HEADER_BYTES: usize = 256 * 1024;

/// Identifies an original from its first bytes: its media type and, for images whose header
/// fits in `header`, their pixel size.
pub fn inspect_media(header: &[u8]) -> MediaInfo {
    if header.starts_with(b"%PDF-") {
        return MediaInfo {
            media_type: "application/pdf".to_owned(),
            width: None,
            height: None,
        };
    }
    let Some(media_type) = imagesize::image_type(header)
        .ok()
        .and_then(image_media_type)
    else {
        return MediaInfo::unknown();
    };
    let size = imagesize::blob_size(header).ok();
    MediaInfo {
        media_type: media_type.to_owned(),
        width: size.and_then(|size| u32::try_from(size.width).ok()),
        height: size.and_then(|size| u32::try_from(size.height).ok()),
    }
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
        ImageType::Heif(Compression::Av1) => Some("image/avif"),
        ImageType::Heif(Compression::Hevc) => Some("image/heic"),
        // Texture and editing formats are kept as opaque originals.
        _ => None,
    }
}
