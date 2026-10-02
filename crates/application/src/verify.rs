use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{ApplicationError, PortError};

/// A Derived Asset the catalog records, by the hashes of its original and of its output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedDerivative {
    pub original_hash: String,
    pub object_hash: String,
}

/// What the catalog says the object store should hold.
pub trait VaultCatalogPort {
    /// The originals retained Assets reference, each once.
    fn referenced_originals(&self) -> Result<Vec<String>, PortError>;

    fn recorded_derivatives(&self) -> Result<Vec<RecordedDerivative>, PortError>;
}

/// The two areas of the object store: originals and the Derived Assets kept apart from them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectArea {
    Original,
    Derived,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectCheck {
    Intact,
    Missing,
    /// The stored bytes no longer hash to the object's address.
    Corrupt {
        actual_hash: String,
    },
    /// The stored bytes could not be read, so they could not be checked.
    Unreadable {
        reason: String,
    },
}

/// What the object store holds; checking an object rehashes its bytes.
pub trait VaultStorePort {
    fn stored_objects(&self, area: ObjectArea) -> Result<Vec<String>, PortError>;

    fn check_object(&self, area: ObjectArea, hash: &str) -> Result<ObjectCheck, PortError>;

    /// Staging files left behind, which only an interrupted store leaves while no other
    /// process stores objects.
    fn staging_files(&self) -> Result<Vec<String>, PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CorruptObject {
    pub hash: String,
    pub actual_hash: String,
}

/// A stored object whose bytes could not be read to check them, such as a file the vault may
/// not open or a damaged disk sector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnreadableObject {
    pub hash: String,
    pub reason: String,
}

/// Every disagreement between the catalog and the object store, in catalog and then store
/// order. Verifying changes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct VaultReport {
    /// Originals retained Assets reference that the store lacks.
    pub missing_originals: Vec<String>,
    pub corrupt_originals: Vec<CorruptObject>,
    pub unreadable_originals: Vec<UnreadableObject>,
    /// Stored originals no retained Asset references, such as those below the quality
    /// requirements of their run.
    pub unreferenced_originals: Vec<String>,
    /// Outputs of recorded Derived Assets, each reported once however many record it.
    pub missing_derived: Vec<String>,
    pub corrupt_derived: Vec<CorruptObject>,
    pub unreadable_derived: Vec<UnreadableObject>,
    /// Outputs only Derived Assets of unreferenced originals record, then derived files no
    /// record lists.
    pub orphaned_derived: Vec<String>,
    pub interrupted_staging: Vec<String>,
}

impl VaultReport {
    pub fn is_healthy(&self) -> bool {
        self == &Self::default()
    }
}

/// Where a checked object goes in the report.
struct ObjectFindings<'a> {
    missing: &'a mut Vec<String>,
    corrupt: &'a mut Vec<CorruptObject>,
    unreadable: &'a mut Vec<UnreadableObject>,
}

impl ObjectFindings<'_> {
    fn record(&mut self, hash: &str, check: ObjectCheck) {
        match check {
            ObjectCheck::Intact => {}
            ObjectCheck::Missing => self.missing.push(hash.to_owned()),
            ObjectCheck::Corrupt { actual_hash } => self.corrupt.push(CorruptObject {
                hash: hash.to_owned(),
                actual_hash,
            }),
            ObjectCheck::Unreadable { reason } => self.unreadable.push(UnreadableObject {
                hash: hash.to_owned(),
                reason,
            }),
        }
    }
}

