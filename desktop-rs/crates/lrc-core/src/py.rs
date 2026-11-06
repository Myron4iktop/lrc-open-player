//! Small helpers that behave like the Python built-ins `lrcplayer.py` relies on.
//!
//! Rust's `trim`, `lines` and `to_lowercase` differ from Python's `strip`, `splitlines`
//! and `casefold` in a few characters. Lyrics files are full of odd characters, so the
//! port uses these instead wherever the Python code calls the built-in.

use std::sync::LazyLock;

use regex::Regex;

/// Python's `str.isspace()` for one character.
///
/// Rust's `char::is_whitespace` plus the four information separators (U+001C to U+001F),
/// which Python also counts as whitespace.
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()`.
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// Python's `str.rstrip()`.
pub fn rstrip(s: &str) -> &str {
    s.trim_end_matches(is_space)
}

/// Python's `str.splitlines()`: splits at every line boundary Python knows, and drops the
/// empty piece after a final line break.
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let boundary = matches!(
            c,
            '\n' | '\r'
                | '\u{0b}'
                | '\u{0c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if !boundary {
            continue;
        }
        out.push(&s[start..i]);
        let mut end = i + c.len_utf8();
        if c == '\r'
            && let Some(&(j, '\n')) = chars.peek()
        {
            chars.next();
            end = j + 1;
        }
        start = end;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// Python's `str.casefold()` (full Unicode case folding: "Straße" becomes "strasse").
pub fn casefold(s: &str) -> String {
    caseless::default_case_fold_str(s)
}

/// Python's `round()` for a float that becomes an int: halves go to the even neighbour.
pub fn round_half_even(x: f64) -> f64 {
    x.round_ties_even()
}

/// The value of a character Python's `\d` matches (Unicode category Nd), for `int()`.
///
/// Unicode puts every decimal digit in a run of ten, from 0 to 9, so the value is the
/// position in that run.
pub fn digit_value(c: char) -> Option<u32> {
    if c.is_ascii_digit() {
        return Some(c as u32 - '0' as u32);
    }
    if !is_digit(c) {
        return None;
    }
    let mut back = 0;
    let mut code = c as u32;
    while let Some(prev) = code.checked_sub(1).and_then(char::from_u32) {
        if !is_digit(prev) {
            break;
        }
        back += 1;
        code -= 1;
    }
    Some(back % 10)
}

fn matches_class(re: &Regex, c: char) -> bool {
    let mut buf = [0u8; 4];
    re.is_match(c.encode_utf8(&mut buf))
}

/// Python's `\d`: any decimal digit (Unicode category Nd), not only 0-9.
pub fn is_digit(c: char) -> bool {
    static ND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\p{Nd}$").unwrap());
    c.is_ascii_digit() || (!c.is_ascii() && matches_class(&ND, c))
}

/// Python's `\w`: letters, numbers and the underscore.
pub fn is_word(c: char) -> bool {
    static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\p{L}\p{N}_]$").unwrap());
    c.is_ascii_alphanumeric() || c == '_' || (!c.is_ascii() && matches_class(&WORD, c))
}

/// The digits of a `\d+` match as an ASCII number string, for parsing.
pub fn ascii_digits(s: &str) -> String {
    s.chars()
        .map(|c| match digit_value(c) {
            Some(d) => char::from_digit(d, 10).unwrap(),
            None => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_like_python() {
        assert_eq!(strip("\u{1f} a b \u{3000}\n"), "a b");
        assert_eq!(rstrip("  a  "), "  a");
    }

    #[test]
    fn splitlines_like_python() {
        assert_eq!(splitlines("a\r\nb\rc\nd"), ["a", "b", "c", "d"]);
        assert_eq!(splitlines("a\n"), ["a"]);
        assert_eq!(splitlines("a\n\nb"), ["a", "", "b"]);
        assert_eq!(
            splitlines("a\u{2028}b\u{85}c\u{0b}d\u{1e}e"),
            ["a", "b", "c", "d", "e"]
        );
        assert!(splitlines("").is_empty());
        assert_eq!(splitlines("\n"), [""]);
    }

    #[test]
    fn casefold_like_python() {
        assert_eq!(casefold("Straße"), "strasse");
        assert_eq!(casefold("ΣΊΣΥΦΟΣ"), casefold("σίσυφος"));
    }

    #[test]
    fn digits_like_python() {
        assert_eq!(digit_value('7'), Some(7));
        assert_eq!(digit_value('７'), Some(7));
        assert_eq!(digit_value('٣'), Some(3));
        assert_eq!(digit_value('𝟗'), Some(9)); // mathematical bold nine, runs of ten back to back
        assert_eq!(digit_value('a'), None);
        assert_eq!(digit_value('½'), None);
        assert!(is_word('é') && is_word('_') && is_word('日') && !is_word('-'));
    }

    #[test]
    fn rounds_half_to_even() {
        assert_eq!(round_half_even(0.5), 0.0);
        assert_eq!(round_half_even(1.5), 2.0);
        assert_eq!(round_half_even(2.5), 2.0);
        assert_eq!(round_half_even(-0.5), -0.0);
    }
}
