use std::{
    ffi::OsString,
    fs,
    io::{Cursor, Read},
    path::Path,
    process::Command,
};

use game_media_vault_application::{
    AcquisitionRequestValidationError, ApplicationError, ConnectorPort, ParkedReview, PortError,
    ReferenceCatalogRepositoryPort, ReviewRepositoryPort, RunRepositoryPort, candidate_identity,
};
use game_media_vault_cli::CliError;
use game_media_vault_domain::{
    AcquisitionRequest, AcquisitionWorkItem, AssetCandidate, AssetType, ConnectorCapabilities,
    MatchEvidence, MatchSignal, MatchingPolicy, MatchingPolicyValidationError, NewReviewItem,
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
            provider_candidate_id: None,
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

/// Refuses every plan, as a Source that cannot satisfy a request does.
struct RefusingConnector;

impl ConnectorPort for RefusingConnector {
    fn source_id(&self) -> &'static str {
        "libretro-thumbnails"
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        FixtureConnector.capabilities()
    }

    fn unsupported_request_reason(
        &self,
        _request: &AcquisitionRequest,
    ) -> Result<Option<String>, PortError> {
        Ok(Some(
            "the fixture Source declares no such platform".to_owned(),
        ))
    }

    fn discover(&self, _request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        unreachable!("a refused plan is never discovered")
    }

    fn download(&self, _candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        unreachable!("a refused plan downloads nothing")
    }
}

fn cli_args(vault: &Path, args: &[&str]) -> Vec<OsString> {
    let mut command = vec![
        OsString::from("game-media-vault"),
        OsString::from("--vault"),
        vault.as_os_str().to_owned(),
    ];
    command.extend(args.iter().map(OsString::from));
    command
}

/// The validated request of an `acquire` command line, built without starting a run.
fn request_from(vault: &Path, args: &[&str]) -> serde_json::Value {
    serde_json::to_value(
        game_media_vault_cli::acquisition_request_from_args(cli_args(vault, args)).unwrap(),
    )
    .unwrap()
}

/// Runs the CLI with the fixture connector, so no command reaches the network.
fn run_in_vault(vault: &Path, args: &[&str]) -> Result<String, CliError> {
    game_media_vault_cli::run_with_connectors(cli_args(vault, args), &[&FixtureConnector])
}

