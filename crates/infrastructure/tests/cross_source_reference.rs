use game_media_vault_application::{CatalogPort, ReferenceCatalogRepositoryPort};
use game_media_vault_domain::{
    ImportedReleaseEdition, ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField,
    SourceId,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::{TempDir, tempdir};

const GAME_BOY: &str = "Nintendo - Game Boy";

#[derive(Clone, Copy)]
struct Release<'a> {
    source: &'a str,
    title: &'a str,
    platform: &'a str,
    region: &'a str,
    sha1: Option<&'a str>,
}

fn record(release: &Release<'_>) -> ReferenceReleaseRecord {
    let assertion = |field, qualifier: Option<&str>, value: &str| ReleaseAssertion {
        source_id: SourceId::from(release.source),
        source_location: format!("C:/catalogs/{}.xml", release.source),
        field,
        qualifier: qualifier.map(str::to_owned),
        value: value.to_owned(),
    };
    let mut assertions = vec![
        assertion(ReleaseAssertionField::Title, None, release.title),
        assertion(ReleaseAssertionField::Region, None, release.region),
        assertion(
            ReleaseAssertionField::Identifier,
            Some("source_record"),
            &format!("{}|{}|{}", release.title, release.platform, release.region),
        ),
    ];
    // Several dumps, such as the ROMs or tracks of one release, are separated by commas.
    for sha1 in release.sha1.into_iter().flat_map(|sha1| sha1.split(',')) {
        assertions.push(assertion(
            ReleaseAssertionField::Identifier,
            Some("sha1"),
            sha1,
        ));
    }
    ReferenceReleaseRecord {
        game_title: release.title.to_owned(),
        platform: release.platform.to_owned(),
        region: release.region.to_owned(),
        revision: None,
        edition_name: "Standard".to_owned(),
        assertions,
    }
}

fn catalog() -> (TempDir, SqliteCatalog) {
    let temp = tempdir().unwrap();
    let catalog = SqliteCatalog::open(temp.path().join("catalog.sqlite3")).unwrap();
    (temp, catalog)
}

fn import(catalog: &SqliteCatalog, release: &Release<'_>) -> ImportedReleaseEdition {
    catalog.persist_reference_release(record(release)).unwrap()
}

/// What `source` asserts as identifiers, as `qualifier=value`, on the edition `id`.
fn identifiers_of(catalog: &SqliteCatalog, id: i64, source: &str) -> Vec<String> {
    catalog
        .list_library()
        .unwrap()
        .into_iter()
        .find(|entry| entry.release_edition_id == id)
        .unwrap()
        .assertions
        .into_iter()
        .filter(|assertion| {
            assertion.source_id.as_str() == source
                && assertion.field == ReleaseAssertionField::Identifier
        })
        .map(|assertion| {
            format!(
                "{}={}",
                assertion.qualifier.unwrap_or_default(),
                assertion.value
            )
        })
        .collect()
}

fn links_of(catalog: &SqliteCatalog, id: i64, source: &str) -> Vec<String> {
    identifiers_of(catalog, id, source)
        .into_iter()
        .filter(|identifier| identifier.starts_with("linked_by="))
        .collect()
}

#[test]
fn the_same_release_from_two_sources_is_one_release_edition_carrying_both() {
    let (_temp, catalog) = catalog();
    let no_intro = import(
        &catalog,
        &Release {
            source: "no-intro",
            title: "Tetris",
            platform: GAME_BOY,
            region: "World",
            sha1: Some("aaaa"),
        },
    );
    // Another dump format, so no checksum is shared.
    let mame = import(
        &catalog,
        &Release {
            source: "mame-software-lists",
            title: "TETRIS",
            platform: GAME_BOY,
            region: "World",
            sha1: Some("bbbb"),
        },
    );

    assert_eq!(mame, no_intro);
    let library = catalog.list_library().unwrap();
    assert_eq!(library.len(), 1);
    let sources: Vec<&str> = library[0]
        .assertions
        .iter()
        .map(|assertion| assertion.source_id.as_str())
        .collect();
    assert!(sources.contains(&"no-intro"));
    assert!(sources.contains(&"mame-software-lists"));
    // The evidence of the link stays with the assertions of the source that made it.
    assert_eq!(
        links_of(&catalog, mame.release_edition_id, "mame-software-lists"),
        ["linked_by=title"]
    );
}

