//! Matching a translation to synced lyrics (the "Translation alignment" part of
//! `lrcplayer.py`): section labels, clauses, the prompt that asks a language model to
//! match line numbers, and reading its answer back.
//!
//! The prompt is copied word for word from the Python app, and the tests compare every
//! function here with the Python code itself (`tools/make_fixtures.py pure`).

use std::collections::{BTreeMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::py;

/// Python's `\s` (Rust's `\s` leaves out the four information separators).
macro_rules! ws {
    () => {
        r"[\s\x1c-\x1f]"
    };
}

const HEADER_WORDS: &str = r"(?:verse|chorus|pre-?chorus|post-?chorus|bridge|intro|outro|hook|refrain|interlude|instrumental|break|repeat|coda|spoken|rap)";

static HDR_SQUARE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(concat!(r"^", ws!(), r"*[\[【].*[\]】]", ws!(), r"*$")).unwrap());
static HDR_PAREN: LazyLock<Regex> = LazyLock::new(|| {
    let s = ws!();
    Regex::new(&format!(
        r"(?i)^{s}*[(（][^)）]*\b{HEADER_WORDS}\b[^)）]*[)）]{s}*$|^{s}*[(（]{s}*[x×]{s}*\d+{s}*[)）]{s}*$"
    ))
    .unwrap()
});
static HDR_BARE: LazyLock<Regex> = LazyLock::new(|| {
    let s = ws!();
    Regex::new(&format!(
        r"(?i)^{s}*(?:repeat{s}+)?{HEADER_WORDS}(?:{s}*\d+)?(?:{s}*[x×]{s}*\d+)?{s}*:?{s}*$"
    ))
    .unwrap()
});

/// `_LRC_LEADING`: the timestamps at the start of a line, and the rest.
pub(crate) static LRC_LEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^((?:\[\d{1,3}:\d{1,2}(?:[.:,]\d{1,6})?\])+)(.*)$").unwrap());
/// `_LRC_META`: tags such as `[ar:Artist]`.
pub(crate) static LRC_META: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[[A-Za-z#]+:.*\]$").unwrap());

/// `is_section_header`: [Chorus], (Verse 2), Chorus x2 … are labels in a translation,
/// not sung lines. Plain parentheses are left alone, since "(come on)" is usually a
/// backing vocal.
pub fn is_section_header(line: &str) -> bool {
    HDR_SQUARE.is_match(line) || HDR_PAREN.is_match(line) || HDR_BARE.is_match(line)
}

/// `clean_translation_lines`: translation text as non-empty lines, without LRC
/// timestamps or tags (section labels are kept).
pub fn clean_translation_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in py::splitlines(text) {
        let mut line = py::strip(raw);
        if line.is_empty() || LRC_META.is_match(line) {
            continue;
        }
        if let Some(caps) = LRC_LEADING.captures(line) {
            line = py::strip(caps.get(2).map_or("", |m| m.as_str()));
        }
        if !line.is_empty() && line != "♪" {
            out.push(line.to_owned());
        }
    }
    out
}

/// `_TRAILING_PUNCT`: the marks a clause may end with.
pub const TRAILING_PUNCT: &[char] = &[',', ';', ':', '，', '、', '；', '：'];

/// `_CLAUSE_BREAK.split`: splits at whitespace that follows a clause mark.
fn split_at_clause_breaks(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<char> = None;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if py::is_space(c) && prev.is_some_and(|p| TRAILING_PUNCT.contains(&p)) {
            let mut end = i + c.len_utf8();
            let mut last = c;
            while let Some(&(j, d)) = chars.peek() {
                if !py::is_space(d) {
                    break;
                }
                end = j + d.len_utf8();
                last = d;
                chars.next();
            }
            out.push(&s[start..i]);
            start = end;
            prev = Some(last);
            continue;
        }
        prev = Some(c);
    }
    out.push(&s[start..]);
    out
}

/// `split_clauses`: "I'm provoked by the sun, my body heats" → ["I'm provoked by the
/// sun,", "my body heats"].
///
/// Long translation lines are shown to the model in parts, so when one translated line
/// covers two sung lines, each sung line can get its own part (without any text being
/// written).
pub fn split_clauses(line: &str) -> Vec<String> {
    if is_section_header(line) {
        return vec![line.to_owned()];
    }
    let parts: Vec<String> = split_at_clause_breaks(py::strip(line))
        .into_iter()
        .map(py::strip)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    if (2..=8).contains(&parts.len()) {
        parts
    } else {
        vec![line.to_owned()]
    }
}

/// `line_is_used`: true if every part of a translation line appears in some row (it
/// may be split over rows).
pub fn line_is_used(line: &str, texts: &[String]) -> bool {
    split_clauses(line).iter().all(|part| {
        let part = part.trim_end_matches(TRAILING_PUNCT);
        texts.iter().any(|t| t.contains(part))
    })
}

