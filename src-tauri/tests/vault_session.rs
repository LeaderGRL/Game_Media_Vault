use game_media_vault_infrastructure::SqliteCatalog;
use game_media_vault_tauri::{CommandError, VaultSession};
use tempfile::tempdir;

#[test]
fn commands_need_an_open_vault() {
    let session = VaultSession::default();

    assert_eq!(
        session.root().unwrap_err(),
        CommandError {
            kind: "invalid_request",
            message: "no vault is open".to_owned(),
        }
    );
}

#[test]
fn opening_an_existing_vault_scopes_later_commands_to_it() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let session = VaultSession::default();

    session.open(&vault, false).unwrap();

    assert_eq!(session.root().unwrap(), vault);
}

#[test]
fn opening_a_missing_vault_fails_without_creating_it() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("missing");
    let session = VaultSession::default();

    let error = session.open(&vault, false).unwrap_err();

    assert_eq!(error.kind, "external");
    assert!(!vault.join("catalog.sqlite3").exists());
    assert!(session.root().is_err());
}

#[test]
fn opening_with_create_initializes_a_new_vault() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("new-vault");
    let session = VaultSession::default();

    session.open(&vault, true).unwrap();

    assert!(vault.join("catalog.sqlite3").is_file());
    assert_eq!(session.root().unwrap(), vault);
}

#[test]
fn a_failed_open_closes_the_previous_vault() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let session = VaultSession::default();
    session.open(&vault, false).unwrap();

    session
        .open(&temp.path().join("missing"), false)
        .unwrap_err();

    assert!(session.root().is_err());
}

#[test]
fn equivalent_spellings_of_a_vault_path_open_the_same_vault() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    let other = temp.path().join("other");
    SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    SqliteCatalog::open(other.join("catalog.sqlite3")).unwrap();
    let session = VaultSession::default();

    let direct = session.open(&vault, false).unwrap();
    let dotted = session
        .open(&temp.path().join(".").join("vault"), false)
        .unwrap();
    let roundabout = session
        .open(&other.join("..").join("vault"), false)
        .unwrap();
    let elsewhere = session.open(&other, false).unwrap();

    assert_eq!(dotted, direct);
    assert_eq!(roundabout, direct);
    assert_ne!(elsewhere, direct);
    // Later commands still use the path as given.
    assert_eq!(session.root().unwrap(), other);
}

#[cfg(windows)]
#[test]
fn windows_paths_that_differ_only_in_case_open_the_same_vault() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    SqliteCatalog::open(vault.join("catalog.sqlite3")).unwrap();
    let session = VaultSession::default();

    let lower = session.open(&vault, false).unwrap();
    let upper = session.open(&temp.path().join("VAULT"), false).unwrap();

    assert_eq!(upper, lower);
}