/// Compares what the catalog references with what the object store holds, rehashing every
/// referenced object. The store is listed before the catalog is read, so an object another
/// task stores and records meanwhile is seen as referenced rather than unreferenced.
pub fn verify_vault(
    catalog: &dyn VaultCatalogPort,
    store: &dyn VaultStorePort,
) -> Result<VaultReport, ApplicationError> {
    let stored_originals = store.stored_objects(ObjectArea::Original)?;
    let stored_derived = store.stored_objects(ObjectArea::Derived)?;
    let staging = store.staging_files()?;
    let referenced = catalog.referenced_originals()?;
    let derivatives = catalog.recorded_derivatives()?;
    let mut report = VaultReport::default();

    let mut originals = ObjectFindings {
        missing: &mut report.missing_originals,
        corrupt: &mut report.corrupt_originals,
        unreadable: &mut report.unreadable_originals,
    };
    for hash in &referenced {
        originals.record(hash, store.check_object(ObjectArea::Original, hash)?);
    }
    let referenced: HashSet<&str> = referenced.iter().map(String::as_str).collect();
    report.unreferenced_originals = stored_originals
        .into_iter()
        .filter(|hash| !referenced.contains(hash.as_str()))
        .collect();

    // An output several Derived Assets share is checked once, and stays in use while any of
    // them belongs to a referenced original.
    let mut outputs: Vec<&str> = Vec::new();
    let mut recorded: HashSet<&str> = HashSet::new();
    let mut in_use: HashSet<&str> = HashSet::new();
    for derivative in &derivatives {
        if recorded.insert(&derivative.object_hash) {
            outputs.push(&derivative.object_hash);
        }
        if referenced.contains(derivative.original_hash.as_str()) {
            in_use.insert(&derivative.object_hash);
        }
    }
    let mut derived = ObjectFindings {
        missing: &mut report.missing_derived,
        corrupt: &mut report.corrupt_derived,
        unreadable: &mut report.unreadable_derived,
    };
    for output in &outputs {
        derived.record(output, store.check_object(ObjectArea::Derived, output)?);
    }
    report.orphaned_derived = outputs
        .iter()
        .filter(|output| !in_use.contains(*output))
        .map(|output| (*output).to_owned())
        .chain(
            stored_derived
                .into_iter()
                .filter(|hash| !recorded.contains(hash.as_str())),
        )
        .collect();

    report.interrupted_staging = staging;
    Ok(report)
}

/// The catalog side of the repairs a vault allows.
pub trait VaultRepairCatalogPort: VaultCatalogPort {
    /// Forgets the Derived Assets whose outputs are `object_hashes`, so their recipes run again.
    fn forget_derivatives(&self, object_hashes: &[String]) -> Result<(), PortError>;
}

/// The store side of the repairs a vault allows; removing what is already gone succeeds.
pub trait VaultRepairStorePort: VaultStorePort {
    fn remove_object(&self, area: ObjectArea, hash: &str) -> Result<(), PortError>;

    fn remove_staging_file(&self, name: &str) -> Result<(), PortError>;
}

/// Repairs to apply, each named explicitly. Missing and corrupt originals are never repaired:
/// their bytes cannot be recovered from the vault.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairActions {
    /// Deletes the staging files interrupted stores left.
    pub remove_interrupted_staging: bool,
    /// Forgets the Derived Assets of unreferenced originals and deletes derived files no record
    /// lists.
    pub remove_orphaned_derived: bool,
    /// Forgets missing and corrupt Derived Assets, deleting corrupt files, so they render again.
    pub reset_damaged_derived: bool,
    /// Deletes stored originals no retained Asset references.
    pub collect_unreferenced_originals: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepairSummary {
    pub removed_staging: Vec<String>,
    /// Derived Assets forgotten or deleted: damaged ones, then orphaned ones.
    pub removed_derived: Vec<String>,
    pub collected_originals: Vec<String>,
    /// What verification still reports once the repairs are done.
    pub remaining: VaultReport,
}

/// Verifies the vault, applies the requested `actions` to what it found, then verifies again.
/// It assumes no other process uses the vault meanwhile: an interrupted store and one still
/// running look alike.
pub fn repair_vault(
    catalog: &dyn VaultRepairCatalogPort,
    store: &dyn VaultRepairStorePort,
    actions: RepairActions,
) -> Result<RepairSummary, ApplicationError> {
    if actions == RepairActions::default() {
        return Err(ApplicationError::NoRepairAction);
    }
    let report = verify_vault(catalog, store)?;
    let mut summary = RepairSummary {
        removed_staging: Vec::new(),
        removed_derived: Vec::new(),
        collected_originals: Vec::new(),
        remaining: VaultReport::default(),
    };

    if actions.remove_interrupted_staging {
        for name in report.interrupted_staging {
            store.remove_staging_file(&name)?;
            summary.removed_staging.push(name);
        }
    }
    let mut removed_derived = Vec::new();
    if actions.reset_damaged_derived {
        removed_derived.extend(report.missing_derived);
        removed_derived.extend(report.corrupt_derived.into_iter().map(|object| object.hash));
    }
    if actions.remove_orphaned_derived {
        removed_derived.extend(report.orphaned_derived);
    }
    if !removed_derived.is_empty() {
        catalog.forget_derivatives(&removed_derived)?;
        for hash in &removed_derived {
            store.remove_object(ObjectArea::Derived, hash)?;
        }
        summary.removed_derived = removed_derived;
    }
    if actions.collect_unreferenced_originals {
        for hash in report.unreferenced_originals {
            store.remove_object(ObjectArea::Original, &hash)?;
            summary.collected_originals.push(hash);
        }
    }

    summary.remaining = verify_vault(catalog, store)?;
    Ok(summary)
}
