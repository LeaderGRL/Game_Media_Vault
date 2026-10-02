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
