use std::{cell::RefCell, collections::BTreeMap};

use game_media_vault_application::{
    CorruptObject, ObjectArea, ObjectCheck, PortError, RecordedDerivative, UnreadableObject,
    VaultCatalogPort, VaultReport, VaultStorePort, verify_vault,
};

/// Marks a stored file that cannot be read.
const UNREADABLE: &str = "!unreadable";

/// What the catalog references and what the store holds, with the hash each stored file
/// actually has.
#[derive(Default)]
struct FakeVault {
    referenced: Vec<String>,
    derivatives: Vec<RecordedDerivative>,
    originals: BTreeMap<String, String>,
    derived: BTreeMap<String, String>,
    staging: Vec<String>,
}

impl VaultCatalogPort for FakeVault {
    fn referenced_originals(&self) -> Result<Vec<String>, PortError> {
        Ok(self.referenced.clone())
    }

    fn recorded_derivatives(&self) -> Result<Vec<RecordedDerivative>, PortError> {
        Ok(self.derivatives.clone())
    }
}

impl FakeVault {
    fn area(&self, area: ObjectArea) -> &BTreeMap<String, String> {
        match area {
            ObjectArea::Original => &self.originals,
            ObjectArea::Derived => &self.derived,
        }
    }
}

impl VaultStorePort for FakeVault {
    fn stored_objects(&self, area: ObjectArea) -> Result<Vec<String>, PortError> {
        Ok(self.area(area).keys().cloned().collect())
    }

    fn check_object(&self, area: ObjectArea, hash: &str) -> Result<ObjectCheck, PortError> {
        Ok(match self.area(area).get(hash) {
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
        Ok(self.staging.clone())
    }
}

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
    }
}

#[test]
fn a_vault_whose_references_and_objects_agree_is_healthy() {
    let vault = FakeVault {
        referenced: vec!["aaa".to_owned()],
        derivatives: vec![derivative("aaa", "thumb-aaa")],
        originals: intact(&["aaa"]),
        derived: intact(&["thumb-aaa"]),
        ..FakeVault::default()
    };

    let report = verify_vault(&vault, &vault).unwrap();

    assert_eq!(report, VaultReport::default());
    assert!(report.is_healthy());
}

#[test]
fn reports_missing_corrupt_and_unreferenced_originals_without_changing_anything() {
    let mut originals = intact(&["aaa", "loose"]);
    originals.insert("bbb".to_owned(), "not-bbb".to_owned());
    let vault = FakeVault {
        referenced: vec!["aaa".to_owned(), "bbb".to_owned(), "gone".to_owned()],
        originals,
        ..FakeVault::default()
    };

    let report = verify_vault(&vault, &vault).unwrap();

    assert_eq!(report.missing_originals, ["gone"]);
    assert_eq!(
        report.corrupt_originals,
        [CorruptObject {
            hash: "bbb".to_owned(),
            actual_hash: "not-bbb".to_owned(),
        }]
    );
    assert_eq!(report.unreferenced_originals, ["loose"]);
    assert!(!report.is_healthy());
    assert_eq!(vault.originals.len(), 3);
}

#[test]
fn reports_derived_assets_missing_corrupt_or_orphaned() {
    let mut derived = intact(&["thumb-aaa", "thumb-old", "stray"]);
    derived.insert("thumb-ccc".to_owned(), "garbled".to_owned());
    let vault = FakeVault {
        referenced: vec!["aaa".to_owned(), "ccc".to_owned(), "ddd".to_owned()],
        derivatives: vec![
            derivative("aaa", "thumb-aaa"),
            derivative("ccc", "thumb-ccc"),
            derivative("ddd", "thumb-ddd"),
            // Its original is no longer referenced by any retained Asset.
            derivative("old", "thumb-old"),
        ],
        originals: intact(&["aaa", "ccc", "ddd"]),
        derived,
        ..FakeVault::default()
    };

    let report = verify_vault(&vault, &vault).unwrap();

    assert_eq!(report.missing_derived, ["thumb-ddd"]);
    assert_eq!(
        report.corrupt_derived,
        [CorruptObject {
            hash: "thumb-ccc".to_owned(),
            actual_hash: "garbled".to_owned(),
        }]
    );
    // Derived files no record lists are orphaned too.
    assert_eq!(report.orphaned_derived, ["thumb-old", "stray"]);
}

