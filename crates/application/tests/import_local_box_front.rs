use std::{cell::RefCell, fs, path::Path};

use game_media_vault_application::{
    CatalogPort, ImportLocalBoxFrontRequest, ObjectStorePort, PortError, import_local_box_front,
};
use game_media_vault_domain::{
    AssetType, ImportedAsset, LibraryEntry, PersistAsset, SourceKind, StoredObject,
};
use tempfile::tempdir;

struct FakeObjectStore {
    expected_source: std::path::PathBuf,
}

impl ObjectStorePort for FakeObjectStore {
    fn store_original(&self, source: &Path) -> Result<StoredObject, PortError> {
        assert_eq!(source, self.expected_source);
        Ok(StoredObject {
            hash: "abc123".to_owned(),
            byte_len: 4096,
        })
    }
}

#[derive(Default)]
struct RecordingCatalog {
    persisted: RefCell<Vec<PersistAsset>>,
}

impl CatalogPort for RecordingCatalog {
    fn persist_asset(&self, record: PersistAsset) -> Result<ImportedAsset, PortError> {
        self.persisted.borrow_mut().push(record);
        Ok(ImportedAsset {
            game_id: 1,
            release_edition_id: 2,
            asset_id: 3,
            object_hash: "abc123".to_owned(),
            byte_len: 4096,
        })
    }

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError> {
        Ok(Vec::new())
    }
}

#[test]
fn imports_a_local_box_front_through_the_application_seam() {
    let catalog = RecordingCatalog::default();
    let temp = tempdir().unwrap();
    let source = temp.path().join("cover-front.png");
    fs::write(&source, b"cover bytes").unwrap();
    let expected_source_location = source.to_string_lossy().into_owned();
    let request = ImportLocalBoxFrontRequest {
        existing_game_id: None,
        game_title: "Metal Gear Solid".to_owned(),
        platform: "PlayStation".to_owned(),
        region: "France".to_owned(),
        edition_name: "Original".to_owned(),
        source_path: source.clone(),
    };

    let imported = import_local_box_front(
        &catalog,
        &FakeObjectStore {
            expected_source: source,
        },
        request,
    )
    .unwrap();

    assert_eq!(imported.asset_id, 3);
    assert_eq!(
        catalog.persisted.into_inner(),
        vec![PersistAsset {
            existing_game_id: None,
            game_title: "Metal Gear Solid".to_owned(),
            platform: "PlayStation".to_owned(),
            region: "France".to_owned(),
            edition_name: "Original".to_owned(),
            asset_type: AssetType::BoxFront,
            object_hash: "abc123".to_owned(),
            byte_len: 4096,
            original_filename: "cover-front.png".to_owned(),
            source_kind: SourceKind::LocalImport,
            source_location: expected_source_location,
        }]
    );
}
