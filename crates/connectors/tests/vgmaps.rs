use std::{
    collections::HashMap,
    io::{Cursor, Read},
    sync::Mutex,
};

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_connectors::{HttpTransport, VGMAPS_SOURCE_ID, VgMapsConnector};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType,
    AssetTypeSelector, GameSelection, RetentionPolicy, SourceSelection,
};

const NES: &str = "Nintendo - Nintendo Entertainment System";
const NES_ATLAS: &str = "https://www.vgmaps.com/Atlas/NES/index.htm";

/// Serves pages by their URL, and records each request.
struct FixtureSite {
    pages: HashMap<String, Vec<u8>>,
    requests: Mutex<Vec<String>>,
}

impl FixtureSite {
    fn serving(pages: &[(&str, &str)]) -> Self {
        Self {
            pages: pages
                .iter()
                .map(|(url, body)| ((*url).to_owned(), body.as_bytes().to_vec()))
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
        let body = self
            .pages
            .get(url)
            .cloned()
            .unwrap_or_else(|| b"vgmaps map fixture".to_vec());
        Ok(Box::new(Cursor::new(body)))
    }
}

fn request(change: impl FnOnce(&mut AcquisitionRequestDraft)) -> AcquisitionRequest {
    let mut draft = AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec![VGMAPS_SOURCE_ID.to_owned()]),
        platforms: vec![NES.to_owned()],
        games: GameSelection::Explicit(vec!["Super Mario Bros. (World)".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::Map],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };
    change(&mut draft);
    AcquisitionRequest::try_from_draft(draft).unwrap()
}

/// One game's table on an atlas page: its title row, then a row per map, each naming its area,
/// linking its image and crediting its author.
fn game(anchor: &str, title: &str, maps: &[(&str, &str, &str)]) -> String {
    let rows: String = maps
        .iter()
        .map(|(area, file, label)| {
            format!(
                r#"<TR><TD WIDTH="19%">{area}</TD><TD WIDTH="19%"><a href="{file}">{label}</a></TD><TD>3392 x 448</TD><TD colspan="2">9.29 kB</TD><TD>PNG</TD><TD>ripped</TD><TD><a href="/cdn-cgi/l/email-protection#e286">Author</a></TD></TR>"#
            )
        })
        .collect();
    format!(
        r#"<P ALIGN="center"><TABLE BORDER CELLSPACING=1 CELLPADDING=7 WIDTH="1180"><TR><TD COLSPAN=8 align="center"><a NAME="{anchor}"><IMG SRC="../../System/Titles/{anchor}(NES).png"></a></TD></TR><TR><TD COLSPAN=4><P ALIGN="center">{title} Maps</TD><TD COLSPAN=4><p align="center">&copy; 1985 Nintendo</TD></TR><tr><TD colspan="8"><a href="http://www.nesmaps.com/nesmapsstore.html#SMBMap">Order poster maps of this game from <b>NESMaps</b></a></TD></tr>{rows}</TABLE>"#
    )
}

/// An atlas page, whose game tables sit inside the site's layout table.
fn atlas() -> String {
    let games = [
        game(
            "SuperMarioBros",
            "Super Mario Bros.",
            &[
                ("World 1", "SuperMarioBros-World1-1.png", "1-1"),
                ("World 1", "SuperMarioBros-World1-2.png", "1-2"),
                ("Bonus Maps", "SuperMarioBros-WarpZone.png", "Warp Zone"),
                (
                    "Ending",
                    "http://elsewhere.example.com/ending.png",
                    "Ending",
                ),
            ],
        ),
        game(
            "SuperMarioBros2",
            "Super Mario Bros. 2",
            &[("World 1", "SuperMarioBros2-World1-1.png", "1-1")],
        ),
        game(
            "LegendofZelda",
            "Legend of Zelda, The",
            &[("Overworld", "LegendofZelda-Overworld.png", "Overworld")],
        ),
        game(
            "MegaManII",
            "Mega Man II",
            &[("Stages", "MegaManII-AirMan.png", "Air Man")],
        ),
        game(
            "AdventuresofLolo",
            "The Adventures Of Lolo",
            &[("Floors", "AdventuresofLolo-Floor1.png", "Floor 1")],
        ),
    ]
    .concat();
    format!(
        r##"<HTML><BODY><table border="0" width="1200"><tr><TD><p>Nintendo Entertainment System / Famicom / Famicom Disk System</p><a href="#S">S</a>{games}</TD></tr></table></BODY></HTML>"##
    )
}

fn candidate(file: &str, label: &str) -> AssetCandidate {
    AssetCandidate {
        provider_candidate_id: Some(format!("{NES}/NES/{file}")),
        game_title: "Super Mario Bros.".to_owned(),
        platform: NES.to_owned(),
        region: "Unknown".to_owned(),
        edition_name: "Unspecified".to_owned(),
        asset_type: AssetType::Map,
        source_id: VGMAPS_SOURCE_ID.into(),
        source_asset_label: Some(label.to_owned()),
        source_url: format!("https://www.vgmaps.com/Atlas/NES/{file}"),
        original_filename: file.to_owned(),
    }
}

#[test]
fn vgmaps_provides_maps_at_a_polite_pace() {
    let site = FixtureSite::serving(&[]);
    let connector = VgMapsConnector::with_transport(&site);

    assert_eq!(connector.capabilities().asset_types, [AssetType::Map]);
    assert!(connector.capabilities().direct_media_download);
    assert!(connector.rate_limits().is_some());
    assert!(!connector.needs_api_key());
}

