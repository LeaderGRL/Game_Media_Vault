use std::{fs, path::Path};

use game_media_vault_application::{ApplicationError, ExportTargetPort};
use game_media_vault_infrastructure::ExportFolder;
use tempfile::tempdir;

#[test]
fn writes_a_file_with_its_folders_and_knows_it_holds_it() {
    let temp = tempdir().unwrap();
    let folder = ExportFolder::new(temp.path().join("export"));
    let path = Path::new("Nintendo - Nintendo Entertainment System")
        .join("Super Mario Bros. (World)")
        .join("Box Front")
        .join("front.png");

    assert!(!folder.holds(&path, 5).unwrap());
    folder.write(&path, &mut &b"front"[..]).unwrap();

    assert_eq!(
        fs::read(temp.path().join("export").join(&path)).unwrap(),
        b"front"
    );
    assert!(folder.holds(&path, 5).unwrap());
    // A file of another size is no copy of that original.
    assert!(!folder.holds(&path, 6).unwrap());
}

#[test]
fn writing_again_replaces_the_file_and_leaves_nothing_half_written() {
    let temp = tempdir().unwrap();
    let folder = ExportFolder::new(temp.path());
    let path = Path::new("Map").join("map.png");
    folder.write(&path, &mut &b"old map"[..]).unwrap();

    folder.write(&path, &mut &b"new"[..]).unwrap();

    assert_eq!(fs::read(temp.path().join(&path)).unwrap(), b"new");
    let names: Vec<String> = fs::read_dir(temp.path().join("Map"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["map.png"]);
}

#[test]
fn a_write_leaves_the_partial_copy_of_another_export_alone() {
    let temp = tempdir().unwrap();
    let folder = ExportFolder::new(temp.path());
    let path = Path::new("Map").join("map.png");
    fs::create_dir_all(temp.path().join("Map")).unwrap();
    // Another export writing the same file at the same time.
    let other = temp.path().join("Map").join("map.png.partial");
    fs::write(&other, b"other export").unwrap();

    folder.write(&path, &mut &b"map"[..]).unwrap();

    assert_eq!(fs::read(temp.path().join(&path)).unwrap(), b"map");
    assert_eq!(fs::read(&other).unwrap(), b"other export");
}

#[test]
fn a_folder_within_the_vault_takes_no_export() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    fs::create_dir_all(vault.join("objects")).unwrap();

    for within in [
        vault.clone(),
        vault.join("objects"),
        vault.join("not yet").join("there"),
    ] {
        assert!(
            matches!(
                ExportFolder::outside_vault(&within, &vault),
                Err(ApplicationError::ExportWithinVault)
            ),
            "{}",
            within.display()
        );
    }
    assert!(ExportFolder::outside_vault(&temp.path().join("export"), &vault).is_ok());
}
