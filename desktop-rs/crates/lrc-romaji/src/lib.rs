//! Japanese lyrics to Hepburn romaji: `RomajiConverter` and `_display_for` in
//! lrcplayer.py.
//!
//! The Python app uses cutlet, which reads each word with MeCab and UniDic. This crate
//! ports cutlet's rules (`romaji_tokens`, `romaji_word`, `map_kana` and its tables) and
//! feeds them Lindera's words instead of MeCab's. Lindera embeds its dictionary in the
//! binary: UniDic with the `unidic` feature (the default, the same dictionary family as
//! cutlet), or IPADIC with `ipadic` (what the Android app uses).
//!
//! Loading the dictionary takes a moment, so [`Romaji::load`] is meant to run on a
//! worker thread the first time romaji is switched on.

mod cutlet;
mod dict;
mod tables;

use std::collections::HashMap;
use std::sync::Mutex;

pub use dict::DICTIONARY;
pub use lrc_core::lrc::same_text;

/// The same character ranges as `_JP_CHARS`: kana, CJK ideographs and half-width kana.
pub fn has_japanese(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{3040}'..='\u{30ff}'
            | '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{ff66}'..='\u{ff9f}')
    })
}

/// The loaded word splitter and cutlet's rules, with a cache per line.
pub struct Romaji {
    tagger: dict::Tagger,
    cache: Mutex<HashMap<String, String>>,
}

impl Romaji {
    /// Loads the embedded dictionary (`RomajiConverter.load`). Slow the first time: call
    /// it off the UI thread.
    pub fn load() -> Result<Romaji, String> {
        Ok(Romaji {
            tagger: dict::Tagger::load()?,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// `RomajiConverter.convert`: romaji for a lyric line, or "" when it has no Japanese,
    /// the conversion fails, or the romaji is the line itself.
    pub fn convert(&self, text: &str) -> String {
        if !has_japanese(text) {
            return String::new();
        }
        if let Some(hit) = self.cache.lock().ok().and_then(|c| c.get(text).cloned()) {
            return hit;
        }
        let mut result = self.romaji(text).unwrap_or_default().trim().to_owned();
        if result == text {
            result.clear();
        }
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(text.to_owned(), result.clone());
        }
        result
    }

    /// `Cutlet.romaji(text)` with the app's settings (Hepburn, no foreign spellings,
    /// ASCII only). `None` where cutlet raises, which the app turns into "".
    pub fn romaji(&self, text: &str) -> Option<String> {
        if text.is_empty() {
            return Some(String::new());
        }
        let text = cutlet::normalize_text(text);
        let words = self.tagger.words(&text)?;
        cutlet::romaji(&words)
    }
}

/// `_display_for`: the (main, sub) a lyric card shows.
///
/// With romaji on, Japanese lines become romaji and a translation stays below. A paired
/// line that already is the romaji of the main line is dropped, so lyrics that carry
/// their own romaji don't show it twice. `convert` is [`Romaji::convert`].
pub fn display_for(
    enabled: bool,
    convert: impl Fn(&str) -> String,
    main: &str,
    sub: &str,
) -> (String, String) {
    if !enabled {
        return (main.to_owned(), sub.to_owned());
    }
    let or_self = |t: &str| {
        let r = convert(t);
        if r.is_empty() { t.to_owned() } else { r }
    };
    let main_r = or_self(main);
    let mut sub_r = if sub.is_empty() {
        String::new()
    } else {
        or_self(sub)
    };
    if !sub_r.is_empty() && same_text(&sub_r, &main_r) {
        sub_r.clear();
    }
    (main_r, sub_r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn japanese_ranges_match_the_python_regex() {
        assert!(has_japanese("ひらがな"));
        assert!(has_japanese("カタカナ"));
        assert!(has_japanese("漢字"));
        assert!(has_japanese("ｶﾅ"));
        assert!(!has_japanese("Hello, world ♪"));
        assert!(!has_japanese("한국어"));
    }

    #[test]
    fn display_for_keeps_lines_when_off() {
        let conv = |_: &str| "x".to_owned();
        assert_eq!(
            display_for(false, conv, "今", "now"),
            ("今".to_owned(), "now".to_owned())
        );
    }

    #[test]
    fn display_for_drops_a_romaji_line_that_repeats_the_main_line() {
        let conv = |t: &str| {
            if t == "今" {
                "Ima".to_owned()
            } else {
                String::new()
            }
        };
        assert_eq!(
            display_for(true, conv, "今", "ima!"),
            ("Ima".to_owned(), String::new())
        );
        assert_eq!(
            display_for(true, conv, "今", "Now"),
            ("Ima".to_owned(), "Now".to_owned())
        );
        assert_eq!(
            display_for(true, conv, "Hello", ""),
            ("Hello".to_owned(), String::new())
        );
    }
}
