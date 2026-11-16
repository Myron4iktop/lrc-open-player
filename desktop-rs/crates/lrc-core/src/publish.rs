//! Publishing synced lyrics to LRCLIB (not in the Python app).
//!
//! LRCLIB takes lyrics from anyone, without an account. Each publish carries a token
//! earned with a small proof of work, which keeps spam down:
//!
//! 1. `POST /api/request-challenge` gives a `prefix` and a `target` (64 hex digits).
//! 2. Find a number (the nonce) such that SHA-256 of the prefix followed by the number
//!    in decimal is below the target, compared as 32 big-endian bytes ([`solve`]).
//! 3. `POST /api/publish` with `X-Publish-Token: {prefix}:{nonce}` and the lyrics. A
//!    token works once, and a challenge expires after 5 minutes.
//!
//! LRCLIB keeps every earlier version of a song's lyrics, so a publish adds one; it
//! never deletes. Lyrics with no text at all would mark the song as instrumental, so
//! [`PublishLyrics::from_lrc`] refuses them.
//!
//! The steps are plain functions, with the HTTP part handed in ([`publish_with`]), so
//! the rules are tested without the network.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ring::digest::{Context, SHA256};
use serde_json::{Value, json};

use crate::lrc::{self, EMPTY_LINE_MARK, LyricLine};
use crate::lrclib::USER_AGENT;
use crate::py;

const CHALLENGE_URL: &str = "https://lrclib.net/api/request-challenge";
const PUBLISH_URL: &str = "https://lrclib.net/api/publish";

/// The longest song LRCLIB takes, in seconds.
pub const MAX_DURATION_SEC: u64 = 3600;

/// The title of the window and its message boxes.
pub const PUBLISH_TITLE: &str = "Publish to LRCLIB";

/// What the window says about publishing, under the song's details.
pub const PUBLISH_NOTES: &str = "Anyone can then find these lyrics on LRCLIB, and players that use it (this one included) may download them. LRCLIB keeps earlier versions too, so publishing adds yours rather than replacing what is there. Translations aren't sent: only the sung lines.";

/// Shown while the proof of work runs.
pub const SOLVING_TEXT: &str = "Solving LRCLIB's anti-spam check… (this can take a little while)";
/// Shown while the lyrics are sent.
pub const SENDING_TEXT: &str = "Sending the lyrics…";
/// Shown after a cancel.
pub const CANCELLED_TEXT: &str = "Cancelled. Nothing was published.";

/// The lyrics as they are sent: the sung lines only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishLyrics {
    /// LRC, one stamped line each, ♪ breaks as empty stamped lines.
    pub synced: String,
    /// The same lines without times or breaks.
    pub plain: String,
    /// Lines with words (breaks not counted).
    pub sung_lines: usize,
}

impl PublishLyrics {
    /// The lyrics to send, from the sync editor's LRC. The editor writes a translation
    /// as a second line with the same stamp; LRCLIB would show it as another sung line,
    /// so it is dropped. Errors are sentences for the window.
    pub fn from_lrc(text: &str) -> Result<Self, &'static str> {
        let parsed = lrc::parse(text);
        if !parsed.synced {
            return Err("Only synced lyrics can be published: stamp the lines first.");
        }
        let lines: Vec<LyricLine> = parsed
            .lines
            .into_iter()
            .map(|l| LyricLine {
                sub: String::new(),
                ..l
            })
            .collect();
        let plain: Vec<&str> = lines
            .iter()
            .map(|l| l.main.as_str())
            .filter(|m| *m != EMPTY_LINE_MARK && !py::strip(m).is_empty())
            .collect();
        if plain.is_empty() {
            // LRCLIB marks a song without lyrics as instrumental.
            return Err("There are no sung lines to publish.");
        }
        Ok(PublishLyrics {
            synced: lrc::to_text(&lines, true),
            plain: plain.join("\n"),
            sung_lines: plain.len(),
        })
    }
}

/// The song's details as they are sent; the user can correct them first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishForm {
    pub track: String,
    pub artist: String,
    pub album: String,
    /// The song's length, in whole seconds.
    pub duration_sec: u64,
}

