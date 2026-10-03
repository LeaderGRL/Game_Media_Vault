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

/// A name compared regardless of case, punctuation and spacing.
pub(crate) fn name_key(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_alphanumeric() || character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
