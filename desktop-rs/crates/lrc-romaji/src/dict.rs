//! Lindera's words, in the shape cutlet reads from MeCab's nodes.
//!
//! cutlet reads these UniDic fields: `pos1`, `pos2`, `pron` and `kana` (the reading of
//! the word as written). Lindera's UniDic is unidic-mecab 2.1.2, the same dictionary as
//! the Python app's unidic-lite, but it has no `kana` field, so [`kana_from_unidic`] works
//! it out from the lemma's reading and the pronunciation. IPADIC has the reading, and
//! its parts of speech are renamed to UniDic's where cutlet's rules look at them.

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;

#[cfg(all(feature = "unidic", feature = "ipadic"))]
compile_error!("lrc-romaji: enable only one of the `unidic` and `ipadic` features");
#[cfg(not(any(feature = "unidic", feature = "ipadic")))]
compile_error!("lrc-romaji: enable the `unidic` or the `ipadic` feature");

/// The dictionary this build embeds.
#[cfg(feature = "unidic")]
pub const DICTIONARY: &str = "unidic";
#[cfg(all(feature = "ipadic", not(feature = "unidic")))]
pub const DICTIONARY: &str = "ipadic";

/// One word, with the fields cutlet uses (`fugashi.Node`).
#[derive(Debug, Clone, Default)]
pub(crate) struct Word {
    pub surface: String,
    pub pos1: String,
    pub pos2: String,
    pub pron: String,
    pub kana: String,
    pub is_unk: bool,
    /// Whitespace came before this word (`node.white_space` is not empty).
    pub white_space: bool,
}

pub(crate) struct Tagger {
    segmenter: Segmenter,
}

impl Tagger {
    pub fn load() -> Result<Tagger, String> {
        let dict =
            load_dictionary(&format!("embedded://{DICTIONARY}")).map_err(|e| e.to_string())?;
        Ok(Tagger {
            segmenter: Segmenter::new(Mode::Normal, dict, None),
        })
    }

    /// The words of `text` (`Tagger(text)`). Whitespace is skipped, as MeCab does, and
    /// marks the word after it.
    pub fn words(&self, text: &str) -> Option<Vec<Word>> {
        let mut tokens = self.segmenter.segment(text.into()).ok()?;
        let mut out = Vec::with_capacity(tokens.len());
        let mut prev_end = 0;
        for t in tokens.iter_mut() {
            let white_space = t.byte_start > prev_end;
            prev_end = t.byte_end;
            let surface = t.surface.to_string();
            let is_unk = t.word_id.is_unknown();
            let details: Vec<String> = t.details().iter().map(|s| (*s).to_owned()).collect();
            let mut w = word_from_details(&surface, &details);
            #[cfg(all(feature = "ipadic", not(feature = "unidic")))]
            if is_unk && !surface.chars().any(char::is_alphanumeric) {
                // IPADIC files unknown symbols (?, ♪) as nouns; UniDic as punctuation.
                w.pos1 = "補助記号".to_owned();
            }
            w.surface = surface;
            w.is_unk = is_unk;
            w.white_space = white_space;
            out.push(w);
        }
        Some(out)
    }
}

/// A part of speech as MeCab gives it, `*` included.
fn pos(d: &[String], i: usize) -> &str {
    d.get(i).map(String::as_str).unwrap_or("*")
}

/// A reading: `*` and a missing field are empty (`None` in fugashi).
fn field(d: &[String], i: usize) -> &str {
    match d.get(i).map(String::as_str) {
        None | Some("*") => "",
        Some(s) => s,
    }
}

/// UniDic (unidic-mecab 2.1.2): pos1, pos2, pos3, pos4, cType, cForm, lForm, lemma, orth,
/// pron, orthBase, pronBase, goshu, …
#[cfg(feature = "unidic")]
fn word_from_details(surface: &str, d: &[String]) -> Word {
    let orth = match field(d, 8) {
        "" => surface,
        o => o,
    };
    let pron = field(d, 9);
    Word {
        pos1: pos(d, 0).to_owned(),
        pos2: pos(d, 1).to_owned(),
        pron: pron.to_owned(),
        kana: kana_from_unidic(orth, field(d, 6), field(d, 10), pron),
        ..Word::default()
    }
}

/// IPADIC: pos1, pos2, pos3, pos4, cType, cForm, base, reading, pron.
#[cfg(all(feature = "ipadic", not(feature = "unidic")))]
fn word_from_details(surface: &str, d: &[String]) -> Word {
    let pos2 = pos(d, 1);
    // cutlet's rules name UniDic's parts of speech; these are IPADIC's for the same words.
    let pos1 = match (pos(d, 0), pos2) {
        (_, "接尾") => "接尾辞",
        // てる, でる, ちゃう: UniDic's auxiliaries, IPADIC's dependent verbs.
        ("動詞", "非自立") if surface.starts_with(['て', 'で', 'ち', 'じ']) => "助動詞",
        ("記号", "アルファベット") => "記号",
        ("記号", _) => "補助記号",
        ("接頭詞", _) => "接頭辞",
        (p, _) => p,
    };
    Word {
        pos1: pos1.to_owned(),
        pos2: pos2.to_owned(),
        pron: field(d, 8).to_owned(),
        kana: field(d, 7).to_owned(),
        ..Word::default()
    }
}

fn is_kana(c: char) -> bool {
    matches!(c, 'ぁ'..='ゖ' | 'ァ'..='ヺ' | 'ー')
}

