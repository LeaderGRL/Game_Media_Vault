//! libretro-database: the No-Intro and Redump datafiles libretro republishes for each platform,
//! named after the platform as the catalog names it, in clrmamepro format. It gives every game
//! of a platform without the user importing a datafile.

use std::{
    collections::HashMap,
    io::Read,
    sync::{Mutex, PoisonError},
};

use game_media_vault_application::{PlatformCatalogSourcePort, PortError, ReferenceCatalogRead};
use serde_json::Value;
use url::Url;

use crate::{
    HttpTransport, NO_INTRO, REDUMP, ReqwestHttpTransport,
    datafile::{DatafileGame, DatafileSource},
    naming::platform_name,
    selection::name_key,
};

/// The directories of game lists, each with the catalog its datafiles come from, in the order
/// a platform is looked for.
const DIRECTORIES: [(&str, &DatafileSource); 2] = [("no-intro", &NO_INTRO), ("redump", &REDUMP)];

const CONTENTS_API: &str =
    "https://api.github.com/repos/libretro/libretro-database/contents/metadat/";

/// Where GitHub's raw host serves the game lists, by directory.
const RAW_LISTS: &str =
    "https://raw.githubusercontent.com/libretro/libretro-database/master/metadat/";

/// The only host game lists are downloaded from.
const RAW_HOST: &str = "raw.githubusercontent.com";

/// The largest game list trusted, which keeps a misbehaving answer from filling memory; the
/// largest lists are a few megabytes.
const MAX_LIST_BYTES: u64 = 64 * 1024 * 1024;

/// A game list a directory holds: the platform it is named after, and where it is downloaded.
#[derive(Clone)]
struct ListedFile {
    platform: String,
    location: Url,
}

/// Lists each directory once, through GitHub's contents API, then downloads the list of each
/// platform asked for from GitHub's raw host.
pub struct LibretroDatabase<T = ReqwestHttpTransport> {
    transport: T,
    listings: Mutex<HashMap<&'static str, Vec<ListedFile>>>,
}

impl LibretroDatabase<ReqwestHttpTransport> {
    pub fn new() -> Self {
        Self::with_transport(ReqwestHttpTransport::default())
    }
}

impl Default for LibretroDatabase<ReqwestHttpTransport> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LibretroDatabase<T> {
    pub fn with_transport(transport: T) -> Self {
        Self {
            transport,
            listings: Mutex::new(HashMap::new()),
        }
    }
}

impl<T: HttpTransport> PlatformCatalogSourcePort for LibretroDatabase<T> {
    /// The platform is the file named exactly after it, read without listing any directory, or
    /// else the file named after it with the same words, regardless of case and punctuation, as
    /// `NEC - PC Engine - TurboGrafx 16` for `NEC - PC Engine - TurboGrafx-16`.
    fn platform_releases(&self, platform: &str) -> Result<Option<ReferenceCatalogRead>, PortError> {
        for (directory, source) in DIRECTORIES {
            let file = ListedFile {
                platform: platform.to_owned(),
                location: raw_location(directory, platform),
            };
            match self.read(&file.location) {
                Ok(text) => return parse_game_list(&text, source, &file).map(Some),
                // No list of that exact name: another directory, or a listing, may hold it.
                Err(error) if error.is_unavailable() => {}
                Err(error) => return Err(error),
            }
        }
        let wanted = name_key(platform);
        for (directory, source) in DIRECTORIES {
            let Some(file) = self
                .listing(directory)?
                .into_iter()
                .find(|file| name_key(&file.platform) == wanted)
            else {
                continue;
            };
            let text = self.read(&file.location)?;
            return parse_game_list(&text, source, &file).map(Some);
        }
        Ok(None)
    }
}

