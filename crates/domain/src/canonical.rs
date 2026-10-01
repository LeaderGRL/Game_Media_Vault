use serde::{Deserialize, Serialize};

use crate::{LibraryEntry, ReleaseAssertion, ReleaseAssertionField};

/// Fields a Release Edition has a single value for. Identifiers are multi-valued (one checksum
/// per dump, one record per source) and stay plain assertions.
const SINGLE_VALUED_FIELDS: [ReleaseAssertionField; 3] = [
    ReleaseAssertionField::Title,
    ReleaseAssertionField::Region,
    ReleaseAssertionField::Revision,
];

/// Value Game Media Vault currently selects for one field of a Release Edition, with the
/// assertions it rests on. Assertions are never rewritten: a new claim changes the selection
/// while earlier claims remain history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalValue {
    pub field: ReleaseAssertionField,
    pub qualifier: Option<String>,
    pub value: String,
    /// Share of the asserting Sources that agree with `value`, in percent.
    pub confidence: u8,
    /// Current claims of the Sources that agree with `value`.
    pub contributing: Vec<ReleaseAssertion>,
    /// Current claims of the Sources that assert another value.
    pub conflicting: Vec<ReleaseAssertion>,
}

/// A library Release Edition with the Canonical Values derived from its assertions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryRelease {
    #[serde(flatten)]
    pub entry: LibraryEntry,
    pub canonical_values: Vec<CanonicalValue>,
}

impl From<LibraryEntry> for LibraryRelease {
    fn from(entry: LibraryEntry) -> Self {
        let canonical_values = canonical_values(&entry.assertions);
        Self {
            entry,
            canonical_values,
        }
    }
}

/// Derives the Canonical Values of single-valued fields from assertions listed in recording
/// order. Each Source takes part with its most recently recorded claim; the value most Sources
/// agree on (ignoring case and spacing) wins, and a tie keeps the value claimed first.
pub fn canonical_values(assertions: &[ReleaseAssertion]) -> Vec<CanonicalValue> {
    let mut values = Vec::new();
    for field in SINGLE_VALUED_FIELDS {
        let mut qualifiers: Vec<Option<&str>> = Vec::new();
        for assertion in assertions
            .iter()
            .filter(|assertion| assertion.field == field)
        {
            if !qualifiers.contains(&assertion.qualifier.as_deref()) {
                qualifiers.push(assertion.qualifier.as_deref());
            }
        }
        for qualifier in qualifiers {
            let claims: Vec<&ReleaseAssertion> = assertions
                .iter()
                .filter(|assertion| {
                    assertion.field == field && assertion.qualifier.as_deref() == qualifier
                })
                .collect();
            values.push(select(field, qualifier, &current_claims(&claims)));
        }
    }
    values
}

/// The latest claim of each Source, in recording order.
fn current_claims<'a>(claims: &[&'a ReleaseAssertion]) -> Vec<&'a ReleaseAssertion> {
    claims
        .iter()
        .enumerate()
        .filter(|(index, claim)| {
            !claims[index + 1..]
                .iter()
                .any(|later| later.source_id == claim.source_id)
        })
        .map(|(_, claim)| *claim)
        .collect()
}

fn select(
    field: ReleaseAssertionField,
    qualifier: Option<&str>,
    current: &[&ReleaseAssertion],
) -> CanonicalValue {
    let mut tallies: Vec<(String, usize)> = Vec::new();
    for claim in current {
        let key = comparable(&claim.value);
        match tallies.iter_mut().find(|(value, _)| *value == key) {
            Some((_, count)) => *count += 1,
            None => tallies.push((key, 1)),
        }
    }
    // `max_by_key` keeps the last maximum, so search the tallies backwards to keep the first.
    let winner = tallies
        .iter()
        .rev()
        .max_by_key(|(_, count)| *count)
        .map(|(value, _)| value.clone())
        .unwrap_or_default();
    let (contributing, conflicting): (Vec<ReleaseAssertion>, Vec<ReleaseAssertion>) = current
        .iter()
        .map(|claim| (*claim).clone())
        .partition(|claim| comparable(&claim.value) == winner);
    let confidence = contributing.len() * 100 / current.len().max(1);
    CanonicalValue {
        field,
        qualifier: qualifier.map(str::to_owned),
        value: contributing
            .first()
            .map(|claim| claim.value.trim().to_owned())
            .unwrap_or_default(),
        confidence: u8::try_from(confidence).unwrap_or(100),
        contributing,
        conflicting,
    }
}

fn comparable(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
