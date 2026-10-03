use game_media_vault_application::{DerivedStorePort, ObjectStorePort};
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
            document: None,
        }
    );
    assert_eq!(
        stored_media(&jpeg_header(640, 480)),
        MediaInfo {
            media_type: "image/jpeg".to_owned(),
            width: Some(640),
            height: Some(480),
            document: None,
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
            document: None,
        }
    );
    assert_eq!(
        stored_media(b"<html><script>"),
        MediaInfo {
            media_type: "application/octet-stream".to_owned(),
            width: None,
            height: None,
            document: None,
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
            document: None,
        }
    );
}

#[test]
fn jpeg_xl_codestreams_after_large_container_boxes_still_give_the_pixel_size() {
    // Signature and file type boxes, a 300 000-byte metadata box declared with a 64-bit size,
    // then the codestream of a 64 x 64 image split over two partial codestream boxes, the
    // second one marked last.
    let mut jxl = b"\0\0\0\x0cJXL \r\n\x87\n".to_vec();
    jxl.extend_from_slice(b"\0\0\0\x14ftypjxl \0\0\0\0jxl ");
    let metadata_len: u64 = 300_000;
    jxl.extend_from_slice(b"\0\0\0\x01Exif");
    jxl.extend_from_slice(&metadata_len.to_be_bytes());
    jxl.resize(jxl.len() + metadata_len as usize - 16, 0);
    let codestream = [0xff, 0x0a, 0x4f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    for (index, part) in [(0_u32, &codestream[..3]), (0x8000_0001, &codestream[3..])] {
        jxl.extend_from_slice(&(12 + part.len() as u32).to_be_bytes());
        jxl.extend_from_slice(b"jxlp");
        jxl.extend_from_slice(&index.to_be_bytes());
        jxl.extend_from_slice(part);
    }

    assert_eq!(
        stored_media(&jxl),
        MediaInfo {
            media_type: "image/jxl".to_owned(),
            width: Some(64),
            height: Some(64),
            document: None,
        }
    );
}

#[test]
fn heif_properties_after_large_boxes_still_give_the_pixel_size() {
    // File type box, a 300 000-byte free box, then the meta box whose item properties hold
    // the image spatial extents of a 1920 x 1080 image.
    let mut avif = b"\0\0\0\x14ftypavif\0\0\0\0mif1".to_vec();
    avif.extend_from_slice(&300_000_u32.to_be_bytes());
    avif.extend_from_slice(b"free");
    avif.resize(avif.len() + 300_000 - 8, 0);
    let mut ispe = b"\0\0\0\x14ispe\0\0\0\0".to_vec();
    ispe.extend_from_slice(&1920_u32.to_be_bytes());
    ispe.extend_from_slice(&1080_u32.to_be_bytes());
    let boxed = |box_type: &[u8], content: &[u8]| {
        let mut bytes = (8 + content.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(box_type);
        bytes.extend_from_slice(content);
        bytes
    };
    let iprp = boxed(b"iprp", &boxed(b"ipco", &ispe));
    avif.extend(boxed(b"meta", &[&[0, 0, 0, 0][..], &iprp].concat()));

    assert_eq!(
        stored_media(&avif),
        MediaInfo {
            media_type: "image/avif".to_owned(),
            width: Some(1920),
            height: Some(1080),
            document: None,
        }
    );
}

#[test]
fn other_raster_originals_are_recorded_as_images() {
    let image = |media_type: &str| MediaInfo {
        media_type: media_type.to_owned(),
        width: Some(640),
        height: Some(480),
        document: None,
    };
    let mut qoi = b"qoif".to_vec();
    qoi.extend_from_slice(&640_u32.to_be_bytes());
    qoi.extend_from_slice(&480_u32.to_be_bytes());
    qoi.extend_from_slice(&[4, 0]);
    let mut farbfeld = b"farbfeld".to_vec();
    farbfeld.extend_from_slice(&640_u32.to_be_bytes());
    farbfeld.extend_from_slice(&480_u32.to_be_bytes());
    // Uncompressed true-color TGA header with an 8-bit alpha channel.
    let mut tga = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    tga.extend_from_slice(&640_u16.to_le_bytes());
    tga.extend_from_slice(&480_u16.to_le_bytes());
    tga.extend_from_slice(&[32, 8]);
    tga.extend_from_slice(&[0; 16]);

    assert_eq!(
        stored_media(b"P6\n640 480\n255\n\0\0\0"),
        image("image/x-portable-anymap")
    );
    assert_eq!(stored_media(&qoi), image("image/qoi"));
    assert_eq!(stored_media(&farbfeld), image("image/x-farbfeld"));
    assert_eq!(stored_media(&tga), image("image/x-tga"));
}

#[test]
fn portable_anymap_headers_with_long_comments_still_give_the_pixel_size() {
    // A 300 000-byte comment separates the magic number from the dimensions.
    let mut pnm = b"P6\n# ".to_vec();
    pnm.extend(std::iter::repeat_n(b'x', 300_000));
    pnm.extend_from_slice(b"\n640 480\n255\n\0\0\0");

    assert_eq!(
        stored_media(&pnm),
        MediaInfo {
            media_type: "image/x-portable-anymap".to_owned(),
            width: Some(640),
            height: Some(480),
            document: None,
        }
    );
}

#[test]
fn text_resembling_a_portable_anymap_signature_stays_opaque() {
    // Two bytes are all the PNM signature is, so a header without dimensions is not trusted.
    assert_eq!(
        stored_media(b"P1 review notes\nnothing to see"),
        MediaInfo::unknown()
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
            document: None,
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
            document: None,
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
            document: None,
        }
    );
}

#[test]
fn a_gltf_binary_is_recorded_as_a_3d_model() {
    let temp = tempdir().unwrap();
    let mut glb = b"glTF".to_vec();
    glb.extend_from_slice(&2_u32.to_le_bytes());
    glb.extend_from_slice(&12_u32.to_le_bytes());

    let stored = ContentAddressedStore::new(temp.path())
        .store_derived(&mut glb.as_slice())
        .unwrap();

    assert_eq!(
        stored.media,
        MediaInfo {
            media_type: "model/gltf-binary".to_owned(),
            width: None,
            height: None,
            document: None,
        }
    );
}

#[test]
fn mp4_and_webm_originals_are_recorded_as_videos() {
    // An ISO base media file opens with its `ftyp` box, naming its brands.
    let mut mp4 = vec![0, 0, 0, 0x18];
    mp4.extend_from_slice(b"ftypisom");
    mp4.extend_from_slice(&[0, 0, 2, 0]);
    mp4.extend_from_slice(b"isommp41");
    // A Matroska or WebM file opens with its EBML header.
    let webm = [0x1a, 0x45, 0xdf, 0xa3, 0x9f, 0x42, 0x86, 0x81, 0x01];

    for (bytes, media_type) in [(&mp4[..], "video/mp4"), (&webm[..], "video/webm")] {
        assert_eq!(
            stored_media(bytes),
            MediaInfo {
                media_type: media_type.to_owned(),
                width: None,
                height: None,
                document: None,
            }
        );
    }
}

#[test]
fn image_sequences_and_extended_type_boxes_are_no_videos() {
    // An AVIF image sequence names its own brand.
    let mut sequence = vec![0, 0, 0, 0x18];
    sequence.extend_from_slice(b"ftypavis");
    sequence.extend_from_slice(&[0, 0, 0, 0]);
    sequence.extend_from_slice(b"avismif1");
    // A type box of extended size puts its brand after the 64-bit size.
    let mut extended = vec![0, 0, 0, 1];
    extended.extend_from_slice(b"ftyp");
    extended.extend_from_slice(&28_u64.to_be_bytes());
    extended.extend_from_slice(b"avif");
    extended.extend_from_slice(&[0, 0, 0, 0]);
    extended.extend_from_slice(b"mif1");

    for bytes in [sequence, extended] {
        assert!(
            !stored_media(&bytes).media_type.starts_with("video/"),
            "{:?}",
            stored_media(&bytes)
        );
    }
}
