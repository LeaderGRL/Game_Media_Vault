use std::{
    ffi::OsString,
    fs,
    io::{Cursor, Read},
    path::Path,
    process::Command,
};

use game_media_vault_application::{
    AcquisitionRequestValidationError, ApplicationError, CatalogPort, ConnectorPort, PortError,
    ReferenceCatalogRepositoryPort, complete_acquisition_run, complete_acquisition_work,
    queue_acquisition_work,
};
use game_media_vault_cli::CliError;
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, MatchEvidence,
    MatchSignal, MatchingPolicy, MatchingPolicyValidationError, NewReviewItem,
    ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField, ReviewMatchCandidate,
    SourceId,
};
use game_media_vault_infrastructure::SqliteCatalog;
use tempfile::tempdir;

struct FixtureConnector;

impl ConnectorPort for FixtureConnector {
    fn source_id(&self) -> &'static str {
        "libretro-thumbnails"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        ConnectorCapabilities {
            asset_types: vec![AssetType::BoxFront],
            direct_media_download: true,
        }
    }

    fn discover(&self, _request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        Ok(vec![AssetCandidate {
            game_title: "Super Mario Bros. (World)".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "World".to_owned(),
            edition_name: "Unspecified".to_owned(),
            asset_type: AssetType::BoxFront,
            source_id: SourceId::from("libretro-thumbnails"),
            source_asset_label: Some("Named_Boxarts".to_owned()),
            source_url: "https://example.invalid/smb-box-front.png".to_owned(),
            original_filename: "Super Mario Bros. (World).png".to_owned(),
        }])
    }

    fn download(&self, _candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        Ok(Box::new(Cursor::new(b"cli connector fixture".to_vec())))
    }
}

fn run_in_vault(vault: &Path, args: &[&str]) -> Result<String, CliError> {
    let mut command = vec![
        OsString::from("game-media-vault"),
        OsString::from("--vault"),
        vault.as_os_str().to_owned(),
    ];
    command.extend(args.iter().map(OsString::from));
    game_media_vault_cli::run(command)
}

fn review_work_key(candidate: &AssetCandidate) -> String {
    let mut key = "connector".to_owned();
    for part in [
        candidate.source_id.as_str(),
        candidate.platform.as_str(),
        candidate.game_title.as_str(),
        candidate.region.as_str(),
        candidate.edition_name.as_str(),
        "box_front",
        candidate.source_url.as_str(),
    ] {
        key.push(':');
        key.push_str(&part.len().to_string());
        key.push(':');
        key.push_str(part);
    }
    key
}

fn seed_review_item(vault: &Path) -> i64 {
    let started: serde_json::Value = serde_json::from_str(
        &run_in_vault(
            vault,
            &[
                "acquire",
                "--source",
                "fixture-provider",
                "--platform",
                "Nintendo Entertainment System",
                "--game",
                "Review Game",
                "--asset-type",
                "box-front",
            ],
        )
        .unwrap(),
    )
    .unwrap();
    let run_id = started["id"].as_i64().unwrap();
    let catalog = SqliteCatalog::open_existing(vault.join("catalog.sqlite3")).unwrap();
    let candidate = AssetCandidate {
        game_title: "Review Game".to_owned(),
        platform: "Nintendo Entertainment System".to_owned(),
        region: "USA".to_owned(),
        edition_name: "Collector".to_owned(),
        asset_type: AssetType::BoxFront,
        source_id: SourceId::from("fixture-provider"),
        source_asset_label: Some("front".to_owned()),
        source_url: "fixture://review/front".to_owned(),
        original_filename: "front.png".to_owned(),
    };
    let work_key = review_work_key(&candidate);
    queue_acquisition_work(&catalog, run_id, work_key.clone()).unwrap();
    complete_acquisition_work(&catalog, run_id, &work_key).unwrap();
    complete_acquisition_run(&catalog, run_id).unwrap();
    catalog
        .persist_review_item(NewReviewItem {
            run_id,
            candidate_identity: "connector:cli-review".to_owned(),
            candidate,
            competing_matches: vec![ReviewMatchCandidate {
                game_id: 301,
                release_edition_id: 201,
                game_title: "Review Game".to_owned(),
                platform: "Nintendo Entertainment System".to_owned(),
                region: "USA".to_owned(),
                edition_name: "Standard".to_owned(),
                score: 90,
                evidence: vec![MatchEvidence {
                    signal: MatchSignal::Title,
                    candidate_value: "Review Game".to_owned(),
                    release_value: "Review Game".to_owned(),
                    score_delta: 50,
                }],
                assertions: Vec::new(),
            }],
        })
        .unwrap();
    catalog.list_review_items().unwrap()[0].id
}

