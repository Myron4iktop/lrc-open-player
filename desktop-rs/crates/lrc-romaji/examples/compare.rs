//! Compares this crate's romaji with the Python app's (cutlet), line by line.
//!
//! ```text
//! python tools/make_fixtures.py romaji "D:\Music" romaji.json
//! cargo run -p lrc-romaji --release --example compare -- romaji.json
//! cargo run -p lrc-romaji --release --no-default-features --features ipadic --example compare -- romaji.json
//! ```
//!
//! Prints how many lines come out identical, and lists the ones that differ.

use std::time::Instant;

use lrc_romaji::{DICTIONARY, Romaji, same_text};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: compare ROMAJI.json   (written by tools/make_fixtures.py romaji)");
        std::process::exit(2);
    };
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(2);
    });
    let data: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("{path} is not valid JSON: {e}");
        std::process::exit(2);
    });
    let lines = data["lines"].as_array().cloned().unwrap_or_default();

    let start = Instant::now();
    let romaji = Romaji::load().unwrap_or_else(|e| {
        eprintln!("the dictionary did not load: {e}");
        std::process::exit(1);
    });
    let load = start.elapsed();

    let start = Instant::now();
    let mut same = 0;
    let mut close = 0;
    let mut diffs = Vec::new();
    for line in &lines {
        let src = line["text"].as_str().unwrap_or_default();
        let want = line["romaji"].as_str().unwrap_or_default();
        let got = romaji.convert(src);
        if got == want {
            same += 1;
        } else {
            // What the card shows: the line itself when there is no romaji.
            let shown = |r: &str| {
                if r.is_empty() {
                    src.to_owned()
                } else {
                    r.to_owned()
                }
            };
            if same_text(&shown(&got), &shown(want)) {
                close += 1;
            }
            diffs.push((
                line["file"].as_str().unwrap_or_default().to_owned(),
                src,
                want,
                got,
            ));
        }
    }
    let convert = start.elapsed();

    let n = lines.len().max(1);
    let pct = |k: usize| 100.0 * k as f64 / n as f64;
    println!("Dictionary: {DICTIONARY}");
    println!(
        "Lines: {}   identical: {same} ({:.1}%)   identical apart from case, spaces and punctuation: {} ({:.1}%)",
        lines.len(),
        pct(same),
        same + close,
        pct(same + close),
    );
    println!(
        "Dictionary load: {} ms   converting every line: {} ms",
        load.as_millis(),
        convert.as_millis()
    );
    if !diffs.is_empty() {
        println!("\nLines that differ (line / cutlet / this crate):");
        for (file, src, want, got) in &diffs {
            println!("\n  {file}\n  {src}\n  cutlet: {want}\n  rust:   {got}");
        }
    }
}
