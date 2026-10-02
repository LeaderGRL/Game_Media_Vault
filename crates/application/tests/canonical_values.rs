mod support;

use game_media_vault_application::list_library;
use game_media_vault_domain::{
    CanonicalValue, LibraryEntry, ReleaseAssertion,
    ReleaseAssertionField::{self, Identifier, Region, Revision, Title},
    SourceId,
};
use support::FakeVault;

fn claim(source_id: &str, field: ReleaseAssertionField, value: &str) -> ReleaseAssertion {
    ReleaseAssertion {
        source_id: SourceId::from(source_id),
        source_location: format!("C:/catalogs/{source_id}.dat"),
        field,
        qualifier: None,
        value: value.to_owned(),
    }
}

fn release_with(assertions: Vec<ReleaseAssertion>) -> LibraryEntry {
    LibraryEntry {
        game_id: 1,
        game_title: "Super Mario Land".to_owned(),
        release_edition_id: 2,
        platform: "Nintendo - Game Boy".to_owned(),
        region: "World".to_owned(),
        edition_name: "Rev 1".to_owned(),
        assertions,
        assets: Vec::new(),
    }
}

fn canonical_values(assertions: Vec<ReleaseAssertion>) -> Vec<CanonicalValue> {
    let vault = FakeVault::with_library(vec![release_with(assertions)]);
    list_library(&vault).unwrap().remove(0).canonical_values
}

#[test]
fn the_value_most_sources_assert_becomes_canonical() {
    let mame = claim("mame", Title, "Super Mario Land DX");
    let no_intro = claim("no-intro", Title, "Super Mario Land");
    let redump = claim("redump", Title, " super mario land");

    let values = canonical_values(vec![mame.clone(), no_intro.clone(), redump.clone()]);

    assert_eq!(
        values,
        vec![CanonicalValue {
            field: Title,
            qualifier: None,
            value: "Super Mario Land".to_owned(),
            confidence: 66,
            contributing: vec![no_intro, redump],
            conflicting: vec![mame],
        }]
    );
}

#[test]
fn a_source_contributes_its_latest_claim_and_keeps_its_history() {
    let earlier = claim("no-intro", Region, "USA");
    let latest = claim("no-intro", Region, "USA, Europe");
    let vault = FakeVault::with_library(vec![release_with(vec![earlier.clone(), latest.clone()])]);

    let release = list_library(&vault).unwrap().remove(0);

    assert_eq!(
        release.canonical_values,
        vec![CanonicalValue {
            field: Region,
            qualifier: None,
            value: "USA, Europe".to_owned(),
            confidence: 100,
            contributing: vec![latest.clone()],
            conflicting: Vec::new(),
        }]
    );
    assert_eq!(release.entry.assertions, vec![earlier, latest]);
}

#[test]
fn claims_differing_only_in_spacing_agree() {
    let spaced = claim("no-intro", Region, "USA, Europe");
    let compact = claim("redump", Region, "USA,Europe");

    let values = canonical_values(vec![spaced.clone(), compact.clone()]);

    assert_eq!(values[0].value, "USA, Europe");
    assert_eq!(values[0].confidence, 100);
    assert_eq!(values[0].contributing, vec![spaced, compact]);
}

#[test]
fn a_tie_keeps_the_value_observed_first() {
    let first = claim("no-intro", Revision, "Rev 1");
    let second = claim("redump", Revision, "Rev A");

    let values = canonical_values(vec![first.clone(), second.clone()]);

    assert_eq!(values[0].value, "Rev 1");
    assert_eq!(values[0].confidence, 50);
    assert_eq!(values[0].contributing, vec![first]);
    assert_eq!(values[0].conflicting, vec![second]);
}

#[test]
fn identifiers_stay_plain_assertions() {
    let checksum = |source_id: &str, value: &str| ReleaseAssertion {
        qualifier: Some("sha1".to_owned()),
        ..claim(source_id, Identifier, value)
    };

    let values = canonical_values(vec![
        checksum("no-intro", "74591CC9504F3BDEBDAE9D9F8F9D7D68A6B4873B"),
        checksum("redump", "0000000000000000000000000000000000000000"),
    ]);

    assert!(values.is_empty());
}

#[test]
fn each_field_has_its_own_canonical_value() {
    let values = canonical_values(vec![
        claim("no-intro", Revision, "Rev 1"),
        claim("no-intro", Title, "Super Mario Land"),
        claim("no-intro", Region, "World"),
    ]);

    let fields: Vec<_> = values.iter().map(|value| value.field).collect();
    assert_eq!(fields, vec![Title, Region, Revision]);
}
