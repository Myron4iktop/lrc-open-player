//! Compares tag reading and lyrics embedding with the Python player on real songs.
//!
//! The songs and `media.json` are made by `tools/make_fixtures.py media DIR` (ffmpeg and
//! mutagen), which CI runs before the tests. Point `LRC_MEDIA_FIXTURES` at that folder.
//! Without it these tests are skipped, unless `LRC_REQUIRE_MEDIA_FIXTURES` is set (CI
//! sets it, so a missing folder fails there). With `LRC_EMBED_OUT` set, the songs with
//! embedded lyrics are kept in that folder for `make_fixtures.py check-embedded`.

use std::path::{Path, PathBuf};

use lrc_core::library::{SUPPORTED_EXTS, scan_album_folder};
use lrc_core::tags::{
    EmbedError, TrackInfo, extract_metadata_and_cover, read_track_tags, write_embedded_lyrics,
};
use serde_json::Value;

fn fixtures() -> Option<(PathBuf, Value)> {
    let Some(dir) = std::env::var_os("LRC_MEDIA_FIXTURES") else {
        assert!(
            std::env::var_os("LRC_REQUIRE_MEDIA_FIXTURES").is_none(),
            "LRC_MEDIA_FIXTURES is not set"
        );
        eprintln!("skipped: set LRC_MEDIA_FIXTURES to a folder from make_fixtures.py media");
        return None;
    };
    let dir = PathBuf::from(dir);
    let text = std::fs::read_to_string(dir.join("media.json")).expect("media.json");
    Some((dir, serde_json::from_str(&text).unwrap()))
}

fn files(data: &Value) -> &[Value] {
    data["files"].as_array().unwrap()
}

/// The fields of `info` that must match the Python app exactly.
fn info_json(info: &TrackInfo, root: &Path) -> Value {
    serde_json::json!({
        "title": info.title,
        "artist": info.artist,
        "album": info.album,
        "album_artist": info.album_artist,
        "track_number": info.track_number,
        "disc_number": info.disc_number,
        "genre": info.genre,
        "year": info.year,
        "duration_sec": info.duration_sec,
        "duration_str": info.duration_str,
        "mime_type": info.mime_type,
        "bitrate_kbps": info.bitrate_kbps,
        "sample_rate_khz": info.sample_rate_khz,
        "bits_per_sample": info.bits_per_sample,
        "channels": info.channels,
        "is_hd": info.is_hd,
        "file_path": info.file_path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"),
        "title_from_tags": info.title_from_tags,
    })
}

fn describe(path: &Path, root: &Path) -> Value {
    let meta = extract_metadata_and_cover(path);
    let tags = read_track_tags(path);
    serde_json::json!({
        "track_tags": {"title": tags.title, "track": tags.track, "disc": tags.disc},
        "lyrics": meta.lyrics,
        "cover": meta.cover.map(|c| vec![c.image.width(), c.image.height()]),
        "info": info_json(&meta.info, root),
    })
}

fn differences(name: &str, want: &Value, got: &Value, out: &mut Vec<String>) {
    for key in ["track_tags", "lyrics", "cover"] {
        if want[key] != got[key] {
            out.push(format!(
                "{name}: {key}\n    python {}\n    rust   {}",
                want[key], got[key]
            ));
        }
    }
    for (key, w) in want["info"].as_object().unwrap() {
        let g = &got["info"][key];
        let same = match (w.as_f64(), g.as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => w == g,
        };
        if !same {
            out.push(format!("{name}: info.{key}: python {w}, rust {g}"));
        }
    }
}

#[test]
fn tags_match_mutagen() {
    let Some((dir, data)) = fixtures() else {
        return;
    };
    let mut failures = Vec::new();
    for entry in files(&data) {
        let name = entry["file"].as_str().unwrap();
        let got = describe(&dir.join(name), &dir);
        differences(name, entry, &got, &mut failures);
    }
    assert!(
        failures.is_empty(),
        "lrc-core reads tags differently from the Python app:\n  {}",
        failures.join("\n  ")
    );
}

