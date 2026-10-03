use game_media_vault_application::ObjectStorePort;
use game_media_vault_infrastructure::ContentAddressedStore;
use tempfile::tempdir;

/// A PDF of two blank pages naming its title, with `extra` objects after its own and `trailer`
/// entries added to its trailer, and a correct cross-reference table.
fn pdf(extra: &[&str], trailer: &str) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [4 0 R 5 0 R] /Count 2 >>".to_owned(),
        "<< /Title (Manual) >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned(),
    ];
    objects.extend(extra.iter().map(|object| (*object).to_owned()));
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        bytes.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R {trailer} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

#[test]
fn a_pdf_describes_itself_once_stored() {
    let temp = tempdir().unwrap();
    let store = ContentAddressedStore::new(temp.path());

    let stored = store.store_original(&mut pdf(&[], "").as_slice()).unwrap();

    let document = stored.media.document.unwrap();
    assert_eq!(document.page_count, Some(2));
    assert_eq!(document.title.as_deref(), Some("Manual"));
    assert!(!document.encrypted);
}

#[test]
fn an_encrypted_pdf_whose_pages_cannot_be_read_leaves_its_page_count_unknown() {
    let temp = tempdir().unwrap();
    let store = ContentAddressedStore::new(temp.path());
    // A standard security handler whose user password is not empty, so no page can be read.
    let digest = "00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF";
    let encryption =
        format!("<< /Filter /Standard /V 1 /R 2 /O <{digest}> /U <{digest}> /P -44 >>");
    let id = "<00112233445566778899AABBCCDDEEFF>";

    let stored = store
        .store_original(
            &mut pdf(&[&encryption], &format!("/Encrypt 6 0 R /ID [{id} {id}]")).as_slice(),
        )
        .unwrap();

    let document = stored.media.document.unwrap();
    assert!(document.encrypted);
    assert_eq!(document.page_count, None);
}

#[test]
fn a_pdf_larger_than_the_metadata_limit_is_kept_undescribed() {
    let temp = tempdir().unwrap();
    let bytes = pdf(&[], "");
    let store =
        ContentAddressedStore::new(temp.path()).with_max_document_bytes(bytes.len() as u64 - 1);

    let stored = store.store_original(&mut bytes.as_slice()).unwrap();

    assert_eq!(stored.media.media_type, "application/pdf");
    assert_eq!(stored.byte_len, bytes.len() as u64);
    assert_eq!(stored.media.document, None);
}