#[test]
fn review_commands_show_evidence_and_persist_accept_reject_and_defer() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let review_item_id = seed_review_item(&vault);
    let id = review_item_id.to_string();

    let listed: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["review", "list"]).unwrap()).unwrap();
    assert_eq!(
        listed[0]["candidate"]["source_url"],
        "fixture://review/front"
    );
    assert_eq!(listed[0]["competing_matches"][0]["score"], 90);
    assert_eq!(
        listed[0]["competing_matches"][0]["evidence"][0]["signal"],
        "title"
    );

    let accepted: serde_json::Value = serde_json::from_str(
        &run_in_vault(
            &vault,
            &["review", "accept", &id, "--release-edition-id", "201"],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(accepted["decision"]["decision"], "accept");
    assert_eq!(accepted["decision"]["release_edition_id"], 201);

    let rejected: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["review", "reject", &id]).unwrap()).unwrap();
    assert_eq!(rejected["decision"]["decision"], "reject");

    let deferred: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["review", "defer", &id]).unwrap()).unwrap();
    assert_eq!(deferred["decision"]["decision"], "defer");

    let reopened = SqliteCatalog::open_existing(vault.join("catalog.sqlite3")).unwrap();
    assert_eq!(
        reopened
            .get_review_item(review_item_id)
            .unwrap()
            .unwrap()
            .decision,
        Some(game_media_vault_domain::ReviewDecision::Defer)
    );
}

#[test]
fn help_exits_successfully() {
    let status = Command::new(env!("CARGO_BIN_EXE_game-media-vault"))
        .arg("--help")
        .status()
        .unwrap();

    assert!(status.success());
}

#[test]
fn cli_adapter_can_execute_a_persisted_run_through_a_connector() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    SqliteCatalog::open(vault.join("catalog.sqlite3"))
        .unwrap()
        .persist_reference_release(ReferenceReleaseRecord {
            game_title: "Super Mario Bros. (World)".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "World".to_owned(),
            revision: None,
            edition_name: "Unspecified".to_owned(),
            assertions: vec![ReleaseAssertion {
                source_id: SourceId::from("fixture-reference"),
                source_location: "fixture://reference".to_owned(),
                field: ReleaseAssertionField::Identifier,
                qualifier: Some("source_record".to_owned()),
                value: "fixture:super-mario-bros-world".to_owned(),
            }],
        })
        .unwrap();
    let started = run_in_vault(
        &vault,
        &[
            "acquire",
            "--source",
            "libretro-thumbnails",
            "--platform",
            "Nintendo - Nintendo Entertainment System",
            "--game",
            "Super Mario Bros. (World)",
            "--asset-type",
            "box-front",
        ],
    )
    .unwrap();
    let started: serde_json::Value = serde_json::from_str(&started).unwrap();
    let run_id = started["id"].as_i64().unwrap();

    let completed = game_media_vault_cli::execute_acquisition_run_in_vault_with_connector(
        &vault,
        run_id,
        &FixtureConnector,
        MatchingPolicy {
            high_confidence_threshold: 80,
            medium_confidence_threshold: 50,
        },
    )
    .unwrap();

    assert_eq!(
        completed.status,
        game_media_vault_domain::AcquisitionRunStatus::Completed
    );
    let library = run_in_vault(&vault, &["library"]).unwrap();
    assert!(library.contains("Super Mario Bros. (World)"));
    assert!(library.contains("https://example.invalid/smb-box-front.png"));
}

