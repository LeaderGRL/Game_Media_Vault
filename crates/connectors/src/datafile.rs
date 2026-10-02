//! Logiqx datafiles, as No-Intro and Redump publish them: a header naming the platform and the
//! datafile version, then one game entry per release with the identifiers of its ROMs or tracks.

use std::{fs::File, io::BufReader, path::Path};

use game_media_vault_application::{PortError, ReferenceCatalogRead};
use game_media_vault_domain::{
    ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField, SourceId,
};
use quick_xml::{Reader, events::Event};

use crate::{
    naming::{parse_release_name, platform_name},
    xml::push_xml_reference,
};

/// The Source a datafile comes from, which every assertion it yields names.
pub(crate) struct DatafileSource {
    pub(crate) id: &'static str,
    /// How messages name the Source.
    pub(crate) name: &'static str,
}

/// Reads up to `max_games` releases from the datafile at `path`, skipping and counting game
/// entries too malformed to read.
pub(crate) fn read_datafile(
    source: &DatafileSource,
    path: &Path,
    max_games: usize,
) -> Result<ReferenceCatalogRead, PortError> {
    if max_games == 0 {
        return Ok(ReferenceCatalogRead::default());
    }
    let file = File::open(path).map_err(|error| {
        PortError::new(format!(
            "failed to open {} datafile {}: {error}",
            source.name,
            path.display()
        ))
    })?;
    let source_location = path.to_string_lossy().into_owned();
    parse_datafile(source, BufReader::new(file), &source_location, max_games)
}

/// The header fields a datafile records about itself.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HeaderField {
    Name,
    Version,
}

/// The game entry being read: one to import, or one skipped as malformed.
enum GameEntry {
    Reading(DatafileGame),
    Skipped,
}

