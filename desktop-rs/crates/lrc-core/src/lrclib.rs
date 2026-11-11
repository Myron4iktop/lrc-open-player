//! Looking a song up on LRCLIB (`LrcFetchWorker` in `lrcplayer.py`).
//!
//! It never writes files: the window decides what to keep. The rules that pick an answer
//! are plain functions on the JSON ([`pick_from_get`], [`pick_from_search`]), so they are
//! tested without the network.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::library::clean_query_string;
use crate::lrc::same_text;
use crate::py;

/// A search hit this many seconds longer or shorter than the song still counts as it.
pub const MATCH_TOLERANCE_SEC: f64 = 4.0;

/// Sent with every request (the Python app sends `LRCOpenPlayer/3.0 (Desktop; Python/PyQt6)`).
pub const USER_AGENT: &str = "LRCOpenPlayer/3.0 (Desktop; Rust)";

const GET_URL: &str = "https://lrclib.net/api/get";
const SEARCH_URL: &str = "https://lrclib.net/api/search";

/// The Python code would raise an exception here. In the `/api/get` step that moves
/// the lookup on to the search; in the search it ends as "No lyrics found online."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Raised;

impl std::fmt::Display for Raised {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("unexpected data from LRCLIB")
    }
}

impl std::error::Error for Raised {}

/// What the lookup found (`finished_signal(lyrics, message, trusted, lrclib_duration_sec)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResult {
    /// The lyrics, or "" when nothing was found.
    pub lyrics: String,
    /// The status message the player shows.
    pub message: String,
    /// LRCLIB's entry is as long as the song (an exact lookup, or a search hit within a
    /// few seconds with the same title and artist), so it is almost certainly the right
    /// one. A looser search hit can be another version of the song, or another song with
    /// the same name.
    pub trusted: bool,
    /// The length LRCLIB gives for its entry, rounded to whole seconds.
    pub lrclib_duration_sec: i64,
}

impl FetchResult {
    fn new(lyrics: &str, message: &str, trusted: bool, duration: f64) -> Result<Self, Raised> {
        let rounded = py::round_half_even(duration);
        if !rounded.is_finite() {
            // Python's int() raises here, and the lookup ends as "nothing found".
            return Err(Raised);
        }
        Ok(FetchResult {
            lyrics: lyrics.to_owned(),
            message: message.to_owned(),
            trusted,
            lrclib_duration_sec: rounded as i64,
        })
    }

    fn not_found() -> Self {
        FetchResult {
            lyrics: String::new(),
            message: "No lyrics found online.".into(),
            trusted: false,
            lrclib_duration_sec: 0,
        }
    }
}

/// The search terms the lookup uses for a song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub title: String,
    /// "" when the artist is unknown.
    pub artist: String,
    /// 0 when the length is unknown.
    pub duration_sec: i64,
}

impl Query {
    /// The clean-up `LrcFetchWorker.__init__` does: a title from the file name loses its
    /// track number and copy suffix, and "Unknown Artist" means no artist.
    pub fn new(title: &str, artist: &str, duration_sec: i64, title_from_tags: bool) -> Self {
        Query {
            title: clean_query_string(title, !title_from_tags),
            artist: if artist == "Unknown Artist" {
                String::new()
            } else {
                clean_query_string(artist, false)
            },
            duration_sec,
        }
    }

    /// The `/api/get` address.
    pub fn get_url(&self) -> String {
        let mut params = vec![("track_name", self.title.clone())];
        if !self.artist.is_empty() {
            params.push(("artist_name", self.artist.clone()));
        }
        if self.duration_sec > 0 {
            params.push(("duration", self.duration_sec.to_string()));
        }
        format!("{GET_URL}?{}", urlencode(&params))
    }

    /// The `/api/search` address.
    pub fn search_url(&self) -> String {
        let terms = py::strip(&format!("{} {}", self.title, self.artist)).to_owned();
        format!("{SEARCH_URL}?{}", urlencode(&[("q", terms)]))
    }
}