fn seed_review_item(vault: &Path) -> i64 {
    let started: serde_json::Value = serde_json::from_str(
        &run_in_vault(
            vault,
            &[
                "acquire",
                "--source",
                "libretro-thumbnails",
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
        provider_candidate_id: None,
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
    let work_key = candidate_identity(candidate.source_id.as_str(), &candidate);
    catalog
        .record_discovery(
            run_id,
            "fixture-provider",
            &[AcquisitionWorkItem {
                key: work_key.clone(),
                candidate: candidate.clone(),
            }],
        )
        .unwrap();
    let parked = catalog
        .park_work_for_review(
            run_id,
            &work_key,
            NewReviewItem {
                candidate_identity: work_key.clone(),
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
            },
        )
        .unwrap();
    let ParkedReview::Parked(item) = parked else {
        panic!("expected a new review item");
    };
    item.id
}

#[test]
fn review_commands_show_evidence_and_persist_accept_reject_and_defer() {
    let temp = tempdir().unwrap();
    let accept_vault = temp.path().join("accept-vault");
    let reject_vault = temp.path().join("reject-vault");
    let defer_vault = temp.path().join("defer-vault");
    let accepted_review_item_id = seed_review_item(&accept_vault);
    let rejected_review_item_id = seed_review_item(&reject_vault);
    let deferred_review_item_id = seed_review_item(&defer_vault);

    let listed: serde_json::Value =
        serde_json::from_str(&run_in_vault(&accept_vault, &["review", "list"]).unwrap()).unwrap();
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
            &accept_vault,
            &[
                "review",
                "accept",
                &accepted_review_item_id.to_string(),
                "--release-edition-id",
                "201",
            ],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(accepted["decision"]["decision"], "accept");
    assert_eq!(accepted["decision"]["release_edition_id"], 201);

    let rejected: serde_json::Value = serde_json::from_str(
        &run_in_vault(
            &reject_vault,
            &["review", "reject", &rejected_review_item_id.to_string()],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(rejected["decision"]["decision"], "reject");

    let deferred: serde_json::Value = serde_json::from_str(
        &run_in_vault(
            &defer_vault,
            &["review", "defer", &deferred_review_item_id.to_string()],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(deferred["decision"]["decision"], "defer");

    let reopened = SqliteCatalog::open_existing(defer_vault.join("catalog.sqlite3")).unwrap();
    assert_eq!(
        reopened
            .get_review_item(deferred_review_item_id)
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

    let completed = game_media_vault_cli::execute_acquisition_run_in_vault_with_connectors(
        &vault,
        run_id,
        &[&FixtureConnector],
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
fn library_lists_canonical_values_with_their_confidence() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let assertion = |field, qualifier: Option<&str>, value: &str| ReleaseAssertion {
        source_id: SourceId::from("no-intro"),
        source_location: "C:/catalogs/Nintendo - Game Boy.dat".to_owned(),
        field,
        qualifier: qualifier.map(str::to_owned),
        value: value.to_owned(),
    };
    SqliteCatalog::open(vault.join("catalog.sqlite3"))
        .unwrap()
        .persist_reference_release(ReferenceReleaseRecord {
            game_title: "Tetris".to_owned(),
            platform: "Nintendo - Game Boy".to_owned(),
            region: "World".to_owned(),
            revision: Some("Rev 1".to_owned()),
            edition_name: "Rev 1".to_owned(),
            assertions: vec![
                assertion(
                    ReleaseAssertionField::Identifier,
                    Some("source_record"),
                    "no-intro:tetris",
                ),
                assertion(ReleaseAssertionField::Title, None, "Tetris"),
            ],
        })
        .unwrap();

    let library: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["library"]).unwrap()).unwrap();

    let canonical = &library[0]["canonical_values"][0];
    assert_eq!(canonical["field"], "title");
    assert_eq!(canonical["value"], "Tetris");
    assert_eq!(canonical["confidence"], 100);
    assert_eq!(library[0]["game_title"], "Tetris");
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

    game_media_vault_cli::execute_acquisition_run_in_vault_with_connectors(
        &vault,
        run_id,
        &[&FixtureConnector],
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

    let error = game_media_vault_cli::execute_acquisition_run_in_vault_with_connectors(
        &vault,
        run_id,
        &[&FixtureConnector],
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
    // No connector executes this plan yet, so the request is built without starting a run.
    let request = game_media_vault_cli::acquisition_request_from_args(cli_args(
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
    ))
    .unwrap();

    let request = serde_json::to_value(&request).unwrap();
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
fn acquire_refuses_a_plan_its_connector_cannot_execute() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");

    let error = game_media_vault_cli::run_with_connectors(
        cli_args(
            &vault,
            &[
                "acquire",
                "--source",
                "libretro-thumbnails",
                "--platform",
                "Nintendo - Famicom Disk Sytem",
                "--game",
                "Zelda no Densetsu",
                "--asset-type",
                "box-front",
            ],
        ),
        &[&RefusingConnector],
    )
    .unwrap_err();

    assert_eq!(error.exit_code(), 5);
    assert!(
        error
            .to_string()
            .contains("the fixture Source declares no such platform"),
        "{error}"
    );
    let runs: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["run", "list"]).unwrap()).unwrap();
    assert_eq!(runs, serde_json::json!([]));
}

#[test]
fn plan_explains_which_source_acquires_each_requested_type_without_starting_a_run() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");

    let plan: serde_json::Value = serde_json::from_str(
        &run_in_vault(
            &vault,
            &[
                "plan",
                "--auto-source",
                "--platform",
                "Nintendo - Nintendo Entertainment System",
                "--game",
                "Super Mario Bros. (World)",
                "--asset-type",
                "box-front",
            ],
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
        plan,
        serde_json::json!({
            "sources": [{ "source_id": "libretro-thumbnails", "asset_types": ["box_front"] }],
            "excluded": [],
            "coverage": [{ "selector": "box_front", "sources": ["libretro-thumbnails"] }],
        })
    );
    // Planning leaves the vault untouched.
    assert!(!vault.join("catalog.sqlite3").exists());
}

#[test]
fn plan_refuses_a_request_no_selected_source_serves() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");

    let error = game_media_vault_cli::run_with_connectors(
        cli_args(
            &vault,
            &[
                "plan",
                "--source",
                "libretro-thumbnails",
                "--platform",
                "Nintendo - Famicom Disk Sytem",
                "--game",
                "Zelda no Densetsu",
                "--asset-type",
                "box-front",
            ],
        ),
        &[&RefusingConnector],
    )
    .unwrap_err();

    assert_eq!(error.exit_code(), 5);
    assert!(
        error
            .to_string()
            .contains("libretro-thumbnails: the fixture Source declares no such platform"),
        "{error}"
    );
}

#[test]
fn acquire_accepts_canonical_3d_asset_type_names() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let request = request_from(
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
    );
    assert_eq!(
        request["asset_types"],
        serde_json::json!(["box_3d_render", "box_3d_model", "3d_model"])
    );
}

#[test]
fn acquire_preserves_asset_type_family_selection() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let request = request_from(
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
    );
    assert_eq!(
        request["asset_types"],
        serde_json::json!(["packaging", "documentation"])
    );
}

#[test]
fn acquire_preserves_platform_bound_game_targeting() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let request = request_from(
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
    );
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
    let request = request_from(
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
    );
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
            "libretro-thumbnails",
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

struct LibretroFixtureTransport;

impl game_media_vault_connectors::HttpTransport for LibretroFixtureTransport {
    fn get_stream(&self, url: &str) -> Result<Box<dyn Read + Send>, PortError> {
        let body: &[u8] = if url.ends_with("/.gitmodules") {
            b"[submodule \"Nintendo - Game Boy\"]\n\
              path = Nintendo - Game Boy\n\
              url = https://github.com/libretro-thumbnails/Nintendo_-_Game_Boy.git\n\
              branch = master\n"
        } else {
            b"tetris box front"
        };
        Ok(Box::new(Cursor::new(body.to_vec())))
    }
}

#[test]
fn libretro_box_front_auto_links_to_an_imported_no_intro_release() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("connectors")
        .join("tests")
        .join("fixtures")
        .join("no_intro_sample.dat");
    run_in_vault(
        &vault,
        &[
            "import-no-intro",
            "--file",
            fixture.to_str().unwrap(),
            "--max-games",
            "10",
        ],
    )
    .unwrap();
    let started: serde_json::Value = serde_json::from_str(
        &run_in_vault(
            &vault,
            &[
                "acquire",
                "--source",
                "libretro-thumbnails",
                "--platform",
                "Nintendo - Game Boy",
                "--game",
                "Tetris (World) (Rev 1)",
                "--asset-type",
                "box-front",
            ],
        )
        .unwrap(),
    )
    .unwrap();
    let connector = game_media_vault_connectors::LibretroThumbnailsConnector::with_transport(
        LibretroFixtureTransport,
    );

    let completed = game_media_vault_cli::execute_acquisition_run_in_vault_with_connectors(
        &vault,
        started["id"].as_i64().unwrap(),
        &[&connector],
        MatchingPolicy {
            high_confidence_threshold: 80,
            medium_confidence_threshold: 50,
        },
    )
    .unwrap();

    assert_eq!(completed.completed_work, 1);
    assert_eq!(completed.awaiting_review_work, 0);
    let library: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["library"]).unwrap()).unwrap();
    let tetris = library
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["game_title"] == "Tetris")
        .unwrap();
    assert_eq!(tetris["region"], "World");
    let provenance = &tetris["assets"][0]["provenance"][0];
    assert_eq!(provenance["source_id"], "libretro-thumbnails");
    let decision = &provenance["match_decision"];
    assert_eq!(decision["confidence"], "high");
    assert_eq!(decision["score"], 100);
}