impl PublishForm {
    /// The details from a song's tags. The player's stand-ins for missing tags
    /// ("Unknown Artist") are left empty, so they get filled in by hand.
    pub fn from_tags(
        title: &str,
        title_from_tags: bool,
        artist: &str,
        album: &str,
        duration_sec: u64,
    ) -> Self {
        let real = |s: &str, placeholder: &str| {
            let s = py::strip(s);
            if s == placeholder {
                String::new()
            } else {
                s.to_owned()
            }
        };
        PublishForm {
            track: if title_from_tags {
                py::strip(title).to_owned()
            } else {
                String::new()
            },
            artist: real(artist, "Unknown Artist"),
            album: real(album, "Unknown Album"),
            duration_sec,
        }
    }

    /// What is missing or wrong, as a sentence for the window; `None` when it can go.
    pub fn problem(&self) -> Option<&'static str> {
        if py::strip(&self.track).is_empty() {
            Some("Fill in the song's title.")
        } else if py::strip(&self.artist).is_empty() {
            Some("Fill in the artist.")
        } else if py::strip(&self.album).is_empty() {
            Some("Fill in the album: LRCLIB needs one. For a single, use the song's title.")
        } else if self.duration_sec == 0 {
            Some("The song's length couldn't be read, and LRCLIB needs it.")
        } else if self.duration_sec > MAX_DURATION_SEC {
            Some("LRCLIB only takes songs up to an hour long.")
        } else {
            None
        }
    }

    /// The body of `POST /api/publish`.
    pub fn body(&self, lyrics: &PublishLyrics) -> Value {
        json!({
            "trackName": py::strip(&self.track),
            "artistName": py::strip(&self.artist),
            "albumName": py::strip(&self.album),
            "duration": self.duration_sec,
            "plainLyrics": lyrics.plain,
            "syncedLyrics": lyrics.synced,
        })
    }
}

/// `m:ss`, as the window shows the length.
pub fn length_text(sec: u64) -> String {
    format!("{}:{:02}", sec / 60, sec % 60)
}

/// What the window says above the button: how many lines go out.
pub fn summary_text(lyrics: &PublishLyrics) -> String {
    match lyrics.sung_lines {
        1 => "1 synced line will be published.".to_owned(),
        n => format!("{n} synced lines will be published."),
    }
}

/// A challenge's target as 32 bytes, or `None` if it isn't 64 hex digits.
pub fn parse_target(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// True if SHA-256 of `prefix` + `nonce` (in decimal) is below `target`.
pub fn nonce_ok(prefix: &str, nonce: u64, target: &[u8; 32]) -> bool {
    let mut ctx = Context::new(&SHA256);
    ctx.update(prefix.as_bytes());
    ctx.update(nonce.to_string().as_bytes());
    ctx.finish().as_ref() < target.as_slice()
}

/// The smallest nonce that solves a challenge, counting up from 0, or `None` when
/// `cancelled` was set first. Checks `cancelled` every few thousand tries.
pub fn solve(prefix: &str, target: &[u8; 32], cancelled: &AtomicBool) -> Option<u64> {
    // The prefix is hashed once; each try only adds its number.
    let mut base = Context::new(&SHA256);
    base.update(prefix.as_bytes());
    let mut digits = itoa_buf();
    for nonce in 0u64.. {
        if nonce % 4096 == 0 && cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let mut ctx = base.clone();
        ctx.update(write_decimal(nonce, &mut digits));
        if ctx.finish().as_ref() < target.as_slice() {
            return Some(nonce);
        }
    }
    None
}

fn itoa_buf() -> [u8; 20] {
    [0; 20]
}

/// `n` in decimal, written into the end of `buf` (no allocation per try).
fn write_decimal(mut n: u64, buf: &mut [u8; 20]) -> &[u8] {
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    &buf[i..]
}

/// An answer from LRCLIB: the status and the body as text. `Err` when it couldn't be
/// reached at all (the reason, for the window).
pub type Reply = Result<(u16, String), String>;

/// How a publish ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Published,
    /// Stopped by the user; nothing was sent.
    Cancelled,
    /// A sentence for the window.
    Failed(String),
}

/// What happened, as the window says it.
pub fn outcome_text(outcome: &Outcome, track: &str) -> String {
    match outcome {
        Outcome::Published => format!(
            "Published “{track}” to LRCLIB. Thank you! It may take a while before searches find it."
        ),
        Outcome::Cancelled => CANCELLED_TEXT.to_owned(),
        Outcome::Failed(why) => why.clone(),
    }
}