/// `match_in_order`: the simple method, the n-th sung line gets the n-th translation
/// line (labels skipped).
pub fn match_in_order(n_rows: usize, lines: &[String]) -> Vec<String> {
    let content: Vec<&String> = lines.iter().filter(|l| !is_section_header(l)).collect();
    (0..n_rows)
        .map(|i| content.get(i).map_or_else(String::new, |s| (*s).clone()))
        .collect()
}

/// One chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

/// The system message of the alignment request.
pub const ALIGN_SYSTEM: &str = "You align song lyrics with a translation of them. You never translate, rewrite or comment; you only match line numbers.";

/// `build_alignment_messages`: chat messages asking the model to match line numbers,
/// never to write text itself.
pub fn build_alignment_messages(originals: &[String], translations: &[String]) -> Vec<Message> {
    let o_block = originals
        .iter()
        .enumerate()
        .map(|(i, t)| format!("O{}: {t}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let mut t_rows = Vec::new();
    for (i, t) in translations.iter().enumerate() {
        let parts = split_clauses(t);
        if parts.len() == 1 {
            t_rows.push(format!("T{}: {t}", i + 1));
        } else {
            t_rows.extend(
                parts
                    .iter()
                    .enumerate()
                    .map(|(k, p)| format!("T{}.{}: {p}", i + 1, k + 1)),
            );
        }
    }
    let t_block = t_rows.join("\n");
    let n = originals.len();
    let user = format!(
        "Below are the lines of a song as they are sung (O1, O2, …) and a translation of the same song (T1, T2, …). The translation may split or join lines differently, leave out repeated parts, add section labels like [Chorus], or say \"(repeat)\".

For every sung line, give the translation line(s) that say the same thing.

Rules:
- Answer with exactly one line per sung line, from O1 to O{n}, in order, and nothing else.
- Format: O<number>: T<number>   or   O<number>: T<number> T<number>   or   O<number>: none
- Use several T numbers, in reading order, when one sung line is translated across several translation lines.
- Longer translation lines are shown in parts (T5.1, T5.2 …). When the whole line belongs to one sung line, just write T5. Only when one translation line covers two or more sung lines, give each sung line the part(s) that translate it.
- The same T line may be used for several sung lines, for example a chorus that is sung twice but translated once.
- Use \"none\" when nothing fits: vocalisations, ad-libs, or lines the translation leaves out.
- Never give section labels or notes such as [Chorus] or (x2) as a translation.

Example
Sung:
O1: 君の声が聞こえる
O2: 遠い空の向こうから
O3: ラララ
O4: 夜が明けて
O5: 光が差す
O6: 君の声が聞こえる
Translation:
T1: [Verse]
T2: I can hear your voice
T3: from beyond
T4: the distant sky
T5.1: The night is ending,
T5.2: and the light shines in
Answer:
O1: T2
O2: T3 T4
O3: none
O4: T5.1
O5: T5.2
O6: T2

Now the real song.
Sung:
{o_block}
Translation:
{t_block}
Answer:"
    );
    vec![
        Message {
            role: "system",
            content: ALIGN_SYSTEM.to_owned(),
        },
        Message {
            role: "user",
            content: user,
        },
    ]
}

static THINK_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<think>.*?</think>").unwrap());
static ANSWER_LINE: LazyLock<Regex> = LazyLock::new(|| {
    let s = ws!();
    Regex::new(&format!(
        r"(?im)^(?:{s}|[*>`•\-])*O{s}*(\d+){s}*\**{s}*(?:[:=→>]|-+>?|–){s}*(.*)$"
    ))
    .unwrap()
});
static T_REF: LazyLock<Regex> = LazyLock::new(|| {
    let s = ws!();
    Regex::new(&format!(
        r"(?i)T{s}*(\d+)(?:\.(\d+))?(?:{s}*(?:-|–|—|to){s}*T?{s}*(\d+)(?:\.(\d+))?)?"
    ))
    .unwrap()
});
/// `_NONE_WORDS`. Python's `\W` is "not a letter, number or underscore".
static NONE_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^[^\p{L}\p{N}_]*(?:none|nothing|n/?a|no match|skip|empty|—|-|–|∅)[^\p{L}\p{N}_]*$",
    )
    .unwrap()
});

/// Python's `str.lower().index` / `rindex`, mapped back onto the original text the way
/// the Python code does it: by character position.
fn char_slice_from(text: &str, chars: usize) -> &str {
    match text.char_indices().nth(chars) {
        Some((i, _)) => &text[i..],
        None => "",
    }
}

fn char_slice_to(text: &str, chars: usize) -> &str {
    match text.char_indices().nth(chars) {
        Some((i, _)) => &text[..i],
        None => text,
    }
}

