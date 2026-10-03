use game_media_vault_application::{CredentialState, Machine};
use game_media_vault_infrastructure::{MachineSettingsFile, NoCredentials};

/// A machine whose settings live in `settings` and which keeps no credential.
fn machine(settings: &MachineSettingsFile) -> Machine<'_> {
    Machine {
        settings,
        credentials: &NoCredentials,
    }
}

#[test]
fn the_desktop_describes_every_registered_source_without_a_vault() {
    let temp = tempfile::tempdir().unwrap();
    let settings = MachineSettingsFile::at(temp.path().join("settings.json"));

    let sources = game_media_vault_tauri::list_sources_on_machine(machine(&settings)).unwrap();

    let source_ids: Vec<&str> = sources
        .iter()
        .map(|source| source.source_id.as_str())
        .collect();
    assert_eq!(
        source_ids,
        [
            "libretro-thumbnails",
            "launchbox-games-db",
            "steamgriddb",
            "thegamesdb",
            "screenscraper",
            "rawg",
            "psx-datacenter",
            "vgmaps"
        ]
    );
    assert!(sources.iter().all(|source| source.direct_media_download));
    assert!(sources.iter().all(|source| source.enabled));
    // Only SteamGridDB, TheGamesDB, ScreenScraper and RAWG need credentials, which this machine
    // does not store.
    let credentials: Vec<CredentialState> =
        sources.iter().map(|source| source.credential).collect();
    assert_eq!(
        credentials,
        [
            CredentialState::NotNeeded,
            CredentialState::NotNeeded,
            CredentialState::Missing,
            CredentialState::Missing,
            CredentialState::Missing,
            CredentialState::Missing,
            CredentialState::NotNeeded,
            CredentialState::NotNeeded,
        ]
    );
}

#[test]
fn the_desktop_summarizes_the_failures_the_vault_recorded_per_source() {
    use game_media_vault_application::RunRepositoryPort;
    use game_media_vault_domain::{
        AcquisitionLimits, AcquisitionRequest, AcquisitionRequestDraft, AssetTypeSelector,
        GameSelection, RetentionPolicy, SourceFailureStage, SourceSelection,
    };
    use game_media_vault_infrastructure::SqliteCatalog;

    let temp = tempfile::tempdir().unwrap();
    let vault = temp.path().join("vault");
    let catalog = SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let request = AcquisitionRequest::try_from_draft(AcquisitionRequestDraft {
        sources: SourceSelection::Explicit(vec!["libretro-thumbnails".to_owned()]),
        platforms: vec!["Nintendo - Game Boy".to_owned()],
        games: GameSelection::Explicit(vec!["Tetris".to_owned()]),
        regions: Vec::new(),
        languages: Vec::new(),
        asset_types: vec![AssetTypeSelector::BoxFront],
        quality: None,
        retention: RetentionPolicy::KeepEverything,
        limits: AcquisitionLimits::default(),
    })
    .unwrap();
    let run = catalog
        .create_run(request, vec!["libretro-thumbnails".to_owned()])
        .unwrap();
    for message in ["timed out", "HTTP 503"] {
        catalog
            .record_source_failure(
                run.id,
                "libretro-thumbnails",
                SourceFailureStage::Discovery,
                message,
            )
            .unwrap();
    }

    let summaries = game_media_vault_tauri::load_source_failures(&vault, 1).unwrap();

    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].failures, 2);
    assert_eq!(summaries[0].latest[0].message, "HTTP 503");
}

#[test]
fn the_desktop_disables_a_source_on_this_machine_and_enables_it_again() {
    let temp = tempfile::tempdir().unwrap();
    let settings = MachineSettingsFile::at(temp.path().join("settings.json"));

    let disabled = game_media_vault_tauri::set_source_enabled_on_machine(
        machine(&settings),
        "launchbox-games-db",
        false,
    )
    .unwrap();
    let listed = game_media_vault_tauri::list_sources_on_machine(machine(&settings)).unwrap();
    let enabled = game_media_vault_tauri::set_source_enabled_on_machine(
        machine(&settings),
        "launchbox-games-db",
        true,
    )
    .unwrap();

    let launchbox = |sources: &[game_media_vault_application::SourceDescription]| {
        sources
            .iter()
            .find(|source| source.source_id == "launchbox-games-db")
            .unwrap()
            .enabled
    };
    assert!(!launchbox(&disabled));
    assert!(!launchbox(&listed));
    assert!(launchbox(&enabled));
}

#[test]
fn the_desktop_gives_no_api_key_to_a_source_that_needs_none() {
    let temp = tempfile::tempdir().unwrap();
    let settings = MachineSettingsFile::at(temp.path().join("settings.json"));

    let error = game_media_vault_tauri::set_source_credential_on_machine(
        machine(&settings),
        "libretro-thumbnails",
        None,
        "zq-test-secret",
    )
    .unwrap_err();

    assert_eq!(error.kind, "invalid_request");
    assert!(
        !error.message.contains("zq-test-secret"),
        "{}",
        error.message
    );
}

#[test]
fn the_desktop_gives_a_source_no_credential_it_does_not_ask_for() {
    let temp = tempfile::tempdir().unwrap();
    let settings = MachineSettingsFile::at(temp.path().join("settings.json"));

    let error = game_media_vault_tauri::set_source_credential_on_machine(
        machine(&settings),
        "steamgriddb",
        Some("dev-password"),
        "zq-test-secret",
    )
    .unwrap_err();

    assert_eq!(error.kind, "invalid_request");
    assert!(error.message.contains("dev-password"), "{}", error.message);
    assert!(!error.message.contains("zq-test-secret"));
}
