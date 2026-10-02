//! MAME Software Lists: one XML file per system (`hash/nes.xml`, say) describing every known
//! software package with its release metadata and the checksums of its dumps.

use std::{fs::File, io::BufReader, path::Path};

use game_media_vault_application::{PortError, ReferenceCatalogRead, ReferenceCatalogSourcePort};
use game_media_vault_domain::{
    ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField, SourceId,
};
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

use crate::{
    naming::{is_revision_tag, split_trailing_tags},
    xml::push_xml_reference,
};

pub const MAME_SOFTWARE_LISTS_SOURCE_ID: &str = "mame-software-lists";
const SOURCE_NAME: &str = "MAME software list";

/// Software lists by the platform name No-Intro and Redump give their system.
const LIST_PLATFORMS: &[(&str, &str)] = &[
    ("32x", "Sega - 32X"),
    ("a2600", "Atari - 2600"),
    ("a7800", "Atari - 7800"),
    ("coleco", "Coleco - ColecoVision"),
    ("dc", "Sega - Dreamcast"),
    ("gameboy", "Nintendo - Game Boy"),
    ("gamegear", "Sega - Game Gear"),
    ("gba", "Nintendo - Game Boy Advance"),
    ("gbcolor", "Nintendo - Game Boy Color"),
    ("genesis", "Sega - Mega Drive - Genesis"),
    ("intv", "Mattel - Intellivision"),
    ("lynx", "Atari - Lynx"),
    ("megacd", "Sega - Mega-CD - Sega CD"),
    ("megadriv", "Sega - Mega Drive - Genesis"),
    ("n64", "Nintendo - Nintendo 64"),
    ("nes", "Nintendo - Nintendo Entertainment System"),
    ("ngp", "SNK - Neo Geo Pocket"),
    ("ngpc", "SNK - Neo Geo Pocket Color"),
    ("pce", "NEC - PC Engine - TurboGrafx 16"),
    ("psx", "Sony - PlayStation"),
    ("saturn", "Sega - Saturn"),
    ("segacd", "Sega - Mega-CD - Sega CD"),
    ("sg1000", "Sega - SG-1000"),
    ("sms", "Sega - Master System - Mark III"),
    ("snes", "Nintendo - Super Nintendo Entertainment System"),
    ("tg16", "NEC - PC Engine - TurboGrafx 16"),
    ("vboy", "Nintendo - Virtual Boy"),
    ("wscolor", "Bandai - WonderSwan Color"),
    ("wswan", "Bandai - WonderSwan"),
];

/// Region abbreviations of software descriptions, by the name No-Intro gives the region.
const REGION_ABBREVIATIONS: &[(&str, &str)] = &[
    ("Aus", "Australia"),
    ("Bra", "Brazil"),
    ("Can", "Canada"),
    ("Chn", "China"),
    ("Den", "Denmark"),
    ("Euro", "Europe"),
    ("Fin", "Finland"),
    ("Fra", "France"),
    ("Ger", "Germany"),
    ("Hol", "Netherlands"),
    ("Ita", "Italy"),
    ("Jpn", "Japan"),
    ("Kor", "Korea"),
    ("Nor", "Norway"),
    ("Por", "Portugal"),
    ("Rus", "Russia"),
    ("Spa", "Spain"),
    ("Swe", "Sweden"),
    ("Tai", "Taiwan"),
];

/// Regions descriptions already spell as No-Intro does.
const REGION_NAMES: &[&str] = &[
    "Asia",
    "Australia",
    "Brazil",
    "Canada",
    "China",
    "Europe",
    "France",
    "Germany",
    "Italy",
    "Japan",
    "Korea",
    "Netherlands",
    "Spain",
    "Sweden",
    "Taiwan",
    "UK",
    "USA",
    "World",
];

/// The `info` entries kept as identifiers: package codes, release dates and alternate names.
const KEPT_INFO: &[&str] = &[
    "alt_title",
    "barcode",
    "developer",
    "language",
    "release",
    "serial",
    "version",
];

/// Reads MAME software lists as reference catalogs. The lists record no version of their own,
/// so the MAME release they came with can be given to identify which edition asserted a release.
#[derive(Debug, Default, Clone)]
pub struct MameSoftwareListCatalog {
    mame_version: Option<String>,
}

