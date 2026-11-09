//! The rules behind the tray icon, the Windows media controls and the error message,
//! which need no Windows to test: `_update_tray`, `WindowsMediaControls` and
//! `install_error_handler` in lrcplayer.py.

use std::time::{Duration, Instant};

use crate::queue::Repeat;

/// The title of the one-time hint after closing to the tray.
pub const TRAY_HINT_TITLE: &str = "Still playing";
pub const TRAY_HINT_TEXT: &str =
    "The player keeps running in the tray. Right-click the icon to quit.";
/// How long the hint stays up, in milliseconds (`showMessage(…, 5000)`).
pub const TRAY_HINT_MS: u32 = 5000;
/// Where "Keep playing in tray when closed" is kept. Not the Python app's
/// `ui/close_to_tray`, which that app writes as on at every start.
pub const TRAY_ON_CLOSE_KEY: &str = "ui/tray_on_close";
/// The View menu item's tooltip when Windows has no tray.
pub const NO_TRAY_TEXT: &str = "No system tray available";

/// What the tray menu and tooltip show (`_update_tray`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayTexts {
    /// The first, greyed-out line of the menu.
    pub now: String,
    pub tooltip: String,
    pub play: &'static str,
    pub repeat: &'static str,
    pub window: &'static str,
}

/// `_update_tray`. `track` is the title and artist of the current song, if there is one.
pub fn tray_texts(
    track: Option<(&str, &str)>,
    playing: bool,
    repeat: Repeat,
    window_shown: bool,
) -> TrayTexts {
    let (title, artist) = track.unwrap_or(("", ""));
    let artist = if artist == "Unknown Artist" {
        ""
    } else {
        artist
    };
    let now = if !title.is_empty() && !artist.is_empty() {
        format!("{title} — {artist}")
    } else if !title.is_empty() {
        title.to_owned()
    } else {
        "Nothing playing".to_owned()
    };
    let shown = if now.chars().count() <= 60 {
        now.clone()
    } else {
        now.chars().take(57).collect::<String>() + "…"
    };
    let tooltip = if track.is_some() {
        now.chars().take(120).collect()
    } else {
        crate::APP_TITLE.to_owned()
    };
    TrayTexts {
        now: shown,
        tooltip,
        play: if playing { "Pause" } else { "Play" },
        repeat: match repeat {
            Repeat::Off => "Repeat: off",
            Repeat::Track => "Repeat: this track",
            Repeat::Album => "Repeat: whole album",
        },
        window: if window_shown {
            "Hide player"
        } else {
            "Show player"
        },
    }
}

/// `WindowsMediaControls._clean`: a tag that holds only the placeholder counts as empty.
pub fn media_clean<'a>(value: &'a str, placeholder: &str) -> &'a str {
    if value.is_empty() || value == placeholder {
        ""
    } else {
        value
    }
}

/// The track number for the media flyout: the leading digits of the tag ("3/12" → 3),
/// or 0 (`re.match(r"\d+", …)`).
pub fn media_track_number(tag: &str) -> u32 {
    let digits: String = tag.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap_or(0)
}

/// `update_timeline`'s throttle: about once a second, unless forced (a seek, a pause, a
/// new song).
#[derive(Debug, Default)]
pub struct TimelineThrottle {
    last: Option<Instant>,
}