impl<T: HttpTransport> LibretroDatabase<T> {
    /// The game lists `directory` holds, listed once.
    fn listing(&self, directory: &'static str) -> Result<Vec<ListedFile>, PortError> {
        let mut listings = self.listings.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(files) = listings.get(directory) {
            return Ok(files.clone());
        }
        let url = format!("{CONTENTS_API}{directory}");
        let body = self.transport.get_bytes(&url).map_err(|error| {
            // GitHub refuses listings once an address spends its hourly allowance.
            if ["HTTP 403", "HTTP 429"]
                .iter()
                .any(|status| error.message().contains(status))
            {
                PortError::new(format!(
                    "{}; GitHub allows 60 listings an hour to an address without an account, so try again later",
                    error.message()
                ))
            } else {
                error
            }
        })?;
        let answer: Value = serde_json::from_slice(&body).map_err(|error| {
            PortError::invalid_source_data(format!("GitHub answered {url} with no JSON: {error}"))
        })?;
        let Some(entries) = answer.as_array() else {
            // GitHub explains a refusal, such as a spent rate limit, in a message.
            let message = answer
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no reason given");
            return Err(PortError::new(format!(
                "GitHub refused to list {url}: {message}"
            )));
        };
        let files: Vec<ListedFile> = entries.iter().filter_map(listed_file).collect();
        listings.insert(directory, files.clone());
        Ok(files)
    }

    fn read(&self, location: &Url) -> Result<String, PortError> {
        let mut bytes = Vec::new();
        self.transport
            .get_stream(location.as_str())?
            .take(MAX_LIST_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| PortError::new(format!("failed to read {location}: {error}")))?;
        if bytes.len() as u64 > MAX_LIST_BYTES {
            return Err(PortError::invalid_source_data(format!(
                "{location} exceeds {MAX_LIST_BYTES} bytes"
            )));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Where GitHub's raw host would serve the list of `directory` named exactly after `platform`.
fn raw_location(directory: &str, platform: &str) -> Url {
    let mut location = Url::parse(RAW_LISTS)
        .and_then(|lists| lists.join(&format!("{directory}/")))
        .expect("the raw host has a valid location");
    location
        .path_segments_mut()
        .expect("an https location has path segments")
        .pop_if_empty()
        .push(&format!("{platform}.dat"));
    location
}

/// The game list a listing entry names, when it is a datafile GitHub's raw host serves.
fn listed_file(entry: &Value) -> Option<ListedFile> {
    if entry.get("type").and_then(Value::as_str) != Some("file") {
        return None;
    }
    let platform = entry
        .get("name")?
        .as_str()?
        .strip_suffix(".dat")?
        .to_owned();
    let location = Url::parse(entry.get("download_url")?.as_str()?).ok()?;
    (location.scheme() == "https" && location.host_str() == Some(RAW_HOST))
        .then_some(ListedFile { platform, location })
}

/// The releases of a clrmamepro game list: a `clrmamepro` header naming the platform and the
/// list's version, then a `game` entry per dump of a release, which entries of the same name
/// merge into one release. An entry without a name is skipped and counted.
fn parse_game_list(
    text: &str,
    source: &DatafileSource,
    file: &ListedFile,
) -> Result<ReferenceCatalogRead, PortError> {
    let invalid = |reason: &str| {
        PortError::invalid_source_data(format!(
            "{} game list {} {reason}",
            source.name, file.location
        ))
    };
    let entries = parse_entries(text).map_err(|reason| invalid(&reason))?;
    let mut platform = file.platform.clone();
    let mut version = None;
    let mut games: Vec<DatafileGame> = Vec::new();
    let mut names: HashMap<String, usize> = HashMap::new();
    let mut skipped_records = 0;
    let location = file.location.to_string();
    for (kind, fields) in &entries {
        match kind.as_str() {
            "clrmamepro" => {
                if let Some(name) =
                    text_field(fields, "name").filter(|name| !name.trim().is_empty())
                {
                    // A variant the header qualifies the platform with, such as `(Headered)`,
                    // is no part of its name.
                    platform = platform_name(name);
                }
                version = text_field(fields, "version").map(str::to_owned);
            }
            "game" => {
                let Some(name) = text_field(fields, "name").filter(|name| !name.trim().is_empty())
                else {
                    skipped_records += 1;
                    continue;
                };
                let index = *names.entry(name.to_owned()).or_insert_with(|| {
                    games.push(DatafileGame::new(name.to_owned(), &location));
                    games.len() - 1
                });
                for dump in fields.iter().filter_map(|(key, value)| match value {
                    Field::Block(dump) if key == "rom" => Some(dump),
                    _ => None,
                }) {
                    let dump: Vec<(&str, &str)> = dump
                        .iter()
                        .filter_map(|(key, value)| match value {
                            Field::Text(text) => Some((key.as_str(), text.as_str())),
                            Field::Block(_) => None,
                        })
                        .collect();
                    games[index].add_dump(&dump);
                }
            }
            _ => {}
        }
    }
    Ok(ReferenceCatalogRead {
        releases: games
            .into_iter()
            .map(|game| game.finish(source, &platform, version.as_deref()))
            .collect(),
        skipped_records,
    })
}

/// The fields of a clrmamepro entry or block, each with its key.
type Fields = Vec<(String, Field)>;

/// A field of a clrmamepro entry: a word or quoted text, or a block of fields of its own.
enum Field {
    Text(String),
    Block(Fields),
}

fn text_field<'a>(fields: &'a [(String, Field)], wanted: &str) -> Option<&'a str> {
    fields.iter().find_map(|(key, value)| match value {
        Field::Text(text) if key == wanted => Some(text.as_str()),
        _ => None,
    })
}

enum Token {
    Open,
    Close,
    Word(String),
}

/// The words, quoted texts and parentheses of a clrmamepro file, in order.
fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut characters = text.chars().peekable();
    while let Some(&character) = characters.peek() {
        match character {
            '(' => {
                characters.next();
                tokens.push(Token::Open);
            }
            ')' => {
                characters.next();
                tokens.push(Token::Close);
            }
            '"' => {
                characters.next();
                let mut quoted = String::new();
                loop {
                    match characters.next() {
                        Some('"') => break,
                        Some(character) => quoted.push(character),
                        None => return Err("ends inside a quoted text".to_owned()),
                    }
                }
                tokens.push(Token::Word(quoted));
            }
            _ if character.is_whitespace() => {
                characters.next();
            }
            _ => {
                let mut word = String::new();
                while let Some(&character) = characters.peek() {
                    if character.is_whitespace() || matches!(character, '(' | ')' | '"') {
                        break;
                    }
                    word.push(character);
                    characters.next();
                }
                tokens.push(Token::Word(word));
            }
        }
    }
    Ok(tokens)
}

