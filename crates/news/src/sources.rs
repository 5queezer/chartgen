//! The sources the news is read from (ADR-0088): configuration, not code. A JSON file
//! named by `CHARTGEN_NEWS_SOURCES`, read again when it changes; else ADR-0086's
//! `CHARTGEN_NEWS_FEEDS`; else the list built in, `sources.json`.

use serde::Deserialize;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// The sources when neither variable is set, each vetted on 2026-11-25 (ADR-0088).
const BUILT_IN: &str = include_str!("../sources.json");

/// What a source is. Every kind is read as RSS or Atom; the kind says what its items are, so
/// a later reader can weigh them apart.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// An outlet's RSS feed.
    #[default]
    Rss,
    /// An outlet's Atom feed.
    Atom,
    /// A subreddit's feed (`/r/<name>/.rss`).
    Reddit,
    /// A YouTube channel's feed (`/feeds/videos.xml?channel_id=…`).
    Youtube,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Rss => "rss",
            Kind::Atom => "atom",
            Kind::Reddit => "reddit",
            Kind::Youtube => "youtube",
        }
    }
}

fn english() -> String {
    "en".into()
}

fn yes() -> bool {
    true
}

/// One source of headlines.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// Stable, as it is recorded with each headline: lower case letters, digits and `-`.
    pub id: String,
    /// What an answer calls it.
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub kind: Kind,
    /// Of its headlines, default `en`.
    #[serde(default = "english")]
    pub language: String,
    /// Switched off: neither read nor answered from.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// It mostly republishes others (wire copies, press releases).
    #[serde(default)]
    pub syndicates: bool,
    /// The only coins it is about, as tickers (a coin's subreddit); empty: any coin, as
    /// its titles say.
    #[serde(default)]
    pub coins: Vec<String>,
    /// In an answer that names no sources. When no enabled source is marked, all are.
    #[serde(default)]
    pub default: bool,
}

impl Default for Source {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            url: String::new(),
            kind: Kind::Rss,
            language: english(),
            enabled: true,
            syndicates: false,
            coins: Vec::new(),
            default: false,
        }
    }
}

/// Which sources an answer is from.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// Those marked `default` (all, when none is).
    Default,
    /// Every enabled source.
    All,
    /// These, by id.
    Ids(Vec<String>),
}

/// The sources as configured now. An error only when none could ever be read; a file that
/// stops parsing keeps the sources read from it before, and says why in the log once.
pub fn sources() -> Result<Arc<Vec<Source>>, String> {
    if cfg!(test) {
        return Ok(test_sources());
    }
    if let Some(path) = std::env::var_os("CHARTGEN_NEWS_SOURCES") {
        return from_file(&std::path::PathBuf::from(path));
    }
    static FIXED: OnceLock<Result<Arc<Vec<Source>>, String>> = OnceLock::new();
    FIXED
        .get_or_init(|| match std::env::var("CHARTGEN_NEWS_FEEDS") {
            Ok(list) => from_feeds(&list).map(Arc::new),
            Err(_) => parse(BUILT_IN).map(Arc::new),
        })
        .clone()
}

#[cfg(test)]
fn test_sources() -> Arc<Vec<Source>> {
    static TEST: OnceLock<Arc<Vec<Source>>> = OnceLock::new();
    TEST.get_or_init(|| Arc::new(crate::tests::sources()))
        .clone()
}

#[cfg(not(test))]
fn test_sources() -> Arc<Vec<Source>> {
    Arc::default()
}

/// The file's sources, read again when its modification time changes.
fn from_file(path: &std::path::Path) -> Result<Arc<Vec<Source>>, String> {
    /// When the file was last read, and the sources it last gave.
    type Held = Option<(Option<SystemTime>, Arc<Vec<Source>>)>;
    static HELD: Mutex<Held> = Mutex::new(None);
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some((at, sources)) = &*held {
        if *at == modified {
            return Ok(sources.clone());
        }
    }
    let read = std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|text| parse(&text));
    match (read, held.take()) {
        (Ok(sources), before) => {
            let sources = Arc::new(sources);
            if before.is_some() {
                eprintln!(
                    "[news] {} read again: {} sources",
                    path.display(),
                    sources.len()
                );
            }
            *held = Some((modified, sources.clone()));
            Ok(sources)
        }
        (Err(e), Some((_, sources))) => {
            eprintln!(
                "[news] {}: {e}; the sources read before stay",
                path.display()
            );
            *held = Some((modified, sources.clone()));
            Ok(sources)
        }
        (Err(e), None) => Err(format!("CHARTGEN_NEWS_SOURCES {}: {e}", path.display())),
    }
}