fn run_binary(vault: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_game-media-vault"))
        .arg("--vault")
        .arg(vault)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn exit_codes_follow_the_error_kind() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let invalid = run_binary(
        &vault,
        &[
            "acquire",
            "--platform",
            "Windows",
            "--asset-type",
            "box-front",
        ],
    );
    assert_eq!(invalid.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&invalid.stderr)
            .contains("acquisition request must include at least one source")
    );

    // Libretro refuses unbounded game selections without reaching the network.
    let unsupported = run_binary(
        &vault,
        &[
            "acquire",
            "--source",
            "libretro-thumbnails",
            "--platform",
            "Windows",
            "--asset-type",
            "box-front",
        ],
    );
    assert_eq!(unsupported.status.code(), Some(5));
    assert!(
        String::from_utf8_lossy(&unsupported.stderr)
            .contains("requires an explicit bounded game selection")
    );

    run_in_vault(
        &vault,
        &[
            "acquire",
            "--source",
            "libretro-thumbnails",
            "--platform",
            "Windows",
            "--asset-type",
            "box-front",
        ],
    )
    .unwrap();
    let missing = run_binary(&vault, &["run", "show", "99"]);
    assert_eq!(missing.status.code(), Some(3));
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("acquisition run #99 does not exist")
    );

    let conflict = run_binary(&vault, &["run", "resume", "1"]);
    assert_eq!(conflict.status.code(), Some(0));
    run_binary(&vault, &["run", "cancel", "1"]);
    let conflict = run_binary(&vault, &["run", "pause", "1"]);
    assert_eq!(conflict.status.code(), Some(4));
}