#[test]
fn cli_adapter_uses_the_configured_matching_thresholds() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    SqliteCatalog::open(vault.join("catalog.sqlite3"))
        .unwrap()
        .persist_reference_release(ReferenceReleaseRecord {
            game_title: "Super Mario Bros. (World)".to_owned(),
            platform: "Nintendo - Nintendo Entertainment System".to_owned(),
            region: "World".to_owned(),
            revision: None,
            edition_name: "Unspecified".to_owned(),
            assertions: vec![ReleaseAssertion {
                source_id: SourceId::from("fixture-reference"),
                source_location: "fixture://reference".to_owned(),
                field: ReleaseAssertionField::Identifier,
                qualifier: Some("source_record".to_owned()),
                value: "fixture:custom-thresholds".to_owned(),
            }],
        })
        .unwrap();
    let started = run_in_vault(
        &vault,
        &[
            "acquire",
            "--source",
            "libretro-thumbnails",
            "--platform",
            "Nintendo - Nintendo Entertainment System",
            "--game",
            "Super Mario Bros. (World)",
            "--asset-type",
            "box-front",
        ],
    )
    .unwrap();
    let started: serde_json::Value = serde_json::from_str(&started).unwrap();
    let run_id = started["id"].as_i64().unwrap();

    game_media_vault_cli::execute_acquisition_run_in_vault_with_connector(
        &vault,
        run_id,
        &FixtureConnector,
        MatchingPolicy {
            high_confidence_threshold: 96,
            medium_confidence_threshold: 50,
        },
    )
    .unwrap();

    let library = run_in_vault(&vault, &["library"]).unwrap();
    let entries: serde_json::Value = serde_json::from_str(&library).unwrap();
    assert!(entries[0]["assets"].as_array().unwrap().is_empty());
}

#[test]
fn cli_adapter_rejects_invalid_matching_threshold_order() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let started = run_in_vault(
        &vault,
        &[
            "acquire",
            "--source",
            "libretro-thumbnails",
            "--platform",
            "Nintendo - Nintendo Entertainment System",
            "--game",
            "Super Mario Bros. (World)",
            "--asset-type",
            "box-front",
        ],
    )
    .unwrap();
    let started: serde_json::Value = serde_json::from_str(&started).unwrap();
    let run_id = started["id"].as_i64().unwrap();

    let error = game_media_vault_cli::execute_acquisition_run_in_vault_with_connector(
        &vault,
        run_id,
        &FixtureConnector,
        MatchingPolicy {
            high_confidence_threshold: 60,
            medium_confidence_threshold: 80,
        },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CliError::Application(ApplicationError::InvalidMatchingPolicy(
            MatchingPolicyValidationError::MediumThresholdAboveHighThreshold
        ))
    ));
}

#[test]
fn imports_then_lists_a_local_box_front() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let source = temp.path().join("cover-front.png");
    fs::write(&source, b"cli cover bytes").unwrap();

    let imported = game_media_vault_cli::run([
        "game-media-vault".into(),
        "--vault".into(),
        vault.as_os_str().to_owned(),
        "import-box-front".into(),
        "--game".into(),
        "Metal Gear Solid".into(),
        "--platform".into(),
        "PlayStation".into(),
        "--region".into(),
        "France".into(),
        "--edition".into(),
        "Original".into(),
        "--file".into(),
        source.as_os_str().to_owned(),
    ])
    .unwrap();
    assert!(imported.contains("Imported Box Front"));

    let library = game_media_vault_cli::run([
        "game-media-vault".into(),
        "--vault".into(),
        vault.as_os_str().to_owned(),
        "library".into(),
    ])
    .unwrap();

    assert!(library.contains("Metal Gear Solid"));
    assert!(library.contains("PlayStation"));
    assert!(library.contains("cover-front.png"));
}

#[test]
fn game_id_links_a_second_import_to_an_existing_game() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let first_source = temp.path().join("first-front.png");
    let second_source = temp.path().join("localized-front.png");
    fs::write(&first_source, b"first cli cover").unwrap();
    fs::write(&second_source, b"second cli cover").unwrap();

    game_media_vault_cli::run([
        "game-media-vault".into(),
        "--vault".into(),
        vault.as_os_str().to_owned(),
        "import-box-front".into(),
        "--game".into(),
        "Original Title".into(),
        "--platform".into(),
        "Windows".into(),
        "--region".into(),
        "Worldwide".into(),
        "--edition".into(),
        "Standard".into(),
        "--file".into(),
        first_source.as_os_str().to_owned(),
    ])
    .unwrap();

    let first_library = game_media_vault_cli::run([
        "game-media-vault".into(),
        "--vault".into(),
        vault.as_os_str().to_owned(),
        "library".into(),
    ])
    .unwrap();
    let first_entries: serde_json::Value = serde_json::from_str(&first_library).unwrap();
    let game_id = first_entries[0]["game_id"].as_i64().unwrap();

    game_media_vault_cli::run([
        "game-media-vault".into(),
        "--vault".into(),
        vault.as_os_str().to_owned(),
        "import-box-front".into(),
        "--game-id".into(),
        game_id.to_string().into(),
        "--game".into(),
        "Localized Title".into(),
        "--platform".into(),
        "Windows".into(),
        "--region".into(),
        "Worldwide".into(),
        "--edition".into(),
        "Standard".into(),
        "--file".into(),
        second_source.as_os_str().to_owned(),
    ])
    .unwrap();

    let library = game_media_vault_cli::run([
        "game-media-vault".into(),
        "--vault".into(),
        vault.as_os_str().to_owned(),
        "library".into(),
    ])
    .unwrap();
    let entries: serde_json::Value = serde_json::from_str(&library).unwrap();
    let entries = entries.as_array().unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["game_id"].as_i64(), Some(game_id));
    assert_eq!(entries[0]["assets"].as_array().unwrap().len(), 2);
}

