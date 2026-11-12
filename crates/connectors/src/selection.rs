//! The games an explicit request names, as connectors that look games up by name read them.

use game_media_vault_domain::{AcquisitionRequest, GameSelection};

use crate::naming::parse_release_name;

/// The games the request names, by title without release tags, with the platform each is
/// requested on.
pub(crate) fn wanted_games(request: &AcquisitionRequest) -> Vec<(String, String)> {
    let title = |game: &str| parse_release_name(game).game_title;
    let mut wanted: Vec<(String, String)> = Vec::new();
    let mut want = |game: &str, platform: &str| {
        let pair = (title(game), platform.to_owned());
        if !wanted.contains(&pair) {
            wanted.push(pair);
        }
    };
    match request.games() {
        GameSelection::All => {}
        GameSelection::Explicit(games) => {
            for platform in request.platforms() {
                for game in games {
                    want(game, platform);
                }
            }
        }
        GameSelection::PlatformBound(games) | GameSelection::QueryResult(games) => {
            // Requested platforms, when there are any, narrow the bound games.
            for game in games.iter().filter(|game| {
                request.platforms().is_empty()
                    || request
                        .platforms()
                        .iter()
                        .any(|platform| platform == &game.platform)
            }) {
                want(&game.game, &game.platform);
            }
        }
    }
    wanted
}

/// A name compared regardless of case, punctuation and spacing, so that `Spider-Man`,
/// `Spider Man` and `spiderman` are one name.
pub(crate) fn name_key(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A platform as the provider candidate ids of media that apply to several platforms name it,
/// regardless of how a request spells it, so that one candidate keeps one identity.
pub(crate) fn platform_key(platform: &str) -> String {
    name_key(platform)
}

/// The platforms a request names, or else those of the games it binds.
pub(crate) fn requested_platforms(request: &AcquisitionRequest) -> Vec<&str> {
    match request.games() {
        GameSelection::PlatformBound(games) | GameSelection::QueryResult(games)
            if request.platforms().is_empty() =>
        {
            games.iter().map(|game| game.platform.as_str()).collect()
        }
        _ => request.platforms().iter().map(String::as_str).collect(),
    }
}

/// The articles a title may file after its main title, as No-Intro does.
pub(crate) const ARTICLES: [&str; 3] = ["the", "a", "an"];

/// Compares titles regardless of case, spacing, subtitle separator (No-Intro writes " - ",
/// LaunchBox ": ") and article placement: No-Intro moves a leading article after the main
/// title ("Legend of Zelda, The - A Link to the Past").
pub(crate) fn title_key(title: &str) -> String {
    let words = title.split_whitespace().collect::<Vec<_>>().join(" ");
    words
        .to_lowercase()
        .replace(": ", " - ")
        .split(" - ")
        .map(|part| {
            ARTICLES
                .iter()
                .find_map(|article| {
                    part.strip_suffix(&format!(", {article}"))
                        .map(|rest| format!("{article} {rest}"))
                })
                .unwrap_or_else(|| part.to_owned())
        })
        .collect::<Vec<_>>()
        .join(": ")
}
