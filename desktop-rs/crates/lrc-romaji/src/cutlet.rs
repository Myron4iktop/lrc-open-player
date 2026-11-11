//! cutlet 0.5.2's romanizer (`cutlet/cutlet.py`), with the Python app's settings:
//! `Cutlet()` (Hepburn, `ensure_ascii`) and `use_foreign_spelling = False`.
//!
//! Each step names the cutlet code it ports. Where cutlet raises (a kana its table
//! doesn't have), these return `None`, and the app shows the line as it is.

use std::collections::HashMap;
use std::sync::OnceLock;

use unicode_normalization::UnicodeNormalization;

use crate::dict::Word;
use crate::tables::{EXCEPTIONS, HEPBURN};

const SUTEGANA: &str = "ゃゅょぁぃぅぇぉ";
const ODORI: &str = "々〃ゝゞヽゞ";
const UNVOICED: &str = "かきくけこさしすせそたちつてとはひふへほ";
const VOICED: &str = "がぎぐげござじずぜぞだぢづでどばびぶべぼ";

fn table() -> &'static HashMap<&'static str, &'static str> {
    static T: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    T.get_or_init(|| HEPBURN.iter().copied().collect())
}

fn exception(surface: &str) -> Option<&'static str> {
    EXCEPTIONS
        .iter()
        .find(|(k, _)| *k == surface)
        .map(|&(_, v)| v)
}

/// `normalize_text`: NFKC, which also does mojimoji's full-width Latin to half-width and
/// half-width katakana to full-width.
pub(crate) fn normalize_text(text: &str) -> String {
    text.nfkc().collect()
}

/// `jaconv.kata2hira`: ァ to ヶ, ヽ and ヾ become hiragana.
fn kata2hira(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'ァ'..='ヶ' | 'ヽ' | 'ヾ' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// MeCab's character classes in UniDic's char.def, for the first character.
#[derive(PartialEq)]
enum CharType {
    Alpha,
    Hiragana,
    Katakana,
    Other,
}

fn char_type(surface: &str) -> CharType {
    match surface.chars().next() {
        Some('A'..='Z' | 'a'..='z' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ') => CharType::Alpha,
        Some('\u{3041}'..='\u{309f}') => CharType::Hiragana,
        Some('\u{30a1}'..='\u{30ff}' | '\u{31f0}'..='\u{31ff}' | '\u{ff66}'..='\u{ff9d}') => {
            CharType::Katakana
        }
        _ => CharType::Other,
    }
}

/// Python's `str.isdigit` for the text after NFKC.
fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Python's `str.title`: a letter after a non-letter is upper case, the rest lower case.
fn title(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_cased = false;
    for c in s.chars() {
        let cased = c.is_lowercase() || c.is_uppercase();
        if cased && !prev_cased {
            out.extend(c.to_uppercase());
        } else if cased {
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
        prev_cased = cased;
    }
    out
}

/// `add_dakuten`
fn add_dakuten(kk: char) -> Option<char> {
    let i = UNVOICED.chars().position(|c| c == kk)?;
    VOICED.chars().nth(i)
}

fn get(key: &str) -> Option<&'static str> {
    table().get(key).copied()
}

fn get_pair(a: char, b: char) -> Option<&'static str> {
    let mut key = String::with_capacity(8);
    key.push(a);
    key.push(b);
    get(&key)
}

fn get_char(c: char) -> Option<&'static str> {
    let mut buf = [0; 4];
    get(c.encode_utf8(&mut buf))
}

/// `get_single_mapping`: one kana, given its neighbours.
fn single_mapping(pk: Option<char>, kk: char, nk: Option<char>) -> Option<String> {
    if ODORI.contains(kk) {
        if "ゝヽ".contains(kk) {
            return Some(pk.map(String::from).unwrap_or_default());
        }
        if "ゞヾ".contains(kk) {
            let Some(pk) = pk else {
                return Some(String::new());
            };
            return match add_dakuten(pk) {
                Some(vv) => get_char(vv).map(str::to_owned),
                None => Some(String::new()),
            };
        }
        return Some(String::new());
    }

    if let Some(pk) = pk
        && let Some(r) = get_pair(pk, kk)
    {
        return Some(r.to_owned());
    }
    if let Some(nk) = nk
        && get_pair(kk, nk).is_some()
    {
        return Some(String::new());
    }

    if let Some(nk) = nk
        && SUTEGANA.contains(nk)
    {
        if kk == 'っ' {
            return Some(String::new());
        }
        let base = get_char(kk)?;
        let small = get_char(nk)?;
        let mut stem: Vec<char> = base.chars().collect();
        stem.pop();
        return Some(stem.into_iter().collect::<String>() + small);
    }
    if SUTEGANA.contains(kk) {
        return Some(String::new());
    }

    if kk == 'ー' {
        return Some(match pk.and_then(get_char) {
            Some(r) => r.chars().last().map(String::from).unwrap_or_default(),
            None => "-".to_owned(),
        });
    }

    if kk == 'っ' {
        return match nk {
            Some('ち') => Some("t".to_owned()),
            Some(n) if "あいうえおっ".contains(n) => Some("-".to_owned()),
            Some(n) => get_char(n)?.chars().next().map(String::from),
            None => Some("っ".to_owned()),
        };
    }

    if kk == 'ん' {
        return Some(match nk {
            Some(n) if "あいうえおやゆよ".contains(n) => "n'".to_owned(),
            _ => "n".to_owned(),
        });
    }

    get_char(kk).map(str::to_owned)
}

