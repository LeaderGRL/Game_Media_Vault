use std::fs;

use game_media_vault_application::{
    AcquisitionRequestInput, CatalogPort, CorruptObject, DerivativeRepositoryPort,
    DerivedStorePort, ObjectArea, ObjectStorePort, RepairActions, ReviewRepositoryPort,
    RunRepositoryPort, StaleWork, StaleWorkReason, VaultRepairStorePort, cancel_acquisition_run,
    repair_vault, start_acquisition_run, verify_vault,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionWorkItem, AssetCandidate, AssetType, AssetTypeSelector,
    DerivationRecipe, GameSelection, NewReviewItem, PersistAsset, RetentionPolicy, SourceId,
    SourceSelection, StoredObject,
};
use game_media_vault_infrastructure::{ContentAddressedStore, SqliteCatalog};
use tempfile::tempdir;

const THUMBNAIL: DerivationRecipe = DerivationRecipe::Thumbnail { max_edge: 256 };

fn box_front(game_title: &str, object_hash: &str, byte_len: u64) -> PersistAsset {
    PersistAsset {
        existing_game_id: None,
        existing_release_edition_id: None,
        match_decision: None,
        game_title: game_title.to_owned(),
        platform: "Sony - PlayStation".to_owned(),
        region: "France".to_owned(),
        edition_name: "Original".to_owned(),
        asset_type: AssetType::BoxFront,
        object_hash: object_hash.to_owned(),
        byte_len,
        media: game_media_vault_domain::MediaInfo::unknown(),
        original_filename: "front.png".to_owned(),
        source_id: SourceId::from("local_import"),
        source_asset_label: None,
        source_location: format!("C:/covers/{game_title}.png"),
    }
}

#[test]
fn verification_compares_the_catalog_with_the_stored_bytes() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let intact = store.store_original(&mut &b"intact cover"[..]).unwrap();
    let corrupt = store.store_original(&mut &b"cover to corrupt"[..]).unwrap();
    let missing = store.store_original(&mut &b"cover to lose"[..]).unwrap();
    let loose = store
        .store_original(&mut &b"below quality cover"[..])
        .unwrap();
    for (title, object) in [
        ("Intact", &intact),
        ("Corrupt", &corrupt),
        ("Missing", &missing),
    ] {
        catalog
            .persist_asset(box_front(title, &object.hash, object.byte_len))
            .unwrap();
    }
    let thumbnail = store.store_derived(&mut &b"intact thumbnail"[..]).unwrap();
    catalog
        .record_derivative(&intact.hash, &THUMBNAIL, &thumbnail)
        .unwrap();
    let stray = store.store_derived(&mut &b"stray thumbnail"[..]).unwrap();
    fs::write(store.object_path(&corrupt.hash), b"bit rot").unwrap();
    fs::remove_file(store.object_path(&missing.hash)).unwrap();
    fs::write(vault.join("staging").join("4242-0.tmp"), b"interrupted").unwrap();

    let report = verify_vault(&catalog, &store).unwrap();

    assert_eq!(report.missing_originals, [missing.hash]);
    assert_eq!(
        report.corrupt_originals,
        [CorruptObject {
            hash: corrupt.hash.clone(),
            actual_hash: blake3::hash(b"bit rot").to_hex().to_string(),
        }]
    );
    assert_eq!(report.unreferenced_originals, [loose.hash]);
    assert!(report.missing_derived.is_empty());
    assert_eq!(report.orphaned_derived, [stray.hash]);
    assert_eq!(report.interrupted_staging, ["4242-0.tmp"]);
    // Verifying repairs nothing.
    assert_eq!(
        fs::read(store.object_path(&corrupt.hash)).unwrap(),
        b"bit rot"
    );
    assert_eq!(catalog.list_library().unwrap().len(), 3);
}

#[test]
fn an_empty_vault_is_healthy() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);

    assert!(verify_vault(&catalog, &store).unwrap().is_healthy());
}

#[test]
fn objects_that_cannot_be_read_or_are_not_named_by_a_hash_are_reported_unreadable() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let blocked = store.store_original(&mut &b"blocked cover"[..]).unwrap();
    catalog
        .persist_asset(box_front("Blocked", &blocked.hash, blocked.byte_len))
        .unwrap();
    catalog
        .persist_asset(box_front("Tampered", "../../outside", 1))
        .unwrap();
    // A directory where the object should be cannot be read as one.
    fs::remove_file(store.object_path(&blocked.hash)).unwrap();
    fs::create_dir(store.object_path(&blocked.hash)).unwrap();

    let report = verify_vault(&catalog, &store).unwrap();

    let unreadable: Vec<&str> = report
        .unreadable_originals
        .iter()
        .map(|object| object.hash.as_str())
        .collect();
    assert_eq!(unreadable, ["../../outside", blocked.hash.as_str()]);
    assert!(report.missing_originals.is_empty());
}

