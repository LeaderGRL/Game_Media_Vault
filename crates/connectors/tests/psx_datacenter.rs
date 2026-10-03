use std::{
    collections::HashMap,
    io::{Cursor, Read},
    sync::Mutex,
};

use game_media_vault_application::{ConnectorPort, PortError};
use game_media_vault_connectors::{
    HttpTransport, PSX_DATACENTER_SOURCE_ID, PsxDataCenterConnector,
};
use game_media_vault_domain::{
    AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetCandidate, AssetType,
    AssetTypeSelector, GameSelection, RetentionPolicy, SourceId, SourceSelection,
};

const SITE: &str = "https://psxdatacenter.com";
const PLAYSTATION: &str = "Sony - PlayStation";

/// PSX DataCenter served from fixtures, recording every request.
struct Website {
    pages: HashMap<String, String>,
    requests: Mutex<Vec<String>>,
}

impl Website {
    fn with(pages: &[(&str, &str)]) -> Self {
        Self {
            pages: pages
                .iter()
                .map(|(path, body)| (format!("{SITE}/{path}"), (*body).to_owned()))
                .collect(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requested(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl HttpTransport for &Website {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        self.requests.lock().unwrap().push(url.to_owned());
        match self.pages.get(url) {
            Some(body) => Ok(Box::new(Cursor::new(body.clone().into_bytes()))),
            None if url.ends_with(".jpg") => Ok(Box::new(Cursor::new(b"cover".to_vec()))),
            None => Err(PortError::unavailable(format!(
                "download returned HTTP 404 Not Found for {url}"
            ))),
        }
    }
}

/// A region's game list with one row per `(page, serials, title)`, as the site lays it out.
fn list(rows: &[(&str, &str, &str)]) -> String {
    let rows: String = rows
        .iter()
        .map(|(page, serials, title)| {
            format!(
                r#"<tr>
<td class="col1"><a target="ulist" href="{page}">INFO</a></td>
<td class="col2">{serials}</td>
<td class="col3">&nbsp;{title}</td>
<td class="col4">&nbsp;[E]</td>
</tr>"#
            )
        })
        .collect();
    format!(r#"<html><body><table class="sectiontable">{rows}</table></body></html>"#)
}

const FINAL_FANTASY_VII: &str = r#"<html><head><title>FINAL FANTASY VII - (NTSC-U)</title></head><body>
<table><tr>
<td><img border="0" src="../../../images/screens/P/F/SCES-00900/ss1.jpg" width="320" height="240"></td>
<td><img border="0" src="../../../images/screens/P/F/SCES-00900/ss2.jpg" width="320" height="240"></td>
<td><img border="0" src="../../../images/screens/U/F/SCUS-94163/ss1.jpg" width="320" height="240"></td>
</tr></table>
<table><tr>
<td><a target="_blank" href="../../../images/hires/U/F/SCUS-94163/SCUS-94163-F-ALL.html">
<img border="0" src="../../../images/thumbs/U/F/SCUS-94163/SCUS-94163-F-ALL.jpg"></a></td>
<td><a target="_blank" href="../../../images/hires/U/F/SCUS-94163/SCUS-94163-B-ALL.html">
<img border="0" src="../../../images/thumbs/U/F/SCUS-94163/SCUS-94163-B-ALL.jpg"></a></td>
<td><a target="_blank" href="../../../images/hires/U/F/SCUS-94163/SCUS-94163-I-1.html">
<img border="0" src="../../../images/thumbs/U/F/SCUS-94163/SCUS-94163-I-1.jpg"></a></td>
<td><a target="_blank" href="../../../images/hires/U/F/SCUS-94163/SCUS-94163-F-GH.html">
<img border="0" src="../../../images/thumbs/U/F/SCUS-94163/SCUS-94163-F-GH.jpg"></a></td>
<td><a target="_blank" href="../../../images/hires/U/F/SCUS-94163/SCUS-94163-F-P.html">
<img border="0" src="../../../images/thumbs/U/F/SCUS-94163/SCUS-94163-F-P.jpg"></a></td>
<td><a target="_blank" href="../../../images/hires/U/F/SCUS-94163/SCUS-94163-B-P.html?v=2">
<img border="0" src="../../../images/thumbs/U/F/SCUS-94163/SCUS-94163-B-P.jpg"></a></td>
<td><a target="_blank" href="../../../images/hires/U/F/SCUS-94163/SCUS-94163-F-XX.html">
<img border="0" src="../../../images/thumbs/U/F/SCUS-94163/SCUS-94163-F-XX.jpg"></a></td>
<td><a target="_blank" href="https://elsewhere.example/images/hires/U/F/SCUS-94163/SCUS-94163-B-GH.html">
<img border="0" src="../../../images/thumbs/U/F/SCUS-94163/SCUS-94163-B-GH.jpg"></a></td>
<td><a target="_blank" href="../../../images/hires/J/F/SLPS-00700/SLPS-00700-A-1.html">
<img border="0" src="../../../images/thumbs/J/F/SLPS-00700/SLPS-00700-A-1.jpg"></a></td>
</tr></table>
</body></html>"#;

fn site() -> Website {
    Website::with(&[
        (
            "ulist.html",
            &list(&[
                (
                    "games/U/F/SCUS-94163.html",
                    "SCUS-94163<br>SCUS-94164<br>SCUS-94165",
                    "FINAL FANTASY VII&nbsp; -&nbsp; [ 3 DISCS ]",
                ),
                (
                    "games/U/F/SCUS-94180.html",
                    "SCUS-94180",
                    "FINAL FANTASY TACTICS",
                ),
            ]),
        ),
        ("games/U/F/SCUS-94163.html", FINAL_FANTASY_VII),
    ])
}

fn request(change: impl FnOnce(&mut AcquisitionRequestDraft)) -> AcquisitionRequest {
    let mut draft = AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec![PSX_DATACENTER_SOURCE_ID.to_owned()]),
        platforms: vec![PLAYSTATION.to_owned()],
        games: GameSelection::Explicit(vec!["Final Fantasy VII (USA) (Disc 1)".to_owned()]),
        regions: vec!["USA".to_owned()],
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront, AssetTypeSelector::BoxBack],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    };
    change(&mut draft);
    AcquisitionRequest::try_from_draft(draft).unwrap()
}

#[test]
fn acquires_cover_scans_and_screenshots_of_playstation_games() {
    let site = Website::with(&[]);
    let capabilities = PsxDataCenterConnector::with_transport(&site).capabilities();

    assert_eq!(
        capabilities.asset_types,
        [
            AssetType::BoxFront,
            AssetType::BoxBack,
            AssetType::Screenshot
        ]
    );
    assert!(capabilities.direct_media_download);
}

#[test]
fn refuses_requests_it_cannot_serve_without_reaching_the_site() {
    let site = Website::with(&[]);
    let connector = PsxDataCenterConnector::with_transport(&site);

    for refused in [
        request(|draft| draft.platforms = vec!["Nintendo - Game Boy".to_owned()]),
        // Part of the request it could not serve would go missing silently.
        request(|draft| {
            draft.platforms = vec![PLAYSTATION.to_owned(), "Sega - Saturn".to_owned()];
        }),
        request(|draft| draft.games = GameSelection::All),
        request(|draft| draft.languages = vec!["en".to_owned()]),
        request(|draft| draft.regions = vec!["Brazil".to_owned()]),
        // The PAL list records its releases in Europe, not in a country it also covers.
        request(|draft| draft.regions = vec!["Australia".to_owned()]),
    ] {
        assert!(
            connector
                .unsupported_request_reason(&refused)
                .unwrap()
                .is_some()
        );
    }
    assert!(site.requested().is_empty());
}

#[test]
fn discovers_the_hires_covers_of_a_game_its_region_list_names_exactly() {
    let site = site();

    let candidates = PsxDataCenterConnector::with_transport(&site)
        .discover(&request(|_| {}))
        .unwrap();

    let cover = |code: &str, asset_type, label: &str, edition: &str| AssetCandidate {
        provider_candidate_id: Some(format!("U/F/SCUS-94163/SCUS-94163-{code}")),
        game_title: "Final Fantasy VII".to_owned(),
        platform: PLAYSTATION.to_owned(),
        region: "USA".to_owned(),
        edition_name: edition.to_owned(),
        asset_type,
        source_id: SourceId::from(PSX_DATACENTER_SOURCE_ID),
        source_asset_label: Some(label.to_owned()),
        source_url: format!("{SITE}/images/hires/U/F/SCUS-94163/SCUS-94163-{code}.jpg"),
        original_filename: format!("SCUS-94163-{code}.jpg"),
    };
    assert_eq!(
        candidates,
        [
            cover("F-ALL", AssetType::BoxFront, "front", "Unspecified"),
            cover("B-ALL", AssetType::BoxBack, "back", "Unspecified"),
            cover("F-GH", AssetType::BoxFront, "front: GH", "Greatest Hits"),
            cover("F-P", AssetType::BoxFront, "front: P", "Platinum"),
        ]
    );
    assert_eq!(
        site.requested(),
        [
            format!("{SITE}/ulist.html"),
            format!("{SITE}/games/U/F/SCUS-94163.html")
        ]
    );
}

#[test]
fn discovers_screenshots_when_they_are_requested() {
    let site = site();

    let candidates = PsxDataCenterConnector::with_transport(&site)
        .discover(&request(|draft| {
            draft.asset_types = vec![AssetTypeSelector::Screenshot];
        }))
        .unwrap();

    let urls: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.source_url.as_str())
        .collect();
    assert_eq!(
        urls,
        // Screenshots another region's directory holds belong to that region's release.
        ["https://psxdatacenter.com/images/screens/U/F/SCUS-94163/ss1.jpg"]
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.asset_type == AssetType::Screenshot)
    );
}

