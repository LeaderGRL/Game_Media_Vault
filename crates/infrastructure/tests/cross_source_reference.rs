use game_media_vault_application::{
    CatalogPort, ReferenceCatalogRepositoryPort, ReferenceReviewOutcome,
    ReferenceReviewRepositoryPort,
};
use game_media_vault_domain::{
    AssetType, ImportedReleaseEdition, MediaInfo, PersistAsset, ReferenceReleaseRecord,
    ReleaseAssertion, ReleaseAssertionField, SourceId,
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

/// The SHA-1 of the dump a fixture names `token`: the token itself, in its case, when it is made
/// of hexadecimal digits, and otherwise its bytes in hexadecimal, padded to forty digits.
fn sha1_of(token: &str) -> String {
    let digits = if token.chars().all(|digit| digit.is_ascii_hexdigit()) {
        token.to_owned()
    } else {
        token.bytes().map(|byte| format!("{byte:02x}")).collect()
    };
    assert!(digits.len() <= 40, "fixture dump name too long: {token}");
    format!("{digits:0<40}")
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
    for (number, sha1) in release
        .sha1
        .into_iter()
        .flat_map(|sha1| sha1.split(','))
        .enumerate()
    {
        let name = format!("dump {number}.bin");
        assertions.push(assertion(
            ReleaseAssertionField::Identifier,
            Some("rom_name"),
            &name,
        ));
        assertions.push(assertion(
            ReleaseAssertionField::Identifier,
            Some("sha1"),
            &sha1_of(sha1),
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
    assert!(catalog.list_reference_review_items().unwrap().is_empty());
}

#[test]
fn a_shared_set_of_dumps_links_releases_whose_titles_differ() {
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

    // The very same dumps, whatever the case of their checksums.
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
        ["linked_by=sha1"]
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
    let items = catalog.list_reference_review_items().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].release_edition_id, third.release_edition_id);
    assert_eq!(items[0].evidence, "title");
    let mut candidates = items[0].candidates.clone();
    candidates.sort_unstable();
    let mut expected = vec![first.release_edition_id, second.release_edition_id];
    expected.sort_unstable();
    assert_eq!(candidates, expected);
}

fn release<'a>(source: &'a str, title: &'a str, sha1: &'a str) -> Release<'a> {
    Release {
        source,
        title,
        platform: GAME_BOY,
        region: "World",
        sha1: Some(sha1),
    }
}

#[test]
fn a_set_of_dumps_several_editions_share_links_none_of_them() {
    let (_temp, catalog) = catalog();
    // One catalog lists two releases of the very same dumps.
    let game_a = import(&catalog, &release("no-intro", "Game A", "aaaa"));
    let game_b = import(&catalog, &release("no-intro", "Game B", "aaaa"));

    let other = import(&catalog, &release("mame-software-lists", "Game C", "aaaa"));

    assert_eq!(catalog.list_library().unwrap().len(), 3);
    assert!(links_of(&catalog, other.release_edition_id, "mame-software-lists").is_empty());
    // A human tells which of them, if any, the record describes.
    let items = catalog.list_reference_review_items().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].source_id.as_str(), "mame-software-lists");
    assert_eq!(items[0].release_edition_id, other.release_edition_id);
    assert_eq!(items[0].evidence, "sha1");
    assert_eq!(
        items[0].candidates,
        [game_a.release_edition_id, game_b.release_edition_id]
    );
}

#[test]
fn part_of_a_set_of_dumps_is_no_evidence() {
    let (_temp, catalog) = catalog();
    import(&catalog, &release("no-intro", "Game A", "prg,shared-chr"));

    let other = import(
        &catalog,
        &release("mame-software-lists", "Game B", "shared-chr"),
    );

    assert_eq!(catalog.list_library().unwrap().len(), 2);
    assert!(links_of(&catalog, other.release_edition_id, "mame-software-lists").is_empty());
}

#[test]
fn a_release_with_a_dump_lacking_a_checksum_gives_no_dump_evidence() {
    let (_temp, catalog) = catalog();
    let mut partial = record(&release("no-intro", "Game A", "shared"));
    // A second dump without a SHA-1, so the set of dumps is not known whole.
    partial.assertions.push(ReleaseAssertion {
        source_id: SourceId::from("no-intro"),
        source_location: "C:/catalogs/no-intro.xml".to_owned(),
        field: ReleaseAssertionField::Identifier,
        qualifier: Some("rom_name".to_owned()),
        value: "unhashed.bin".to_owned(),
    });
    catalog.persist_reference_release(partial).unwrap();

    let other = import(
        &catalog,
        &release("mame-software-lists", "Game B", "shared"),
    );

    assert_eq!(catalog.list_library().unwrap().len(), 2);
    assert!(links_of(&catalog, other.release_edition_id, "mame-software-lists").is_empty());
}

