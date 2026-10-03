use std::{
    collections::HashMap,
    io::{Cursor, Read},
    sync::Mutex,
};

use game_media_vault_application::{PlatformCatalogSourcePort, PortError};
use game_media_vault_connectors::{HttpTransport, LibretroDatabase};
use game_media_vault_domain::{ReleaseAssertion, ReleaseAssertionField};

const NO_INTRO_LISTING: &str =
    "https://api.github.com/repos/libretro/libretro-database/contents/metadat/no-intro";
const REDUMP_LISTING: &str =
    "https://api.github.com/repos/libretro/libretro-database/contents/metadat/redump";
const RAW: &str = "https://raw.githubusercontent.com/libretro/libretro-database/master/metadat";
const NES: &str = "Nintendo - Nintendo Entertainment System";

/// Serves pages by their URL, and records each request.
struct FixtureSite {
    pages: HashMap<String, String>,
    requests: Mutex<Vec<String>>,
}

impl FixtureSite {
    fn serving(pages: &[(&str, &str)]) -> Self {
        Self {
            pages: pages
                .iter()
                .map(|(url, body)| ((*url).to_owned(), (*body).to_owned()))
                .collect(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requested(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl HttpTransport for &FixtureSite {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        self.requests.lock().unwrap().push(url.to_owned());
        self.pages
            .get(url)
            .map(|body| Box::new(Cursor::new(body.clone().into_bytes())) as Box<dyn Read + Send>)
            .ok_or_else(|| PortError::unavailable(format!("download returned HTTP 404 for {url}")))
    }
}

/// A directory listing of the GitHub contents API, naming each file with its raw location.
fn listing(directory: &str, files: &[&str]) -> String {
    let entries: Vec<String> = files
        .iter()
        .map(|file| {
            let encoded = file.replace(' ', "%20");
            format!(
                r#"{{"name":"{file}","path":"metadat/{directory}/{file}","type":"file","download_url":"{RAW}/{directory}/{encoded}"}}"#
            )
        })
        .collect();
    format!("[{}]", entries.join(","))
}

fn raw(directory: &str, file: &str) -> String {
    format!("{RAW}/{directory}/{}", file.replace(' ', "%20"))
}

const NES_DAT: &str = r#"clrmamepro (
	name "Nintendo - Nintendo Entertainment System"
	description "Nintendo - Nintendo Entertainment System"
	version "2026.08.01"
	homepage "http://github.com/robloach/libretro-dats"
)

game (
	name "Super Mario Bros. (World)"
	rom ( name "Super Mario Bros. (World).nes" size 40976 crc 3337EC46 md5 8E3630186E35D477231BF8FD50E54CDD sha1 EA343F4E445A9050D4B4FBAC2C77D0693B1D0922 )
)
game (
	name "Super Mario Bros. (World)"
	rom ( name "Super Mario Bros. (World).unh" size 40960 crc D445F698 sha1 FACEE9C577A5262DBE33AC4930BB0B58C8C037F7 )
)
game (
	name "Tetris (USA) (Rev 1)"
	rom ( name "Tetris (USA) (Rev 1).nes" size 49168 crc 1394F57E )
)
game (
	description "An entry without a name"
	rom ( name "Nameless.nes" size 16 crc 00000000 )
)
"#;

fn assertion<'a>(
    assertions: &'a [ReleaseAssertion],
    field: ReleaseAssertionField,
    qualifier: Option<&str>,
) -> Vec<&'a str> {
    assertions
        .iter()
        .filter(|assertion| assertion.field == field && assertion.qualifier.as_deref() == qualifier)
        .map(|assertion| assertion.value.as_str())
        .collect()
}

#[test]
fn reads_a_platforms_game_list_merging_the_dumps_of_each_game() {
    let nes_listing = listing("no-intro", &[&format!("{NES}.dat"), "README.md"]);
    let site = FixtureSite::serving(&[
        (NO_INTRO_LISTING, &nes_listing),
        (&raw("no-intro", &format!("{NES}.dat")), NES_DAT),
    ]);

    let read = LibretroDatabase::with_transport(&site)
        .platform_releases(NES)
        .unwrap()
        .unwrap();

    assert_eq!(read.skipped_records, 1);
    let releases: Vec<(&str, &str, &str, Option<&str>, &str)> = read
        .releases
        .iter()
        .map(|release| {
            (
                release.game_title.as_str(),
                release.platform.as_str(),
                release.region.as_str(),
                release.revision.as_deref(),
                release.edition_name.as_str(),
            )
        })
        .collect();
    assert_eq!(
        releases,
        [
            ("Super Mario Bros.", NES, "World", None, "Standard"),
            ("Tetris", NES, "USA", Some("Rev 1"), "Rev 1"),
        ]
    );
    let mario = &read.releases[0].assertions;
    // Both dumps of the one game are its identifiers, from No-Intro, read at the file's address.
    assert_eq!(
        assertion(mario, ReleaseAssertionField::Identifier, Some("crc")),
        ["3337EC46", "D445F698"]
    );
    assert_eq!(
        assertion(
            mario,
            ReleaseAssertionField::Identifier,
            Some("dat_version")
        ),
        ["2026.08.01"]
    );
    assert!(
        mario
            .iter()
            .all(|assertion| assertion.source_id.as_str() == "no-intro")
    );
    assert!(
        mario
            .iter()
            .all(|assertion| assertion.source_location == raw("no-intro", &format!("{NES}.dat")))
    );
}

#[test]
fn finds_a_platform_by_its_words_in_either_catalog_and_lists_each_directory_once() {
    let site = FixtureSite::serving(&[
        (
            NO_INTRO_LISTING,
            &listing("no-intro", &["NEC - PC Engine - TurboGrafx 16.dat"]),
        ),
        (
            REDUMP_LISTING,
            &listing("redump", &["Sony - PlayStation.dat"]),
        ),
        (
            &raw("no-intro", "NEC - PC Engine - TurboGrafx 16.dat"),
            "clrmamepro ( name \"NEC - PC Engine - TurboGrafx 16\" )\ngame ( name \"Bonk's Adventure (USA)\" rom ( name \"b.pce\" crc 11111111 ) )\n",
        ),
        (
            &raw("redump", "Sony - PlayStation.dat"),
            "clrmamepro ( name \"Sony - PlayStation\" )\ngame ( name \"Ridge Racer (USA)\" rom ( name \"r.bin\" crc 22222222 ) )\n",
        ),
    ]);
    let database = LibretroDatabase::with_transport(&site);

    let pc_engine = database
        .platform_releases("NEC - PC Engine - TurboGrafx-16")
        .unwrap()
        .unwrap();
    let playstation = database
        .platform_releases("Sony - PlayStation")
        .unwrap()
        .unwrap();
    let unknown = database
        .platform_releases("Nintendo - Wonder Console")
        .unwrap();

    assert_eq!(pc_engine.releases[0].game_title, "Bonk's Adventure");
    assert_eq!(playstation.releases[0].game_title, "Ridge Racer");
    assert_eq!(
        playstation.releases[0].assertions[0].source_id.as_str(),
        "redump"
    );
    assert!(unknown.is_none());
    let listings = site
        .requested()
        .iter()
        .filter(|url| url.starts_with("https://api.github.com"))
        .count();
    assert_eq!(listings, 2);
}

#[test]
fn a_listing_whose_file_lives_elsewhere_is_not_followed() {
    let elsewhere = format!(
        r#"[{{"name":"{NES}.dat","type":"file","download_url":"https://elsewhere.example.com/{NES}.dat"}}]"#
    );
    let site = FixtureSite::serving(&[(NO_INTRO_LISTING, &elsewhere), (REDUMP_LISTING, "[]")]);

    let read = LibretroDatabase::with_transport(&site)
        .platform_releases(NES)
        .unwrap();

    assert!(read.is_none());
    assert!(
        site.requested()
            .iter()
            .all(|url| !url.contains("elsewhere.example.com"))
    );
}

#[test]
fn a_listing_github_refuses_is_reported() {
    let site = FixtureSite::serving(&[(
        NO_INTRO_LISTING,
        r#"{"message":"API rate limit exceeded","documentation_url":"https://docs.github.com"}"#,
    )]);

    let error = LibretroDatabase::with_transport(&site)
        .platform_releases(NES)
        .unwrap_err();

    assert!(
        error.message().contains("API rate limit exceeded"),
        "{}",
        error.message()
    );
}

#[test]
fn a_game_list_that_does_not_parse_is_invalid_source_data() {
    let site = FixtureSite::serving(&[
        (
            NO_INTRO_LISTING,
            &listing("no-intro", &[&format!("{NES}.dat")]),
        ),
        (
            &raw("no-intro", &format!("{NES}.dat")),
            "clrmamepro ( name \"x\" \ngame ( name \"Unclosed",
        ),
    ]);

    let error = LibretroDatabase::with_transport(&site)
        .platform_releases(NES)
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{}", error.message());
}
