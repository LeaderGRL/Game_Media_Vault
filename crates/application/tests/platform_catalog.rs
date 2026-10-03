use std::cell::RefCell;

use game_media_vault_application::{
    ApplicationError, ErrorKind, PlatformCatalogSourcePort, PlatformCatalogSummary, PortError,
    ReferenceCatalogRead, ReferenceCatalogRepositoryPort, sync_platform_catalog,
};
use game_media_vault_domain::{ImportedReleaseEdition, ReferenceReleaseRecord};

const NES: &str = "Nintendo - Nintendo Entertainment System";

/// Lists the games of one platform, or of none.
struct GameLists {
    platform: &'static str,
    releases: Vec<ReferenceReleaseRecord>,
}

impl PlatformCatalogSourcePort for GameLists {
    fn platform_releases(&self, platform: &str) -> Result<Option<ReferenceCatalogRead>, PortError> {
        Ok((platform == self.platform).then(|| ReferenceCatalogRead {
            releases: self.releases.clone(),
            skipped_records: 1,
        }))
    }
}

#[derive(Default)]
struct RecordingCatalog {
    persisted: RefCell<Vec<ReferenceReleaseRecord>>,
}

impl ReferenceCatalogRepositoryPort for RecordingCatalog {
    fn persist_reference_release(
        &self,
        record: ReferenceReleaseRecord,
    ) -> Result<ImportedReleaseEdition, PortError> {
        self.persisted.borrow_mut().push(record);
        let id = self.persisted.borrow().len() as i64;
        Ok(ImportedReleaseEdition {
            game_id: id,
            release_edition_id: id,
        })
    }
}

fn release(title: &str, region: &str, edition: &str) -> ReferenceReleaseRecord {
    ReferenceReleaseRecord {
        game_title: title.to_owned(),
        platform: NES.to_owned(),
        region: region.to_owned(),
        revision: None,
        edition_name: edition.to_owned(),
        assertions: Vec::new(),
    }
}

#[test]
fn a_platforms_game_list_is_imported_without_the_entries_no_retail_release_stands_for() {
    let catalog = RecordingCatalog::default();
    let source = GameLists {
        platform: NES,
        releases: vec![
            release("Super Mario Bros.", "World", "Standard"),
            release("Action 52", "USA", "Unl"),
            release("Tetris", "USA", "Proto"),
            release("The Legend of Zelda", "USA", "Beta 2"),
            release("Kirby's Adventure", "USA", "Demo · Kiosk"),
            release("Mega Man", "Japan", "Hack"),
            release("Micro Mages", "World", "Aftermarket"),
            release("[BIOS] Family Computer Disk System", "Japan", "Standard"),
        ],
    };

    let summary = sync_platform_catalog(&catalog, &source, NES).unwrap();

    assert_eq!(
        summary,
        PlatformCatalogSummary {
            platform: NES.to_owned(),
            imported_releases: 2,
            skipped_records: 1,
            left_out_releases: 6,
        }
    );
    let titles: Vec<String> = catalog
        .persisted
        .borrow()
        .iter()
        .map(|record| record.game_title.clone())
        .collect();
    assert_eq!(titles, ["Super Mario Bros.", "Action 52"]);
}

#[test]
fn a_platform_without_a_known_game_list_is_reported() {
    let catalog = RecordingCatalog::default();
    let source = GameLists {
        platform: NES,
        releases: Vec::new(),
    };

    let error = sync_platform_catalog(&catalog, &source, "Nintendo - Wonder Console").unwrap_err();

    assert_eq!(
        error,
        ApplicationError::PlatformCatalogNotFound("Nintendo - Wonder Console".to_owned())
    );
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(catalog.persisted.borrow().is_empty());
}
