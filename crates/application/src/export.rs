//! Copies of a vault's originals in a folder people browse, named after their platform, game
//! and Asset Type rather than their content hash.

use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
};

use game_media_vault_domain::{LibraryAsset, LibraryEntry};
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

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

/// The most bytes one name takes in UTF-8, which leaves room for the suffix telling clashing
/// names apart within the 255 bytes, or UTF-16 units, file systems hold per name.
const MAX_NAME_BYTES: usize = 240;

/// Copies every original of the vault, or of its releases of `platforms` when some are named,
/// to `<platform>/<game>/<Asset Type>/<file>` under the target, the game named as Sources name
/// its release, such as `Super Mario Bros. (World)`. Names Windows cannot hold are made safe, a
/// file name without an extension gets the one of its media type, and two originals one name
/// would stand for are each told apart by the start of their hash, whatever order the vault
/// lists them in. A file already there with the same size is left as it is, so exporting again
/// copies only what is new.
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
    let library = catalog.list_library()?;
    let exported: Vec<(&LibraryAsset, PathBuf)> = library
        .iter()
        .filter(|release| {
            platforms.is_empty()
                || platforms.iter().any(|platform| {
                    platform
                        .trim()
                        .eq_ignore_ascii_case(release.platform.trim())
                })
        })
        .flat_map(|release| {
            release
                .assets
                .iter()
                .map(move |asset| (asset, export_path(release, asset)))
        })
        .collect();
    let mut holders: HashMap<String, usize> = HashMap::new();
    for (_, path) in &exported {
        *holders.entry(path_key(path)).or_default() += 1;
    }
    for (asset, mut path) in exported {
        // Every original a name would stand for is told apart, so that no name depends on which
        // of them the vault lists first.
        if holders[&path_key(&path)] > 1 {
            path = told_apart(&path, asset);
        }
        if target.holds(&path, asset.byte_len)? {
            summary.already_exported += 1;
            continue;
        }
        let mut original = originals.open_original(&asset.object_hash)?;
        target.write(&path, &mut original)?;
        summary.exported += 1;
    }
    Ok(summary)
}

/// Where `asset` of `release` goes, before it is told apart from originals of the same name.
fn export_path(release: &LibraryEntry, asset: &LibraryAsset) -> PathBuf {
    PathBuf::from(safe_name(&release.platform))
        .join(safe_name(&release_name(release)))
        .join(safe_name(asset.asset_type.label()))
        .join(file_name(asset))
}

/// `path` with the start of the hash of `asset` after its file's stem.
fn told_apart(path: &Path, asset: &LibraryAsset) -> PathBuf {
    let file = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, extension) = split_extension(&file);
    let hash: String = asset.object_hash.chars().take(8).collect();
    let distinct = match extension {
        Some(extension) => format!("{stem} ({hash}).{extension}"),
        None => format!("{stem} ({hash})"),
    };
    path.with_file_name(distinct)
}

/// File systems that ignore case take two names differing in case for one; names are already
/// in one Unicode form, as file systems that normalize names take them.
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
        "image/heif" => "heif",
        "image/jxl" => "jxl",
        "image/qoi" => "qoi",
        "image/vnd.radiance" => "hdr",
        "image/x-exr" => "exr",
        "image/x-farbfeld" => "ff",
        "image/x-icon" => "ico",
        "image/x-ilbm" => "iff",
        "image/x-portable-anymap" => "pnm",
        "image/x-tga" => "tga",
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

/// `name` as one folder or file name every common file system holds, in Unicode's composed
/// form: characters Windows refuses become `-`, trailing dots and spaces go, a device name gets
/// a leading `_`, and a name too long in characters or bytes is shortened, keeping its
/// extension.
fn safe_name(name: &str) -> String {
    let replaced: String = name
        .nfc()
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
    if safe.chars().count() > MAX_NAME_CHARS || safe.len() > MAX_NAME_BYTES {
        let (stem, extension) = split_extension(&safe);
        let extension = extension
            .map(|extension| format!(".{extension}"))
            .unwrap_or_default();
        let mut kept = String::new();
        for character in stem.chars() {
            if kept.chars().count() + 1 + extension.len() > MAX_NAME_CHARS
                || kept.len() + character.len_utf8() + extension.len() > MAX_NAME_BYTES
            {
                break;
            }
            kept.push(character);
        }
        safe = format!("{}{extension}", kept.trim_end_matches(['.', ' ']));
    }
    safe
}