#[test]
fn the_same_dumps_in_another_region_stay_another_release() {
    let (_temp, catalog) = catalog();
    let world = import(&catalog, &release("no-intro", "Game A", "aaaa"));

    let japan = import(
        &catalog,
        &Release {
            region: "Japan",
            ..release("mame-software-lists", "Game A (J)", "aaaa")
        },
    );

    assert_ne!(japan.release_edition_id, world.release_edition_id);
    assert!(links_of(&catalog, japan.release_edition_id, "mame-software-lists").is_empty());
}

#[test]
fn a_corrected_set_of_dumps_replaces_the_one_a_source_asserted_before() {
    let (_temp, catalog) = catalog();
    let original = import(&catalog, &release("no-intro", "Game A", "old-dump"));
    // The same release, imported again from a corrected datafile.
    let mut corrected = record(&release("no-intro", "Game A", "new-dump"));
    corrected
        .assertions
        .retain(|assertion| assertion.qualifier.as_deref() != Some("source_record"));
    corrected.assertions.extend(
        record(&release("no-intro", "Game A", "old-dump"))
            .assertions
            .into_iter()
            .filter(|assertion| assertion.qualifier.as_deref() == Some("source_record")),
    );
    catalog.persist_reference_release(corrected).unwrap();

    let stale = import(
        &catalog,
        &release("mame-software-lists", "Game B", "old-dump"),
    );
    let current = import(&catalog, &release("redump", "Game C", "new-dump"));

    assert_ne!(stale.release_edition_id, original.release_edition_id);
    assert_eq!(current.release_edition_id, original.release_edition_id);
    assert_eq!(
        links_of(&catalog, current.release_edition_id, "redump"),
        ["linked_by=sha1"]
    );
}

#[test]
fn a_set_of_dumps_never_joins_two_releases_of_one_source() {
    let (_temp, catalog) = catalog();
    let first = import(&catalog, &release("no-intro", "Game A", "aaaa"));
    import(&catalog, &release("mame-software-lists", "Game A", "aaaa"));

    // The first catalog lists another release of the very same dumps.
    let second = import(&catalog, &release("no-intro", "Game B", "aaaa"));

    assert_ne!(second.release_edition_id, first.release_edition_id);
    assert!(links_of(&catalog, second.release_edition_id, "no-intro").is_empty());
}

#[test]
fn dumps_outweigh_a_title_another_edition_shares() {
    let (_temp, catalog) = catalog();
    import(&catalog, &release("no-intro", "Tetris", "aaaa"));
    let deluxe = import(&catalog, &release("no-intro", "Tetris DX", "bbbb"));

    // Titled like the first release, dumped like the second.
    let other = import(&catalog, &release("mame-software-lists", "Tetris", "bbbb"));

    assert_eq!(other.release_edition_id, deluxe.release_edition_id);
    assert_eq!(
        links_of(&catalog, other.release_edition_id, "mame-software-lists"),
        ["linked_by=sha1"]
    );
}

#[test]
fn dumps_several_editions_share_link_none_even_once_one_holds_the_importing_source() {
    let (_temp, catalog) = catalog();
    let first = import(&catalog, &release("no-intro", "Game A", "aaaa"));
    import(&catalog, &release("mame-software-lists", "Game A", "aaaa"));
    // The first catalog then lists another release of the very same dumps.
    import(&catalog, &release("no-intro", "Game B", "aaaa"));

    let other = import(&catalog, &release("mame-software-lists", "Game C", "aaaa"));

    assert_ne!(other.release_edition_id, first.release_edition_id);
    assert!(links_of(&catalog, other.release_edition_id, "mame-software-lists").is_empty());
    assert_eq!(catalog.list_library().unwrap().len(), 3);
}

#[test]
fn a_checksum_that_is_no_sha1_gives_no_dump_evidence() {
    let (_temp, catalog) = catalog();
    // Both catalogs fill an unknown checksum with the same placeholder.
    let with_placeholder = |source, title| {
        let mut placeholder = record(&release(source, title, "aaaa"));
        for assertion in &mut placeholder.assertions {
            if assertion.qualifier.as_deref() == Some("sha1") {
                assertion.value = "none".to_owned();
            }
        }
        placeholder
    };
    catalog
        .persist_reference_release(with_placeholder("no-intro", "Game A"))
        .unwrap();

    let other = catalog
        .persist_reference_release(with_placeholder("mame-software-lists", "Game B"))
        .unwrap();

    assert_eq!(catalog.list_library().unwrap().len(), 2);
    assert!(links_of(&catalog, other.release_edition_id, "mame-software-lists").is_empty());
}

#[test]
fn dumps_pointing_at_an_edition_of_the_importing_source_forbid_a_title_link() {
    let (_temp, catalog) = catalog();
    let first = import(&catalog, &release("no-intro", "Alpha", "aaaa"));
    // Another catalog links its differently titled release by the shared dumps.
    import(&catalog, &release("mame-software-lists", "Beta", "aaaa"));

    // The first catalog's second release of those dumps carries the other catalog's title.
    let second = import(&catalog, &release("no-intro", "Beta", "aaaa"));

    assert_ne!(second.release_edition_id, first.release_edition_id);
    assert!(links_of(&catalog, second.release_edition_id, "no-intro").is_empty());
    // The only edition they point at is another release of that catalog: nothing is uncertain.
    assert!(catalog.list_reference_review_items().unwrap().is_empty());
}