#[test]
fn a_malformed_reference_catalog_exits_as_a_source_failure() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let datafile = temp.path().join("broken.dat");
    fs::write(
        &datafile,
        "<datafile><header><name>Nintendo - Game Boy</name></header><game>",
    )
    .unwrap();

    let error = run_in_vault(
        &vault,
        &[
            "import-no-intro",
            "--file",
            datafile.to_str().unwrap(),
            "--max-games",
            "10",
        ],
    )
    .unwrap_err();

    assert_eq!(error.exit_code(), 6, "{error}");
}

fn titles(page: &serde_json::Value) -> Vec<String> {
    page["releases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|release| release["game_title"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn search_pages_through_matching_releases() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("connectors")
        .join("tests")
        .join("fixtures")
        .join("no_intro_sample.dat");
    run_in_vault(
        &vault,
        &[
            "import-no-intro",
            "--file",
            fixture.to_str().unwrap(),
            "--max-games",
            "10",
        ],
    )
    .unwrap();
    let search = |args: &[&str]| -> serde_json::Value {
        let mut command = vec!["search"];
        command.extend_from_slice(args);
        serde_json::from_str(&run_in_vault(&vault, &command).unwrap()).unwrap()
    };

    // Game Boy cardboard boxes without their Assets are Partial.
    let first = search(&["--status", "partial", "--limit", "4"]);
    assert_eq!(first["total"], 6);
    assert_eq!(
        titles(&first),
        [
            "Kirby's Dream Land",
            "Region Test Denmark",
            "Region Test Poland",
            "Super Mario Land"
        ]
    );
    let after = first["next_after"].to_string();
    let as_of = first["as_of"].to_string();
    let second = search(&[
        "--status", "partial", "--limit", "4", "--after", &after, "--as-of", &as_of,
    ]);
    assert_eq!(titles(&second), ["Tetris", "Tom & Jerry"]);
    assert_eq!(second["next_after"], serde_json::Value::Null);

    let mario = search(&["--text", "MARIO", "--platform", "Nintendo - Game Boy"]);
    assert_eq!(titles(&mario), ["Super Mario Land"]);
    assert_eq!(search(&["--status", "needs-review"])["total"], 0);
}

#[test]
fn derive_thumbnails_renders_each_original_once() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let source = temp.path().join("cover-front.png");
    image::RgbImage::from_pixel(400, 300, image::Rgb([200, 30, 30]))
        .save(&source)
        .unwrap();
    run_in_vault(
        &vault,
        &[
            "import-box-front",
            "--game",
            "Metal Gear Solid",
            "--platform",
            "Sony - PlayStation",
            "--region",
            "France",
            "--edition",
            "Original",
            "--file",
            source.to_str().unwrap(),
        ],
    )
    .unwrap();

    let summary: serde_json::Value = serde_json::from_str(
        &run_in_vault(&vault, &["derive-thumbnails", "--max-edge", "100"]).unwrap(),
    )
    .unwrap();

    assert_eq!(summary["derived"], 1);
    let library: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["library"]).unwrap()).unwrap();
    let derived = &library[0]["assets"][0]["derived"][0];
    assert_eq!(derived["recipe"]["transform"], "thumbnail");
    assert_eq!(
        (derived["width"].as_u64(), derived["height"].as_u64()),
        (Some(100), Some(75))
    );
    let again: serde_json::Value = serde_json::from_str(
        &run_in_vault(&vault, &["derive-thumbnails", "--max-edge", "100"]).unwrap(),
    )
    .unwrap();
    assert_eq!(again["derived"], 0);
}

