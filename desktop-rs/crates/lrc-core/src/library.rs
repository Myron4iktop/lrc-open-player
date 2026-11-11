//! Albums and search text: `scan_album_folder`, `album_sort_key`, `_natural_key`, the
//! `AlbumScanWorker` loop and `clean_query_string` from `lrcplayer.py`.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::py;
use crate::tags::{TrackTags, read_track_tags};

/// The songs the player opens (`SUPPORTED_EXTS`).
pub const SUPPORTED_EXTS: [&str; 6] = [".flac", ".mp3", ".wav", ".m4a", ".ogg", ".opus"];

/// True if the name ends with one of [`SUPPORTED_EXTS`], in any case.
pub fn is_supported_audio(name: &str) -> bool {
    has_ext(name, &SUPPORTED_EXTS)
}

fn has_ext(name: &str, exts: &[&str]) -> bool {
    let lower = name.to_lowercase();
    exts.iter().any(|e| lower.ends_with(e))
}

/// One piece of a natural sort key: text, or the value of a run of digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeyPart {
    Text(String),
    Number(Number),
}

/// A whole number of any size, compared by value ("007" equals "7").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Number(String);

impl Number {
    fn new(digits: &str) -> Self {
        let ascii = py::ascii_digits(digits);
        let trimmed = ascii.trim_start_matches('0');
        Number(trimmed.to_owned())
    }
}

impl Ord for Number {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.cmp(&other.0))
    }
}

impl PartialOrd for Number {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Sorts "Disc 2" before "Disc 10" and "2 Song" before "10 Song" (`_natural_key`).
///
/// Like Python's `re.split(r"(\d+)", text)`: text and numbers alternate, starting and
/// ending with text (which may be empty), and text is compared case-folded.
pub fn natural_key(text: &str) -> Vec<KeyPart> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_digits = false;
    for c in text.chars() {
        let digit = py::is_digit(c);
        if digit != in_digits {
            parts.push(if in_digits {
                KeyPart::Number(Number::new(&current))
            } else {
                KeyPart::Text(py::casefold(&current))
            });
            current.clear();
            in_digits = digit;
        }
        current.push(c);
    }
    parts.push(if in_digits {
        KeyPart::Number(Number::new(&current))
    } else {
        KeyPart::Text(py::casefold(&current))
    });
    if in_digits {
        parts.push(KeyPart::Text(String::new()));
    }
    parts
}

/// Album order: folder by folder (CD1 before CD2), then disc, track number and file name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AlbumSortKey {
    pub folders: Vec<Vec<KeyPart>>,
    pub disc: u64,
    pub track: u64,
    pub name: Vec<KeyPart>,
}

/// The sort key of one song (`album_sort_key`). `root` is the album folder it was found
/// in, or `None` for a song opened on its own.
pub fn album_sort_key(file_path: &Path, tags: &TrackTags, root: Option<&Path>) -> AlbumSortKey {
    let folder = file_path.parent().unwrap_or(Path::new(""));
    let rel: PathBuf = match root {
        Some(root) if !root.as_os_str().is_empty() => match folder.strip_prefix(root) {
            Ok(rest) => rest.to_path_buf(),
            Err(_) => folder.to_path_buf(),
        },
        _ => folder.to_path_buf(),
    };
    let rel = rel.to_string_lossy();
    let folders = if rel.is_empty() || rel == "." {
        Vec::new()
    } else {
        rel.split(['\\', '/'])
            .filter(|p| !p.is_empty())
            .map(natural_key)
            .collect()
    };
    let name = file_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    AlbumSortKey {
        folders,
        disc: tags.disc.unwrap_or(1),
        track: tags.track.unwrap_or(999),
        name: natural_key(&name),
    }
}

/// A song found by a scan, with its title from the tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedSong {
    pub path: PathBuf,
    pub title: Option<String>,
}

/// Every file under `folder` (top-down, sub-folders in natural order) whose name ends
/// with one of `exts`. Folders that can't be listed are skipped, like `os.walk` does, and
/// links to folders are not followed.
fn walk(folder: &Path, exts: &[&str], found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // os.walk sorts an entry by what it points to, but only walks into real folders.
        if path.is_dir() {
            let is_link = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
            if !is_link {
                dirs.push((natural_key(&name), path));
            }
        } else if has_ext(&name, exts) {
            found.push(path);
        }
    }
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, dir) in dirs {
        walk(&dir, exts, found);
    }
}