/// Two releases of one catalog with the very same dumps, and a third catalog's record of those
/// dumps, which awaits review: the editions of the first two, the third's, and its item id.
fn reviewed_dumps(catalog: &SqliteCatalog) -> (i64, i64, i64, i64) {
    let game_a = import(catalog, &release("no-intro", "Game A", "aaaa"));
    let game_b = import(catalog, &release("no-intro", "Game B", "aaaa"));
    let other = import(catalog, &release("mame-software-lists", "Game C", "aaaa"));
    let item = catalog.list_reference_review_items().unwrap()[0].id;
    (
        game_a.release_edition_id,
        game_b.release_edition_id,
        other.release_edition_id,
        item,
    )
}

#[test]
fn linking_a_reviewed_record_merges_its_edition_into_the_chosen_one() {
    let (_temp, catalog) = catalog();
    let (_, game_b, other, item) = reviewed_dumps(&catalog);

    let outcome = catalog.link_reference_review_item(item, game_b).unwrap();

    assert_eq!(outcome, ReferenceReviewOutcome::Decided);
    // The record's claims join the chosen edition, with the evidence of the decision.
    assert_eq!(
        links_of(&catalog, game_b, "mame-software-lists"),
        ["linked_by=review"]
    );
    // Its own edition, emptied, is gone, and nothing awaits review anymore.
    let library = catalog.list_library().unwrap();
    assert_eq!(library.len(), 2);
    assert!(
        library
            .iter()
            .all(|entry| entry.release_edition_id != other)
    );
    assert!(catalog.list_reference_review_items().unwrap().is_empty());
    // A later import of the record keeps the decision.
    let again = import(&catalog, &release("mame-software-lists", "Game C", "aaaa"));
    assert_eq!(again.release_edition_id, game_b);
    assert!(catalog.list_reference_review_items().unwrap().is_empty());
}

#[test]
fn keeping_a_reviewed_record_apart_leaves_every_edition_as_it_is() {
    let (_temp, catalog) = catalog();
    let (_, _, other, item) = reviewed_dumps(&catalog);

    let outcome = catalog.keep_reference_review_item_apart(item).unwrap();

    assert_eq!(outcome, ReferenceReviewOutcome::Decided);
    assert_eq!(catalog.list_library().unwrap().len(), 3);
    assert!(links_of(&catalog, other, "mame-software-lists").is_empty());
    assert!(catalog.list_reference_review_items().unwrap().is_empty());
}

#[test]
fn a_reviewed_record_links_only_to_one_of_its_candidates() {
    let (_temp, catalog) = catalog();
    let (game_a, _, other, item) = reviewed_dumps(&catalog);

    assert_eq!(
        catalog.link_reference_review_item(item, other).unwrap(),
        ReferenceReviewOutcome::NotACandidate
    );
    assert_eq!(
        catalog
            .link_reference_review_item(item + 1, game_a)
            .unwrap(),
        ReferenceReviewOutcome::ItemNotPending
    );
    assert_eq!(catalog.list_reference_review_items().unwrap().len(), 1);
    assert_eq!(catalog.list_library().unwrap().len(), 3);
}

#[test]
fn an_edition_already_holding_assets_is_not_merged_yet() {
    let (_temp, catalog) = catalog();
    let (game_a, _, other, item) = reviewed_dumps(&catalog);
    let game_c = catalog
        .list_library()
        .unwrap()
        .into_iter()
        .find(|entry| entry.release_edition_id == other)
        .unwrap()
        .game_id;
    // An acquisition attached an Asset to the record's edition meanwhile.
    catalog
        .persist_asset(PersistAsset {
            existing_game_id: Some(game_c),
            existing_release_edition_id: Some(other),
            match_decision: None,
            game_title: "Game C".to_owned(),
            platform: GAME_BOY.to_owned(),
            region: "World".to_owned(),
            edition_name: "Standard".to_owned(),
            asset_type: AssetType::BoxFront,
            object_hash: "ab".repeat(32),
            byte_len: 1,
            media: MediaInfo::unknown(),
            original_filename: "front.png".to_owned(),
            source_id: SourceId::from("local_import"),
            source_asset_label: None,
            source_location: "C:/media/front.png".to_owned(),
        })
        .unwrap();

    let outcome = catalog.link_reference_review_item(item, game_a).unwrap();

    assert_eq!(outcome, ReferenceReviewOutcome::EditionHoldsAssets);
    assert_eq!(catalog.list_reference_review_items().unwrap().len(), 1);
    assert_eq!(catalog.list_library().unwrap().len(), 3);
}