pub(crate) fn to_katakana(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'ぁ'..='ゖ' => char::from_u32(c as u32 + 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

fn vowel_of(c: char) -> Option<char> {
    const ROWS: [(&str, char); 5] = [
        ("アカサタナハマヤラワガザダバパァャヮ", 'ア'),
        ("イキシチニヒミリギジヂビピィ", 'イ'),
        ("ウクスツヌフムユルグズヅブプゥュヴ", 'ウ'),
        ("エケセテネヘメレゲゼデベペェ", 'エ'),
        ("オコソトノホモヨロヲゴゾドボポォョ", 'オ'),
    ];
    ROWS.iter()
        .find(|(row, _)| row.contains(c))
        .map(|&(_, v)| v)
}

/// Whether `kana` (the spelling) can be read as `pron` (the pronunciation): the same
/// length, with long vowels written out, and particles and ぢ/づ spelled as written.
fn reads_as(kana: &[char], pron: &[char]) -> bool {
    kana.len() == pron.len()
        && kana.iter().zip(pron).all(|(&k, &p)| {
            k == p
                || (p == 'ー' && "アイウエオァィゥェォ".contains(k))
                || matches!(
                    (k, p),
                    ('ハ', 'ワ') | ('ヒ', 'イ') | ('フ', 'ウ') | ('ヘ', 'エ') | ('ホ', 'オ')
                )
                || matches!((k, p), ('ヲ', 'オ') | ('ヅ', 'ズ') | ('ヂ', 'ジ'))
        })
}

/// The pronunciation with its long vowels written the usual way: おう, えい, and the
/// vowel itself after the other rows.
fn spell_pron(pron: &[char]) -> Vec<char> {
    let mut out: Vec<char> = Vec::with_capacity(pron.len());
    for &c in pron {
        if c == 'ー'
            && let Some(v) = out.last().copied().and_then(vowel_of)
        {
            out.push(match v {
                'オ' => 'ウ',
                'エ' => 'イ',
                v => v,
            });
            continue;
        }
        out.push(c);
    }
    out
}

/// UniDic's `kana` field (the reading as written), which unidic-mecab leaves out.
///
/// Kana words are their own reading. Otherwise the lemma's reading (`lForm`) gets the
/// inflected ending: 消える → キエル, so 消え → キエ. When that doesn't fit the
/// pronunciation, the pronunciation is spelled out instead (センセー → センセイ), keeping
/// any kana the word ends with. Checked against every word of the dictionary that
/// unidic-lite also has: 99.3% come out the same as its `kana` field, and the rest are
/// old spellings (追ふ, 伺は) that lyrics hardly use.
#[cfg_attr(not(feature = "unidic"), allow(dead_code))]
pub(crate) fn kana_from_unidic(orth: &str, lform: &str, orth_base: &str, pron: &str) -> String {
    if !orth.is_empty() && orth.chars().all(is_kana) {
        return to_katakana(orth);
    }
    let pron_c: Vec<char> = pron.chars().collect();
    if !lform.is_empty() {
        let o: Vec<char> = orth.chars().collect();
        let b: Vec<char> = orth_base.chars().collect();
        let common = o.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let base_tail: Vec<char> = to_katakana(&b[common..].iter().collect::<String>())
            .chars()
            .collect();
        let tail: String = o[common..].iter().collect();
        let l: Vec<char> = lform.chars().collect();
        if l.ends_with(&base_tail) {
            let mut cand: Vec<char> = l[..l.len() - base_tail.len()].to_vec();
            cand.extend(to_katakana(&tail).chars());
            if pron_c.is_empty() || reads_as(&cand, &pron_c) {
                return cand.into_iter().collect();
            }
        }
    }
    if pron_c.is_empty() {
        return if lform.is_empty() {
            orth.to_owned()
        } else {
            lform.to_owned()
        };
    }
    let spelled = spell_pron(&pron_c);
    let o: Vec<char> = orth.chars().collect();
    let kana_tail = o
        .iter()
        .rev()
        .take_while(|&&c| is_kana(c) && c != 'ー')
        .count();
    if kana_tail > 0 && kana_tail < spelled.len() {
        let mut cand = spelled[..spelled.len() - kana_tail].to_vec();
        cand.extend(to_katakana(&o[o.len() - kana_tail..].iter().collect::<String>()).chars());
        if reads_as(&cand, &pron_c) {
            return cand.into_iter().collect();
        }
    }
    spelled.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unidic_kana_follows_the_inflection() {
        assert_eq!(kana_from_unidic("消え", "キエル", "消える", "キエ"), "キエ");
        assert_eq!(
            kana_from_unidic("行こう", "イク", "行く", "イコー"),
            "イコウ"
        );
        assert_eq!(
            kana_from_unidic("大きい", "オオキイ", "大きい", "オーキー"),
            "オオキイ"
        );
        assert_eq!(
            kana_from_unidic("先生", "センセイ", "先生", "センセー"),
            "センセイ"
        );
        assert_eq!(kana_from_unidic("そう", "ソウ", "そう", "ソー"), "ソウ");
        assert_eq!(kana_from_unidic("は", "ハ", "は", "ワ"), "ハ");
        assert_eq!(
            kana_from_unidic("東京", "トウキョウ", "東京", "トーキョー"),
            "トウキョウ"
        );
    }
}