/// Python's `urllib.parse.urlencode`: UTF-8, spaces as `+`, only `A-Za-z0-9_.-~` kept.
pub fn urlencode(params: &[(&str, String)]) -> String {
    fn quote_plus(s: &str, out: &mut String) {
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                    out.push(b as char)
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
    }
    let mut out = String::new();
    for (i, (key, value)) in params.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        quote_plus(key, &mut out);
        out.push('=');
        quote_plus(value, &mut out);
    }
    out
}

/// Python's truthiness for a JSON value.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// A lyrics field the Python app can pass on: a non-empty string. `Err` where Python
/// would raise (a truthy value that isn't text).
fn lyrics_field<'a>(item: &'a Value, key: &str) -> Result<Option<&'a str>, Raised> {
    let v = item.get(key);
    if !truthy(v) {
        return Ok(None);
    }
    v.and_then(Value::as_str).map(Some).ok_or(Raised)
}

/// `data.get("duration") or 0` passed to `round()`: `Err` where Python would raise.
fn get_duration(item: &Value) -> Result<f64, Raised> {
    match item.get("duration") {
        v if !truthy(v) => Ok(0.0),
        Some(Value::Number(n)) => n.as_f64().ok_or(Raised),
        Some(Value::Bool(true)) => Ok(1.0),
        _ => Err(Raised),
    }
}

/// `float(item.get("duration") or 0)`, 0 where Python's float() gives up.
fn search_duration(item: &Value) -> f64 {
    match item.get("duration") {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::Bool(true)) => 1.0,
        Some(Value::String(s)) if !s.is_empty() => {
            let t = py::strip(s).replace('_', "");
            match t.to_ascii_lowercase().as_str() {
                "inf" | "+inf" | "infinity" | "+infinity" => f64::INFINITY,
                "-inf" | "-infinity" => f64::NEG_INFINITY,
                "nan" | "+nan" | "-nan" => f64::NAN,
                _ => t.parse().unwrap_or(0.0),
            }
        }
        _ => 0.0,
    }
}

/// The answer from `/api/get`, if it has lyrics. `Err` means the Python code would have
/// raised, which sends the lookup on to the search.
pub fn pick_from_get(data: &Value, query: &Query) -> Result<Option<FetchResult>, Raised> {
    // LRCLIB only returns an entry of that length, by that artist. Without an artist (an
    // untagged file) it is a title-only match, so it isn't trusted: see `matches` below.
    let exact = query.duration_sec > 0 && !query.artist.is_empty();
    if !data.is_object() {
        return Ok(None);
    }
    if let Some(text) = lyrics_field(data, "syncedLyrics")? {
        let duration = get_duration(data)?;
        return FetchResult::new(
            text,
            "Synced lyrics downloaded from LRCLIB.",
            exact,
            duration,
        )
        .map(Some);
    }
    if let Some(text) = lyrics_field(data, "plainLyrics")? {
        let duration = get_duration(data)?;
        return FetchResult::new(
            text,
            "Plain lyrics downloaded from LRCLIB.",
            exact,
            duration,
        )
        .map(Some);
    }
    Ok(None)
}

/// The best answer from `/api/search`: a synced hit that matches the song, then any
/// synced hit, then any plain one. `Err` means the Python code would have raised, which
/// ends the lookup as "No lyrics found online."
pub fn pick_from_search(results: &Value, query: &Query) -> Result<Option<FetchResult>, Raised> {
    let results: Vec<&Value> = match results {
        Value::Array(a) => a.iter().filter(|r| r.is_object()).collect(),
        _ => Vec::new(),
    };
    let fits = |item: &Value| {
        query.duration_sec > 0
            && (search_duration(item) - query.duration_sec as f64).abs() <= MATCH_TOLERANCE_SEC
    };
    let text_of = |item: &Value, key: &str| -> Result<String, Raised> {
        let v = item.get(key);
        if !truthy(v) {
            return Ok(String::new());
        }
        v.and_then(Value::as_str).map(str::to_owned).ok_or(Raised)
    };
    // "Trusted" means safe to keep without asking, so /api/search hits must agree on the
    // song, not only its length: another track of the same length is a different song.
    // Without an artist to compare (an untagged file) nothing is trusted: plenty of songs
    // are called "Home" or "Intro" and run about as long, and a wrong .lrc saved next to
    // the song would be shown from then on.
    let matches = |item: &Value| -> Result<bool, Raised> {
        if query.artist.is_empty() || !fits(item) {
            return Ok(false);
        }
        if !same_text(&text_of(item, "trackName")?, &query.title) {
            return Ok(false);
        }
        Ok(same_text(&text_of(item, "artistName")?, &query.artist))
    };

    for item in &results {
        if truthy(item.get("syncedLyrics")) && matches(item)? {
            let text = lyrics_field(item, "syncedLyrics")?.unwrap_or_default();
            let d = search_duration(item);
            return FetchResult::new(text, "Synced lyrics matched on LRCLIB.", true, d).map(Some);
        }
    }
    for item in &results {
        if let Some(text) = lyrics_field(item, "syncedLyrics")? {
            let d = search_duration(item);
            let msg = "Synced lyrics found on LRCLIB (maybe another version).";
            return FetchResult::new(text, msg, false, d).map(Some);
        }
    }
    for item in &results {
        if let Some(text) = lyrics_field(item, "plainLyrics")? {
            let d = search_duration(item);
            let trusted = matches(item)?;
            return FetchResult::new(text, "Plain lyrics found on LRCLIB.", trusted, d).map(Some);
        }
    }
    Ok(None)
}

