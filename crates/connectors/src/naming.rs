//! Release naming conventions shared by No-Intro style catalogs and connectors, such as
//! `Tetris (World) (Rev 1)`: a title followed by parenthesized region and edition tags.

pub(crate) struct ReleaseName {
    pub(crate) game_title: String,
    pub(crate) region: String,
    pub(crate) revision: Option<String>,
    pub(crate) edition_name: String,
}

pub(crate) fn parse_release_name(raw: &str) -> ReleaseName {
    let (game_title, tags) = split_trailing_tags(raw);
    let revision = tags.iter().find(|tag| is_revision_tag(tag)).cloned();
    let region_index = tags
        .first()
        .filter(|tag| is_region_candidate(tag))
        .map(|_| 0);
    let region = region_index
        .map(|index| tags[index].clone())
        .unwrap_or_else(|| "Unknown".to_owned());
    let edition_tags = tags
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != region_index)
        .map(|(_, tag)| tag)
        .cloned()
        .collect::<Vec<_>>();
    let edition_name = if edition_tags.is_empty() {
        "Standard".to_owned()
    } else {
        edition_tags.join(" · ")
    };

    ReleaseName {
        game_title,
        region,
        revision,
        edition_name,
    }
}

fn split_trailing_tags(raw: &str) -> (String, Vec<String>) {
    let mut base = raw.trim_end();
    let mut tags = Vec::new();
    while base.ends_with(')') {
        let Some(open_index) = base.rfind(" (") else {
            break;
        };
        let tag = &base[open_index + 2..base.len() - 1];
        if tag.is_empty() {
            break;
        }
        tags.push(tag.to_owned());
        base = base[..open_index].trim_end();
    }
    tags.reverse();
    (base.to_owned(), tags)
}

fn is_region_candidate(tag: &str) -> bool {
    !is_revision_tag(tag)
        && !is_status_tag(tag)
        && !is_language_tag(tag)
        && !is_date_tag(tag)
        && !is_edition_tag(tag)
}

fn is_revision_tag(tag: &str) -> bool {
    tag.starts_with("Rev ")
        || tag.starts_with("Revision ")
        || tag.strip_prefix('v').is_some_and(is_version_number)
        || tag.strip_prefix("Version ").is_some_and(is_version_number)
}

fn is_version_number(value: &str) -> bool {
    value
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_digit())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_'))
}

fn is_status_tag(tag: &str) -> bool {
    const STATUS_MARKERS: &[&str] = &[
        "Alpha",
        "Beta",
        "Demo",
        "Kiosk",
        "Preview",
        "Promo",
        "Proto",
        "Prototype",
        "Sample",
        "Test",
        "Debug",
        "Pre-Release",
        "Prerelease",
        "Unl",
        "Unlicensed",
        "Pirate",
        "Aftermarket",
        "Homebrew",
    ];

    STATUS_MARKERS.iter().any(|marker| {
        tag == *marker
            || tag
                .strip_prefix(marker)
                .is_some_and(|suffix| suffix.starts_with(' ') || suffix.starts_with('-'))
    })
}

fn is_language_tag(tag: &str) -> bool {
    let mut parts = tag.split(',').map(str::trim);
    let Some(first) = parts.next() else {
        return false;
    };
    is_language_code(first) && parts.all(is_language_code)
}

fn is_language_code(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(bytes.len(), 2 | 3)
        && bytes[0].is_ascii_uppercase()
        && bytes[1..].iter().all(|byte| byte.is_ascii_lowercase())
}

fn is_date_tag(tag: &str) -> bool {
    let bytes = tag.as_bytes();
    bytes.len() >= 5 && bytes[..4].iter().all(|byte| byte.is_ascii_digit()) && bytes[4] == b'-'
}

fn is_edition_tag(tag: &str) -> bool {
    ["Edition", "Bundle", "Pack", "Disc", "Disk", "Side", "Alt"]
        .iter()
        .any(|marker| tag.contains(marker))
}

/// Qualifiers that describe how a DAT stores dumps rather than which platform it covers.
const DAT_VARIANT_QUALIFIERS: &[&str] = &[
    "Headered",
    "Headerless",
    "Decrypted",
    "Encrypted",
    "BigEndian",
    "ByteSwapped",
    "LittleEndian",
    "Parent-Clone",
];

/// Platform name without DAT variant qualifiers, e.g.
/// `Nintendo - Nintendo Entertainment System (Headered)` becomes
/// `Nintendo - Nintendo Entertainment System`, as Libretro names it.
pub(crate) fn platform_name(dat_name: &str) -> String {
    let mut name = dat_name.trim();
    while let Some(stripped) = DAT_VARIANT_QUALIFIERS
        .iter()
        .find_map(|qualifier| name.strip_suffix(&format!(" ({qualifier})")))
    {
        name = stripped.trim_end();
    }
    name.to_owned()
}