/// Reads like an original while noting which staging files verification would report then.
struct InspectingReader<'a> {
    store: &'a ContentAddressedStore,
    staging_seen: Option<Vec<String>>,
    sent: bool,
}

impl std::io::Read for InspectingReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.sent {
            return Ok(0);
        }
        self.staging_seen =
            Some(game_media_vault_application::VaultStorePort::staging_files(self.store).unwrap());
        self.sent = true;
        buffer[..5].copy_from_slice(b"cover");
        Ok(5)
    }
}

#[test]
fn a_store_in_progress_is_not_reported_but_a_left_over_file_of_this_process_is() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    // Left by an earlier process that had this process id.
    let left_over = format!("{}-999999.tmp", std::process::id());
    fs::create_dir_all(vault.join("staging")).unwrap();
    fs::write(vault.join("staging").join(&left_over), b"interrupted").unwrap();
    let mut reader = InspectingReader {
        store: &store,
        staging_seen: None,
        sent: false,
    };

    store.store_original(&mut reader).unwrap();

    assert_eq!(
        reader.staging_seen.unwrap(),
        std::slice::from_ref(&left_over)
    );
    assert_eq!(
        verify_vault(&catalog, &store).unwrap().interrupted_staging,
        [left_over]
    );
}

#[test]
fn a_derived_record_not_named_by_a_hash_is_reported_unreadable() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let original = store.store_original(&mut &b"kept cover"[..]).unwrap();
    catalog
        .persist_asset(box_front("Kept", &original.hash, original.byte_len))
        .unwrap();
    let tampered = StoredObject {
        hash: "../../outside".to_owned(),
        byte_len: 1,
        media: game_media_vault_domain::MediaInfo::unknown(),
    };
    catalog
        .record_derivative(&original.hash, &THUMBNAIL, &tampered)
        .unwrap();

    let report = verify_vault(&catalog, &store).unwrap();

    assert_eq!(report.unreadable_derived.len(), 1);
    assert_eq!(report.unreadable_derived[0].hash, "../../outside");
}

#[test]
fn repairs_remove_what_verification_found_and_keep_damaged_originals() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let kept = store.store_original(&mut &b"kept cover"[..]).unwrap();
    let corrupt = store.store_original(&mut &b"cover to corrupt"[..]).unwrap();
    let loose = store
        .store_original(&mut &b"below quality cover"[..])
        .unwrap();
    for (title, object) in [("Kept", &kept), ("Corrupt", &corrupt)] {
        catalog
            .persist_asset(box_front(title, &object.hash, object.byte_len))
            .unwrap();
    }
    let thumbnail = store.store_derived(&mut &b"kept thumbnail"[..]).unwrap();
    catalog
        .record_derivative(&kept.hash, &THUMBNAIL, &thumbnail)
        .unwrap();
    let stray = store.store_derived(&mut &b"stray thumbnail"[..]).unwrap();
    fs::write(store.derived_path(&thumbnail.hash), b"garbled").unwrap();
    fs::write(store.object_path(&corrupt.hash), b"bit rot").unwrap();
    fs::write(vault.join("staging").join("4242-0.tmp"), b"interrupted").unwrap();
    // A file the store did not name is not one of its objects.
    fs::write(
        store
            .derived_path(&stray.hash)
            .with_file_name("desktop.ini"),
        b"",
    )
    .unwrap();

    let summary = repair_vault(
        &catalog,
        &store,
        RepairActions {
            remove_interrupted_staging: true,
            remove_orphaned_derived: true,
            reset_damaged_derived: true,
            collect_unreferenced_originals: true,
        },
    )
    .unwrap();

    assert_eq!(
        summary.collected_originals,
        std::slice::from_ref(&loose.hash)
    );
    assert_eq!(
        summary.removed_derived,
        [thumbnail.hash.clone(), stray.hash.clone()]
    );
    assert!(!store.object_path(&loose.hash).exists());
    assert!(!store.derived_path(&thumbnail.hash).exists());
    assert!(!vault.join("staging").join("4242-0.tmp").exists());
    // The thumbnail renders again, while the corrupt original stays for a human to restore.
    assert_eq!(catalog.originals_without(&THUMBNAIL).unwrap().len(), 2);
    assert_eq!(
        fs::read(store.object_path(&corrupt.hash)).unwrap(),
        b"bit rot"
    );
    assert_eq!(summary.remaining.corrupt_originals.len(), 1);
    assert!(summary.remaining.orphaned_derived.is_empty());
}

#[test]
fn only_files_at_their_own_address_count_as_stored_objects() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let loose = store.store_original(&mut &b"loose cover"[..]).unwrap();
    let misplaced = vault.join("objects").join("zz").join("zz");
    fs::create_dir_all(&misplaced).unwrap();
    fs::rename(store.object_path(&loose.hash), misplaced.join(&loose.hash)).unwrap();
    fs::write(
        store
            .object_path(&loose.hash)
            .with_file_name(loose.hash.to_uppercase()),
        b"aliased",
    )
    .unwrap();

    assert!(verify_vault(&catalog, &store).unwrap().is_healthy());
}