/// GETs a URL and parses the JSON, or `None` for anything but a 200 with valid JSON.
fn fetch_json(url: &str, timeout_secs: u64) -> Option<Value> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs)))
        .http_status_as_error(true)
        .user_agent(USER_AGENT)
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let mut response = agent.get(url).call().ok()?;
    if response.status() != 200 {
        return None;
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .ok()?;
    serde_json::from_str(&body).ok()
}

/// Looks a song up: `/api/get` first (6 s), then `/api/search` (7 s).
///
/// Returns `None` when `cancelled` was set (the answer is no longer wanted, so the
/// worker sends nothing). Runs on a worker thread; it blocks for the requests.
pub fn fetch(query: &Query, cancelled: &AtomicBool) -> Option<FetchResult> {
    fetch_with(query, cancelled, fetch_json)
}

/// [`fetch`] with the HTTP part handed in: `get(url, timeout_secs)` returns the parsed
/// JSON of a 200 answer, or `None` for an error of any kind.
pub fn fetch_with(
    query: &Query,
    cancelled: &AtomicBool,
    mut get: impl FnMut(&str, u64) -> Option<Value>,
) -> Option<FetchResult> {
    let done = |r: FetchResult| (!cancelled.load(Ordering::Relaxed)).then_some(r);

    if let Some(data) = get(&query.get_url(), 6)
        && let Ok(Some(found)) = pick_from_get(&data, query)
    {
        return done(found);
    }
    if cancelled.load(Ordering::Relaxed) {
        return None;
    }
    if let Some(results) = get(&query.search_url(), 7)
        && let Ok(Some(found)) = pick_from_search(&results, query)
    {
        return done(found);
    }
    done(FetchResult::not_found())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn q(title: &str, artist: &str, duration: i64) -> Query {
        Query::new(title, artist, duration, true)
    }

    #[test]
    fn query_clean_up() {
        let query = Query::new("01 - Song (2)", "Unknown Artist", 200, false);
        assert_eq!(query.title, "Song");
        assert_eq!(query.artist, "");
        let query = Query::new("7 Rings", "Ariana [MP3]", 0, true);
        assert_eq!(
            (query.title.as_str(), query.artist.as_str()),
            ("7 Rings", "Ariana")
        );
    }

    #[test]
    fn urls_like_urlencode() {
        let query = q("Café & Co", "A/B", 215);
        assert_eq!(
            query.get_url(),
            "https://lrclib.net/api/get?track_name=Caf%C3%A9+%26+Co&artist_name=A%2FB&duration=215"
        );
        assert_eq!(
            query.search_url(),
            "https://lrclib.net/api/search?q=Caf%C3%A9+%26+Co+A%2FB"
        );
        assert_eq!(
            q("Song", "", 0).get_url(),
            "https://lrclib.net/api/get?track_name=Song"
        );
    }

    #[test]
    fn get_prefers_synced_and_trusts_exact_lookups() {
        let data = json!({"syncedLyrics": "[00:01.00]a", "plainLyrics": "a", "duration": 215.5});
        let r = pick_from_get(&data, &q("Song", "A", 215)).unwrap().unwrap();
        assert_eq!(r.message, "Synced lyrics downloaded from LRCLIB.");
        assert!(r.trusted);
        assert_eq!(r.lrclib_duration_sec, 216); // 215.5 rounds to even
        let data = json!({"syncedLyrics": null, "plainLyrics": "a"});
        let r = pick_from_get(&data, &q("Song", "A", 0)).unwrap().unwrap();
        assert_eq!(r.message, "Plain lyrics downloaded from LRCLIB.");
        assert!(!r.trusted);
        assert_eq!(
            pick_from_get(&json!({"syncedLyrics": ""}), &q("S", "", 1)),
            Ok(None)
        );
        // A duration that isn't a number makes Python fall through to the search.
        assert!(
            pick_from_get(
                &json!({"plainLyrics": "a", "duration": "x"}),
                &q("S", "", 1)
            )
            .is_err()
        );
    }

    #[test]
    fn search_order_and_trust() {
        let query = q("My Song", "The Band", 200);
        let results = json!([
            {"trackName": "Other", "artistName": "The Band", "duration": 200, "syncedLyrics": "[00:01.00]x"},
            {"trackName": "my song!", "artistName": "the band", "duration": 203.9, "syncedLyrics": "[00:01.00]y"},
        ]);
        let r = pick_from_search(&results, &query).unwrap().unwrap();
        assert_eq!(r.lyrics, "[00:01.00]y");
        assert_eq!(r.message, "Synced lyrics matched on LRCLIB.");
        assert!(r.trusted);
        assert_eq!(r.lrclib_duration_sec, 204);

        let results = json!([
            {"trackName": "My Song", "artistName": "The Band", "duration": 205, "syncedLyrics": "[00:01.00]x"},
        ]);
        let r = pick_from_search(&results, &query).unwrap().unwrap();
        assert_eq!(
            r.message,
            "Synced lyrics found on LRCLIB (maybe another version)."
        );
        assert!(!r.trusted);

        let results = json!([
            "junk",
            {"trackName": "My Song", "artistName": "The Band", "duration": "199", "plainLyrics": "words"},
        ]);
        let r = pick_from_search(&results, &query).unwrap().unwrap();
        assert_eq!(r.message, "Plain lyrics found on LRCLIB.");
        assert!(r.trusted);
        assert_eq!(r.lrclib_duration_sec, 199);

        assert_eq!(
            pick_from_search(&json!({"not": "a list"}), &query),
            Ok(None)
        );
        assert_eq!(pick_from_search(&json!([]), &query), Ok(None));
    }

    #[test]
    fn unknown_length_is_never_trusted_from_search() {
        let query = q("My Song", "", 0);
        let results = json!([{"trackName": "My Song", "duration": 0, "plainLyrics": "w"}]);
        let r = pick_from_search(&results, &query).unwrap().unwrap();
        assert!(!r.trusted);
    }

    #[test]
    fn untagged_songs_are_never_trusted() {
        // Same title and length, but no artist to compare: maybe another song called "Home".
        let query = q("Home", "", 200);
        let results = json!([{"trackName": "Home", "artistName": "Someone", "duration": 200, "syncedLyrics": "[00:01.00]x"}]);
        let r = pick_from_search(&results, &query).unwrap().unwrap();
        assert!(!r.trusted);
        let data = json!({"syncedLyrics": "[00:01.00]x", "duration": 200});
        assert!(!pick_from_get(&data, &query).unwrap().unwrap().trusted);
    }

    #[test]
    fn not_found_message() {
        assert_eq!(FetchResult::not_found().message, "No lyrics found online.");
    }

    /// Run by hand: `cargo test -p lrc-core -- --ignored lrclib`.
    #[test]
    #[ignore = "needs the internet"]
    fn lrclib_live_lookup() {
        let query = Query::new("Never Gonna Give You Up", "Rick Astley", 213, true);
        let result = fetch(&query, &AtomicBool::new(false)).unwrap();
        assert!(!result.lyrics.is_empty(), "{result:?}");
    }
}