fn char_pos(haystack: &str, byte: usize) -> usize {
    haystack[..byte].chars().count()
}

/// `strip_thinking`: removes the `<think>…</think>` reasoning that some models write
/// before their answer.
pub fn strip_thinking(text: &str) -> String {
    let mut text = THINK_BLOCK.replace_all(text, "").into_owned();
    let low = text.to_lowercase();
    if let Some(at) = low.rfind("</think>") {
        // The opening tag was part of the prompt template.
        let from = char_pos(&low, at) + "</think>".len();
        text = char_slice_from(&text, from).to_owned();
    }
    let low = text.to_lowercase();
    if let Some(at) = low.find("<think>") {
        // Cut off while still thinking.
        text = char_slice_to(&text, char_pos(&low, at)).to_owned();
    }
    text
}

/// `count_answer_lines`: how many "O<n>: …" lines the answer has so far.
pub fn count_answer_lines(text: &str) -> usize {
    ANSWER_LINE.find_iter(&strip_thinking(text)).count()
}

/// Python's `int()` of a `\d+` match. A number too big for `u64` is out of every range.
fn num(s: &str) -> u64 {
    py::ascii_digits(s).parse().unwrap_or(u64::MAX)
}

/// One reference to a translation line: (line, part), 1-based; part 0 is the whole line.
pub type Ref = (usize, usize);

/// What the model answered: the references for each sung row it answered, 1-based.
pub type Alignment = BTreeMap<usize, Vec<Ref>>;

/// The part of an answer line before any note: `re.split(r"[(\"“«#]|//", …)[0]`.
fn before_note(s: &str) -> &str {
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '(' | '"' | '“' | '«' | '#') {
            return &s[..i];
        }
        if c == '/' && chars.peek().is_some_and(|&(_, d)| d == '/') {
            return &s[..i];
        }
    }
    s
}

/// `parse_alignment`: the model's answer as {row: [(line, part)]}, 1-based; part 0
/// means the whole line.
///
/// Invalid numbers are dropped; a part number a line doesn't have counts as the whole
/// line.
pub fn parse_alignment(text: &str, n_rows: usize, translations: &[String]) -> Alignment {
    let parts_of: Vec<Vec<String>> = translations.iter().map(|t| split_clauses(t)).collect();
    let n_trans = translations.len() as u64;
    let parts_len = |line: u64| parts_of[(line - 1) as usize].len() as u64;

    let reference = |line: u64, part: u64| -> Option<Ref> {
        if !(1..=n_trans).contains(&line) {
            return None;
        }
        let n = parts_len(line);
        let part = if part != 0 && (1..=n).contains(&part) && n > 1 {
            part
        } else {
            0
        };
        Some((line as usize, part as usize))
    };

    let mut result = Alignment::new();
    for caps in ANSWER_LINE.captures_iter(&strip_thinking(text)) {
        let row = num(&caps[1]);
        if !(1..=n_rows as u64).contains(&row) {
            continue;
        }
        let rest = py::strip(before_note(caps.get(2).map_or("", |m| m.as_str())))
            .trim_matches(|c| matches!(c, '*' | '`' | ' '));
        let mut refs: Vec<Ref> = Vec::new();
        let mut add = |r: Option<Ref>| {
            if let Some(r) = r
                && !refs.contains(&r)
            {
                refs.push(r);
            }
        };

        if !NONE_WORDS.is_match(rest) {
            for tm in T_REF.captures_iter(rest) {
                let a_line = num(&tm[1]);
                let a_part = tm.get(2).map_or(0, |m| num(m.as_str()));
                let Some(b) = tm.get(3) else {
                    add(reference(a_line, a_part));
                    continue;
                };
                let b_line = num(b.as_str());
                let b_part_text = tm.get(4);
                let b_part = b_part_text.map_or(0, |m| num(m.as_str()));
                let a_valid = (1..=n_trans).contains(&a_line);
                if b_line == a_line && a_part != 0 && b_part != 0 && b_part >= a_part {
                    // T5.1-T5.3. Only the parts the line actually has, so an answer
                    // like "T1.1-T1.999999999" can't spin for minutes.
                    let max_part = if a_valid { parts_len(a_line) } else { 0 };
                    let mut k = a_part;
                    while k <= b_part.min(max_part) {
                        add(reference(a_line, k));
                        k += 1;
                    }
                } else if a_part != 0
                    && b_part_text.is_none()
                    && a_valid
                    && a_part < b_line
                    && b_line <= parts_len(a_line)
                {
                    // T5.1-3
                    for k in a_part..=b_line {
                        add(reference(a_line, k));
                    }
                } else if a_line < b_line && b_line <= a_line.saturating_add(6) {
                    // T2-T4
                    add(reference(a_line, a_part));
                    for line in a_line + 1..b_line {
                        add(reference(line, 0));
                    }
                    add(reference(b_line, b_part));
                } else {
                    add(reference(a_line, a_part));
                }
            }
        }
        result.insert(row as usize, refs);
    }
    result
}