#[test]
fn reads_only_the_lists_of_the_requested_regions() {
    let other = list(&[("games/P/O/SLES-00001.html", "SLES-00001", "OTHER GAME")]);
    let site = Website::with(&[("plist.html", &other), ("jlist.html", &other)]);

    let candidates = PsxDataCenterConnector::with_transport(&site)
        .discover(&request(|draft| {
            draft.regions = vec!["Europe".to_owned(), "Japan".to_owned()];
        }))
        .unwrap();

    assert!(candidates.is_empty());
    assert_eq!(
        site.requested(),
        [format!("{SITE}/plist.html"), format!("{SITE}/jlist.html")]
    );
}

#[test]
fn a_game_no_list_names_exactly_is_never_fetched() {
    let site = site();

    let candidates = PsxDataCenterConnector::with_transport(&site)
        .discover(&request(|draft| {
            draft.games = GameSelection::Explicit(vec!["Final Fantasy".to_owned()]);
        }))
        .unwrap();

    assert!(candidates.is_empty());
    assert_eq!(site.requested(), [format!("{SITE}/ulist.html")]);
}

#[test]
fn downloads_a_cover_from_its_location() {
    let site = site();
    let connector = PsxDataCenterConnector::with_transport(&site);
    let candidate = connector.discover(&request(|_| {})).unwrap().remove(0);

    let mut bytes = Vec::new();
    connector
        .download(&candidate)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();

    assert_eq!(bytes, b"cover");
    assert_eq!(site.requested().last().unwrap(), &candidate.source_url);
}