#[test]
fn import_redump_records_disc_releases_in_the_library() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../connectors/tests/fixtures")
        .join("redump_sample.dat");

    let summary: serde_json::Value = serde_json::from_str(
        &run_in_vault(
            &vault,
            &[
                "import-redump",
                "--file",
                fixture.to_str().unwrap(),
                "--max-games",
                "10",
            ],
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(summary["imported_releases"], 2);
    let library: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["library"]).unwrap()).unwrap();
    assert_eq!(library[0]["game_title"], "Final Fantasy VII");
    assert_eq!(library[0]["edition_name"], "Disc 1");
    assert!(
        library[0]["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|assertion| assertion["source_id"] == "redump")
    );
}

#[test]
fn an_exported_request_starts_the_same_acquisition_in_another_vault() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let started: serde_json::Value = serde_json::from_str(
        &run_in_vault(
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
                "--region",
                "World",
            ],
        )
        .unwrap(),
    )
    .unwrap();
    let document = run_in_vault(
        &vault,
        &[
            "run",
            "export",
            &started["id"].as_i64().unwrap().to_string(),
        ],
    )
    .unwrap();
    let document_path = temp.path().join("request.json");
    std::fs::write(&document_path, &document).unwrap();

    let other_vault = temp.path().join("other-vault");
    let restarted: serde_json::Value = serde_json::from_str(
        &run_in_vault(
            &other_vault,
            &["run", "start", document_path.to_str().unwrap()],
        )
        .unwrap(),
    )
    .unwrap();

    let document: serde_json::Value = serde_json::from_str(&document).unwrap();
    assert_eq!(document["format_version"], 1);
    assert_eq!(restarted["request"], started["request"]);
    assert_eq!(restarted["status"], "running");
}

#[test]
fn a_request_document_of_another_format_version_is_unsupported() {
    let temp = tempdir().unwrap();
    let document_path = temp.path().join("request.json");
    std::fs::write(&document_path, r#"{"format_version": 99, "request": {}}"#).unwrap();

    let error = run_in_vault(
        &temp.path().join("vault"),
        &["run", "start", document_path.to_str().unwrap()],
    )
    .unwrap_err();

    assert_eq!(error.exit_code(), 5);
}

#[test]
fn a_malformed_request_document_is_an_invalid_request() {
    let temp = tempdir().unwrap();
    let document_path = temp.path().join("request.json");
    std::fs::write(&document_path, r#"{"format_version": 1, "request": {}}"#).unwrap();

    let error = run_in_vault(
        &temp.path().join("vault"),
        &["run", "start", document_path.to_str().unwrap()],
    )
    .unwrap_err();

    assert_eq!(error.exit_code(), 2, "{error}");
}

#[test]
fn a_request_document_with_a_misspelled_key_is_an_invalid_request() {
    let temp = tempdir().unwrap();
    let document_path = temp.path().join("request.json");
    // `min_widht` would otherwise be dropped, starting a run without its size requirement.
    std::fs::write(
        &document_path,
        r#"{"format_version": 1, "request": {
            "sources": {"mode": "explicit", "values": ["libretro-thumbnails"]},
            "platforms": ["Nintendo - Nintendo Entertainment System"],
            "games": {"mode": "explicit", "values": ["Super Mario Bros. (World)"]},
            "regions": [], "languages": [], "asset_types": ["box_front"],
            "quality": {"min_widht": 1000},
            "retention": "keep_everything",
            "limits": {}
        }}"#,
    )
    .unwrap();

    let error = run_in_vault(
        &temp.path().join("vault"),
        &["run", "start", document_path.to_str().unwrap()],
    )
    .unwrap_err();

    assert_eq!(error.exit_code(), 2, "{error}");
    assert!(error.to_string().contains("min_widht"), "{error}");
}

#[test]
fn a_request_document_that_cannot_be_read_is_an_invalid_request() {
    let temp = tempdir().unwrap();

    let error = run_in_vault(
        &temp.path().join("vault"),
        &[
            "run",
            "start",
            temp.path().join("missing.json").to_str().unwrap(),
        ],
    )
    .unwrap_err();

    assert_eq!(error.exit_code(), 2, "{error}");
}

