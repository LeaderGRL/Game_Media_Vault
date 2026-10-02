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
    if let Some(sha1) = release.sha1 {
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
fn a_shared_dump_checksum_links_releases_whose_titles_differ() {
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

    assert_eq!(mame, no_intro);
    assert_eq!(
        links_of(&catalog, mame.release_edition_id, "mame-software-lists"),
        ["linked_by=sha1"]
    );
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
    // A checksum never links releases of another platform.
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
        ["linked_by=sha1"]
    );
}

#[test]
fn a_checksum_several_editions_share_links_none_of_them() {
    let (_temp, catalog) = catalog();
    for region in ["USA", "Europe"] {
        import(
            &catalog,
            &Release {
                source: "no-intro",
                title: "Dr. Mario",
                platform: GAME_BOY,
                region,
                sha1: Some("dddd"),
            },
        );
    }

    let mame = import(
        &catalog,
        &Release {
            source: "mame-software-lists",
            title: "Doctor Mario",
            platform: GAME_BOY,
            region: "World",
            sha1: Some("dddd"),
        },
    );

    assert_eq!(catalog.list_library().unwrap().len(), 3);
    assert!(links_of(&catalog, mame.release_edition_id, "mame-software-lists").is_empty());
}

#[test]
fn canonical_values_of_a_linked_release_weigh_both_sources() {
    let (_temp, catalog) = catalog();
    import(
        &catalog,
        &Release {
            source: "no-intro",
            title: "Zelda no Densetsu 1 - The Hyrule Fantasy",
            platform: GAME_BOY,
            region: "Japan",
            sha1: Some("cccc"),
        },
    );
    import(
        &catalog,
        &Release {
            source: "mame-software-lists",
            title: "Zelda no Densetsu - The Hyrule Fantasy",
            platform: GAME_BOY,
            region: "Japan",
            sha1: Some("cccc"),
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
    // The sources agree on the region and disagree on the title.
    assert_eq!(
        canonical(ReleaseAssertionField::Region).contributing.len(),
        2
    );
    let title = canonical(ReleaseAssertionField::Title);
    assert_eq!(title.contributing.len() + title.conflicting.len(), 2);
    assert_eq!(title.conflicting.len(), 1);
}