/// Every song under `folder` in album order, with its title (`scan_album_folder`).
///
/// Reads each song's tags once. That takes a while for big folders or slow drives, so
/// the player runs it on a background thread. `cancelled` is asked before every song
/// (a cancelled scan returns nothing), and `progress` hears the count every 25 songs.
pub fn scan_album_folder(
    folder: &Path,
    exts: &[&str],
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize),
) -> Vec<ScannedSong> {
    let mut found = Vec::new();
    walk(folder, exts, &mut found);
    let mut entries = Vec::with_capacity(found.len());
    for (i, path) in found.into_iter().enumerate() {
        if cancelled() {
            return Vec::new();
        }
        let tags = read_track_tags(&path);
        entries.push((album_sort_key(&path, &tags, Some(folder)), path, tags.title));
        if i % 25 == 24 {
            progress(i + 1);
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
        .into_iter()
        .map(|(_, path, title)| ScannedSong { path, title })
        .collect()
}

/// What `AlbumScanWorker.run` does: every song of `folders` in album order, then the
/// loose `files` (sorted the same way when `sort_files` is set).
///
/// Returns `None` when cancelled; the worker sends nothing then.
pub fn scan_songs(
    folders: &[PathBuf],
    files: &[PathBuf],
    exts: &[&str],
    sort_files: bool,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize),
) -> Option<Vec<ScannedSong>> {
    let mut entries: Vec<ScannedSong> = Vec::new();
    for folder in folders {
        let base = entries.len();
        let songs = scan_album_folder(folder, exts, cancelled, &mut |count| progress(base + count));
        entries.extend(songs);
        if cancelled() {
            return None;
        }
    }
    let mut loose = Vec::new();
    for path in files {
        if cancelled() {
            return None;
        }
        let tags = read_track_tags(path);
        loose.push((album_sort_key(path, &tags, None), path.clone(), tags.title));
    }
    if sort_files {
        loose.sort_by(|a, b| a.0.cmp(&b.0));
    }
    entries.extend(
        loose
            .into_iter()
            .map(|(_, path, title)| ScannedSong { path, title }),
    );
    if cancelled() {
        return None;
    }
    Some(entries)
}

/// The length of a track-number prefix at the start of a file name, such as "01 ",
/// "01. ", "01 - " or "1-03 " (`_FILENAME_TRACK_PREFIX`).
///
/// The Python pattern `^(?:\d{1,2}[-.])?\d{1,3}(?:\s*[-._]\s*|\s+)(?=\S)` uses a
/// look-ahead, which the `regex` crate lacks, so this tries the same alternatives in the
/// same order as Python's backtracking matcher.
fn track_prefix_len(chars: &[char]) -> Option<usize> {
    let digits_at = |start: usize, max: usize| -> usize {
        chars[start..]
            .iter()
            .take(max)
            .take_while(|&&c| py::is_digit(c))
            .count()
    };
    let non_space_at = |i: usize| chars.get(i).is_some_and(|&c| !py::is_space(c));
    let spaces_at = |start: usize| {
        chars[start..]
            .iter()
            .take_while(|&&c| py::is_space(c))
            .count()
    };

    // The optional "1-" or "01." disc part: tried first with it, then without.
    let mut starts = Vec::new();
    let disc = digits_at(0, 2);
    for n in (1..=disc).rev() {
        if matches!(chars.get(n), Some('-' | '.')) {
            starts.push(n + 1);
        }
    }
    starts.push(0);

    for start in starts {
        let track = digits_at(start, 3);
        for n in (1..=track).rev() {
            let after = start + n;
            // \s*[-._]\s*
            let lead = spaces_at(after);
            for l in (0..=lead).rev() {
                let sep = after + l;
                if matches!(chars.get(sep), Some('-' | '.' | '_')) {
                    let trail = spaces_at(sep + 1);
                    for t in (0..=trail).rev() {
                        let end = sep + 1 + t;
                        if non_space_at(end) {
                            return Some(end);
                        }
                    }
                }
            }
            // \s+
            let run = spaces_at(after);
            for s in (1..=run).rev() {
                if non_space_at(after + s) {
                    return Some(after + s);
                }
            }
        }
    }
    None
}

static COPY_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\s\x1c-\x1f]*\(\d+\)\n?\z").unwrap());
static FORMAT_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\[(FLAC|MP3|WAV|HQ)\]").unwrap());

