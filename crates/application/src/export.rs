//! Copies of a vault's originals in a folder people browse, named after their platform, game
//! and Asset Type rather than their content hash.

use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
};

use game_media_vault_domain::{LibraryAsset, LibraryEntry};
use serde::Serialize;

use crate::{ApplicationError, CatalogPort, DerivedStorePort, PortError, platforms::release_name};

/// The folder an export writes copies of originals to.
pub trait ExportTargetPort {
    /// Whether `relative_path` already holds a file of `byte_len` bytes, which an export leaves
    /// as it is.
    fn holds(&self, relative_path: &Path, byte_len: u64) -> Result<bool, PortError>;

    /// Writes the bytes `reader` gives at `relative_path`, creating its folders, so that the file
    /// appears whole or not at all.
    fn write(&self, relative_path: &Path, reader: &mut dyn Read) -> Result<(), PortError>;
}

/// What an export copied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportSummary {
    pub exported: usize,
    /// Originals a file of the same name and size already stood for.
    pub already_exported: usize,
}

/// The longest name given to one folder or file, which keeps whole paths within what file
/// systems allow.
const MAX_NAME_CHARS: usize = 120;

/// Copies every original of the vault, or of its releases of `platforms` when some are named,
/// to `<platform>/<game>/<Asset Type>/<file>` under the target, the game named as Sources name
/// its release, such as `Super Mario Bros. (World)`. Names Windows cannot hold are made safe, a
/// file name without an extension gets the one of its media type, and two originals one name
/// would stand for are told apart by the start of their hash. A file already there with the
/// same size is left as it is, so exporting again copies only what is new.
pub fn export_library(
    catalog: &dyn CatalogPort,
    originals: &dyn DerivedStorePort,
    target: &dyn ExportTargetPort,
    platforms: &[String],
) -> Result<ExportSummary, ApplicationError> {
    let mut summary = ExportSummary {
        exported: 0,
        already_exported: 0,
    };
    let mut taken: HashSet<String> = HashSet::new();
    for release in catalog.list_library()?.iter().filter(|release| {
        platforms.is_empty()
            || platforms.iter().any(|platform| {
                platform
                    .trim()
                    .eq_ignore_ascii_case(release.platform.trim())
            })
    }) {
        for asset in &release.assets {
            let path = export_path(release, asset, &mut taken);
            if target.holds(&path, asset.byte_len)? {
                summary.already_exported += 1;
                continue;
            }
            let mut original = originals.open_original(&asset.object_hash)?;
            target.write(&path, &mut original)?;
            summary.exported += 1;
        }
    }
    Ok(summary)
}

/// Where `asset` of `release` goes, telling it apart from the paths already `taken`.
fn export_path(
    release: &LibraryEntry,
    asset: &LibraryAsset,
    taken: &mut HashSet<String>,
) -> PathBuf {
    let folder = PathBuf::from(safe_name(&release.platform))
        .join(safe_name(&release_name(release)))
        .join(safe_name(asset.asset_type.label()));
    let file = file_name(asset);
    let mut path = folder.join(&file);
    if !taken.insert(path_key(&path)) {
        let (stem, extension) = split_extension(&file);
        let hash: String = asset.object_hash.chars().take(8).collect();
        let distinct = match extension {
            Some(extension) => format!("{stem} ({hash}).{extension}"),
            None => format!("{stem} ({hash})"),
        };
        path = folder.join(distinct);
        taken.insert(path_key(&path));
    }
    path
}

/// File systems that ignore case take two names differing in case for one.
fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// The original's name made safe, with the extension of its media type when it has none.
fn file_name(asset: &LibraryAsset) -> String {
    let name = safe_name(&asset.original_filename);
    match (
        split_extension(&name).1,
        extension_of(&asset.media.media_type),
    ) {
        (None, Some(extension)) => format!("{name}.{extension}"),
        _ => name,
    }
}

/// A name's stem and extension, when it has one that looks like one.
fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('.') {
        Some((stem, extension))
            if !stem.is_empty()
                && (1..=5).contains(&extension.len())
                && extension.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            (stem, Some(extension))
        }
        _ => (name, None),
    }
}

/// The usual extension of files of `media_type`.
fn extension_of(media_type: &str) -> Option<&'static str> {
    Some(match media_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/tiff" => "tif",
        "image/avif" => "avif",
        "image/heic" => "heic",
        "image/jxl" => "jxl",
        "image/x-icon" => "ico",
        "application/pdf" => "pdf",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "model/gltf-binary" => "glb",
        _ => return None,
    })
}

/// Names Windows reserves for devices, whatever their extension.
const RESERVED_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// `name` as one folder or file name every common file system holds: characters Windows refuses
/// become `-`, trailing dots and spaces go, a device name gets a leading `_`, and an overlong
/// name is shortened, keeping its extension.
fn safe_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            character if character.is_control() => '-',
            character => character,
        })
        .collect();
    let mut safe = replaced.trim().trim_end_matches(['.', ' ']).to_owned();
    if safe.is_empty() {
        safe = "_".to_owned();
    }
    let stem = safe
        .split('.')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        safe.insert(0, '_');
    }
    if safe.chars().count() > MAX_NAME_CHARS {
        let (stem, extension) = split_extension(&safe);
        let extension = extension
            .map(|extension| format!(".{extension}"))
            .unwrap_or_default();
        let kept: String = stem
            .chars()
            .take(MAX_NAME_CHARS - extension.chars().count())
            .collect();
        safe = format!("{}{extension}", kept.trim_end_matches(['.', ' ']));
    }
    safe
}
