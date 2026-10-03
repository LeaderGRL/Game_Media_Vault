use std::{io::Read, path::Path};

use game_media_vault_application::{
    CatalogPort, MediaTransformPort, ObjectStorePort, PortError, derive_assets,
};
use game_media_vault_domain::{AssetType, DerivationRecipe, PersistAsset, SourceId};
use game_media_vault_infrastructure::{
    ContentAddressedStore, ImageTransformer, MediaTransformers, PdfiumRenderer, SqliteCatalog,
};
use tempfile::tempdir;

const THUMBNAIL: DerivationRecipe = DerivationRecipe::Thumbnail { max_edge: 256 };

/// A PDF of `pages` blank US Letter pages, with a correct cross-reference table.
fn pdf(pages: usize) -> Vec<u8> {
    let kids: Vec<String> = (0..pages).map(|page| format!("{} 0 R", 3 + page)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            kids.join(" ")
        ),
    ];
    objects.extend(
        (0..pages).map(|_| "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned()),
    );
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
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

/// A vault at `root` holding one PDF manual.
fn vault_with_a_manual(root: &Path) -> (SqliteCatalog, ContentAddressedStore) {
    let catalog = SqliteCatalog::open(root.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(root);
    let stored = store.store_original(&mut pdf(2).as_slice()).unwrap();
    catalog
        .persist_asset(PersistAsset {
            existing_game_id: None,
            existing_release_edition_id: None,
            match_decision: None,
            game_title: "Metal Gear Solid".to_owned(),
            platform: "Sony - PlayStation".to_owned(),
            region: "France".to_owned(),
            edition_name: "Original".to_owned(),
            asset_type: AssetType::Manual,
            object_hash: stored.hash,
            byte_len: stored.byte_len,
            media: stored.media,
            original_filename: "manual.pdf".to_owned(),
            source_id: SourceId::from("local_import"),
            source_asset_label: None,
            source_location: "C:/manuals/mgs.pdf".to_owned(),
        })
        .unwrap();
    (catalog, store)
}

#[test]
fn pdf_originals_are_skipped_while_this_machine_has_no_pdfium() {
    let temp = tempdir().unwrap();
    let (catalog, store) = vault_with_a_manual(&temp.path().join("vault"));
    let empty = temp.path().join("no-pdfium");
    std::fs::create_dir(&empty).unwrap();
    let renderer = PdfiumRenderer::from_directory(&empty);
    assert!(!renderer.is_available());
    let transformers =
        MediaTransformers::new(vec![Box::new(ImageTransformer::new()), Box::new(renderer)]);

    let summary = derive_assets(&catalog, &store, &transformers, &THUMBNAIL).unwrap();

    assert_eq!((summary.derived, summary.skipped), (0, 1));
    assert!(summary.failed.is_empty());
}

/// Transforms only media of its own type, answering its name.
struct Named(&'static str);

impl MediaTransformPort for Named {
    fn can_transform(&self, media_type: &str, _recipe: &DerivationRecipe) -> bool {
        media_type == self.0
    }

    fn transform(
        &self,
        _original: &mut dyn Read,
        _media_type: &str,
        _recipe: &DerivationRecipe,
    ) -> Result<Vec<u8>, PortError> {
        Ok(self.0.as_bytes().to_vec())
    }
}

#[test]
fn several_transformers_hand_each_media_type_to_the_first_that_takes_it() {
    let transformers = MediaTransformers::new(vec![
        Box::new(Named("image/png")),
        Box::new(Named("application/pdf")),
        Box::new(Named("application/pdf")),
    ]);

    assert!(transformers.can_transform("application/pdf", &THUMBNAIL));
    assert!(!transformers.can_transform("model/gltf-binary", &THUMBNAIL));
    let output = transformers
        .transform(&mut [].as_slice(), "application/pdf", &THUMBNAIL)
        .unwrap();
    assert_eq!(output, b"application/pdf");
    assert!(
        transformers
            .transform(&mut [].as_slice(), "model/gltf-binary", &THUMBNAIL)
            .is_err()
    );
}

/// Needs a pdfium library in the directory `GAME_MEDIA_VAULT_PDFIUM` names, which no build
/// provides yet.
#[test]
#[ignore = "needs a pdfium library in GAME_MEDIA_VAULT_PDFIUM"]
fn pdfium_renders_the_first_page_of_a_pdf_as_its_thumbnail() {
    let directory = std::env::var_os("GAME_MEDIA_VAULT_PDFIUM").expect("GAME_MEDIA_VAULT_PDFIUM");
    let renderer = PdfiumRenderer::from_directory(Path::new(&directory));
    assert!(renderer.is_available());
    assert!(renderer.can_transform("application/pdf", &THUMBNAIL));

    let thumbnail = renderer
        .transform(&mut pdf(2).as_slice(), "application/pdf", &THUMBNAIL)
        .unwrap();

    let image = image::load_from_memory(&thumbnail).unwrap();
    // A US Letter page keeps its proportions within the longest edge.
    assert_eq!(image.height(), 256);
    assert!((197..=199).contains(&image.width()), "{}", image.width());
}

#[test]
fn pdfium_is_never_looked_for_outside_an_absolute_directory() {
    // An empty or relative directory would leave the loader to search the working directory
    // and `PATH` for the library.
    for directory in ["", "pdfium", "./pdfium"] {
        assert!(!PdfiumRenderer::from_directory(Path::new(directory)).is_available());
    }
}
