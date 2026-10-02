//! Logiqx datafiles, as No-Intro and Redump publish them: a header naming the platform and the
//! datafile version, then one game entry per release with the identifiers of its ROMs or tracks.

use std::{fs::File, io::BufReader, path::Path};

use game_media_vault_application::PortError;
use game_media_vault_domain::{
    ReferenceReleaseRecord, ReleaseAssertion, ReleaseAssertionField, SourceId,
};
use quick_xml::{Reader, escape::resolve_xml_entity, events::Event};

use crate::naming::{parse_release_name, platform_name};

/// The Source a datafile comes from, which every assertion it yields names.
pub(crate) struct DatafileSource {
    pub(crate) id: &'static str,
    /// How messages name the Source.
    pub(crate) name: &'static str,
}

/// Reads up to `max_games` releases from the datafile at `path`.
pub(crate) fn read_datafile(
    source: &DatafileSource,
    path: &Path,
    max_games: usize,
) -> Result<Vec<ReferenceReleaseRecord>, PortError> {
    if max_games == 0 {
        return Ok(Vec::new());
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

fn parse_datafile<R: std::io::BufRead>(
    source: &DatafileSource,
    reader: R,
    source_location: &str,
    max_games: usize,
) -> Result<Vec<ReferenceReleaseRecord>, PortError> {
    let mut xml = Reader::from_reader(reader);
    let mut buffer = Vec::new();
    let mut platform = None;
    let mut version = None;
    let mut header_text = String::new();
    let mut in_header = false;
    let mut header_field = None;
    let mut current_game = None;
    let mut releases = Vec::with_capacity(max_games.min(256));

    loop {
        match xml
            .read_event_into(&mut buffer)
            .map_err(|error| xml_error(source, error))?
        {
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
                "game" => {
                    let raw_name = attribute_value(source, &element, "name")?.ok_or_else(|| {
                        PortError::invalid_source_data(format!(
                            "{} game entry is missing its name",
                            source.name
                        ))
                    })?;
                    current_game = Some(DatafileGame::new(raw_name, source_location));
                }
                "rom" => {
                    if let Some(game) = current_game.as_mut() {
                        game.read_identifiers(source, &element)?;
                    }
                }
                _ => {}
            },
            Event::Empty(element) if element.name().as_ref() == "rom" => {
                if let Some(game) = current_game.as_mut() {
                    game.read_identifiers(source, &element)?;
                }
            }
            Event::Text(text) if header_field.is_some() => {
                header_text.push_str(text.xml10_content().as_ref());
            }
            Event::GeneralRef(reference) if header_field.is_some() => {
                if let Some(character) = reference.resolve_char_ref().map_err(|error| {
                    PortError::invalid_source_data(format!(
                        "invalid {} XML character reference: {error}",
                        source.name
                    ))
                })? {
                    header_text.push(character);
                } else if let Some(value) = resolve_xml_entity(reference.as_ref()) {
                    header_text.push_str(value);
                } else {
                    return Err(PortError::invalid_source_data(format!(
                        "unsupported {} XML entity reference: &{};",
                        source.name,
                        reference.as_ref()
                    )));
                }
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
                    let game = current_game.take().ok_or_else(|| {
                        PortError::invalid_source_data(format!(
                            "{} game closing tag has no matching entry",
                            source.name
                        ))
                    })?;
                    let platform = platform
                        .as_deref()
                        .ok_or_else(|| missing_platform(source))?;
                    releases.push(game.finish(source, platform, version.as_deref()));
                    if releases.len() >= max_games {
                        break;
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    if platform.is_none() {
        return Err(missing_platform(source));
    }
    Ok(releases)
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
