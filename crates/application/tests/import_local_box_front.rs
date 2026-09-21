use std::{cell::RefCell, path::Path};

use game_media_vault_application::{
    CatalogPort, ImportLocalBoxFrontRequest, ObjectStorePort, PortError, import_local_box_front,
};
use game_media_vault_domain::{
    AssetType, ImportedAsset, LibraryEntry, PersistLocalBoxFront, SourceKind, StoredObject,
};

struct FakeObjectStore;

impl ObjectStorePort for FakeObjectStore {
    fn store_original(&self, source: &Path) -> Result<StoredObject, PortError> {
        assert_eq!(source, Path::new("fixtures/cover-front.png"));
        Ok(StoredObject {
            hash: "abc123".to_owned(),
            byte_len: 4096,
        })
    }
}

#[derive(Default)]
struct RecordingCatalog {
    persisted: RefCell<Vec<PersistLocalBoxFront>>,
}

impl CatalogPort for RecordingCatalog {
    fn persist_local_box_front(
        &self,
        record: PersistLocalBoxFront,
    ) -> Result<ImportedAsset, PortError> {
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
    let request = ImportLocalBoxFrontRequest {
        game_title: "Metal Gear Solid".to_owned(),
        platform: "PlayStation".to_owned(),
        region: "France".to_owned(),
        edition_name: "Original".to_owned(),
        source_path: "fixtures/cover-front.png".into(),
    };

    let imported = import_local_box_front(&catalog, &FakeObjectStore, request).unwrap();

    assert_eq!(imported.asset_id, 3);
    assert_eq!(
        catalog.persisted.into_inner(),
        vec![PersistLocalBoxFront {
            game_title: "Metal Gear Solid".to_owned(),
            platform: "PlayStation".to_owned(),
            region: "France".to_owned(),
            edition_name: "Original".to_owned(),
            asset_type: AssetType::BoxFront,
            object_hash: "abc123".to_owned(),
            byte_len: 4096,
            original_filename: "cover-front.png".to_owned(),
            source_kind: SourceKind::LocalImport,
            source_location: "fixtures/cover-front.png".to_owned(),
        }]
    );
}