#[test]
fn a_shared_dump_checksum_alone_links_nothing() {
    let (_temp, catalog) = catalog();
    let no_intro = import(
        &catalog,
        &Release {
            source: "no-intro",
            title: "Zelda no Densetsu 1 - The Hyrule Fantasy",
            platform: GAME_BOY,
            region: "Japan",
            sha1: Some("cccc"),
        },
    );

    // The catalog keeps every dump a source ever asserted, so it cannot tell the dumps of one
    // release from those of another: a checksum is no evidence yet.
    let mame = import(
        &catalog,
        &Release {
            source: "mame-software-lists",
            title: "Zelda no Densetsu - The Hyrule Fantasy",
            platform: GAME_BOY,
            region: "Japan",
            sha1: Some("CCCC"),
        },
    );

    assert_ne!(mame.release_edition_id, no_intro.release_edition_id);
    assert!(links_of(&catalog, mame.release_edition_id, "mame-software-lists").is_empty());
}

#[test]
fn releases_of_another_region_platform_or_title_stay_apart() {
    let (_temp, catalog) = catalog();
    let tetris = import(
        &catalog,
        &Release {
            source: "no-intro",
            title: "Tetris",
            platform: GAME_BOY,
            region: "World",
            sha1: Some("aaaa"),
        },
    );

    let japanese = import(
        &catalog,
        &Release {
            source: "mame-software-lists",
            title: "Tetris",
            platform: GAME_BOY,
            region: "Japan",
            sha1: None,
        },
    );
    // Neither a title nor a checksum links releases of another platform.
    let nes = import(
        &catalog,
        &Release {
            source: "mame-software-lists",
            title: "Tetris",
            platform: "Nintendo - Nintendo Entertainment System",
            region: "World",
            sha1: Some("aaaa"),
        },
    );
    let other = import(
        &catalog,
        &Release {
            source: "mame-software-lists",
            title: "Tetris 2",
            platform: GAME_BOY,
            region: "World",
            sha1: None,
        },
    );

    // Another edition of the same game on the same platform is still that game.
    assert_ne!(japanese.release_edition_id, tetris.release_edition_id);
    assert_eq!(japanese.game_id, tetris.game_id);
    assert_ne!(nes.release_edition_id, tetris.release_edition_id);
    assert_ne!(other.game_id, tetris.game_id);
    assert_eq!(catalog.list_library().unwrap().len(), 4);
    assert!(links_of(&catalog, japanese.release_edition_id, "mame-software-lists").is_empty());
}

#[test]
fn importing_a_linked_release_again_keeps_one_link() {
    let (_temp, catalog) = catalog();
    let tetris = Release {
        source: "no-intro",
        title: "Tetris",
        platform: GAME_BOY,
        region: "World",
        sha1: Some("aaaa"),
    };
    let mame = Release {
        source: "mame-software-lists",
        ..tetris
    };
    import(&catalog, &tetris);

    let first = import(&catalog, &mame);
    let again = import(&catalog, &mame);
    import(&catalog, &tetris);

    assert_eq!(again, first);
    assert_eq!(catalog.list_library().unwrap().len(), 1);
    assert_eq!(
        links_of(&catalog, first.release_edition_id, "mame-software-lists"),
        ["linked_by=title"]
    );
}

