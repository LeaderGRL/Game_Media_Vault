use game_media_vault_application::ObjectStorePort;
use game_media_vault_domain::MediaInfo;
use game_media_vault_infrastructure::ContentAddressedStore;
use tempfile::tempdir;

/// PNG signature and IHDR chunk of a `width` x `height` image.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

/// Smallest JPEG prefix with a baseline frame header for a `width` x `height` image.
fn jpeg_header(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08];
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    bytes
}

fn stored_media(bytes: &[u8]) -> MediaInfo {
    let temp = tempdir().unwrap();
    ContentAddressedStore::new(temp.path())
        .store_original(&mut &bytes[..])
        .unwrap()
        .media
}

#[test]
fn storing_an_original_records_its_media_type_and_pixel_size() {
    assert_eq!(
        stored_media(&png_header(1200, 1600)),
        MediaInfo {
            media_type: "image/png".to_owned(),
            width: Some(1200),
            height: Some(1600),
        }
    );
    assert_eq!(
        stored_media(&jpeg_header(640, 480)),
        MediaInfo {
            media_type: "image/jpeg".to_owned(),
            width: Some(640),
            height: Some(480),
        }
    );
}

#[test]
fn originals_that_are_not_images_have_no_pixel_size() {
    assert_eq!(
        stored_media(b"%PDF-1.7 manual"),
        MediaInfo {
            media_type: "application/pdf".to_owned(),
            width: None,
            height: None,
        }
    );
    assert_eq!(
        stored_media(b"<html><script>"),
        MediaInfo {
            media_type: "application/octet-stream".to_owned(),
            width: None,
            height: None,
        }
    );
}

#[test]
fn jpeg_xl_originals_are_recorded_as_images() {
    // JPEG XL codestream signature and a size header for a 64 x 64 image.
    let jxl = [0xff, 0x0a, 0x4f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    assert_eq!(
        stored_media(&jxl),
        MediaInfo {
            media_type: "image/jxl".to_owned(),
            width: Some(64),
            height: Some(64),
        }
    );
}

#[test]
fn icon_originals_are_recorded_as_images() {
    // ICONDIR header and the directory entry of a 32 x 32 image.
    let ico = [
        0, 0, 1, 0, 1, 0, 32, 32, 0, 0, 1, 0, 32, 0, 0x28, 0x04, 0, 0, 0x16, 0, 0, 0,
    ];

    assert_eq!(
        stored_media(&ico),
        MediaInfo {
            media_type: "image/x-icon".to_owned(),
            width: Some(32),
            height: Some(32),
        }
    );
}

#[test]
fn jpeg_frames_after_large_metadata_still_give_the_pixel_size() {
    // Five maximal APP1 segments (about 320 KiB of metadata) precede the frame header.
    let mut jpeg = vec![0xff, 0xd8];
    for _ in 0..5 {
        jpeg.extend_from_slice(&[0xff, 0xe1, 0xff, 0xff]);
        jpeg.extend(std::iter::repeat_n(0, 0xffff - 2));
    }
    jpeg.extend_from_slice(&jpeg_header(2400, 3200)[2..]);

    assert_eq!(
        stored_media(&jpeg),
        MediaInfo {
            media_type: "image/jpeg".to_owned(),
            width: Some(2400),
            height: Some(3200),
        }
    );
}

#[test]
fn tiff_directories_past_the_inspected_prefix_still_give_the_pixel_size() {
    // Little-endian TIFF whose first image file directory starts at byte 300 000.
    let directory_offset: u32 = 300_000;
    let mut tiff = b"II*\0".to_vec();
    tiff.extend_from_slice(&directory_offset.to_le_bytes());
    tiff.resize(directory_offset as usize, 0);
    tiff.extend_from_slice(&2_u16.to_le_bytes());
    for (tag, value) in [(256_u16, 1800_u16), (257, 2400)] {
        tiff.extend_from_slice(&tag.to_le_bytes());
        tiff.extend_from_slice(&3_u16.to_le_bytes()); // SHORT
        tiff.extend_from_slice(&1_u32.to_le_bytes());
        tiff.extend_from_slice(&value.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]);
    }
    tiff.extend_from_slice(&0_u32.to_le_bytes());

    assert_eq!(
        stored_media(&tiff),
        MediaInfo {
            media_type: "image/tiff".to_owned(),
            width: Some(1800),
            height: Some(2400),
        }
    );
}