/// The sentence for an error answer. LRCLIB answers `{"message", "name", "statusCode"}`,
/// or plain text for some mistakes.
pub fn error_text(status: u16, body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let field = |k: &str| {
        parsed
            .as_ref()
            .and_then(|v| v.get(k))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let name = field("name").unwrap_or_default();
    let message = field("message")
        .or_else(|| (!body.trim().is_empty() && parsed.is_none()).then(|| body.trim().to_owned()))
        .unwrap_or_default();
    let detail = if message.is_empty() {
        String::new()
    } else {
        format!("\n\nLRCLIB said: {message}")
    };
    match (status, name.as_str()) {
        (429, _) => format!(
            "LRCLIB is getting too many requests from this computer. Try again in a minute.{detail}"
        ),
        (503, _) => format!("LRCLIB is busy right now. Try again in a moment.{detail}"),
        (400, "ValidationError") => format!("LRCLIB didn't accept the details.{detail}"),
        (400, "IncorrectPublishTokenError") => {
            format!("LRCLIB didn't accept the anti-spam check. Try again.{detail}")
        }
        (500..=599, _) => format!("LRCLIB had a problem ({status}). Try again later.{detail}"),
        _ => format!("LRCLIB refused the lyrics ({status}).{detail}"),
    }
}

/// The steps of a publish, for the window's status line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Solving,
    Sending,
}

