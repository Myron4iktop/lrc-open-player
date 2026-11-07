//! Compares the translation alignment, the AI server's texts and the two lyrics
//! editors with the Python player, on the cases `tools/make_fixtures.py pure` wrote
//! into `fixtures/pure.json` (sections `align`, `ai` and `editor`).

use std::sync::LazyLock;

use base64::Engine;
use lrc_core::ai::{self, Pricing};
use lrc_core::align;
use lrc_core::editor::{self, Card, Entry, TranslationEditor};
use serde_json::{Value, json};

static PURE: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("fixtures/pure.json")).expect("pure.json is valid JSON")
});

fn s(v: &Value) -> &str {
    v.as_str().expect("a string")
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .expect("a list")
        .iter()
        .map(|x| s(x).to_owned())
        .collect()
}

fn report(name: &str, failures: Vec<String>) {
    assert!(
        failures.is_empty(),
        "{} `{name}` cases differ from the Python app:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn line_rules_match_python() {
    let a = &PURE["align"];
    let sets: Vec<Vec<String>> = a["texts_sets"]
        .as_array()
        .unwrap()
        .iter()
        .map(strings)
        .collect();
    let mut fails = Vec::new();
    for c in a["lines"].as_array().unwrap() {
        let line = s(&c["line"]);
        let got = json!({
            "line": line,
            "header": align::is_section_header(line),
            "clauses": align::split_clauses(line),
            "used": sets.iter().map(|t| align::line_is_used(line, t)).collect::<Vec<_>>(),
        });
        if &got != c {
            fails.push(format!("python {c}\nrust   {got}"));
        }
    }
    for c in a["clean"].as_array().unwrap() {
        let got = align::clean_translation_lines(s(&c["text"]));
        if got != strings(&c["lines"]) {
            fails.push(format!(
                "clean {:?}: python {} rust {got:?}",
                c["text"], c["lines"]
            ));
        }
    }
    for c in a["match_in_order"].as_array().unwrap() {
        let got = align::match_in_order(c["n"].as_u64().unwrap() as usize, &strings(&c["lines"]));
        if got != strings(&c["texts"]) {
            fails.push(format!("match_in_order {}: rust {got:?}", c["n"]));
        }
    }
    for c in a["strip_thinking"].as_array().unwrap() {
        let got = align::strip_thinking(s(&c["text"]));
        if got != s(&c["out"]) {
            fails.push(format!(
                "strip_thinking {:?}: python {:?} rust {got:?}",
                c["text"], c["out"]
            ));
        }
    }
    report("align", fails);
}

#[test]
fn prompt_matches_python_word_for_word() {
    let mut fails = Vec::new();
    for c in PURE["align"]["messages"].as_array().unwrap() {
        let got: Vec<Value> = align::build_alignment_messages(
            &strings(&c["originals"]),
            &strings(&c["translations"]),
        )
        .into_iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
        if Value::Array(got.clone()) != c["messages"] {
            fails.push(format!(
                "python {}\nrust   {}",
                c["messages"],
                Value::Array(got)
            ));
        }
    }
    report("messages", fails);
}

#[test]
fn answers_are_read_like_python() {
    let a = &PURE["align"];
    let trans = strings(&a["translations"]);
    let mut fails = Vec::new();
    for c in a["answers"].as_array().unwrap() {
        let text = s(&c["text"]);
        let n = c["n_rows"].as_u64().unwrap() as usize;
        let mapping = align::parse_alignment(text, n, &trans);
        // Python's dict keeps the order rows first appear in; compare as sorted pairs.
        let mut want: Vec<(u64, Value)> = c["mapping"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p[0].as_u64().unwrap(), p[1].clone()))
            .collect();
        want.sort_by_key(|p| p.0);
        let got: Vec<(u64, Value)> = mapping
            .iter()
            .map(|(r, refs)| {
                (
                    *r as u64,
                    json!(refs.iter().map(|&(l, p)| [l, p]).collect::<Vec<_>>()),
                )
            })
            .collect();
        if got != want {
            fails.push(format!(
                "{text:?} ({n} rows):\n  python {want:?}\n  rust   {got:?}"
            ));
        }
        let texts = align::alignment_to_texts(&mapping, n, &trans);
        if json!(texts) != c["texts"] {
            fails.push(format!(
                "{text:?} texts: python {} rust {:?}",
                c["texts"], texts
            ));
        }
        if align::count_answer_lines(text) as u64 != c["count"].as_u64().unwrap() {
            fails.push(format!("{text:?} count"));
        }
    }
    report("answers", fails);
}