/// Search text for LRCLIB (`clean_query_string`).
///
/// Only a title that comes from the file name loses its track number ("01 - Song") and
/// copy suffix ("Song (2)"). Titles and artists from tags stay as they are: "7 Rings",
/// "99 Luftballons" and "50 Cent" really do start with a number.
pub fn clean_query_string(text: &str, from_filename: bool) -> String {
    let mut cleaned = text.to_owned();
    if from_filename {
        let chars: Vec<char> = cleaned.chars().collect();
        if let Some(n) = track_prefix_len(&chars) {
            cleaned = chars[n..].iter().collect();
        }
        // `$` in Python also matches before a final line break, which stays.
        if let Some(m) = COPY_SUFFIX.find(&cleaned) {
            let keep_newline = m.as_str().ends_with('\n');
            cleaned.truncate(m.start());
            if keep_newline {
                cleaned.push('\n');
            }
        }
    }
    let cleaned = FORMAT_TAG.replace_all(&cleaned, "");
    py::strip(&cleaned).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_key_splits_like_python() {
        use KeyPart::*;
        let n = |s: &str| Number(super::Number::new(s));
        assert_eq!(
            natural_key("Disc 10"),
            [Text("disc ".into()), n("10"), Text("".into())]
        );
        assert_eq!(
            natural_key("10"),
            [Text("".into()), n("10"), Text("".into())]
        );
        assert_eq!(natural_key(""), [Text("".into())]);
        assert!(natural_key("Disc 2") < natural_key("disc 10"));
        assert!(natural_key("2 Song") < natural_key("10 Song"));
        assert_eq!(natural_key("007"), natural_key("7"));
    }

    #[test]
    fn album_key_orders_folders_disc_track_name() {
        let tags = |disc, track| TrackTags {
            title: None,
            track,
            disc,
        };
        let root = Path::new("/music/Album");
        let a = album_sort_key(
            Path::new("/music/Album/CD2/01.mp3"),
            &tags(None, Some(1)),
            Some(root),
        );
        let b = album_sort_key(
            Path::new("/music/Album/CD10/01.mp3"),
            &tags(None, Some(1)),
            Some(root),
        );
        assert!(a < b);
        assert_eq!(a.folders, vec![natural_key("CD2")]);
        let top = album_sort_key(
            Path::new("/music/Album/x.mp3"),
            &tags(None, None),
            Some(root),
        );
        assert!(top.folders.is_empty());
        assert_eq!((top.disc, top.track), (1, 999));
        let loose = album_sort_key(Path::new("/music/Album/x.mp3"), &tags(None, None), None);
        assert_eq!(
            loose.folders,
            vec![natural_key("music"), natural_key("Album")]
        );
    }

    #[test]
    fn cleans_file_name_titles() {
        let c = |s| clean_query_string(s, true);
        assert_eq!(c("01 Song"), "Song");
        assert_eq!(c("01. Song"), "Song");
        assert_eq!(c("01 - Song"), "Song");
        assert_eq!(c("1-03 Song"), "Song");
        assert_eq!(c("01_Song"), "Song");
        assert_eq!(c("Song (2)"), "Song");
        assert_eq!(c("01 - "), "-");
        assert_eq!(c("1234 Song"), "1234 Song");
        assert_eq!(c("Song [FLAC]"), "Song");
        assert_eq!(c("Song [hq]"), "Song");
    }

    #[test]
    fn keeps_tag_titles() {
        let c = |s| clean_query_string(s, false);
        assert_eq!(c("7 Rings"), "7 Rings");
        assert_eq!(c("Song (2)"), "Song (2)");
        assert_eq!(c(" 50 Cent [MP3] "), "50 Cent");
    }

    #[test]
    fn supported_extensions() {
        assert!(is_supported_audio("A.FLAC"));
        assert!(is_supported_audio("b.opus"));
        assert!(!is_supported_audio("c.aac"));
    }
}