/// A list of sources in JSON, checked.
fn parse(text: &str) -> Result<Vec<Source>, String> {
    let mut sources: Vec<Source> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut ids = std::collections::HashSet::new();
    for s in &mut sources {
        let id_ok = !s.id.is_empty()
            && s.id != "all"
            && s.id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !id_ok {
            return Err(format!(
                "source id {:?} is not lower case letters, digits and -, or is \"all\"",
                s.id
            ));
        }
        if !ids.insert(s.id.clone()) {
            return Err(format!("source id {:?} is there twice", s.id));
        }
        if !url::Url::parse(&s.url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
            return Err(format!(
                "source {:?}: {:?} is not an http(s) URL",
                s.id, s.url
            ));
        }
        if s.name.trim().is_empty() || s.language.trim().is_empty() {
            return Err(format!("source {:?} needs a name and a language", s.id));
        }
        for coin in &mut s.coins {
            *coin = coin.trim().to_uppercase();
        }
    }
    Ok(sources)
}

/// ADR-0086's `CHARTGEN_NEWS_FEEDS`: entries separated by commas, each `Name=https://…` or a
/// bare URL, which is then named by its host; each an English RSS or Atom feed, its id its
/// name in lower case. Empty, no source.
fn from_feeds(list: &str) -> Result<Vec<Source>, String> {
    let mut sources = Vec::new();
    for entry in list.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let (name, url) = match entry.split_once('=') {
            Some((name, url)) if !name.contains("://") => (Some(name.trim()), url.trim()),
            _ => (None, entry),
        };
        let parsed = url::Url::parse(url)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https"))
            .ok_or_else(|| format!("CHARTGEN_NEWS_FEEDS: {entry:?} is not an http(s) URL"))?;
        let name = name
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .or_else(|| parsed.host_str().map(str::to_string))
            .unwrap_or_else(|| url.to_string());
        let id = name
            .to_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        sources.push(Source {
            id,
            name,
            url: url.to_string(),
            ..Source::default()
        });
    }
    Ok(sources)
}

