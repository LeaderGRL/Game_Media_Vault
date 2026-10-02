use std::fs;

use game_media_vault_application::MachineSettingsPort;
use game_media_vault_infrastructure::MachineSettingsFile;
use tempfile::tempdir;

#[test]
fn a_machine_without_settings_disables_no_source() {
    let temp = tempdir().unwrap();
    let settings = MachineSettingsFile::at(temp.path().join("settings.json"));

    assert!(settings.disabled_sources().unwrap().is_empty());
}

#[test]
fn disabled_sources_stay_disabled_for_every_later_reader() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("config").join("settings.json");
    let settings = MachineSettingsFile::at(&path);

    settings
        .set_source_enabled("launchbox-games-db", false)
        .unwrap();
    settings
        .set_source_enabled("launchbox-games-db", false)
        .unwrap();
    settings.set_source_enabled("no-intro", false).unwrap();

    assert_eq!(
        MachineSettingsFile::at(&path).disabled_sources().unwrap(),
        ["launchbox-games-db", "no-intro"]
    );
}

#[test]
fn a_source_enabled_again_is_no_longer_disabled() {
    let temp = tempdir().unwrap();
    let settings = MachineSettingsFile::at(temp.path().join("settings.json"));
    settings
        .set_source_enabled("launchbox-games-db", false)
        .unwrap();

    settings
        .set_source_enabled("launchbox-games-db", true)
        .unwrap();

    assert!(settings.disabled_sources().unwrap().is_empty());
}

#[test]
fn settings_this_version_does_not_know_are_kept() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("settings.json");
    fs::write(&path, r#"{"disabled_sources":[],"theme":"dark"}"#).unwrap();

    MachineSettingsFile::at(&path)
        .set_source_enabled("no-intro", false)
        .unwrap();

    let written: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(written["theme"], "dark");
    assert_eq!(written["disabled_sources"], serde_json::json!(["no-intro"]));
}

#[test]
fn unreadable_settings_are_refused_rather_than_enabling_every_source() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("settings.json");
    fs::write(&path, "not json").unwrap();
    let settings = MachineSettingsFile::at(&path);

    let error = settings.disabled_sources().unwrap_err();

    assert!(error.message().contains("settings.json"), "{error}");
    assert!(settings.set_source_enabled("no-intro", false).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "not json");
}
