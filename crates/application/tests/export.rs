use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

use game_media_vault_application::{
    CatalogPort, DerivedStorePort, ExportSummary, ExportTargetPort, PortError, export_library,
};
use game_media_vault_domain::{
    AssetType, ImportedAsset, LibraryAsset, LibraryEntry, MediaInfo, PersistAsset,
    ReleaseAssertion, ReleaseAssertionField, StoredObject,
};

const NES: &str = "Nintendo - Nintendo Entertainment System";
const SNES: &str = "Nintendo - Super Nintendo Entertainment System";

struct Library(Vec<LibraryEntry>);

impl CatalogPort for Library {
    fn persist_asset(&self, _record: PersistAsset) -> Result<ImportedAsset, PortError> {
        unreachable!("an export stores nothing in the vault")
    }

    fn list_library(&self) -> Result<Vec<LibraryEntry>, PortError> {
        Ok(self.0.clone())
    }
}

/// Serves each original as the bytes of its hash.
struct Originals;

impl DerivedStorePort for Originals {
    fn open_original(&self, hash: &str) -> Result<Box<dyn Read + Send>, PortError> {
        Ok(Box::new(Cursor::new(
            format!("bytes of {hash}").into_bytes(),
        )))
    }

    fn store_derived(&self, _reader: &mut dyn Read) -> Result<StoredObject, PortError> {
        unreachable!("an export derives nothing")
    }
}

/// A folder holding the files written to it, by their path.
#[derive(Default)]
struct Folder {
    files: RefCell<BTreeMap<PathBuf, Vec<u8>>>,
}

impl ExportTargetPort for Folder {
    fn holds(&self, relative_path: &Path, byte_len: u64) -> Result<bool, PortError> {
        Ok(self
            .files
            .borrow()
            .get(relative_path)
            .is_some_and(|bytes| bytes.len() as u64 == byte_len))
    }

    fn write(&self, relative_path: &Path, reader: &mut dyn Read) -> Result<(), PortError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        self.files
            .borrow_mut()
            .insert(relative_path.to_path_buf(), bytes);
        Ok(())
    }
}

impl Folder {
    fn paths(&self) -> Vec<String> {
        self.files
            .borrow()
            .keys()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .collect()
    }
}

fn asset(
    asset_id: i64,
    asset_type: AssetType,
    hash: &str,
    filename: &str,
    media_type: &str,
) -> LibraryAsset {
    let byte_len = format!("bytes of {hash}").len() as u64;
    LibraryAsset {
        asset_id,
        asset_type,
        object_hash: hash.to_owned(),
        byte_len,
        media: MediaInfo {
            media_type: media_type.to_owned(),
            width: None,
            height: None,
            document: None,
        },
        original_filename: filename.to_owned(),
        provenance: Vec::new(),
        derived: Vec::new(),
    }
}

fn release(
    id: i64,
    title: &str,
    platform: &str,
    region: &str,
    assets: Vec<LibraryAsset>,
) -> LibraryEntry {
    LibraryEntry {
        game_id: id,
        game_title: title.to_owned(),
        release_edition_id: id,
        platform: platform.to_owned(),
        region: region.to_owned(),
        edition_name: "Standard".to_owned(),
        assertions: Vec::new(),
        assets,
    }
}

fn mario() -> LibraryEntry {
    let mut mario = release(
        1,
        "Super Mario Bros.",
        NES,
        "World",
        vec![
            asset(
                1,
                AssetType::BoxFront,
                "aaaa1111",
                "Super Mario Bros. (World).png",
                "image/png",
            ),
            // A map whose name has no extension gets the one of its media type.
            asset(2, AssetType::Map, "bbbb2222", "smb-1-1", "image/png"),
            asset(
                3,
                AssetType::WallpaperArtwork,
                "cccc3333",
                "art.jpg",
                "image/jpeg",
            ),
        ],
    );
    mario.assertions.push(ReleaseAssertion {
        source_id: "no-intro".into(),
        source_location: "https://raw.githubusercontent.com/nes.dat".to_owned(),
        field: ReleaseAssertionField::Identifier,
        qualifier: Some("source_record".to_owned()),
        value: format!("{}:{NES}Super Mario Bros. (World)", NES.len()),
    });
    mario
}

#[test]
fn every_original_is_copied_under_its_platform_game_and_type() {
    let folder = Folder::default();

    let summary = export_library(&Library(vec![mario()]), &Originals, &folder, &[]).unwrap();

    assert_eq!(
        summary,
        ExportSummary {
            exported: 3,
            already_exported: 0,
        }
    );
    assert_eq!(
        folder.paths(),
        [
            format!("{NES}/Super Mario Bros. (World)/Box Front/Super Mario Bros. (World).png"),
            format!("{NES}/Super Mario Bros. (World)/Map/smb-1-1.png"),
            format!("{NES}/Super Mario Bros. (World)/Wallpaper - Artwork/art.jpg"),
        ]
    );
    assert_eq!(
        folder.files.borrow()
            [&PathBuf::from(format!("{NES}/Super Mario Bros. (World)/Map/smb-1-1.png"))],
        b"bytes of bbbb2222"
    );
}

#[test]
fn names_windows_cannot_hold_are_made_safe_and_clashing_ones_told_apart() {
    let folder = Folder::default();
    let puzzle = release(
        2,
        "Bust-A-Move: Puzzle?",
        NES,
        "USA",
        vec![
            asset(
                1,
                AssetType::BoxFront,
                "dddd4444ffff",
                "CON.png",
                "image/png",
            ),
            asset(
                2,
                AssetType::BoxFront,
                "eeee5555ffff",
                "con.png",
                "image/png",
            ),
        ],
    );

    export_library(&Library(vec![puzzle]), &Originals, &folder, &[]).unwrap();

    assert_eq!(
        folder.paths(),
        [
            format!("{NES}/Bust-A-Move- Puzzle- (USA)/Box Front/_CON.png"),
            format!("{NES}/Bust-A-Move- Puzzle- (USA)/Box Front/_con (eeee5555).png"),
        ]
    );
}

#[test]
fn an_export_again_leaves_the_files_already_there_and_keeps_to_the_platforms_named() {
    let folder = Folder::default();
    let library = Library(vec![
        mario(),
        release(
            3,
            "Super Metroid",
            SNES,
            "USA",
            vec![asset(
                1,
                AssetType::BoxFront,
                "ffff6666",
                "sm.png",
                "image/png",
            )],
        ),
    ]);
    export_library(&library, &Originals, &folder, &[NES.to_owned()]).unwrap();

    let again = export_library(&library, &Originals, &folder, &[NES.to_owned()]).unwrap();

    assert_eq!(
        again,
        ExportSummary {
            exported: 0,
            already_exported: 3,
        }
    );
    assert!(folder.paths().iter().all(|path| path.starts_with(NES)));
}