impl TimelineThrottle {
    /// Whether to send the timeline now; if so, the clock restarts.
    pub fn ready(&mut self, now: Instant, force: bool) -> bool {
        if !force
            && self
                .last
                .is_some_and(|t| now.duration_since(t) < Duration::from_secs(1))
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

/// The position and length sent to the media flyout: never negative, and the position
/// never past the end.
pub fn timeline(position_ms: u64, duration_ms: u64) -> (u64, u64) {
    (position_ms.min(duration_ms), duration_ms)
}

/// The first line of the error message.
pub const ERROR_TEXT: &str = "Something went wrong, but the player is still running.";

/// `install_error_handler`'s rule: one message at a time, and not the same one again
/// within 10 s.
#[derive(Debug, Default)]
pub struct ErrorGate {
    showing: bool,
    last: String,
    when: Option<Instant>,
}

impl ErrorGate {
    /// Whether to show a message for `summary` now. If so, it counts as showing until
    /// [`closed`](Self::closed).
    pub fn open(&mut self, summary: &str, now: Instant) -> bool {
        let recent = self
            .when
            .is_some_and(|t| now.duration_since(t) < Duration::from_secs(10));
        if self.showing || (summary == self.last && recent) {
            return false;
        }
        self.showing = true;
        self.last = summary.to_owned();
        self.when = Some(now);
        true
    }

    pub fn closed(&mut self) {
        self.showing = false;
    }
}

/// The summary under the first line: `summary[:400]`.
pub fn error_summary(summary: &str) -> String {
    summary.chars().take(400).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_texts_follow_the_python_app() {
        let t = tray_texts(Some(("Song", "Band")), true, Repeat::Album, true);
        assert_eq!(t.now, "Song — Band");
        assert_eq!(t.tooltip, "Song — Band");
        assert_eq!(t.play, "Pause");
        assert_eq!(t.repeat, "Repeat: whole album");
        assert_eq!(t.window, "Hide player");

        let t = tray_texts(Some(("Song", "Unknown Artist")), false, Repeat::Off, false);
        assert_eq!(t.now, "Song");
        assert_eq!(
            (t.play, t.repeat, t.window),
            ("Play", "Repeat: off", "Show player")
        );

        let t = tray_texts(None, false, Repeat::Track, true);
        assert_eq!(t.now, "Nothing playing");
        assert_eq!(t.tooltip, "LRC Open Player");
        assert_eq!(t.repeat, "Repeat: this track");

        let long = "あ".repeat(70);
        let t = tray_texts(Some((&long, "")), false, Repeat::Off, true);
        assert_eq!(t.now.chars().count(), 58);
        assert!(t.now.ends_with("あ…"));
        assert_eq!(t.tooltip.chars().count(), 70);
        let longer = "x".repeat(200);
        assert_eq!(
            tray_texts(Some((&longer, "")), false, Repeat::Off, true)
                .tooltip
                .len(),
            120
        );
    }

    #[test]
    fn media_fields() {
        assert_eq!(media_clean("Unknown Album", "Unknown Album"), "");
        assert_eq!(media_clean("", "Unknown Album"), "");
        assert_eq!(media_clean("Blue", "Unknown Album"), "Blue");
        assert_eq!(media_track_number("3/12"), 3);
        assert_eq!(media_track_number("07"), 7);
        assert_eq!(media_track_number("-"), 0);
        assert_eq!(media_track_number(""), 0);
        assert_eq!(timeline(5000, 3000), (3000, 3000));
    }

    #[test]
    fn timeline_updates_once_a_second_unless_forced() {
        let mut t = TimelineThrottle::default();
        let start = Instant::now();
        assert!(t.ready(start, false));
        assert!(!t.ready(start + Duration::from_millis(500), false));
        assert!(t.ready(start + Duration::from_millis(600), true));
        assert!(!t.ready(start + Duration::from_millis(1500), false));
        assert!(t.ready(start + Duration::from_millis(1700), false));
    }

    #[test]
    fn one_error_message_at_a_time_and_no_repeats_within_ten_seconds() {
        let mut g = ErrorGate::default();
        let t = Instant::now();
        assert!(g.open("boom", t));
        assert!(!g.open("other", t), "one at a time");
        g.closed();
        assert!(
            !g.open("boom", t + Duration::from_secs(9)),
            "same one too soon"
        );
        assert!(g.open("other", t + Duration::from_secs(9)));
        g.closed();
        assert!(g.open("boom", t + Duration::from_secs(20)));
        assert_eq!(error_summary(&"e".repeat(500)).len(), 400);
    }
}