/// The enabled sources `pick` names, of `language`. An error names an id that is not a
/// source or is switched off.
pub fn select<'a>(
    all: &'a [Source],
    pick: &Pick,
    language: &str,
) -> Result<Vec<&'a Source>, String> {
    let enabled = || all.iter().filter(|s| s.enabled);
    let picked: Vec<&Source> = match pick {
        Pick::All => enabled().collect(),
        Pick::Default => {
            let marked: Vec<_> = enabled().filter(|s| s.default).collect();
            match marked.is_empty() {
                true => enabled().collect(),
                false => marked,
            }
        }
        Pick::Ids(ids) => ids
            .iter()
            .map(|id| match all.iter().find(|s| &s.id == id) {
                Some(s) if s.enabled => Ok(s),
                Some(_) => Err(format!("source {id:?} is switched off")),
                None => Err(format!(
                    "{id:?} is not a source; the sources are {}",
                    enabled()
                        .map(|s| s.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            })
            .collect::<Result<_, _>>()?,
    };
    Ok(picked
        .into_iter()
        .filter(|s| s.language == language)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_sources_are_valid_and_adr_0086s_outlets_but_one_are_the_default() {
        let all = parse(BUILT_IN).unwrap();
        assert_eq!(all.len(), 34);
        let default = select(&all, &Pick::Default, "en").unwrap();
        let names: Vec<&str> = default.iter().map(|s| s.name.as_str()).collect();
        // Bitcoin Magazine refuses some servers (ADR-0088): recorded, not in the default.
        assert_eq!(names, ["CoinDesk", "Cointelegraph", "Decrypt", "The Block"]);
        assert_eq!(select(&all, &Pick::All, "en").unwrap().len(), 34);
        assert!(select(&all, &Pick::All, "de").unwrap().is_empty());
        assert!(all.iter().all(|s| s.url.starts_with("https://")));
    }

    #[test]
    fn a_source_has_defaults_and_a_bad_one_is_refused() {
        let all = parse(
            r#"[{"id": "r-solana", "name": "r/solana", "kind": "reddit",
            "url": "https://www.reddit.com/r/solana/.rss", "coins": ["sol"]}]"#,
        )
        .unwrap();
        assert_eq!(
            all[0],
            Source {
                id: "r-solana".into(),
                name: "r/solana".into(),
                url: "https://www.reddit.com/r/solana/.rss".into(),
                kind: Kind::Reddit,
                coins: vec!["SOL".into()],
                ..Source::default()
            }
        );
        let bad = |json: &str| parse(json).unwrap_err();
        assert!(bad(r#"[{"id": "A b", "name": "x", "url": "https://x.example"}]"#).contains("id"));
        assert!(bad(r#"[{"id": "all", "name": "x", "url": "https://x.example"}]"#).contains("all"));
        assert!(bad(r#"[{"id": "a", "name": "x", "url": "ftp://x.example"}]"#).contains("URL"));
        assert!(
            bad(r#"[{"id": "a", "name": "x", "url": "https://x.example"},
            {"id": "a", "name": "y", "url": "https://y.example"}]"#)
            .contains("twice")
        );
        assert!(
            bad(r#"[{"id": "a", "name": "x", "url": "https://x.example", "lang": "de"}]"#)
                .contains("lang")
        );
    }

    #[test]
    fn a_pick_names_enabled_sources_of_the_language() {
        let all = parse(
            r#"[{"id": "a", "name": "A", "url": "https://a.example", "default": true},
                {"id": "b", "name": "B", "url": "https://b.example"},
                {"id": "c", "name": "C", "url": "https://c.example", "enabled": false, "default": true},
                {"id": "d", "name": "D", "url": "https://d.example", "language": "de"}]"#,
        )
        .unwrap();
        let ids = |pick: Pick, language: &str| -> Result<Vec<String>, String> {
            select(&all, &pick, language).map(|s| s.iter().map(|s| s.id.clone()).collect())
        };
        assert_eq!(ids(Pick::Default, "en").unwrap(), ["a"]);
        assert_eq!(ids(Pick::All, "en").unwrap(), ["a", "b"]);
        assert_eq!(ids(Pick::All, "de").unwrap(), ["d"]);
        assert_eq!(
            ids(Pick::Ids(vec!["b".into(), "a".into()]), "en").unwrap(),
            ["b", "a"]
        );
        assert!(ids(Pick::Ids(vec!["c".into()]), "en")
            .unwrap_err()
            .contains("switched off"));
        assert!(ids(Pick::Ids(vec!["x".into()]), "en")
            .unwrap_err()
            .contains("not a source"));
        // None marked among the enabled: all are the default.
        let unmarked = parse(r#"[{"id": "a", "name": "A", "url": "https://a.example"}]"#).unwrap();
        assert_eq!(select(&unmarked, &Pick::Default, "en").unwrap().len(), 1);
    }

    #[test]
    fn the_feeds_of_adr_0086_are_english_sources_named_by_their_name() {
        let sources =
            from_feeds(" Outlet A=https://a.example/rss , https://b.example/feed?x=1,").unwrap();
        assert_eq!(
            sources,
            [
                Source {
                    id: "outlet-a".into(),
                    name: "Outlet A".into(),
                    url: "https://a.example/rss".into(),
                    ..Source::default()
                },
                Source {
                    id: "b-example".into(),
                    name: "b.example".into(),
                    url: "https://b.example/feed?x=1".into(),
                    ..Source::default()
                },
            ]
        );
        assert_eq!(from_feeds("").unwrap(), []);
        assert!(from_feeds("Old=ftp://a.example/rss").is_err());
    }

    #[test]
    fn a_file_is_read_again_when_it_changes_and_a_broken_one_keeps_the_last() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp")
            .join(format!("news-sources-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sources.json");
        let write = |text: &str, secs: u64| {
            std::fs::write(&path, text).unwrap();
            let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(at)
                .unwrap();
        };
        write(
            r#"[{"id": "a", "name": "A", "url": "https://a.example"}]"#,
            1_000,
        );
        assert_eq!(from_file(&path).unwrap()[0].id, "a");
        write(
            r#"[{"id": "b", "name": "B", "url": "https://b.example"}]"#,
            2_000,
        );
        assert_eq!(from_file(&path).unwrap()[0].id, "b");
        write("[{", 3_000);
        assert_eq!(from_file(&path).unwrap()[0].id, "b");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