#[test]
fn imports_a_bounded_no_intro_fixture_without_ingesting_game_content() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("connectors")
        .join("tests")
        .join("fixtures")
        .join("no_intro_sample.dat");

    let summary = game_media_vault_cli::run([
        "game-media-vault".into(),
        "--vault".into(),
        vault.as_os_str().to_owned(),
        "import-no-intro".into(),
        "--file".into(),
        fixture.as_os_str().to_owned(),
        "--max-games".into(),
        "2".into(),
    ])
    .unwrap();
    let summary: serde_json::Value = serde_json::from_str(&summary).unwrap();
    assert_eq!(summary["imported_releases"], 2);

    let library = game_media_vault_cli::run([
        "game-media-vault".into(),
        "--vault".into(),
        vault.as_os_str().to_owned(),
        "library".into(),
    ])
    .unwrap();
    let entries: serde_json::Value = serde_json::from_str(&library).unwrap();
    let entries = entries.as_array().unwrap();

    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .all(|entry| entry["assets"] == serde_json::json!([]))
    );
    assert!(entries.iter().any(|entry| {
        entry["game_title"] == "Tetris"
            && entry["assertions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|assertion| assertion["field"] == "revision" && assertion["value"] == "Rev 1")
    }));
    assert!(!vault.join("objects").exists());
}

#[test]
fn acquire_uses_the_shared_source_validation() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let error = run_in_vault(
        &vault,
        &[
            "acquire",
            "--platform",
            "Windows",
            "--asset-type",
            "box-front",
        ],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CliError::Validation(AcquisitionRequestValidationError::MissingSources)
    ));
}

#[test]
fn acquire_builds_the_full_request_from_cli_filters() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let output = run_in_vault(
        &vault,
        &[
            "acquire",
            "--source",
            "screenscraper",
            "--source",
            "game-tdb",
            "--platform",
            "PlayStation 2",
            "--game",
            "Metal Gear Solid 3",
            "--region",
            "France",
            "--language",
            "fr",
            "--asset-type",
            "box-front",
            "--asset-type",
            "manual",
            "--asset-type",
            "screenshot",
            "--min-width",
            "1600",
            "--min-height",
            "1200",
            "--min-longest-edge",
            "2000",
            "--min-pixel-count",
            "2000000",
            "--original-only",
            "--mime-type",
            "image/png",
            "--max-compression-ratio",
            "12",
            "--min-bitrate-kbps",
            "320",
            "--preferred-scan-type",
            "raw_scan",
            "--preferred-source-priority",
            "screenscraper",
            "--preferred-source-priority",
            "game-tdb",
            "--best-available",
            "--retention",
            "keep-best-per-type",
            "--max-games",
            "25",
            "--max-downloads",
            "100",
            "--max-concurrent-downloads",
            "4",
            "--max-bytes",
            "5000000000",
        ],
    )
    .unwrap();

    let run: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(run["status"], "running");
    let request = &run["request"];
    assert_eq!(
        request["sources"]["values"],
        serde_json::json!(["screenscraper", "game-tdb"])
    );
    assert_eq!(request["platforms"], serde_json::json!(["PlayStation 2"]));
    assert_eq!(
        request["games"]["values"],
        serde_json::json!(["Metal Gear Solid 3"])
    );
    assert_eq!(request["regions"], serde_json::json!(["France"]));
    assert_eq!(request["languages"], serde_json::json!(["fr"]));
    assert_eq!(
        request["asset_types"],
        serde_json::json!(["box_front", "manual", "screenshot"])
    );
    assert_eq!(request["quality"]["min_width"], 1600);
    assert_eq!(request["quality"]["max_compression_ratio"], 12);
    assert_eq!(request["quality"]["preferred_scan_type"], "raw_scan");
    assert_eq!(
        request["quality"]["preferred_source_priority"],
        serde_json::json!(["screenscraper", "game-tdb"])
    );
    assert_eq!(request["quality"]["best_available"], true);
    assert_eq!(request["retention"], "keep_best_per_type");
    assert_eq!(request["limits"]["max_games"], 25);
    assert_eq!(request["limits"]["max_concurrent_downloads"], 4);
}

