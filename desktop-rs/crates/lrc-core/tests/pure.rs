//! Compares lrc-core with the Python player on the cases in `fixtures/pure.json`.
//!
//! That file is written by `tools/make_fixtures.py pure`, which runs the functions in
//! `desktop/lrcplayer.py` itself. If a case fails here, the Rust port disagrees with
//! the Python app.

use std::path::Path;
use std::sync::LazyLock;
use std::sync::atomic::AtomicBool;

use base64::Engine;
use lrc_core::library::{album_sort_key, clean_query_string};
use lrc_core::lrc::{find_stamps, fmt_lrc_time, parse, same_text};
use lrc_core::lrclib::{Query, fetch_with};
use lrc_core::tags::TrackTags;
use lrc_core::text::decode_lyrics_bytes_with;
use serde_json::Value;

static PURE: LazyLock<Value> = LazyLock::new(|| {
    let text = include_str!("fixtures/pure.json");
    serde_json::from_str(text).expect("pure.json is valid JSON")
});

fn cases(name: &str) -> &'static [Value] {
    PURE[name].as_array().expect("a list of cases")
}

fn s(v: &Value) -> &str {
    v.as_str().expect("a string")
}

/// Runs every case and reports all failures at once, not just the first.
fn check_all(name: &str, mut check: impl FnMut(&Value) -> Option<String>) {
    let failures: Vec<String> = cases(name).iter().filter_map(&mut check).collect();
    assert!(
        failures.is_empty(),
        "{} of {} `{name}` cases differ from the Python app:\n{}",
        failures.len(),
        cases(name).len(),
        failures.join("\n")
    );
}

#[test]
fn decode_lyrics_bytes_matches_python() {
    let b64 = base64::engine::general_purpose::STANDARD;
    check_all("decode", |c| {
        let data = b64.decode(s(&c["data"])).unwrap();
        let cp = c["code_page"].as_u64().unwrap() as u16;
        let got = decode_lyrics_bytes_with(&data, cp);
        (got != s(&c["text"])).then(|| {
            format!(
                "cp{cp} {data:02x?}:\n  python {:?}\n  rust   {got:?}",
                s(&c["text"])
            )
        })
    });
}