#[test]
fn canonical_values_of_a_linked_release_weigh_both_sources() {
    let (_temp, catalog) = catalog();
    import(
        &catalog,
        &Release {
            source: "no-intro",
            title: "Tetris",
            platform: GAME_BOY,
            region: "World",
            sha1: Some("aaaa"),
        },
    );
    import(
        &catalog,
        &Release {
            source: "mame-software-lists",
            title: "TETRIS",
            platform: GAME_BOY,
            region: "World",
            sha1: Some("bbbb"),
        },
    );

    let library = game_media_vault_application::list_library(&catalog).unwrap();

    let canonical = |field| {
        library[0]
            .canonical_values
            .iter()
            .find(|value| value.field == field)
            .unwrap()
    };
    // Both sources back the title, however they spell it, and the region.
    let title = canonical(ReleaseAssertionField::Title);
    assert_eq!(title.contributing.len(), 2);
    assert_eq!(title.value, "Tetris");
    assert_eq!(
        canonical(ReleaseAssertionField::Region).contributing.len(),
        2
    );
}

#[test]
fn a_third_source_joins_the_edition_two_sources_share() {
    let (_temp, catalog) = catalog();
    let tetris = Release {
        source: "no-intro",
        title: "Tetris",
        platform: GAME_BOY,
        region: "World",
        sha1: Some("aaaa"),
    };
    let no_intro = import(&catalog, &tetris);
    import(
        &catalog,
        &Release {
            source: "mame-software-lists",
            ..tetris
        },
    );

    // Two sources titling one edition alike make one candidate, not an ambiguity.
    let third = import(
        &catalog,
        &Release {
            source: "another-catalog",
            sha1: None,
            ..tetris
        },
    );

    assert_eq!(third, no_intro);
    assert_eq!(
        links_of(&catalog, third.release_edition_id, "another-catalog"),
        ["linked_by=title"]
    );
}

#[test]
fn a_link_moves_with_the_catalog_it_was_imported_from() {
    let (_temp, catalog) = catalog();
    let tetris = Release {
        source: "no-intro",
        title: "Tetris",
        platform: GAME_BOY,
        region: "World",
        sha1: Some("aaaa"),
    };
    import(&catalog, &tetris);
    let mame = Release {
        source: "mame-software-lists",
        ..tetris
    };
    let linked = import(&catalog, &mame);

    // The same list imported again from another path.
    let mut moved = record(&mame);
    for assertion in &mut moved.assertions {
        assertion.source_location = "D:/moved/mame.xml".to_owned();
    }
    catalog.persist_reference_release(moved).unwrap();

    let link = catalog
        .list_library()
        .unwrap()
        .into_iter()
        .find(|entry| entry.release_edition_id == linked.release_edition_id)
        .unwrap()
        .assertions
        .into_iter()
        .find(|assertion| assertion.qualifier.as_deref() == Some("linked_by"))
        .unwrap();
    assert_eq!(link.source_location, "D:/moved/mame.xml");
}

#[test]
fn a_title_several_editions_share_links_none_of_them() {
    let (_temp, catalog) = catalog();
    let deluxe = Release {
        source: "no-intro",
        title: "Tetris DX",
        platform: GAME_BOY,
        region: "World",
        sha1: None,
    };
    let first = import(&catalog, &deluxe);
    let second = import(
        &catalog,
        &Release {
            source: "mame-software-lists",
            title: "Tetris",
            ..deluxe
        },
    );
    // No-Intro then retitles its release, which keeps its edition: editions of two Games now
    // carry the same title.
    let mut retitled = record(&deluxe);
    retitled.game_title = "Tetris".to_owned();
    for assertion in &mut retitled.assertions {
        if assertion.field == ReleaseAssertionField::Title {
            assertion.value = "Tetris".to_owned();
        }
    }
    assert_eq!(catalog.persist_reference_release(retitled).unwrap(), first);

    let third = import(
        &catalog,
        &Release {
            source: "another-catalog",
            title: "Tetris",
            ..deluxe
        },
    );

    // Nor does the record join either edition through the Game its title names.
    assert_ne!(third.release_edition_id, first.release_edition_id);
    assert_ne!(third.release_edition_id, second.release_edition_id);
    assert!(links_of(&catalog, third.release_edition_id, "another-catalog").is_empty());
    assert_eq!(catalog.list_library().unwrap().len(), 3);
}
