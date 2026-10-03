//! What a PDF document says of itself, read from the whole stored file since its cross-reference
//! table and Info dictionary sit past the prefix media inspection streams.

use std::{panic, path::Path};

use game_media_vault_domain::DocumentMetadata;

/// What the PDF at `path` says of itself, or `None` when its file cannot be parsed. Originals
/// come from Sources, so a parser failure, even a panic, only leaves the document undescribed.
pub(crate) fn document_metadata(path: &Path) -> Option<DocumentMetadata> {
    let metadata = panic::catch_unwind(|| lopdf::Document::load_metadata(path))
        .ok()?
        .ok()?;
    Some(DocumentMetadata {
        // A document has a page, so none read means its page tree was unreadable, as when it is
        // encrypted with a password.
        page_count: (metadata.page_count > 0).then_some(metadata.page_count),
        version: metadata.version,
        encrypted: metadata.encrypted,
        title: metadata.title,
        author: metadata.author,
        subject: metadata.subject,
        keywords: metadata.keywords,
        creator: metadata.creator,
        producer: metadata.producer,
        creation_date: metadata.creation_date,
        modification_date: metadata.modification_date,
    })
}