#[test]
fn acquire_accepts_canonical_3d_asset_type_names() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let output = run_in_vault(
        &vault,
        &[
            "acquire",
            "--source",
            "screenscraper",
            "--platform",
            "PlayStation 2",
            "--asset-type",
            "box-3d-render",
            "--asset-type",
            "box-3d-model",
            "--asset-type",
            "3d-model",
        ],
    )
    .unwrap();

    let run: serde_json::Value = serde_json::from_str(&output).unwrap();
    let request = &run["request"];
    assert_eq!(
        request["asset_types"],
        serde_json::json!(["box_3d_render", "box_3d_model", "3d_model"])
    );
}

#[test]
fn acquire_preserves_asset_type_family_selection() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let output = run_in_vault(
        &vault,
        &[
            "acquire",
            "--source",
            "screenscraper",
            "--platform",
            "PlayStation 2",
            "--asset-type",
            "packaging",
            "--asset-type",
            "documentation",
        ],
    )
    .unwrap();

    let run: serde_json::Value = serde_json::from_str(&output).unwrap();
    let request = &run["request"];
    assert_eq!(
        request["asset_types"],
        serde_json::json!(["packaging", "documentation"])
    );
}

#[test]
fn acquire_preserves_platform_bound_game_targeting() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let output = run_in_vault(
        &vault,
        &[
            "acquire",
            "--source",
            "screenscraper",
            "--platform-game",
            "PlayStation 2=Metal Gear Solid 3",
            "--asset-type",
            "box-front",
        ],
    )
    .unwrap();

    let run: serde_json::Value = serde_json::from_str(&output).unwrap();
    let request = &run["request"];
    assert_eq!(request["platforms"], serde_json::json!([]));
    assert_eq!(request["games"]["mode"], "platform_bound");
    assert_eq!(
        request["games"]["values"],
        serde_json::json!([{
            "game": "Metal Gear Solid 3",
            "platform": "PlayStation 2"
        }])
    );
}

#[test]
fn acquire_preserves_query_result_game_targeting() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let output = run_in_vault(
        &vault,
        &[
            "acquire",
            "--source",
            "screenscraper",
            "--query-result",
            "PlayStation 2=Metal Gear Solid 3",
            "--asset-type",
            "manual",
        ],
    )
    .unwrap();

    let run: serde_json::Value = serde_json::from_str(&output).unwrap();
    let request = &run["request"];
    assert_eq!(request["platforms"], serde_json::json!([]));
    assert_eq!(request["games"]["mode"], "query_result");
    assert_eq!(
        request["games"]["values"],
        serde_json::json!([{
            "game": "Metal Gear Solid 3",
            "platform": "PlayStation 2"
        }])
    );
}

#[test]
fn acquire_persists_a_run_that_can_be_controlled_and_listed() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");

    let started = run_in_vault(
        &vault,
        &[
            "acquire",
            "--source",
            "screenscraper",
            "--platform",
            "Windows",
            "--asset-type",
            "box-front",
        ],
    )
    .unwrap();
    let started: serde_json::Value = serde_json::from_str(&started).unwrap();
    let run_id = started["id"].as_i64().unwrap();
    let run_id_arg = run_id.to_string();
    assert_eq!(started["status"], "running");

    let paused = run_in_vault(&vault, &["run", "pause", &run_id_arg]).unwrap();
    let paused: serde_json::Value = serde_json::from_str(&paused).unwrap();
    assert_eq!(paused["status"], "paused");

    let resumed = run_in_vault(&vault, &["run", "resume", &run_id_arg]).unwrap();
    let resumed: serde_json::Value = serde_json::from_str(&resumed).unwrap();
    assert_eq!(resumed["status"], "running");

    let listed = run_in_vault(&vault, &["run", "list"]).unwrap();
    let listed: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed[0]["id"], run_id);
    assert_eq!(listed[0]["status"], "running");

    let shown = run_in_vault(&vault, &["run", "show", &run_id_arg]).unwrap();
    let shown: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(shown, listed[0]);

    let cancelled = run_in_vault(&vault, &["run", "cancel", &run_id_arg]).unwrap();
    let cancelled: serde_json::Value = serde_json::from_str(&cancelled).unwrap();
    assert_eq!(cancelled["status"], "cancelled");
}