/// Publishes with the HTTP part handed in: `post(url, token, body)` sends a POST, with
/// `X-Publish-Token` when `token` is set and the JSON `body` when there is one.
///
/// A token LRCLIB turns down (expired, say) is tried once more with a new challenge.
pub fn publish_with(
    form: &PublishForm,
    lyrics: &PublishLyrics,
    cancelled: &AtomicBool,
    mut step: impl FnMut(Step),
    mut post: impl FnMut(&str, Option<&str>, Option<&Value>) -> Reply,
) -> Outcome {
    if let Some(problem) = form.problem() {
        return Outcome::Failed(problem.to_owned());
    }
    let body = form.body(lyrics);
    let stop = || cancelled.load(Ordering::Relaxed);
    for attempt in 0..2 {
        if stop() {
            return Outcome::Cancelled;
        }
        step(Step::Solving);
        let (prefix, target) = match post(CHALLENGE_URL, None, None) {
            Err(why) => return Outcome::Failed(why),
            Ok((200 | 201, text)) => {
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let prefix = v.get("prefix").and_then(Value::as_str).unwrap_or_default();
                let target = v
                    .get("target")
                    .and_then(Value::as_str)
                    .and_then(parse_target);
                match (prefix, target) {
                    (p, Some(t)) if !p.is_empty() => (p.to_owned(), t),
                    _ => {
                        return Outcome::Failed(
                            "LRCLIB sent an anti-spam check this version of the player can't read."
                                .to_owned(),
                        );
                    }
                }
            }
            Ok((status, text)) => return Outcome::Failed(error_text(status, &text)),
        };
        let Some(nonce) = solve(&prefix, &target, cancelled) else {
            return Outcome::Cancelled;
        };
        if stop() {
            return Outcome::Cancelled;
        }
        step(Step::Sending);
        let token = format!("{prefix}:{nonce}");
        match post(PUBLISH_URL, Some(&token), Some(&body)) {
            Err(why) => return Outcome::Failed(why),
            Ok((200..=299, _)) => return Outcome::Published,
            Ok((400, text)) if attempt == 0 && text.contains("IncorrectPublishTokenError") => {
                continue; // a fresh challenge, once
            }
            Ok((status, text)) => return Outcome::Failed(error_text(status, &text)),
        }
    }
    Outcome::Failed(error_text(400, r#"{"name":"IncorrectPublishTokenError"}"#))
}

/// Sends one POST to LRCLIB. Error answers are returned, not turned into `Err`, so
/// their message can be shown.
fn post_http(url: &str, token: Option<&str>, body: Option<&Value>) -> Reply {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let mut request = agent.post(url);
    if let Some(token) = token {
        request = request.header("X-Publish-Token", token);
    }
    let sent = match body {
        Some(body) => request
            .content_type("application/json")
            .send(body.to_string()),
        None => request.send_empty(),
    };
    let mut response = sent.map_err(|e| format!("Couldn't reach LRCLIB: {e}"))?;
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .unwrap_or_default();
    Ok((status, text))
}

/// Publishes the lyrics on LRCLIB. Runs on a worker thread; it blocks for the requests
/// and the proof of work. `step` hears each step as it starts.
pub fn publish(
    form: &PublishForm,
    lyrics: &PublishLyrics,
    cancelled: &AtomicBool,
    step: impl FnMut(Step),
) -> Outcome {
    publish_with(form, lyrics, cancelled, step, post_http)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> PublishForm {
        PublishForm {
            track: "My Song".into(),
            artist: "The Band".into(),
            album: "The Album".into(),
            duration_sec: 215,
        }
    }

    fn lyrics() -> PublishLyrics {
        PublishLyrics::from_lrc("[00:01.00]One\n[00:02.00]Two\n").unwrap()
    }

    #[test]
    fn translations_are_left_out_and_breaks_kept() {
        let text = "[00:01.00]一行目\n[00:01.00]Line one\n[00:05.00]\n[00:09.50]二行目\n[00:09.50]Line two\n";
        let l = PublishLyrics::from_lrc(text).unwrap();
        assert_eq!(l.synced, "[00:01.00]一行目\n[00:05.00]\n[00:09.50]二行目\n");
        assert_eq!(l.plain, "一行目\n二行目");
        assert_eq!(l.sung_lines, 2);
        assert_eq!(summary_text(&l), "2 synced lines will be published.");
        let one = PublishLyrics::from_lrc("[00:01.00]Only\n").unwrap();
        assert_eq!(summary_text(&one), "1 synced line will be published.");
    }

    #[test]
    fn nothing_to_sing_is_never_published() {
        // It would mark the song as instrumental on LRCLIB.
        assert!(PublishLyrics::from_lrc("[00:01.00]\n[00:04.00]\n").is_err());
        assert!(PublishLyrics::from_lrc("").is_err());
        assert!(PublishLyrics::from_lrc("plain words\nno times\n").is_err());
    }

    #[test]
    fn details_from_tags_and_what_is_missing() {
        let f = PublishForm::from_tags(" Song ", true, "Unknown Artist", "Unknown Album", 200);
        assert_eq!(
            (f.track.as_str(), f.artist.as_str(), f.album.as_str()),
            ("Song", "", "")
        );
        assert_eq!(f.problem(), Some("Fill in the artist."));
        // A title made from the file name isn't trusted either.
        let f = PublishForm::from_tags("01 song", false, "A", "B", 200);
        assert_eq!(f.problem(), Some("Fill in the song's title."));

        let mut f = form();
        assert_eq!(f.problem(), None);
        f.album = "  ".into();
        assert!(f.problem().unwrap().starts_with("Fill in the album"));
        f.album = "X".into();
        f.duration_sec = 0;
        assert!(f.problem().is_some());
        f.duration_sec = 3601;
        assert!(f.problem().is_some());
        f.duration_sec = 3600;
        assert_eq!(f.problem(), None);
        assert_eq!(length_text(215), "3:35");
    }

    #[test]
    fn the_body_has_every_field() {
        let mut f = form();
        f.track = " My Song ".into();
        let body = f.body(&lyrics());
        assert_eq!(
            body,
            json!({
                "trackName": "My Song",
                "artistName": "The Band",
                "albumName": "The Album",
                "duration": 215,
                "plainLyrics": "One\nTwo",
                "syncedLyrics": "[00:01.00]One\n[00:02.00]Two\n",
            })
        );
    }

    #[test]
    fn targets_are_64_hex_digits() {
        let t = parse_target(&format!("{}{}", "00ff", "0".repeat(60))).unwrap();
        assert_eq!((t[0], t[1], t[31]), (0, 255, 0));
        assert_eq!(parse_target("abc"), None);
        assert_eq!(parse_target(&"g".repeat(64)), None);
        assert_eq!(parse_target(&"é".repeat(32)), None);
    }

    #[test]
    fn decimal_digits_without_allocating() {
        let mut buf = itoa_buf();
        for n in [0, 7, 10, 4096, 123_456_789, u64::MAX] {
            assert_eq!(write_decimal(n, &mut buf), n.to_string().as_bytes());
        }
    }

    #[test]
    fn solves_like_lrcget() {
        // The solver finds the first nonce a plain search with the same rule finds.
        let target = parse_target(&format!("0{}", "f".repeat(63))).unwrap(); // 1 in 16
        let nonce = solve("prefix", &target, &AtomicBool::new(false)).unwrap();
        assert!(nonce_ok("prefix", nonce, &target));
        assert!((0..nonce).all(|n| !nonce_ok("prefix", n, &target)));
        // A harder one (1 in 4096) still comes quickly.
        let target = parse_target(&format!("000{}", "f".repeat(61))).unwrap();
        let nonce = solve(
            "abcdefghijklmnopqrstuvwxyz012345",
            &target,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(nonce_ok("abcdefghijklmnopqrstuvwxyz012345", nonce, &target));
        // SHA-256 of "abc" is ba7816bf…: below c0…, not below ba…
        let mut ctx = Context::new(&SHA256);
        ctx.update(b"abc");
        assert_eq!(&ctx.finish().as_ref()[..2], &[0xba, 0x78]);
        // Impossible target: cancelling stops it.
        let zero = [0u8; 32];
        assert_eq!(solve("p", &zero, &AtomicBool::new(true)), None);
    }

    /// A stand-in LRCLIB: answers in order, and records what was sent.
    struct Fake {
        replies: Vec<Reply>,
        sent: Vec<(String, Option<String>, Option<Value>)>,
    }

    impl Fake {
        fn new(replies: Vec<Reply>) -> Self {
            Fake {
                replies,
                sent: Vec::new(),
            }
        }

        fn post(&mut self, url: &str, token: Option<&str>, body: Option<&Value>) -> Reply {
            self.sent
                .push((url.to_owned(), token.map(str::to_owned), body.cloned()));
            self.replies.remove(0)
        }
    }

    fn easy_challenge() -> Reply {
        Ok((
            200,
            json!({"prefix": "pfx", "target": format!("0{}", "f".repeat(63))}).to_string(),
        ))
    }

    #[test]
    fn publishes_with_a_solved_token() {
        let mut fake = Fake::new(vec![easy_challenge(), Ok((201, String::new()))]);
        let mut steps = Vec::new();
        let outcome = publish_with(
            &form(),
            &lyrics(),
            &AtomicBool::new(false),
            |s| steps.push(s),
            |u, t, b| fake.post(u, t, b),
        );
        assert_eq!(outcome, Outcome::Published);
        assert_eq!(steps, [Step::Solving, Step::Sending]);
        assert_eq!(fake.sent.len(), 2);
        assert_eq!(fake.sent[0], (CHALLENGE_URL.to_owned(), None, None));
        let (url, token, body) = &fake.sent[1];
        assert_eq!(url, PUBLISH_URL);
        let token = token.as_deref().unwrap();
        let (prefix, nonce) = token.split_once(':').unwrap();
        assert_eq!(prefix, "pfx");
        let target = parse_target(&format!("0{}", "f".repeat(63))).unwrap();
        assert!(nonce_ok(prefix, nonce.parse().unwrap(), &target));
        assert_eq!(body.as_ref().unwrap(), &form().body(&lyrics()));
        assert!(outcome_text(&outcome, "My Song").starts_with("Published “My Song” to LRCLIB."));
    }

    #[test]
    fn a_turned_down_token_is_tried_once_more() {
        let bad = r#"{"message":"Expired","name":"IncorrectPublishTokenError","statusCode":400}"#;
        let mut fake = Fake::new(vec![
            easy_challenge(),
            Ok((400, bad.to_owned())),
            easy_challenge(),
            Ok((201, String::new())),
        ]);
        let outcome = publish_with(
            &form(),
            &lyrics(),
            &AtomicBool::new(false),
            |_| {},
            |u, t, b| fake.post(u, t, b),
        );
        assert_eq!(outcome, Outcome::Published);
        assert_eq!(fake.sent.len(), 4);

        let mut fake = Fake::new(vec![
            easy_challenge(),
            Ok((400, bad.to_owned())),
            easy_challenge(),
            Ok((400, bad.to_owned())),
        ]);
        let outcome = publish_with(
            &form(),
            &lyrics(),
            &AtomicBool::new(false),
            |_| {},
            |u, t, b| fake.post(u, t, b),
        );
        let Outcome::Failed(why) = outcome else {
            panic!()
        };
        assert!(why.contains("anti-spam check"), "{why}");
        assert!(why.contains("LRCLIB said: Expired"), "{why}");
    }

    #[test]
    fn errors_are_explained() {
        let mut fake = Fake::new(vec![
            easy_challenge(),
            Ok((400, r#"{"message":"duration must be <= 3600","name":"ValidationError","statusCode":400}"#.to_owned())),
        ]);
        let Outcome::Failed(why) = publish_with(
            &form(),
            &lyrics(),
            &AtomicBool::new(false),
            |_| {},
            |u, t, b| fake.post(u, t, b),
        ) else {
            panic!()
        };
        assert_eq!(
            why,
            "LRCLIB didn't accept the details.\n\nLRCLIB said: duration must be <= 3600"
        );

        let mut fake = Fake::new(vec![Err("Couldn't reach LRCLIB: timed out".to_owned())]);
        let outcome = publish_with(
            &form(),
            &lyrics(),
            &AtomicBool::new(false),
            |_| {},
            |u, t, b| fake.post(u, t, b),
        );
        assert_eq!(
            outcome,
            Outcome::Failed("Couldn't reach LRCLIB: timed out".to_owned())
        );

        let mut fake = Fake::new(vec![Ok((200, "{}".to_owned()))]);
        let Outcome::Failed(why) = publish_with(
            &form(),
            &lyrics(),
            &AtomicBool::new(false),
            |_| {},
            |u, t, b| fake.post(u, t, b),
        ) else {
            panic!()
        };
        assert!(why.contains("can't read"), "{why}");

        assert!(error_text(429, "").starts_with("LRCLIB is getting too many requests"));
        assert!(error_text(503, "").starts_with("LRCLIB is busy"));
        assert_eq!(
            error_text(502, "Bad gateway"),
            "LRCLIB had a problem (502). Try again later.\n\nLRCLIB said: Bad gateway"
        );
        assert_eq!(error_text(404, ""), "LRCLIB refused the lyrics (404).");
    }

    #[test]
    fn nothing_is_sent_for_bad_details_or_after_a_cancel() {
        let mut f = form();
        f.album.clear();
        let mut fake = Fake::new(vec![]);
        let outcome = publish_with(
            &f,
            &lyrics(),
            &AtomicBool::new(false),
            |_| {},
            |u, t, b| fake.post(u, t, b),
        );
        assert!(matches!(outcome, Outcome::Failed(_)));
        assert!(fake.sent.is_empty());

        let mut fake = Fake::new(vec![]);
        let outcome = publish_with(
            &form(),
            &lyrics(),
            &AtomicBool::new(true),
            |_| {},
            |u, t, b| fake.post(u, t, b),
        );
        assert_eq!(outcome, Outcome::Cancelled);
        assert!(fake.sent.is_empty());
        assert_eq!(outcome_text(&outcome, "x"), CANCELLED_TEXT);

        // Cancelled while solving: the lyrics are never sent.
        let cancelled = AtomicBool::new(false);
        let mut fake = Fake::new(vec![Ok((
            200,
            json!({"prefix": "p", "target": "0".repeat(64)}).to_string(),
        ))]);
        let outcome = publish_with(
            &form(),
            &lyrics(),
            &cancelled,
            |_| {},
            |u, t, b| {
                cancelled.store(true, Ordering::Relaxed);
                fake.post(u, t, b)
            },
        );
        assert_eq!(outcome, Outcome::Cancelled);
        assert_eq!(fake.sent.len(), 1);
    }

    /// Run by hand to check the real challenge: `cargo test -p lrc-core -- --ignored challenge`.
    /// It asks for a challenge and solves it, but publishes nothing.
    #[test]
    #[ignore = "needs the internet"]
    fn live_challenge_solves() {
        let (status, text) = post_http(CHALLENGE_URL, None, None).unwrap();
        assert!(status == 200 || status == 201, "{status} {text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        let prefix = v["prefix"].as_str().unwrap();
        let target = parse_target(v["target"].as_str().unwrap()).unwrap();
        let started = std::time::Instant::now();
        let nonce = solve(prefix, &target, &AtomicBool::new(false)).unwrap();
        assert!(nonce_ok(prefix, nonce, &target));
        eprintln!("solved in {:?} (nonce {nonce})", started.elapsed());
    }
}
