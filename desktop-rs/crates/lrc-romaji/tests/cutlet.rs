//! The sample lyrics in `fixtures/lrc`, against what the Python app's cutlet makes of
//! them (`fixtures/romaji.json`, from `tools/make_fixtures.py romaji`).

use lrc_romaji::{DICTIONARY, Romaji};

/// The share of lines that must come out exactly as cutlet's. UniDic is cutlet's own
/// dictionary; IPADIC reads some words differently (私 is watashi, not watakushi).
const REQUIRED: f64 = if cfg!(feature = "unidic") { 0.98 } else { 0.70 };

#[test]
fn sample_lines_match_cutlet() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/romaji.json")).expect("valid JSON");
    let lines = data["lines"].as_array().expect("a list of lines");
    assert!(lines.len() > 100);
    let romaji = Romaji::load().expect("the embedded dictionary loads");
    let mut same = 0;
    for line in lines {
        let text = line["text"].as_str().unwrap();
        let want = line["romaji"].as_str().unwrap();
        let got = romaji.convert(text);
        if got == want {
            same += 1;
        } else {
            println!("{text}\n  cutlet: {want}\n  rust:   {got}");
        }
    }
    let rate = same as f64 / lines.len() as f64;
    println!("{DICTIONARY}: {same} of {} lines identical", lines.len());
    assert!(rate >= REQUIRED, "only {:.1}% identical", rate * 100.0);
}

#[test]
fn convert_follows_the_python_converter() {
    let romaji = Romaji::load().unwrap();
    // No Japanese: nothing to show instead of the line.
    assert_eq!(romaji.convert("Hello world"), "");
    assert_eq!(romaji.convert(""), "");
    // Cached lines come back the same.
    let first = romaji.convert("そう僕らは今");
    assert_eq!(first, "Sou bokura wa ima");
    assert_eq!(romaji.convert("そう僕らは今"), first);
}
