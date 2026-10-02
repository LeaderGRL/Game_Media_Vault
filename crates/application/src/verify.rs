use std::collections::HashSet;

use serde::Serialize;

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

/// Every disagreement between the catalog and the object store, in catalog and then store
/// order. Verifying changes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct VaultReport {
    /// Originals retained Assets reference that the store lacks.
    pub missing_originals: Vec<String>,
    pub corrupt_originals: Vec<CorruptObject>,
    /// Stored originals no retained Asset references, such as those below the quality
    /// requirements of their run.
    pub unreferenced_originals: Vec<String>,
    pub missing_derived: Vec<String>,
    pub corrupt_derived: Vec<CorruptObject>,
    /// Derived Assets of originals no retained Asset references, then derived files no record
    /// lists.
    pub orphaned_derived: Vec<String>,
    pub interrupted_staging: Vec<String>,
}

impl VaultReport {
    pub fn is_healthy(&self) -> bool {
        self == &Self::default()
    }
}

/// Compares what the catalog references with what the object store holds, rehashing every
/// referenced object.
pub fn verify_vault(
    catalog: &dyn VaultCatalogPort,
    store: &dyn VaultStorePort,
) -> Result<VaultReport, ApplicationError> {
    let mut report = VaultReport::default();

    let referenced = catalog.referenced_originals()?;
    for hash in &referenced {
        match store.check_object(ObjectArea::Original, hash)? {
            ObjectCheck::Intact => {}
            ObjectCheck::Missing => report.missing_originals.push(hash.clone()),
            ObjectCheck::Corrupt { actual_hash } => report.corrupt_originals.push(CorruptObject {
                hash: hash.clone(),
                actual_hash,
            }),
        }
    }
    let referenced: HashSet<String> = referenced.into_iter().collect();
    report.unreferenced_originals = store
        .stored_objects(ObjectArea::Original)?
        .into_iter()
        .filter(|hash| !referenced.contains(hash))
        .collect();

    let derivatives = catalog.recorded_derivatives()?;
    for derivative in &derivatives {
        match store.check_object(ObjectArea::Derived, &derivative.object_hash)? {
            ObjectCheck::Intact => {}
            ObjectCheck::Missing => report.missing_derived.push(derivative.object_hash.clone()),
            ObjectCheck::Corrupt { actual_hash } => report.corrupt_derived.push(CorruptObject {
                hash: derivative.object_hash.clone(),
                actual_hash,
            }),
        }
        if !referenced.contains(&derivative.original_hash) {
            report.orphaned_derived.push(derivative.object_hash.clone());
        }
    }
    let recorded: HashSet<&str> = derivatives
        .iter()
        .map(|derivative| derivative.object_hash.as_str())
        .collect();
    let stray: Vec<String> = store
        .stored_objects(ObjectArea::Derived)?
        .into_iter()
        .filter(|hash| !recorded.contains(hash.as_str()))
        .collect();
    report.orphaned_derived.extend(stray);

    report.interrupted_staging = store.staging_files()?;
    Ok(report)
}