#[test]
fn ai_texts_match_python() {
    let a = &PURE["ai"];
    let mut fails = Vec::new();
    for c in a["urls"].as_array().unwrap() {
        let got = ai::normalize_ai_url(s(&c["url"]));
        if got != s(&c["out"]) {
            fails.push(format!(
                "url {:?}: python {:?} rust {got:?}",
                c["url"], c["out"]
            ));
        }
    }
    let b64 = base64::engine::general_purpose::STANDARD;
    for c in a["http_errors"].as_array().unwrap() {
        let body = b64.decode(s(&c["body"])).unwrap();
        let got = ai::http_error_text(
            c["status"].as_u64().unwrap() as u16,
            &body,
            s(&c["endpoint"]),
            s(&c["model"]),
        );
        if got != s(&c["text"]) {
            fails.push(format!(
                "http {}: python {:?} rust {got:?}",
                c["status"], c["text"]
            ));
        }
    }
    let models = strings(&a["models"]);
    let pricing: Pricing = a["pricing"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), (v[0].as_f64().unwrap(), v[1].as_f64().unwrap())))
        .collect();
    for c in a["costs"].as_array().unwrap() {
        let (text, error) = ai::cost_text(s(&c["typed"]), &models, &pricing);
        if text != s(&c["text"]) || error != c["error"].as_bool().unwrap() {
            fails.push(format!(
                "cost {:?}: python {} {} rust {text:?} {error}",
                c["typed"], c["text"], c["error"]
            ));
        }
    }
    report("ai", fails);
}

fn entry_json(e: &Entry) -> Value {
    json!({"text": e.text, "time": e.time, "sub": e.sub, "break": e.is_break})
}

#[test]
fn editors_match_python() {
    let e = &PURE["editor"];
    let mut fails = Vec::new();
    for c in e["sync"].as_array().unwrap() {
        let entries = editor::parse_text(s(&c["text"]));
        let got: Vec<Value> = entries.iter().map(entry_json).collect();
        if Value::Array(got.clone()) != c["entries"] {
            fails.push(format!(
                "parse {:?}:\n  python {}\n  rust   {}",
                c["text"],
                c["entries"],
                Value::Array(got)
            ));
        }
        if editor::entries_to_text(&entries) != s(&c["as_text"]) {
            fails.push(format!("as text {:?}", c["text"]));
        }
        let (lrc, missing) = editor::build_sync_lrc(&entries);
        if lrc != s(&c["lrc"]) || missing as u64 != c["missing"].as_u64().unwrap() {
            fails.push(format!(
                "lrc {:?}: python {:?} rust {lrc:?}",
                c["text"], c["lrc"]
            ));
        }
    }
    for c in e["translation"].as_array().unwrap() {
        let cards: Vec<Card> = c["cards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Card {
                time_ms: v[0].as_i64().unwrap(),
                main: s(&v[1]).into(),
                sub: s(&v[2]).into(),
            })
            .collect();
        let mut ed = TranslationEditor::new(cards);
        ed.texts = strings(&c["texts"]);
        if ed.build_lrc() != s(&c["lrc"]) {
            fails.push(format!(
                "translation lrc: python {:?} rust {:?}",
                c["lrc"],
                ed.build_lrc()
            ));
        }
    }
    for c in e["row_times"].as_array().unwrap() {
        let got = editor::row_time_text(c["ms"].as_i64().unwrap());
        if got != s(&c["text"]) {
            fails.push(format!(
                "time {}: python {:?} rust {got:?}",
                c["ms"], c["text"]
            ));
        }
    }
    report("editor", fails);
}
