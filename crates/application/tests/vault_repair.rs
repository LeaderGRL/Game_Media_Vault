use std::{cell::RefCell, collections::BTreeMap};

use game_media_vault_application::{
    ObjectArea, ObjectCheck, PortError, RecordedDerivative, RepairActions, UnfinishedWork,
    VaultCatalogPort, VaultRepairCatalogPort, VaultRepairStorePort, VaultStorePort, repair_vault,
};

/// What the catalog references and what the store holds, with the hash each stored file
/// actually has; repairs change them in place. A file whose hash is `UNREADABLE` cannot be read.
#[derive(Default)]
struct FakeVault {
    referenced: Vec<String>,
    derivatives: RefCell<Vec<RecordedDerivative>>,
    originals: RefCell<BTreeMap<String, String>>,
    derived: RefCell<BTreeMap<String, String>>,
    staging: RefCell<Vec<String>>,
}

impl FakeVault {
    fn area(&self, area: ObjectArea) -> &RefCell<BTreeMap<String, String>> {
        match area {
            ObjectArea::Original => &self.originals,
            ObjectArea::Derived => &self.derived,
        }
    }
}

impl VaultCatalogPort for FakeVault {
    fn referenced_originals(&self) -> Result<Vec<String>, PortError> {
        Ok(self.referenced.clone())
    }

    fn recorded_derivatives(&self) -> Result<Vec<RecordedDerivative>, PortError> {
        Ok(self.derivatives.borrow().clone())
    }

    fn unfinished_work(&self) -> Result<Vec<UnfinishedWork>, PortError> {
        Ok(Vec::new())
    }
}

impl VaultRepairCatalogPort for FakeVault {
    fn forget_derivatives(&self, derivatives: &[RecordedDerivative]) -> Result<(), PortError> {
        self.derivatives
            .borrow_mut()
            .retain(|derivative| !derivatives.contains(derivative));
        Ok(())
    }
}

impl VaultStorePort for FakeVault {
    fn stored_objects(&self, area: ObjectArea) -> Result<Vec<String>, PortError> {
        Ok(self.area(area).borrow().keys().cloned().collect())
    }

    fn check_object(&self, area: ObjectArea, hash: &str) -> Result<ObjectCheck, PortError> {
        Ok(match self.area(area).borrow().get(hash) {
            None => ObjectCheck::Missing,
            Some(actual) if actual == UNREADABLE => ObjectCheck::Unreadable {
                reason: "permission denied".to_owned(),
            },
            Some(actual) if actual == hash => ObjectCheck::Intact,
            Some(actual) => ObjectCheck::Corrupt {
                actual_hash: actual.clone(),
            },
        })
    }

    fn staging_files(&self) -> Result<Vec<String>, PortError> {
        Ok(self.staging.borrow().clone())
    }
}

impl VaultRepairStorePort for FakeVault {
    fn remove_object(&self, area: ObjectArea, hash: &str) -> Result<(), PortError> {
        self.area(area).borrow_mut().remove(hash);
        Ok(())
    }

    fn remove_staging_file(&self, name: &str) -> Result<(), PortError> {
        self.staging.borrow_mut().retain(|file| file != name);
        Ok(())
    }
}

const UNREADABLE: &str = "unreadable";

fn intact(hashes: &[&str]) -> BTreeMap<String, String> {
    hashes
        .iter()
        .map(|hash| ((*hash).to_owned(), (*hash).to_owned()))
        .collect()
}

fn derivative(original_hash: &str, object_hash: &str) -> RecordedDerivative {
    RecordedDerivative {
        original_hash: original_hash.to_owned(),
        object_hash: object_hash.to_owned(),
        other_originals: Vec::new(),
    }
}

/// A vault with every kind of problem verification reports.
fn damaged_vault() -> FakeVault {
    let mut originals = intact(&["aaa", "loose"]);
    originals.insert("bbb".to_owned(), "rotten".to_owned());
    let mut derived = intact(&["thumb-old", "stray"]);
    derived.insert("thumb-aaa".to_owned(), "garbled".to_owned());
    FakeVault {
        referenced: vec!["aaa".to_owned(), "bbb".to_owned(), "gone".to_owned()],
        derivatives: RefCell::new(vec![
            derivative("aaa", "thumb-aaa"),
            derivative("bbb", "thumb-bbb"),
            derivative("old", "thumb-old"),
        ]),
        originals: RefCell::new(originals),
        derived: RefCell::new(derived),
        staging: RefCell::new(vec!["4242-0.tmp".to_owned()]),
    }
}

#[test]
fn repairs_only_what_the_requested_actions_name() {
    let vault = damaged_vault();

    let summary = repair_vault(
        &vault,
        &vault,
        RepairActions {
            remove_interrupted_staging: true,
            ..RepairActions::default()
        },
    )
    .unwrap();

    assert_eq!(summary.removed_staging, ["4242-0.tmp"]);
    assert!(summary.removed_derived.is_empty());
    assert!(summary.collected_originals.is_empty());
    assert!(vault.staging.borrow().is_empty());
    assert_eq!(summary.remaining.orphaned_derived, ["thumb-old", "stray"]);
}

