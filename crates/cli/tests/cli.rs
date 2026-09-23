use std::{
    ffi::OsString,
    fs,
    io::{Cursor, Read},
    path::Path,
    process::Command,
};

use game_media_vault_application::{AcquisitionRequestValidationError, ConnectorPort, PortError};
use game_media_vault_cli::CliError;
use game_media_vault_domain::{
    AcquisitionRequest, AssetCandidate, AssetType, ConnectorCapabilities, SourceId,
};
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