/// `alignment_to_texts`: the translation text for each row; `Some("")` is no
/// translation, `None` means the model skipped the row.
///
/// Parts of a line the model left unused join the row that holds a neighbouring part of
/// the same line, so a translation line is never cut short by accident.
pub fn alignment_to_texts(
    mapping: &Alignment,
    n_rows: usize,
    translations: &[String],
) -> Vec<Option<String>> {
    let parts_of: Vec<Vec<String>> = translations.iter().map(|t| split_clauses(t)).collect();
    let mut rows = mapping.clone();

    let mut used: HashSet<Ref> = HashSet::new();
    for refs in rows.values() {
        for &(line, part) in refs {
            if part == 0 {
                used.extend((1..=parts_of[line - 1].len()).map(|k| (line, k)));
            } else {
                used.insert((line, part));
            }
        }
    }
    for (index, parts) in parts_of.iter().enumerate() {
        let line = index + 1;
        if parts.len() < 2 || !(1..=parts.len()).any(|k| used.contains(&(line, k))) {
            continue;
        }
        for k in 1..=parts.len() {
            if used.contains(&(line, k)) {
                continue;
            }
            for refs in rows.values_mut() {
                let placed = if let Some(at) = refs.iter().position(|&r| r == (line, k - 1)) {
                    refs.insert(at + 1, (line, k));
                    true
                } else if let Some(at) = refs.iter().position(|&r| r == (line, k + 1)) {
                    refs.insert(at, (line, k));
                    true
                } else {
                    false
                };
                if placed {
                    used.insert((line, k));
                    break;
                }
            }
        }
    }

    (1..=n_rows)
        .map(|row| {
            let refs = rows.get(&row)?;
            let mut pieces: Vec<&str> = Vec::new();
            let mut last = None;
            for &(line, part) in refs {
                if is_section_header(&translations[line - 1]) {
                    continue;
                }
                pieces.push(if part == 0 {
                    &translations[line - 1]
                } else {
                    &parts_of[line - 1][part - 1]
                });
                last = Some((line, part));
            }
            let mut text = pieces.join(" ");
            if let Some((line, part)) = last
                && part != 0
                && part < parts_of[line - 1].len()
            {
                // Ends mid-line: drop the dangling comma.
                text = text.trim_end_matches(TRAILING_PUNCT).to_owned();
            }
            Some(text)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn headers() {
        for h in [
            "[Chorus]",
            "【サビ】",
            "(Verse 2)",
            "Chorus x2",
            "repeat chorus:",
            "(x2)",
        ] {
            assert!(is_section_header(h), "{h}");
        }
        for l in ["(come on)", "I sing", "Bridge over water"] {
            assert!(!is_section_header(l), "{l}");
        }
    }

    #[test]
    fn clauses() {
        assert_eq!(
            split_clauses("I'm provoked by the sun, my body heats"),
            ["I'm provoked by the sun,", "my body heats"]
        );
        assert_eq!(split_clauses("one,two"), ["one,two"]);
        assert_eq!(split_clauses(" a,  b "), ["a,", "b"]);
    }

    #[test]
    fn parses_ranges() {
        let t = lines(&["[Verse]", "a", "b", "c", "x, y, z"]);
        let m = parse_alignment("O1: T2-T4\nO2: T5.1-3\nO3: none\nO9: T1", 3, &t);
        assert_eq!(m[&1], [(2, 0), (3, 0), (4, 0)]);
        assert_eq!(m[&2], [(5, 1), (5, 2), (5, 3)]);
        assert!(m[&3].is_empty());
        assert!(!m.contains_key(&9));
        assert_eq!(
            parse_alignment("O1: T5.1-T5.999999999999999999999", 1, &t)[&1],
            [(5, 1), (5, 2), (5, 3)]
        );
    }

    #[test]
    fn unused_parts_join_their_neighbour() {
        let t = lines(&["one, two, three"]);
        let mut m = Alignment::new();
        m.insert(1, vec![(1, 1)]);
        m.insert(2, vec![(1, 3)]);
        assert_eq!(
            alignment_to_texts(&m, 3, &t),
            [Some("one, two".to_owned()), Some("three".to_owned()), None]
        );
    }

    #[test]
    fn thinking() {
        assert_eq!(strip_thinking("<think>x</think>O1: T1"), "O1: T1");
        assert_eq!(strip_thinking("x</think>O1"), "O1");
        assert_eq!(strip_thinking("O1<think>hmm"), "O1");
    }
}