#[test]
fn forgets_orphaned_and_damaged_derived_assets_so_they_render_again() {
    let vault = damaged_vault();

    let summary = repair_vault(
        &vault,
        &vault,
        RepairActions {
            remove_orphaned_derived: true,
            reset_damaged_derived: true,
            ..RepairActions::default()
        },
    )
    .unwrap();

    assert_eq!(
        summary.forgotten_derived,
        [
            derivative("aaa", "thumb-aaa"),
            derivative("bbb", "thumb-bbb"),
            derivative("old", "thumb-old"),
        ]
    );
    assert_eq!(summary.removed_derived, ["thumb-aaa", "thumb-old", "stray"]);
    assert!(vault.derivatives.borrow().is_empty());
    assert!(vault.derived.borrow().is_empty());
    assert!(summary.remaining.missing_derived.is_empty());
    assert!(summary.remaining.corrupt_derived.is_empty());
    assert!(summary.remaining.orphaned_derived.is_empty());
}

#[test]
fn collects_unreferenced_originals_but_never_touches_missing_or_corrupt_ones() {
    let vault = damaged_vault();

    let summary = repair_vault(
        &vault,
        &vault,
        RepairActions {
            collect_unreferenced_originals: true,
            ..RepairActions::default()
        },
    )
    .unwrap();

    assert_eq!(summary.collected_originals, ["loose"]);
    assert_eq!(
        vault.originals.borrow().keys().collect::<Vec<_>>(),
        ["aaa", "bbb"]
    );
    assert_eq!(summary.remaining.missing_originals, ["gone"]);
    assert_eq!(summary.remaining.corrupt_originals.len(), 1);
}

#[test]
fn a_repair_without_any_action_is_refused() {
    let vault = damaged_vault();

    let error = repair_vault(&vault, &vault, RepairActions::default()).unwrap_err();

    assert_eq!(
        error.kind(),
        game_media_vault_application::ErrorKind::InvalidRequest
    );
    assert_eq!(vault.staging.borrow().len(), 1);
}

#[test]
fn forgets_orphaned_derived_assets_but_keeps_an_output_a_referenced_original_shares() {
    let vault = FakeVault {
        referenced: vec!["aaa".to_owned()],
        derivatives: RefCell::new(vec![derivative("aaa", "thumb"), derivative("old", "thumb")]),
        originals: RefCell::new(intact(&["aaa"])),
        derived: RefCell::new(intact(&["thumb"])),
        ..FakeVault::default()
    };

    let summary = repair_vault(
        &vault,
        &vault,
        RepairActions {
            remove_orphaned_derived: true,
            ..RepairActions::default()
        },
    )
    .unwrap();

    assert_eq!(summary.forgotten_derived, [derivative("old", "thumb")]);
    assert!(summary.removed_derived.is_empty());
    assert_eq!(*vault.derivatives.borrow(), [derivative("aaa", "thumb")]);
    assert_eq!(vault.derived.borrow().keys().collect::<Vec<_>>(), ["thumb"]);
}

#[test]
fn leaves_unreadable_derived_files_in_place() {
    let mut derived = BTreeMap::new();
    derived.insert("locked".to_owned(), UNREADABLE.to_owned());
    let vault = FakeVault {
        referenced: vec!["aaa".to_owned()],
        derivatives: RefCell::new(vec![derivative("old", "locked")]),
        originals: RefCell::new(intact(&["aaa"])),
        derived: RefCell::new(derived),
        ..FakeVault::default()
    };

    let summary = repair_vault(
        &vault,
        &vault,
        RepairActions {
            remove_orphaned_derived: true,
            ..RepairActions::default()
        },
    )
    .unwrap();

    assert_eq!(summary.forgotten_derived, [derivative("old", "locked")]);
    assert!(summary.removed_derived.is_empty());
    assert!(vault.derived.borrow().contains_key("locked"));
}

#[test]
fn forgets_a_packaging_model_whose_back_or_spine_scan_is_no_longer_referenced() {
    let model = RecordedDerivative {
        other_originals: vec!["old-back".to_owned(), "spine".to_owned()],
        ..derivative("front", "model")
    };
    let vault = FakeVault {
        referenced: vec!["front".to_owned(), "spine".to_owned()],
        derivatives: RefCell::new(vec![model.clone()]),
        originals: RefCell::new(intact(&["front", "spine"])),
        derived: RefCell::new(intact(&["model"])),
        ..FakeVault::default()
    };

    let summary = repair_vault(
        &vault,
        &vault,
        RepairActions {
            remove_orphaned_derived: true,
            ..RepairActions::default()
        },
    )
    .unwrap();

    assert_eq!(summary.forgotten_derived, [model]);
    assert_eq!(summary.removed_derived, ["model"]);
    assert!(summary.remaining.is_healthy(), "{:?}", summary.remaining);
}