fn parse_datafile<R: std::io::BufRead>(
    source: &DatafileSource,
    reader: R,
    source_location: &str,
    max_games: usize,
) -> Result<ReferenceCatalogRead, PortError> {
    let mut xml = Reader::from_reader(reader);
    let mut buffer = Vec::new();
    let mut platform = None;
    let mut version = None;
    let mut header_text = String::new();
    let mut in_header = false;
    let mut header_field = None;
    let mut current_game = None;
    let mut releases = Vec::with_capacity(max_games.min(256));
    let mut skipped_records = 0;
    // Elements opened and not closed yet: a datafile must close them all.
    let mut depth = 0_usize;

    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| xml_error(source, error))?;
        match &event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        match event {
            Event::Start(element) => match element.name().as_ref() {
                "header" => in_header = true,
                "name" if in_header => {
                    header_field = Some(HeaderField::Name);
                    header_text.clear();
                }
                "version" if in_header => {
                    header_field = Some(HeaderField::Version);
                    header_text.clear();
                }
                "game" => current_game = Some(start_game(source, &element, source_location)),
                "rom" => read_rom(source, &mut current_game, &element),
                _ => {}
            },
            Event::Empty(element) => match element.name().as_ref() {
                "rom" => read_rom(source, &mut current_game, &element),
                // A self-closing entry is complete: it names a release without dumps, or none.
                "game" => {
                    let entry = start_game(source, &element, source_location);
                    end_game(
                        source,
                        entry,
                        platform.as_deref(),
                        version.as_deref(),
                        &mut releases,
                        &mut skipped_records,
                        max_games,
                    )?;
                }
                _ => {}
            },
            Event::Text(text) if header_field.is_some() => {
                header_text.push_str(text.xml10_content().as_ref());
            }
            Event::GeneralRef(reference) if header_field.is_some() => {
                push_xml_reference(&mut header_text, &reference, source.name)?;
            }
            Event::End(element) => match element.name().as_ref() {
                "name" if header_field == Some(HeaderField::Name) => {
                    header_field = None;
                    platform = Some(platform_name(&header_text));
                }
                "version" if header_field == Some(HeaderField::Version) => {
                    header_field = None;
                    let text = header_text.trim();
                    version = (!text.is_empty()).then(|| text.to_owned());
                }
                "header" => in_header = false,
                "game" => {
                    let entry = current_game.take().ok_or_else(|| {
                        PortError::invalid_source_data(format!(
                            "{} game closing tag has no matching entry",
                            source.name
                        ))
                    })?;
                    end_game(
                        source,
                        entry,
                        platform.as_deref(),
                        version.as_deref(),
                        &mut releases,
                        &mut skipped_records,
                        max_games,
                    )?;
                }
                _ => {}
            },
            // A datafile cut short, inside an entry or between two, is broken as a whole.
            Event::Eof if depth > 0 => {
                return Err(PortError::invalid_source_data(format!(
                    "{} datafile ends before its elements close",
                    source.name
                )));
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    if platform.is_none() {
        return Err(missing_platform(source));
    }
    Ok(ReferenceCatalogRead {
        releases,
        skipped_records,
    })
}

/// The entry a `game` element starts: one without a readable name identifies no release, while
/// the others still do.
fn start_game(
    source: &DatafileSource,
    element: &quick_xml::events::BytesStart<'_>,
    source_location: &str,
) -> GameEntry {
    match attribute_value(source, element, "name") {
        Ok(Some(raw_name)) if !raw_name.trim().is_empty() => {
            GameEntry::Reading(DatafileGame::new(raw_name, source_location))
        }
        _ => GameEntry::Skipped,
    }
}

/// Ends `entry`: a readable one becomes a release while fewer than `max_games` are read, another
/// is counted as skipped. Entries past the bound are still parsed, so the whole datafile is
/// checked.
fn end_game(
    source: &DatafileSource,
    entry: GameEntry,
    platform: Option<&str>,
    version: Option<&str>,
    releases: &mut Vec<ReferenceReleaseRecord>,
    skipped_records: &mut usize,
    max_games: usize,
) -> Result<(), PortError> {
    if releases.len() >= max_games {
        return Ok(());
    }
    match entry {
        GameEntry::Reading(game) => {
            let platform = platform.ok_or_else(|| missing_platform(source))?;
            releases.push(game.finish(source, platform, version));
        }
        GameEntry::Skipped => *skipped_records += 1,
    }
    Ok(())
}

/// Records the identifiers of a ROM or track of the entry being read; one it cannot read makes
/// the entry skipped.
fn read_rom(
    source: &DatafileSource,
    current_game: &mut Option<GameEntry>,
    element: &quick_xml::events::BytesStart<'_>,
) {
    if let Some(GameEntry::Reading(game)) = current_game
        && game.read_identifiers(source, element).is_err()
    {
        *current_game = Some(GameEntry::Skipped);
    }
}

fn missing_platform(source: &DatafileSource) -> PortError {
    PortError::invalid_source_data(format!(
        "{} datafile header is missing a platform name",
        source.name
    ))
}

struct DatafileGame {
    raw_name: String,
    source_location: String,
    identifiers: Vec<(String, String)>,
}

impl DatafileGame {
    fn new(raw_name: String, source_location: &str) -> Self {
        Self {
            raw_name,
            source_location: source_location.to_owned(),
            identifiers: Vec::new(),
        }
    }

    fn read_identifiers(
        &mut self,
        source: &DatafileSource,
        element: &quick_xml::events::BytesStart<'_>,
    ) -> Result<(), PortError> {
        for name in ["name", "crc", "md5", "sha1", "sha256"] {
            if let Some(value) = attribute_value(source, element, name)? {
                let qualifier = if name == "name" {
                    "rom_name".to_owned()
                } else {
                    name.to_owned()
                };
                self.identifiers.push((qualifier, value));
            }
        }
        Ok(())
    }

    /// The release this entry describes. The datafile `version`, when the header records one,
    /// identifies which edition of the datafile asserted it.
    fn finish(
        self,
        source: &DatafileSource,
        platform: &str,
        version: Option<&str>,
    ) -> ReferenceReleaseRecord {
        let title = parse_release_name(&self.raw_name);
        let assertion = |field, qualifier: Option<&str>, value: &str| ReleaseAssertion {
            source_id: SourceId::from(source.id),
            source_location: self.source_location.clone(),
            field,
            qualifier: qualifier.map(str::to_owned),
            value: value.to_owned(),
        };
        let mut assertions = Vec::with_capacity(5 + self.identifiers.len());
        assertions.push(assertion(
            ReleaseAssertionField::Title,
            None,
            &title.game_title,
        ));
        assertions.push(assertion(
            ReleaseAssertionField::Identifier,
            Some("source_record"),
            &source_record_identifier(platform, &self.raw_name),
        ));
        if title.region != "Unknown" {
            assertions.push(assertion(
                ReleaseAssertionField::Region,
                None,
                &title.region,
            ));
        }
        if let Some(revision) = title.revision.as_deref() {
            assertions.push(assertion(ReleaseAssertionField::Revision, None, revision));
        }
        if let Some(version) = version {
            assertions.push(assertion(
                ReleaseAssertionField::Identifier,
                Some("dat_version"),
                version,
            ));
        }
        assertions.extend(self.identifiers.iter().map(|(qualifier, value)| {
            assertion(ReleaseAssertionField::Identifier, Some(qualifier), value)
        }));

        ReferenceReleaseRecord {
            game_title: title.game_title,
            platform: platform.to_owned(),
            region: title.region,
            revision: title.revision,
            edition_name: title.edition_name,
            assertions,
        }
    }
}

fn attribute_value(
    source: &DatafileSource,
    element: &quick_xml::events::BytesStart<'_>,
    name: &str,
) -> Result<Option<String>, PortError> {
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| {
            PortError::invalid_source_data(format!(
                "invalid {} XML attribute: {error}",
                source.name
            ))
        })?;
        if attribute.key.as_ref() == name {
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| {
                    PortError::invalid_source_data(format!(
                        "invalid {} XML attribute value: {error}",
                        source.name
                    ))
                })?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn source_record_identifier(platform: &str, raw_name: &str) -> String {
    format!("{}:{platform}{raw_name}", platform.len())
}

/// A datafile that cannot be read is an environmental failure; one that does not parse is
/// invalid source data.
fn xml_error(source: &DatafileSource, error: quick_xml::Error) -> PortError {
    match error {
        quick_xml::Error::Io(error) => {
            PortError::new(format!("failed to read {} datafile: {error}", source.name))
        }
        error => PortError::invalid_source_data(format!("invalid {} XML: {error}", source.name)),
    }
}