/// `map_kana`
fn map_kana(kana: &str) -> Option<String> {
    let chars: Vec<char> = kana.chars().collect();
    let mut out = String::new();
    for (i, &kk) in chars.iter().enumerate() {
        let pk = i.checked_sub(1).map(|j| chars[j]);
        let nk = chars.get(i + 1).copied();
        out.push_str(&single_mapping(pk, kk, nk)?);
    }
    Some(out)
}

/// `romaji_word`
fn romaji_word(word: &Word) -> Option<String> {
    let s = word.surface.as_str();
    if let Some(e) = exception(s) {
        return Some(e.to_owned());
    }
    if is_digits(s) || s.is_ascii() {
        return Some(s.to_owned());
    }
    if s == "€" || s == "¥" {
        return Some(String::new());
    }
    if word.is_unk {
        let ct = char_type(s);
        if ct == CharType::Hiragana || ct == CharType::Katakana {
            return map_kana(&kata2hira(s));
        }
        return Some("?".repeat(s.chars().count()));
    }
    if word.pos1 == "補助記号" {
        return Some(get(s).unwrap_or("").to_owned());
    }
    if word.pos1 == "助詞" && word.pron == "ワ" {
        return Some("wa".to_owned());
    }
    if word.pos1 == "助詞" && word.pron == "エ" {
        return Some("e".to_owned());
    }
    if !word.kana.is_empty() {
        return map_kana(&kata2hira(&word.kana));
    }
    Some(s.to_owned())
}

struct Token {
    surface: String,
    space: bool,
}

/// `romaji_tokens` with `capitalize=True, title=False`, then `romaji`'s join and strip.
pub(crate) fn romaji(words: &[Word]) -> Option<String> {
    let mut out: Vec<Token> = Vec::with_capacity(words.len());

    for (wi, word) in words.iter().enumerate() {
        let nw = words.get(wi + 1);

        // handle possessive apostrophe as a special case
        if word.surface == "'"
            && nw.is_some_and(|n| char_type(&n.surface) == CharType::Alpha && !n.white_space)
            && !word.white_space
        {
            if let Some(po) = out.last_mut() {
                po.space = false;
            }
            out.push(Token {
                surface: word.surface.clone(),
                space: false,
            });
            continue;
        }

        // resolve split verbs / adjectives
        let mut roma = romaji_word(word)?;
        if let Some(first) = roma.chars().next()
            && let Some(po) = out.last_mut()
            && po.surface.ends_with('っ')
        {
            po.surface.pop();
            po.surface.push(first);
        }
        if word.pos2 == "固有名詞" {
            roma = title(&roma);
        }

        // handle punctuation with atypical spacing (`in` is a substring test in Python,
        // so an empty romaji counts as "(" here)
        if matches!(word.surface.as_str(), "「" | "『" | "「『")
            || matches!(roma.as_str(), "" | "(" | "[" | "([")
        {
            if let Some(po) = out.last_mut() {
                po.space = true;
            }
            out.push(Token {
                surface: roma,
                space: false,
            });
            continue;
        }
        if roma == "/" {
            out.push(Token {
                surface: roma,
                space: false,
            });
            continue;
        }

        // preserve spaces between ascii tokens
        if word.surface.is_ascii()
            && let Some(n) = nw
            && n.surface.is_ascii()
        {
            out.push(Token {
                surface: word.surface.clone(),
                space: n.white_space,
            });
            continue;
        }

        let no_space = word.pos1 == "接頭辞"
            || nw.is_some_and(|n| {
                n.pos1 == "補助記号"
                    || n.pos1 == "接尾辞"
                    || n.surface == ","
                    // `pos2 in ("接続助詞")` is a substring test
                    || "接続助詞".contains(n.pos2.as_str())
                    || (is_digits(&word.surface) && is_digits(&n.surface))
                    || (matches!(word.pos1.as_str(), "動詞" | "助動詞" | "形容詞")
                        && n.pos1 == "助動詞"
                        && n.surface != "です")
            });
        out.push(Token {
            surface: roma,
            space: !no_space,
        });
    }

    // remove any leftover っ
    for tok in &mut out {
        tok.surface = tok.surface.replace('っ', "");
    }

    // capitalize the first letter
    if let Some(first) = out.first_mut() {
        let mut chars = first.surface.chars();
        if let Some(c) = chars.next() {
            first.surface = c.to_uppercase().chain(chars).collect();
        }
    }

    let mut s = String::new();
    for tok in &out {
        s.push_str(&tok.surface);
        if tok.space {
            s.push(' ');
        }
    }
    Some(s.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kana_follow_cutlets_table() {
        assert_eq!(map_kana("きょう").as_deref(), Some("kyou"));
        assert_eq!(map_kana("がっこう").as_deref(), Some("gakkou"));
        assert_eq!(map_kana("まっちゃ").as_deref(), Some("matcha"));
        assert_eq!(map_kana("こんや").as_deref(), Some("kon'ya"));
        assert_eq!(map_kana("かれー").as_deref(), Some("karee"));
        assert_eq!(map_kana("しまっ").as_deref(), Some("shimaっ"));
        assert_eq!(map_kana("ふぁん").as_deref(), Some("fan"));
        assert_eq!(map_kana("を").as_deref(), Some("wo"));
    }

    #[test]
    fn python_title_case() {
        assert_eq!(title("tokyo"), "Tokyo");
        assert_eq!(title("shin'ya"), "Shin'Ya");
        assert_eq!(title("2nd"), "2Nd");
    }
}