#[test]
fn album_scan_matches_python() {
    let Some((dir, data)) = fixtures() else {
        return;
    };
    let mut seen = Vec::new();
    let songs = scan_album_folder(&dir, &SUPPORTED_EXTS, &|| false, &mut |n| seen.push(n));
    let got: Vec<Value> = songs
        .iter()
        .map(|s| {
            let rel = s
                .path
                .strip_prefix(&dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            serde_json::json!([rel, s.title])
        })
        .collect();
    let want = data["scan"].as_array().unwrap();
    // Leave out what the embed test may have written into the same folder.
    let got: Vec<Value> = got
        .into_iter()
        .filter(|v| !v[0].as_str().unwrap().starts_with("embedded/"))
        .collect();
    assert_eq!(&got, want);
}

#[test]
fn embedded_lyrics_read_back() {
    let Some((dir, data)) = fixtures() else {
        return;
    };
    let text = data["embed_text"].as_str().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let out = std::env::var_os("LRC_EMBED_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| tmp.path().to_path_buf());
    let mut failures = Vec::new();
    let mut written = serde_json::Map::new();
    for entry in files(&data) {
        let name = entry["file"].as_str().unwrap();
        let target = out.join(name);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        // The folder covers come along, so the covers can be compared too.
        for (src, dst) in walk_siblings(&dir.join(name), &target) {
            std::fs::copy(src, dst).unwrap();
        }
        std::fs::copy(dir.join(name), &target).unwrap();
        let before = std::fs::read(&target).unwrap();

        match write_embedded_lyrics(&target, text) {
            Ok(()) => {}
            Err(e) if name == "broken.mp3" => {
                assert!(
                    matches!(e, EmbedError::Unsupported | EmbedError::Tag(_)),
                    "{e:?}"
                );
                assert_eq!(
                    std::fs::read(&target).unwrap(),
                    before,
                    "a failed embed changed the file"
                );
                std::fs::remove_file(&target).unwrap();
                continue;
            }
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        }
        let got = describe(&target, &out);
        // What was embedded is what the player shows, even in v1v2.mp3, whose comment
        // with a timestamp comes first (the Python app would show the comment).
        if got["lyrics"].as_str() != Some(text) {
            failures.push(format!("{name}: lyrics read back as {}", got["lyrics"]));
        }
        let mut want = entry.clone();
        want["lyrics"] = Value::from(text);
        // Embedding adds bytes, which moves the bitrate the Python app works out from the
        // file size for some formats; everything else must stay the same.
        want["info"]["bitrate_kbps"] = got["info"]["bitrate_kbps"].clone();
        differences(name, &want, &got, &mut failures);
        written.insert(name.to_owned(), got);
        // No temporary copy is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".embedding_"))
            .collect();
        if !leftovers.is_empty() {
            failures.push(format!("{name}: left {leftovers:?} behind"));
        }
    }
    // What lrc-core reads from each song, for make_fixtures.py check-embedded.
    std::fs::write(
        out.join("rust.json"),
        serde_json::to_string_pretty(&written).unwrap(),
    )
    .unwrap();
    assert!(
        failures.is_empty(),
        "embedding lyrics went wrong:\n  {}",
        failures.join("\n  ")
    );
}

/// The cover files next to a song, paired with where they go next to its copy.
fn walk_siblings(song: &Path, target: &Path) -> Vec<(PathBuf, PathBuf)> {
    let folder = song.parent().unwrap();
    ["cover.jpg", "cover.png", "folder.jpg", "folder.png"]
        .iter()
        .map(|n| (folder.join(n), target.parent().unwrap().join(n)))
        .filter(|(src, _)| src.exists())
        .collect()
}

#[test]
fn embedding_into_a_missing_file_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let err = write_embedded_lyrics(&dir.path().join("nope.mp3"), "x").unwrap_err();
    assert!(matches!(err, EmbedError::Io(_)), "{err:?}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn embedding_into_a_text_file_says_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.mp3");
    std::fs::write(&path, "not audio").unwrap();
    let err = write_embedded_lyrics(&path, "x").unwrap_err();
    assert_eq!(std::fs::read(&path).unwrap(), b"not audio");
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "the copy was removed"
    );
    // mutagen.File returns None for this, which the Python app reports the same way.
    if let EmbedError::Unsupported = err {
        assert_eq!(
            err.to_string(),
            "This audio format isn't supported for embedding."
        );
    }
}
