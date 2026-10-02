#[test]
fn the_desktop_describes_every_registered_source_without_a_vault() {
    let sources = game_media_vault_tauri::list_registered_sources();

    let source_ids: Vec<&str> = sources
        .iter()
        .map(|source| source.source_id.as_str())
        .collect();
    assert_eq!(source_ids, ["libretro-thumbnails", "launchbox-games-db"]);
    assert!(sources.iter().all(|source| source.direct_media_download));
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