/// The top-level entries of a clrmamepro file, each its kind, as `game`, with its fields.
fn parse_entries(text: &str) -> Result<Vec<(String, Fields)>, String> {
    let mut tokens = tokens(text)?.into_iter();
    let mut entries = Vec::new();
    while let Some(token) = tokens.next() {
        let Token::Word(kind) = token else {
            return Err("has an entry without a kind".to_owned());
        };
        if !matches!(tokens.next(), Some(Token::Open)) {
            return Err(format!("has a {kind} entry without its fields"));
        }
        entries.push((kind, parse_block(&mut tokens)?));
    }
    Ok(entries)
}

/// The fields of a block whose opening parenthesis was just read, up to its closing one.
fn parse_block(tokens: &mut impl Iterator<Item = Token>) -> Result<Fields, String> {
    let mut fields = Vec::new();
    loop {
        match tokens.next() {
            Some(Token::Close) => return Ok(fields),
            Some(Token::Word(key)) => {
                let value = match tokens.next() {
                    Some(Token::Word(text)) => Field::Text(text),
                    Some(Token::Open) => Field::Block(parse_block(tokens)?),
                    Some(Token::Close) | None => {
                        return Err(format!("has a field {key} without a value"));
                    }
                };
                fields.push((key, value));
            }
            Some(Token::Open) => return Err("has a block without a name".to_owned()),
            None => return Err("ends inside an entry".to_owned()),
        }
    }
}