#[test]
fn requests_vgmaps_cannot_serve_are_refused_before_reaching_it() {
    let site = FixtureSite::serving(&[]);
    let connector = VgMapsConnector::with_transport(&site);
    let reason = |change: fn(&mut AcquisitionRequestDraft)| {
        connector
            .unsupported_request_reason(&request(change))
            .unwrap()
            .unwrap()
    };

    assert!(reason(|draft| draft.games = GameSelection::All).contains("explicit game selection"));
    assert!(
        reason(|draft| draft.platforms = vec!["Nintendo - Wonder Console".to_owned()])
            .contains("Nintendo - Wonder Console")
    );
    assert!(reason(|draft| draft.regions = vec!["USA".to_owned()]).contains("region"));
    assert!(reason(|draft| draft.languages = vec!["en".to_owned()]).contains("language"));
    assert!(site.requested().is_empty());
}

#[test]
fn discovery_takes_the_maps_of_the_game_named_exactly_so_on_its_atlas() {
    let atlas = atlas();
    let site = FixtureSite::serving(&[(NES_ATLAS, &atlas)]);

    let candidates = VgMapsConnector::with_transport(&site)
        .discover(&request(|_| {}))
        .unwrap();

    // Maps another site serves are left out, as are the links that are no maps.
    assert_eq!(
        candidates,
        [
            candidate("SuperMarioBros-World1-1.png", "World 1 · 1-1"),
            candidate("SuperMarioBros-World1-2.png", "World 1 · 1-2"),
            candidate("SuperMarioBros-WarpZone.png", "Bonus Maps · Warp Zone"),
        ]
    );
    assert_eq!(site.requested(), [NES_ATLAS]);
}

#[test]
fn a_title_the_atlas_files_under_its_article_is_found_and_the_atlas_read_once() {
    let atlas = atlas();
    let site = FixtureSite::serving(&[(NES_ATLAS, &atlas)]);

    let candidates = VgMapsConnector::with_transport(&site)
        .discover(&request(|draft| {
            draft.games = GameSelection::Explicit(vec![
                "The Legend of Zelda (USA)".to_owned(),
                "Super Mario Bros. 2 (USA)".to_owned(),
            ]);
        }))
        .unwrap();

    let found: Vec<(&str, &str)> = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.game_title.as_str(),
                candidate.source_asset_label.as_deref().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("The Legend of Zelda", "Overworld"),
            ("Super Mario Bros. 2", "World 1 · 1-1"),
        ]
    );
    assert_eq!(site.requested(), [NES_ATLAS]);
}

#[test]
fn an_atlas_page_without_any_game_is_invalid_source_data() {
    let site =
        FixtureSite::serving(&[(NES_ATLAS, "<html><body>Down for maintenance</body></html>")]);

    let error = VgMapsConnector::with_transport(&site)
        .discover(&request(|_| {}))
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{}", error.message());
}

#[test]
fn maps_are_downloaded_from_their_page_on_the_site() {
    let site = FixtureSite::serving(&[]);
    let map = candidate("SuperMarioBros-World1-1.png", "World 1 · 1-1");

    let mut bytes = Vec::new();
    VgMapsConnector::with_transport(&site)
        .download(&map)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();

    assert_eq!(bytes, b"vgmaps map fixture");
    assert_eq!(site.requested(), [map.source_url]);
}

#[test]
fn an_atlas_page_in_windows_1252_keeps_its_accented_titles() {
    let table = game(
        "PokemonRed",
        "Pok\u{e9}mon Red",
        &[("Kanto", "PokemonRed-Kanto.png", "Kanto")],
    );
    let html = format!(
        r#"<HTML><HEAD><meta http-equiv="Content-Type" content="text/html; charset=windows-1252"></HEAD><BODY><table><tr><TD>{table}</TD></tr></table></BODY></HTML>"#
    );
    // The site writes its pages in Windows-1252, where `é` is one byte.
    let bytes: Vec<u8> = html
        .chars()
        .map(|character| {
            if character == '\u{e9}' {
                0xe9
            } else {
                character as u8
            }
        })
        .collect();
    let mut site = FixtureSite::serving(&[]);
    site.pages.insert(
        "https://www.vgmaps.com/Atlas/GB-GBC/index.htm".to_owned(),
        bytes,
    );

    let candidates = VgMapsConnector::with_transport(&site)
        .discover(&request(|draft| {
            draft.platforms = vec!["Nintendo - Game Boy".to_owned()];
            draft.games = GameSelection::Explicit(vec!["Pok\u{e9}mon Red (USA)".to_owned()]);
        }))
        .unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].source_asset_label.as_deref(), Some("Kanto"));
}

#[test]
fn a_title_numbered_or_filed_otherwise_than_the_catalog_does_is_found() {
    let atlas = atlas();
    let site = FixtureSite::serving(&[(NES_ATLAS, &atlas)]);

    // The catalog files articles last and numbers sequels, where the atlas may not.
    let candidates = VgMapsConnector::with_transport(&site)
        .discover(&request(|draft| {
            draft.games = GameSelection::Explicit(vec![
                "Mega Man 2 (USA)".to_owned(),
                "Adventures of Lolo, The (USA)".to_owned(),
            ]);
        }))
        .unwrap();

    let labels: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.source_asset_label.as_deref().unwrap())
        .collect();
    assert_eq!(labels, ["Stages · Air Man", "Floors · Floor 1"]);
}
