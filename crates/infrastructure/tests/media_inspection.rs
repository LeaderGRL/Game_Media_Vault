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