#[test]
fn verify_reports_originals_the_vault_lost_without_repairing_anything() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let source = temp.path().join("cover-front.png");
    std::fs::write(&source, b"cover bytes").unwrap();
    run_in_vault(
        &vault,
        &[
            "import-box-front",
            "--game",
            "Metal Gear Solid",
            "--platform",
            "Sony - PlayStation",
            "--region",
            "France",
            "--edition",
            "Original",
            "--file",
            source.to_str().unwrap(),
        ],
    )
    .unwrap();
    let healthy: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["verify"]).unwrap()).unwrap();
    let library: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["library"]).unwrap()).unwrap();
    let hash = library[0]["assets"][0]["object_hash"].as_str().unwrap();
    std::fs::remove_file(
        vault
            .join("objects")
            .join(&hash[0..2])
            .join(&hash[2..4])
            .join(hash),
    )
    .unwrap();

    let report: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["verify"]).unwrap()).unwrap();

    assert_eq!(healthy["healthy"], true);
    assert_eq!(report["healthy"], false);
    assert_eq!(report["missing_originals"], serde_json::json!([hash]));
    let library: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["library"]).unwrap()).unwrap();
    assert_eq!(library.as_array().unwrap().len(), 1);
}

#[test]
fn repair_applies_only_the_actions_it_is_given() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    std::fs::create_dir_all(vault.join("staging")).unwrap();
    std::fs::write(vault.join("staging").join("4242-0.tmp"), b"interrupted").unwrap();

    let refused = run_in_vault(&vault, &["repair"]).unwrap_err();
    let summary: serde_json::Value = serde_json::from_str(
        &run_in_vault(&vault, &["repair", "--remove-interrupted-staging"]).unwrap(),
    )
    .unwrap();

    assert_eq!(refused.exit_code(), 2);
    assert_eq!(
        summary["removed_staging"],
        serde_json::json!(["4242-0.tmp"])
    );
    assert_eq!(summary["healthy"], true);
}

/// Discovers the fixture candidate but cannot download it, as a Source whose host is down.
struct UnreachableDownloadConnector;

impl ConnectorPort for UnreachableDownloadConnector {
    fn source_id(&self) -> &'static str {
        FixtureConnector.source_id()
    }

    fn capabilities(&self) -> ConnectorCapabilities {
        FixtureConnector.capabilities()
    }

    fn discover(&self, request: &AcquisitionRequest) -> Result<Vec<AssetCandidate>, PortError> {
        FixtureConnector.discover(request)
    }

    fn download(&self, _candidate: &AssetCandidate) -> Result<Box<dyn Read + Send>, PortError> {
        Err(PortError::new("fixture host unreachable".to_owned()))
    }
}

#[test]
fn a_failed_execution_reports_the_run_it_left_and_a_later_one_resumes_it() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    // The release the fixture candidate matches, so its execution downloads it.
    let cover = temp.path().join("smb-front.png");
    fs::write(&cover, b"imported cover").unwrap();
    run_in_vault(
        &vault,
        &[
            "import-box-front",
            "--game",
            "Super Mario Bros. (World)",
            "--platform",
            "Nintendo - Nintendo Entertainment System",
            "--region",
            "World",
            "--edition",
            "Unspecified",
            "--file",
            cover.to_str().unwrap(),
        ],
    )
    .unwrap();
    let started: serde_json::Value = serde_json::from_str(
        &run_in_vault(
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
        .unwrap(),
    )
    .unwrap();
    let run_id = started["id"].as_i64().unwrap().to_string();

    let error = game_media_vault_cli::run_with_connectors(
        cli_args(&vault, &["run", "execute", &run_id]),
        &[&UnreachableDownloadConnector],
    )
    .unwrap_err();

    // Scripts read where the run stands on stdout and why it stopped on stderr.
    assert!(
        error.to_string().contains("fixture host unreachable"),
        "{error}"
    );
    assert_eq!(error.exit_code(), 1);
    let left: serde_json::Value = serde_json::from_str(error.output().unwrap()).unwrap();
    assert_eq!(left["status"], "running");
    assert_eq!(left["queued_work"], 1);

    // A later execution, as after a restart, resumes the persisted work.
    let resumed: serde_json::Value =
        serde_json::from_str(&run_in_vault(&vault, &["run", "execute", &run_id]).unwrap()).unwrap();
    assert_eq!(resumed["status"], "completed");
    assert_eq!(resumed["completed_work"], 1);
}