impl MameSoftwareListCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_mame_version(version: &str) -> Self {
        Self {
            mame_version: Some(version.trim().to_owned()).filter(|version| !version.is_empty()),
        }
    }
}

impl ReferenceCatalogSourcePort for MameSoftwareListCatalog {
    fn read_releases(
        &self,
        source_path: &Path,
        max_games: usize,
    ) -> Result<ReferenceCatalogRead, PortError> {
        if max_games == 0 {
            return Ok(ReferenceCatalogRead::default());
        }
        let file = File::open(source_path).map_err(|error| {
            PortError::new(format!(
                "failed to open {SOURCE_NAME} {}: {error}",
                source_path.display()
            ))
        })?;
        let location = source_path.to_string_lossy().into_owned();
        parse_list(
            BufReader::new(file),
            &location,
            self.mame_version.as_deref(),
            max_games,
        )
    }
}

/// The text element of a software entry being read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextField {
    Description,
    Year,
    Publisher,
}

#[derive(Default)]
struct SoftwareEntry {
    name: Option<String>,
    clone_of: Option<String>,
    description: Option<String>,
    year: Option<String>,
    publisher: Option<String>,
    identifiers: Vec<(String, String)>,
    malformed: bool,
}

fn parse_list<R: std::io::BufRead>(
    reader: R,
    location: &str,
    mame_version: Option<&str>,
    max_games: usize,
) -> Result<ReferenceCatalogRead, PortError> {
    let mut xml = Reader::from_reader(reader);
    let mut buffer = Vec::new();
    let mut list: Option<(String, String)> = None;
    let mut current: Option<SoftwareEntry> = None;
    let mut field: Option<TextField> = None;
    let mut text = String::new();
    let mut read = ReferenceCatalogRead::default();
    // Elements opened and not closed yet: a list must close them all.
    let mut depth = 0_usize;

    loop {
        let event = xml.read_event_into(&mut buffer).map_err(xml_error)?;
        match &event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        match event {
            Event::Start(element) => match element.name().as_ref() {
                "softwarelist" => {
                    // The list name keeps the identities of its records apart from other lists.
                    let name = attribute(&element, "name")?
                        .map(|name| name.trim().to_owned())
                        .filter(|name| !name.is_empty())
                        .ok_or_else(|| {
                            PortError::invalid_source_data(format!(
                                "{SOURCE_NAME} has no name to identify its records"
                            ))
                        })?;
                    let description = attribute(&element, "description")?.unwrap_or_default();
                    list = Some((name, description));
                }
                "software" => current = Some(start_software(&element)),
                "description" | "year" | "publisher" if current.is_some() => {
                    field = Some(match element.name().as_ref() {
                        "description" => TextField::Description,
                        "year" => TextField::Year,
                        _ => TextField::Publisher,
                    });
                    text.clear();
                }
                name => read_package_element(&mut current, name, &element),
            },
            Event::Empty(element) => {
                read_package_element(&mut current, element.name().as_ref(), &element)
            }
            Event::Text(content) if field.is_some() => {
                text.push_str(content.xml10_content().as_ref());
            }
            Event::GeneralRef(reference) if field.is_some() => {
                push_xml_reference(&mut text, &reference, SOURCE_NAME)?;
            }
            Event::End(element) => match element.name().as_ref() {
                "description" | "year" | "publisher" if field.is_some() => {
                    let value = text.trim().to_owned();
                    if let Some(entry) = current.as_mut() {
                        let slot = match field {
                            Some(TextField::Description) => &mut entry.description,
                            Some(TextField::Year) => &mut entry.year,
                            _ => &mut entry.publisher,
                        };
                        *slot = Some(value).filter(|value| !value.is_empty());
                    }
                    field = None;
                }
                "software" => {
                    let Some(entry) = current.take() else {
                        return Err(PortError::invalid_source_data(format!(
                            "{SOURCE_NAME} software closing tag has no matching entry"
                        )));
                    };
                    let (list_name, list_description) = list.as_ref().ok_or_else(missing_list)?;
                    // Entries past the bound are still parsed, so the whole list is checked.
                    if read.releases.len() < max_games {
                        match finish_software(
                            entry,
                            list_name,
                            list_description,
                            location,
                            mame_version,
                        ) {
                            Some(release) => read.releases.push(release),
                            None => read.skipped_records += 1,
                        }
                    }
                }
                _ => {}
            },
            // A list cut short, inside an entry or between two, is broken as a whole.
            Event::Eof if depth > 0 => {
                return Err(PortError::invalid_source_data(format!(
                    "{SOURCE_NAME} ends before its elements close"
                )));
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    if list.is_none() {
        return Err(missing_list());
    }
    Ok(read)
}

fn missing_list() -> PortError {
    PortError::invalid_source_data(format!("{SOURCE_NAME} has no softwarelist element"))
}

fn start_software(element: &BytesStart<'_>) -> SoftwareEntry {
    match (attribute(element, "name"), attribute(element, "cloneof")) {
        (Ok(name), Ok(clone_of)) => SoftwareEntry {
            name: name.filter(|name| !name.trim().is_empty()),
            clone_of,
            ..SoftwareEntry::default()
        },
        _ => SoftwareEntry {
            malformed: true,
            ..SoftwareEntry::default()
        },
    }
}

/// Records what an `info`, `rom` or `disk` element of the entry being read identifies; one whose
/// attributes cannot be read makes the entry skipped.
fn read_package_element(current: &mut Option<SoftwareEntry>, name: &str, element: &BytesStart<'_>) {
    let Some(entry) = current.as_mut() else {
        return;
    };
    let read = match name {
        "info" => read_info(element),
        "rom" => read_dump(element, "rom_name"),
        "disk" => read_dump(element, "disk_name"),
        _ => return,
    };
    match read {
        Ok(identifiers) => entry.identifiers.extend(identifiers),
        Err(_) => entry.malformed = true,
    }
}

fn read_info(element: &BytesStart<'_>) -> Result<Vec<(String, String)>, PortError> {
    let name = attribute(element, "name")?;
    let value = attribute(element, "value")?;
    Ok(match (name, value) {
        (Some(name), Some(value))
            if KEPT_INFO.contains(&name.as_str()) && !value.trim().is_empty() =>
        {
            vec![(name, value)]
        }
        _ => Vec::new(),
    })
}

fn read_dump(
    element: &BytesStart<'_>,
    name_qualifier: &str,
) -> Result<Vec<(String, String)>, PortError> {
    let mut identifiers = Vec::new();
    for (attribute_name, qualifier) in [("name", name_qualifier), ("crc", "crc"), ("sha1", "sha1")]
    {
        // A blank value identifies nothing.
        if let Some(value) =
            attribute(element, attribute_name)?.filter(|value| !value.trim().is_empty())
        {
            identifiers.push((qualifier.to_owned(), value));
        }
    }
    Ok(identifiers)
}

/// The release `entry` describes, or `None` when it lacks the short name or description that
/// identify one.
fn finish_software(
    entry: SoftwareEntry,
    list_name: &str,
    list_description: &str,
    location: &str,
    mame_version: Option<&str>,
) -> Option<ReferenceReleaseRecord> {
    if entry.malformed {
        return None;
    }
    let name = entry.name?;
    let description = entry.description?;
    // A list this importer does not name is placed by its description, else by its name.
    let platform = LIST_PLATFORMS
        .iter()
        .find(|(list, _)| *list == list_name)
        .map(|(_, platform)| (*platform).to_owned())
        .unwrap_or_else(|| {
            let description = list_description.trim();
            if description.is_empty() {
                list_name.to_owned()
            } else {
                description.to_owned()
            }
        });
    let release = mame_release_name(&description);
    let assertion = |field, qualifier: Option<&str>, value: &str| ReleaseAssertion {
        source_id: SourceId::from(MAME_SOFTWARE_LISTS_SOURCE_ID),
        source_location: location.to_owned(),
        field,
        qualifier: qualifier.map(str::to_owned),
        value: value.to_owned(),
    };
    let identifier = |qualifier: &str, value: &str| {
        assertion(ReleaseAssertionField::Identifier, Some(qualifier), value)
    };

    let mut assertions = vec![
        assertion(ReleaseAssertionField::Title, None, &release.game_title),
        identifier(
            "source_record",
            &format!("{}:{list_name}{name}", list_name.len()),
        ),
    ];
    if release.region != "Unknown" {
        assertions.push(assertion(
            ReleaseAssertionField::Region,
            None,
            &release.region,
        ));
    }
    if let Some(revision) = release.revision.as_deref() {
        assertions.push(assertion(ReleaseAssertionField::Revision, None, revision));
    }
    if let Some(version) = mame_version {
        assertions.push(identifier("mame_version", version));
    }
    assertions.push(identifier("software_list", list_name));
    assertions.push(identifier("software", &name));
    if let Some(parent) = entry.clone_of.as_deref() {
        assertions.push(identifier("clone_of", parent));
    }
    if let Some(year) = entry.year.as_deref() {
        assertions.push(identifier("year", year));
    }
    if let Some(publisher) = entry.publisher.as_deref() {
        assertions.push(identifier("publisher", publisher));
    }
    assertions.extend(
        entry
            .identifiers
            .iter()
            .map(|(qualifier, value)| identifier(qualifier, value)),
    );

    Some(ReferenceReleaseRecord {
        game_title: release.game_title,
        platform,
        region: release.region,
        revision: release.revision,
        edition_name: release.edition_name,
        assertions,
    })
}

struct MameReleaseName {
    game_title: String,
    region: String,
    revision: Option<String>,
    edition_name: String,
}

/// Reads a software description such as `Zelda no Densetsu (Jpn, Rev. A)`: the first tag lists
/// its regions, abbreviated, with other qualifiers such as its revision.
fn mame_release_name(description: &str) -> MameReleaseName {
    let (game_title, tags) = split_trailing_tags(description);
    let mut regions = Vec::new();
    let mut revision = None;
    let mut edition = Vec::new();
    for (index, tag) in tags.iter().enumerate() {
        for part in tag
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            if let Some(region) = (index == 0).then(|| mame_region(part)).flatten() {
                regions.push(region);
            } else if let Some(rest) = part
                .strip_prefix("Rev. ")
                .or_else(|| part.strip_prefix("Rev "))
            {
                let name = format!("Rev {rest}");
                revision = Some(name.clone());
                edition.push(name);
            } else if is_revision_tag(part) {
                revision = Some(part.to_owned());
                edition.push(part.to_owned());
            } else {
                edition.push(part.to_owned());
            }
        }
    }
    MameReleaseName {
        game_title,
        region: if regions.is_empty() {
            "Unknown".to_owned()
        } else {
            regions.join(", ")
        },
        revision,
        edition_name: if edition.is_empty() {
            "Standard".to_owned()
        } else {
            edition.join(" · ")
        },
    }
}

fn mame_region(part: &str) -> Option<String> {
    REGION_ABBREVIATIONS
        .iter()
        .find(|(abbreviation, _)| *abbreviation == part)
        .map(|(_, region)| (*region).to_owned())
        .or_else(|| {
            let spelled_out = REGION_NAMES.contains(&part)
                || REGION_ABBREVIATIONS
                    .iter()
                    .any(|(_, region)| *region == part);
            spelled_out.then(|| part.to_owned())
        })
}

fn attribute(element: &BytesStart<'_>, name: &str) -> Result<Option<String>, PortError> {
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| {
            PortError::invalid_source_data(format!("invalid {SOURCE_NAME} XML attribute: {error}"))
        })?;
        if attribute.key.as_ref() == name {
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| {
                    PortError::invalid_source_data(format!(
                        "invalid {SOURCE_NAME} XML attribute value: {error}"
                    ))
                })?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

/// A list that cannot be read is an environmental failure; one that does not parse is invalid
/// source data.
fn xml_error(error: quick_xml::Error) -> PortError {
    match error {
        quick_xml::Error::Io(error) => {
            PortError::new(format!("failed to read {SOURCE_NAME}: {error}"))
        }
        error => PortError::invalid_source_data(format!("invalid {SOURCE_NAME} XML: {error}")),
    }
}
