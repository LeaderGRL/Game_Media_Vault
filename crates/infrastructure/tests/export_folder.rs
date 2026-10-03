use std::{fs, path::Path};

use game_media_vault_application::ExportTargetPort;
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