#[test]
fn a_title_several_rows_of_a_list_share_names_no_single_release() {
    let site = Website::with(&[(
        "ulist.html",
        &list(&[
            (
                "games/U/F/SCUS-94163.html",
                "SCUS-94163",
                "FINAL FANTASY VII",
            ),
            (
                "games/U/F/SCUS-94999.html",
                "SCUS-94999",
                "FINAL FANTASY VII",
            ),
        ]),
    )]);

    let candidates = PsxDataCenterConnector::with_transport(&site)
        .discover(&request(|_| {}))
        .unwrap();

    assert!(candidates.is_empty());
    assert_eq!(site.requested(), [format!("{SITE}/ulist.html")]);
}

#[test]
fn candidates_name_the_platform_by_its_catalog_name() {
    let site = site();

    let candidates = PsxDataCenterConnector::with_transport(&site)
        .discover(&request(|draft| draft.platforms = vec!["PS1".to_owned()]))
        .unwrap();

    assert!(!candidates.is_empty());
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.platform == PLAYSTATION)
    );
}

#[test]
fn a_list_linking_a_game_page_on_another_site_is_not_followed() {
    let site = Website::with(&[(
        "ulist.html",
        &list(&[(
            "http://127.0.0.1/admin.html",
            "SCUS-94163",
            "FINAL FANTASY VII",
        )]),
    )]);

    let candidates = PsxDataCenterConnector::with_transport(&site)
        .discover(&request(|_| {}))
        .unwrap();

    assert!(candidates.is_empty());
    assert_eq!(site.requested(), [format!("{SITE}/ulist.html")]);
}

#[test]
fn a_list_without_games_is_invalid_source_data() {
    let site = Website::with(&[(
        "ulist.html",
        "<html><body>Down for maintenance</body></html>",
    )]);

    let error = PsxDataCenterConnector::with_transport(&site)
        .discover(&request(|_| {}))
        .unwrap_err();

    assert!(error.is_invalid_source_data(), "{}", error.message());
}

#[test]
fn describes_the_pace_it_reads_the_site_at() {
    let site = Website::with(&[]);

    let limits = PsxDataCenterConnector::with_transport(&site)
        .rate_limits()
        .unwrap();

    assert!(limits.contains("a second"), "{limits}");
}