#[test]
fn parse_matches_python() {
    check_all("parse", |c| {
        let got = parse(s(&c["content"]));
        let want_lines: Vec<(i64, &str, &str)> = c["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| (l["time_ms"].as_i64().unwrap(), s(&l["main"]), s(&l["sub"])))
            .collect();
        let got_lines: Vec<(i64, &str, &str)> = got
            .lines
            .iter()
            .map(|l| (l.time_ms, l.main.as_str(), l.sub.as_str()))
            .collect();
        let synced = c["synced"].as_bool().unwrap();
        (got_lines != want_lines || got.synced != synced).then(|| {
            format!(
                "{:?}:\n  python {synced} {want_lines:?}\n  rust   {} {got_lines:?}",
                s(&c["content"]),
                got.synced
            )
        })
    });
}

#[test]
fn stamps_match_python() {
    check_all("stamps", |c| {
        let (ms, end) = find_stamps(s(&c["line"]));
        let want: Vec<i64> = c["ms"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        let want_end = c["end"].as_u64().unwrap() as usize;
        (ms != want || end != want_end).then(|| {
            format!(
                "{:?}: python {want:?} {want_end}, rust {ms:?} {end}",
                s(&c["line"])
            )
        })
    });
}

#[test]
fn fmt_lrc_time_matches_python() {
    check_all("fmt_lrc_time", |c| {
        let ms = c["ms"].as_i64().unwrap();
        let got = fmt_lrc_time(ms);
        (got != s(&c["text"])).then(|| format!("{ms}: python {}, rust {got}", s(&c["text"])))
    });
}

#[test]
fn clean_query_string_matches_python() {
    check_all("clean_query", |c| {
        let from_filename = c["from_filename"].as_bool().unwrap();
        let got = clean_query_string(s(&c["text"]), from_filename);
        (got != s(&c["cleaned"])).then(|| {
            format!(
                "{:?} (from file name: {from_filename}): python {:?}, rust {got:?}",
                s(&c["text"]),
                s(&c["cleaned"])
            )
        })
    });
}

#[test]
fn same_text_matches_python() {
    check_all("same_text", |c| {
        let got = same_text(s(&c["a"]), s(&c["b"]));
        let want = c["same"].as_bool().unwrap();
        (got != want).then(|| {
            format!(
                "{:?} vs {:?}: python {want}, rust {got}",
                s(&c["a"]),
                s(&c["b"])
            )
        })
    });
}

#[test]
fn album_order_matches_python() {
    check_all("album_order", |c| {
        let root = c["root"].as_str().map(Path::new);
        let songs: Vec<(&str, TrackTags)> = c["songs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|song| {
                let tags = TrackTags {
                    title: None,
                    track: song["track"].as_u64(),
                    disc: song["disc"].as_u64(),
                };
                (s(&song["path"]), tags)
            })
            .collect();
        let mut order: Vec<usize> = (0..songs.len()).collect();
        order.sort_by_key(|&i| album_sort_key(Path::new(songs[i].0), &songs[i].1, root));
        let want: Vec<usize> = c["order"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect();
        (order != want).then(|| {
            let names = |o: &[usize]| o.iter().map(|&i| songs[i].0).collect::<Vec<_>>();
            format!(
                "root {root:?}:\n  python {:?}\n  rust   {:?}",
                names(&want),
                names(&order)
            )
        })
    });
}

#[test]
fn lrclib_lookup_matches_python() {
    check_all("lrclib", |c| {
        let query = Query::new(
            s(&c["title"]),
            s(&c["artist"]),
            c["duration_sec"].as_i64().unwrap(),
            c["title_from_tags"].as_bool().unwrap(),
        );
        let mut urls = Vec::new();
        let result = fetch_with(&query, &AtomicBool::new(false), |url, _timeout| {
            urls.push(url.to_owned());
            let body = if url.contains("/api/get?") {
                &c["get"]
            } else {
                &c["search"]
            };
            (!body.is_null()).then(|| body.clone())
        })
        .expect("not cancelled");
        let want = &c["result"];
        let want_urls: Vec<&str> = c["urls"].as_array().unwrap().iter().map(s).collect();
        let same = result.lyrics == s(&want["lyrics"])
            && result.message == s(&want["message"])
            && result.trusted == want["trusted"].as_bool().unwrap()
            && result.lrclib_duration_sec == want["lrclib_duration_sec"].as_i64().unwrap()
            && urls == want_urls
            && query.title == s(&c["query_title"])
            && query.artist == s(&c["query_artist"]);
        (!same).then(|| {
            format!(
                "{}:\n  python {want} {want_urls:?}\n  rust   {result:?} {urls:?}",
                s(&c["name"])
            )
        })
    });
}

#[test]
fn palettes_match_python() {
    check_all("palette", |c| {
        let theme = s(&c["theme"]);
        let accent = s(&c["accent"]);
        let got = lrc_core::theme::build_palette(theme, accent);
        let want = c["palette"].as_object().unwrap();
        let mut diffs = Vec::new();
        for (name, value) in want {
            match name.as_str() {
                "dark" => {
                    if value.as_bool() != Some(got.dark) {
                        diffs.push(format!("dark: python {value}, rust {}", got.dark));
                    }
                }
                "inactive_opacity" | "viz_alpha" => {
                    let mine = if name == "inactive_opacity" {
                        got.inactive_opacity
                    } else {
                        got.viz_alpha
                    };
                    if value.as_f64() != Some(mine) {
                        diffs.push(format!("{name}: python {value}, rust {mine}"));
                    }
                }
                _ => {
                    let mine = got
                        .colors
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, c)| c.as_str());
                    if mine != value.as_str() {
                        diffs.push(format!("{name}: python {value}, rust {mine:?}"));
                    }
                }
            }
        }
        // Nothing extra on the Rust side either.
        for (name, _) in &got.colors {
            if !want.contains_key(*name) {
                diffs.push(format!("{name}: not in the Python palette"));
            }
        }
        (!diffs.is_empty()).then(|| format!("{theme}/{accent}:\n  {}", diffs.join("\n  ")))
    });
}

#[test]
fn window_texts_match_python() {
    let cases = &PURE["window_text"];
    let mut failures = Vec::new();
    for c in cases["format_time"].as_array().unwrap() {
        let ms = c["ms"].as_u64().unwrap();
        let got = lrc_core::window::format_time(ms);
        if got != s(&c["text"]) {
            failures.push(format!(
                "format_time({ms}): python {}, rust {got}",
                s(&c["text"])
            ));
        }
    }
    for c in cases["offset"].as_array().unwrap() {
        let ms = c["ms"].as_i64().unwrap();
        let got = lrc_core::window::offset_text(ms);
        if got != s(&c["text"]) {
            failures.push(format!(
                "offset_text({ms}): python {}, rust {got}",
                s(&c["text"])
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
