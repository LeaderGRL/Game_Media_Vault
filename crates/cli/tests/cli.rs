use std::{fs, process::Command};

use tempfile::tempdir;

#[test]
fn help_exits_successfully() {
    let status = Command::new(env!("CARGO_BIN_EXE_game-media-vault"))
        .arg("--help")
        .status()
        .unwrap();

    assert!(status.success());
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
