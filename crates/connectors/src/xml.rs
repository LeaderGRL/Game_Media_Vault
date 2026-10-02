//! XML helpers shared by the readers of Source documents.

use game_media_vault_application::PortError;
use quick_xml::{escape::resolve_xml_entity, events::BytesRef};

/// Appends the character or predefined entity `reference` stands for in a `source` document.
pub(crate) fn push_xml_reference(
    target: &mut String,
    reference: &BytesRef<'_>,
    source: &str,
) -> Result<(), PortError> {
    if let Some(character) = reference.resolve_char_ref().map_err(|error| {
        PortError::invalid_source_data(format!("invalid {source} XML character reference: {error}"))
    })? {
        target.push(character);
    } else if let Some(value) = resolve_xml_entity(reference.as_ref()) {
        target.push_str(value);
    } else {
        return Err(PortError::invalid_source_data(format!(
            "unsupported {source} XML entity reference: &{};",
            reference.as_ref()
        )));
    }
    Ok(())
}