#[test]
fn forgetting_orphaned_derived_assets_keeps_a_thumbnail_a_retained_original_shares() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let kept = store.store_original(&mut &b"kept cover"[..]).unwrap();
    let loose = store.store_original(&mut &b"loose cover"[..]).unwrap();
    catalog
        .persist_asset(box_front("Kept", &kept.hash, kept.byte_len))
        .unwrap();
    let thumbnail = store.store_derived(&mut &b"same thumbnail"[..]).unwrap();
    for original in [&kept, &loose] {
        catalog
            .record_derivative(&original.hash, &THUMBNAIL, &thumbnail)
            .unwrap();
    }

    let summary = repair_vault(
        &catalog,
        &store,
        RepairActions {
            remove_orphaned_derived: true,
            ..RepairActions::default()
        },
    )
    .unwrap();

    assert_eq!(summary.forgotten_derived.len(), 1);
    assert_eq!(summary.forgotten_derived[0].original_hash, loose.hash);
    assert!(summary.removed_derived.is_empty());
    assert!(store.derived_path(&thumbnail.hash).exists());
    assert_eq!(catalog.originals_without(&THUMBNAIL).unwrap().len(), 0);
    assert!(summary.remaining.orphaned_derived.is_empty());
    assert_eq!(summary.remaining.unreferenced_originals, [loose.hash]);
}

#[test]
fn repairs_never_remove_a_path_the_store_did_not_name() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let store = ContentAddressedStore::new(&vault);
    let kept = store.store_original(&mut &b"kept cover"[..]).unwrap();
    fs::write(vault.join("keep.txt"), b"not an object").unwrap();

    for name in [
        "",
        ".",
        "..",
        "../keep.txt",
        "nested/file.tmp",
        r"nested\file.tmp",
    ] {
        assert!(store.remove_staging_file(name).is_err(), "{name:?}");
    }
    for hash in [
        "../keep.txt".to_owned(),
        kept.hash.to_uppercase(),
        kept.hash[..63].to_owned(),
    ] {
        assert!(
            store.remove_object(ObjectArea::Original, &hash).is_err(),
            "{hash:?}"
        );
    }

    assert!(vault.join("keep.txt").exists());
    assert!(store.object_path(&kept.hash).exists());
}

const RUN_SOURCE: &str = "fixture-provider";

fn run_request() -> AcquisitionRequestInput {
    AcquisitionRequestInput {
        sources: SourceSelection::Explicit(vec![RUN_SOURCE.to_owned()]),
        platforms: vec!["Windows".to_owned()],
        games: GameSelection::All,
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    }
}

fn work(key: &str) -> AcquisitionWorkItem {
    AcquisitionWorkItem {
        key: key.to_owned(),
        candidate: AssetCandidate {
            provider_candidate_id: Some(key.to_owned()),
            game_title: format!("Game for {key}"),
            platform: "Windows".to_owned(),
            region: "Worldwide".to_owned(),
            edition_name: "Standard".to_owned(),
            asset_type: AssetType::BoxFront,
            source_id: SourceId::from(RUN_SOURCE),
            source_asset_label: None,
            source_url: format!("https://example.invalid/{key}.png"),
            original_filename: format!("{key}.png"),
        },
    }
}

#[test]
fn work_no_execution_will_process_is_reported_stale() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let store = ContentAddressedStore::new(&vault);
    let running = start_acquisition_run(&catalog, run_request()).unwrap();
    catalog
        .record_discovery(running.id, RUN_SOURCE, &[work("kept"), work("parked")])
        .unwrap();
    catalog
        .park_work_for_review(
            running.id,
            "parked",
            NewReviewItem {
                candidate_identity: "parked".to_owned(),
                candidate: work("parked").candidate,
                competing_matches: Vec::new(),
            },
        )
        .unwrap();
    // A decision that closed the Review Item without requeueing its work.
    rusqlite::Connection::open(vault.join("catalog.sqlite3"))
        .unwrap()
        .execute("UPDATE review_items SET status = 'accepted'", [])
        .unwrap();
    let cancelled = start_acquisition_run(&catalog, run_request()).unwrap();
    catalog
        .record_discovery(cancelled.id, RUN_SOURCE, &[work("abandoned")])
        .unwrap();
    cancel_acquisition_run(&catalog, cancelled.id).unwrap();

    let report = verify_vault(&catalog, &store).unwrap();

    assert_eq!(
        report.stale_work,
        [
            StaleWork {
                run_id: running.id,
                work_key: "parked".to_owned(),
                reason: StaleWorkReason::ClosedReview,
            },
            StaleWork {
                run_id: cancelled.id,
                work_key: "abandoned".to_owned(),
                reason: StaleWorkReason::CancelledRun,
            },
        ]
    );
}
