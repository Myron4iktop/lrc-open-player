//! LRC timestamps and parsing lyrics into lines (`_LRC_STAMP`, `_fmt_lrc_time`,
//! `_stamp_to_ms` and the parsing half of `parse_and_display_lyrics` in `lrcplayer.py`).

use std::sync::LazyLock;

use regex::Regex;

use crate::py;

/// `[mm:ss]`, `[mm:ss.xx]`, `[mm:ss.xxx]`, and the `[mm:ss:xx]` / `[mm:ss,xx]` that some
/// tools write (`_LRC_STAMP`). Like Python's `\d`, any Unicode digit counts.
static LRC_STAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[(\d{1,3}):(\d{1,2}(?:[.:,]\d{1,6})?)\]").unwrap());

/// The text a stamped line without words shows.
pub const EMPTY_LINE_MARK: &str = "♪";

/// Two stamps at most this far apart are one line and its translation.
pub const PAIR_WINDOW_MS: i64 = 150;

/// `mm:ss.xx` as used inside LRC timestamps (`_fmt_lrc_time`).
pub fn fmt_lrc_time(ms: i64) -> String {
    let cs = py::round_half_even(ms.max(0) as f64 / 10.0) as i64;
    let (minutes, rest) = (cs / 6000, cs % 6000);
    let (seconds, hundredths) = (rest / 100, rest % 100);
    format!("{minutes:02}:{seconds:02}.{hundredths:02}")
}

/// Milliseconds of one stamp's two groups (`_stamp_to_ms`).
fn stamp_to_ms(minutes: &str, seconds: &str) -> i64 {
    let minutes: f64 = py::ascii_digits(minutes).parse().unwrap_or(0.0);
    let seconds: f64 = py::ascii_digits(seconds)
        .replace([':', ','], ".")
        .parse()
        .unwrap_or(0.0);
    py::round_half_even((minutes * 60.0 + seconds) * 1000.0) as i64
}

/// Every timestamp in a line, in milliseconds, with the byte offset where the last one ends.
pub fn find_stamps(line: &str) -> (Vec<i64>, usize) {
    let mut times = Vec::new();
    let mut end = 0;
    for caps in LRC_STAMP.captures_iter(line) {
        times.push(stamp_to_ms(&caps[1], &caps[2]));
        end = caps.get(0).unwrap().end();
    }
    (times, end)
}

/// One lyric line as the player shows it: the line, and the translation under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    /// When the line starts, in milliseconds. 0 for plain lyrics.
    pub time_ms: i64,
    pub main: String,
    /// The translation, or "".
    pub sub: String,
}

/// Lyrics parsed for display.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lyrics {
    /// True when the lines have timestamps.
    pub synced: bool,
    pub lines: Vec<LyricLine>,
}

/// Parses lyrics the way the player's lyrics view does.
///
/// Every line can carry several stamps; the text is whatever follows the last one, and
/// a stamped line without words becomes ♪. Stamped entries are sorted by time, and two
/// entries within 150 ms of each other become one line with its translation. Without
/// any stamps, each non-empty line is shown as it is (plain lyrics).
pub fn parse(content: &str) -> Lyrics {
    // A BOM (also seen inside tags) is never lyrics.
    let content = content.replace('\u{feff}', "");
    let mut raw: Vec<(i64, String)> = Vec::new();
    for raw_line in py::splitlines(&content) {
        let line = py::strip(raw_line);
        if line.is_empty() {
            continue;
        }
        let (times, end) = find_stamps(line);
        if times.is_empty() {
            continue;
        }
        let mut text = py::strip(&line[end..]);
        if text.is_empty() {
            text = EMPTY_LINE_MARK;
        }
        raw.extend(times.into_iter().map(|t| (t, text.to_owned())));
    }

    if raw.is_empty() {
        let lines = py::splitlines(&content)
            .into_iter()
            .map(py::strip)
            .filter(|l| !l.is_empty())
            .map(|l| LyricLine {
                time_ms: 0,
                main: l.to_owned(),
                sub: String::new(),
            })
            .collect();
        return Lyrics {
            synced: false,
            lines,
        };
    }

    // Stable, like Python's sort: same-time entries keep their file order.
    raw.sort_by_key(|e| e.0);
    let mut lines = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let (t1, ref text1) = raw[i];
        if i + 1 < raw.len() && (raw[i + 1].0 - t1).abs() <= PAIR_WINDOW_MS {
            lines.push(LyricLine {
                time_ms: t1,
                main: text1.clone(),
                sub: raw[i + 1].1.clone(),
            });
            i += 2;
        } else {
            lines.push(LyricLine {
                time_ms: t1,
                main: text1.clone(),
                sub: String::new(),
            });
            i += 1;
        }
    }
    Lyrics {
        synced: true,
        lines,
    }
}