#[test]
fn reports_staging_files_left_by_interrupted_stores() {
    let vault = FakeVault {
        staging: vec!["4242-0.tmp".to_owned()],
        ..FakeVault::default()
    };

    let report = verify_vault(&vault, &vault).unwrap();

    assert_eq!(report.interrupted_staging, ["4242-0.tmp"]);
}

#[test]
fn an_object_that_cannot_be_read_is_reported_and_verification_goes_on() {
    let mut originals = intact(&["aaa", "ccc"]);
    originals.insert("bbb".to_owned(), UNREADABLE.to_owned());
    let vault = FakeVault {
        referenced: vec!["aaa".to_owned(), "bbb".to_owned(), "gone".to_owned()],
        originals,
        ..FakeVault::default()
    };

    let report = verify_vault(&vault, &vault).unwrap();

    assert_eq!(
        report.unreadable_originals,
        [UnreadableObject {
            hash: "bbb".to_owned(),
            reason: "permission denied".to_owned(),
        }]
    );
    assert_eq!(report.missing_originals, ["gone"]);
    assert_eq!(report.unreferenced_originals, ["ccc"]);
}

#[test]
fn a_derived_object_shared_with_a_referenced_original_is_not_orphaned() {
    let vault = FakeVault {
        referenced: vec!["aaa".to_owned(), "bbb".to_owned()],
        derivatives: vec![
            // Identical outputs of two originals, one of them no longer referenced.
            derivative("aaa", "thumb-x"),
            derivative("old", "thumb-x"),
            derivative("aaa", "thumb-y"),
            derivative("bbb", "thumb-y"),
        ],
        originals: intact(&["aaa", "bbb"]),
        derived: intact(&["thumb-x"]),
        ..FakeVault::default()
    };

    let report = verify_vault(&vault, &vault).unwrap();

    assert!(
        report.orphaned_derived.is_empty(),
        "{:?}",
        report.orphaned_derived
    );
    // A missing output several Derived Assets share is reported once.
    assert_eq!(report.missing_derived, ["thumb-y"]);
}

/// An original another task stores and records right after verification first reads the vault.
#[derive(Default)]
struct ConcurrentlyStoringVault {
    referenced: RefCell<Vec<String>>,
    originals: RefCell<Vec<String>>,
    stored_meanwhile: RefCell<bool>,
}

impl ConcurrentlyStoringVault {
    fn store_meanwhile(&self) {
        if !self.stored_meanwhile.replace(true) {
            self.originals.borrow_mut().push("late".to_owned());
            self.referenced.borrow_mut().push("late".to_owned());
        }
    }
}

impl VaultCatalogPort for ConcurrentlyStoringVault {
    fn referenced_originals(&self) -> Result<Vec<String>, PortError> {
        let referenced = self.referenced.borrow().clone();
        self.store_meanwhile();
        Ok(referenced)
    }

    fn recorded_derivatives(&self) -> Result<Vec<RecordedDerivative>, PortError> {
        Ok(Vec::new())
    }
}

impl VaultStorePort for ConcurrentlyStoringVault {
    fn stored_objects(&self, area: ObjectArea) -> Result<Vec<String>, PortError> {
        let stored = match area {
            ObjectArea::Original => self.originals.borrow().clone(),
            ObjectArea::Derived => Vec::new(),
        };
        self.store_meanwhile();
        Ok(stored)
    }

    fn check_object(&self, _area: ObjectArea, hash: &str) -> Result<ObjectCheck, PortError> {
        Ok(
            if self.originals.borrow().iter().any(|stored| stored == hash) {
                ObjectCheck::Intact
            } else {
                ObjectCheck::Missing
            },
        )
    }

    fn staging_files(&self) -> Result<Vec<String>, PortError> {
        Ok(Vec::new())
    }
}

#[test]
fn an_original_stored_while_verifying_is_not_reported_unreferenced() {
    let vault = ConcurrentlyStoringVault::default();

    let report = verify_vault(&vault, &vault).unwrap();

    assert!(report.is_healthy(), "{report:?}");
}
