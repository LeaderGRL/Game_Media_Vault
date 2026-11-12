//! The thumbnails `thumbnails.libretro.com` lists in each platform folder.
//!
//! The server RetroArch downloads thumbnails from mirrors every Libretro thumbnail repository
//! under its platform name, with an Apache directory listing per folder, so discovery can offer
//! only the thumbnails that exist instead of a download per name that Libretro may lack.

use std::collections::HashSet;

use game_media_vault_application::PortError;
use percent_encoding::percent_decode_str;
use scraper::{Html, Selector};
use url::Url;

const THUMBNAIL_SERVER: &str = "https://thumbnails.libretro.com/";

/// The listing of a platform's thumbnail folder, such as `Named_Boxarts`.
pub(crate) fn listing_url(platform: &str, folder: &str) -> Result<String, PortError> {
    let mut url = Url::parse(THUMBNAIL_SERVER)
        .map_err(|error| PortError::new(format!("invalid Libretro thumbnail server: {error}")))?;
    url.path_segments_mut()
        .map_err(|_| PortError::new("Libretro thumbnail server cannot hold a path".to_owned()))?
        // The empty last segment asks for the folder itself, as its listing links name.
        .extend([platform, folder, ""]);
    Ok(url.into())
}

/// The file names a directory listing links to, or `None` for a page that is no listing.
pub(crate) fn listed_files(page: &str) -> Option<HashSet<String>> {
    let document = Html::parse_document(page);
    let title = Selector::parse("title").expect("a valid selector");
    let links = Selector::parse("a[href]").expect("a valid selector");
    let is_listing = document.select(&title).any(|title| {
        title
            .text()
            .collect::<String>()
            .trim_start()
            .starts_with("Index of /")
    });
    if !is_listing {
        return None;
    }
    Some(
        document
            .select(&links)
            .filter_map(|link| link.value().attr("href"))
            // Sorting links, the parent folder and subfolders name no thumbnail.
            .filter(|href| !href.starts_with(['?', '/']) && !href.ends_with('/'))
            .filter_map(|href| percent_decode_str(href).decode_utf8().ok())
            .map(|name| name.into_owned())
            .collect(),
    )
}