/// True if two lines only differ in case, spacing or punctuation (`_same_text`).
pub fn same_text(a: &str, b: &str) -> bool {
    fn norm(t: &str) -> String {
        py::casefold(t)
            .chars()
            .filter(|&c| py::is_word(c) && c != '_')
            .collect()
    }
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(t: i64, main: &str, sub: &str) -> LyricLine {
        LyricLine {
            time_ms: t,
            main: main.into(),
            sub: sub.into(),
        }
    }

    #[test]
    fn formats_stamps() {
        assert_eq!(fmt_lrc_time(0), "00:00.00");
        assert_eq!(fmt_lrc_time(-5), "00:00.00");
        assert_eq!(fmt_lrc_time(61_234), "01:01.23");
        assert_eq!(fmt_lrc_time(61_235), "01:01.24");
        assert_eq!(fmt_lrc_time(61_225), "01:01.22"); // 6122.5 rounds to even
        assert_eq!(fmt_lrc_time(6_000_000), "100:00.00");
    }

    #[test]
    fn reads_every_stamp_style() {
        let (t, _) = find_stamps("[01:02]a");
        assert_eq!(t, [62_000]);
        let (t, _) = find_stamps("[1:02.5]a");
        assert_eq!(t, [62_500]);
        let (t, _) = find_stamps("[01:02:34]a");
        assert_eq!(t, [62_340]);
        let (t, _) = find_stamps("[01:02,345]a");
        assert_eq!(t, [62_345]);
        let (t, _) = find_stamps("[ar:Someone]");
        assert!(t.is_empty());
    }

    #[test]
    fn pairs_translations_and_marks_empty_lines() {
        let lrc = "[ar:Artist]\n[00:01.00]Hello\n[00:01.10]Bonjour\n[00:05.00]\n[00:03.00][00:07.00]Again\n";
        let parsed = parse(lrc);
        assert!(parsed.synced);
        assert_eq!(
            parsed.lines,
            [
                line(1000, "Hello", "Bonjour"),
                line(3000, "Again", ""),
                line(5000, "♪", ""),
                line(7000, "Again", ""),
            ]
        );
    }

    #[test]
    fn same_stamp_keeps_file_order() {
        let parsed = parse("[00:02.00]Line\n[00:02.00]Translation\n");
        assert_eq!(parsed.lines, [line(2000, "Line", "Translation")]);
    }

    #[test]
    fn pairing_is_greedy() {
        // 0, 100 and 200 ms: the first two pair up, the third stays alone.
        let parsed = parse("[00:00.00]a\n[00:00.10]b\n[00:00.20]c\n");
        assert_eq!(parsed.lines, [line(0, "a", "b"), line(200, "c", "")]);
        // 151 ms apart is too far.
        let parsed = parse("[00:00.000]a\n[00:00.151]b\n");
        assert_eq!(parsed.lines.len(), 2);
    }

    #[test]
    fn plain_lyrics_keep_every_line() {
        let parsed = parse("\u{feff}First\n\n  Second  \r\n[ar:x]\n");
        assert!(!parsed.synced);
        assert_eq!(
            parsed.lines,
            [
                line(0, "First", ""),
                line(0, "Second", ""),
                line(0, "[ar:x]", "")
            ]
        );
    }

    #[test]
    fn text_after_the_last_stamp() {
        let parsed = parse("[00:01.00]a[00:02.00]b\n");
        assert_eq!(parsed.lines, [line(1000, "b", ""), line(2000, "b", "")]);
    }

    #[test]
    fn same_text_ignores_case_and_punctuation() {
        assert!(same_text("Hello, World!", "hello world"));
        assert!(same_text("Straße", "STRASSE"));
        assert!(!same_text("Hello", "Hallo"));
        assert!(same_text("a_b", "ab"));
    }
}
