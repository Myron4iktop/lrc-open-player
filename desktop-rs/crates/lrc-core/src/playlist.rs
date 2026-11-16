//! Playlist files: `.m3u8` and `.m3u` (not in the Python app).
//!
//! A playlist is a text file with one song per line. Lines that start with `#` are
//! comments; `#EXTINF` ones carry a song's length and title for players that show them.
//! The player writes `.m3u8` (UTF-8) with songs under the playlist's own folder as
//! relative paths, so a folder of music can be moved together with its playlist. It
//! reads both kinds; an `.m3u` in the system's code page is decoded as lyrics files are.

use std::path::{Path, PathBuf};

use crate::py;
use crate::text::{decode_lyrics_bytes, write_lrc};

/// The playlist files the player opens.
pub const PLAYLIST_EXTS: [&str; 2] = [".m3u8", ".m3u"];

/// A playlist bigger than this isn't read: no real one comes close.
pub const PLAYLIST_FILE_LIMIT: u64 = 16 * 1024 * 1024;

/// True if the name ends with one of [`PLAYLIST_EXTS`], in any case.
pub fn is_playlist(name: &str) -> bool {
    let lower = name.to_lowercase();
    PLAYLIST_EXTS.iter().any(|e| lower.ends_with(e))
}

/// `%41` and the like in a `file://` address, back to the bytes they stand for.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The songs a playlist's text names, in order. `folder` is the playlist's own folder,
/// which relative paths start from. Web addresses are left out: the player plays files.
///
/// `windows` says how paths are spelled: a `file:///C:/…` address loses the slash
/// before its drive, and elsewhere a `\` in a relative path (a playlist made on
/// Windows) is read as `/`.
pub fn parse_with(text: &str, folder: &Path, windows: bool) -> Vec<PathBuf> {
    let mut songs = Vec::new();
    for line in py::splitlines(text) {
        let line = py::strip(line).trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        let path = if let Some(rest) = lower
            .starts_with("file://")
            .then(|| &line["file://".len()..])
        {
            // file:///C:/Music/a.mp3, file://localhost/home/a.mp3 or file:///home/a.mp3
            let rest = rest.strip_prefix("localhost").unwrap_or(rest);
            let decoded = percent_decode(rest);
            let local = if windows {
                decoded.trim_start_matches('/').to_owned()
            } else {
                decoded
            };
            PathBuf::from(local)
        } else if line.contains("://") {
            continue;
        } else if windows {
            PathBuf::from(line)
        } else {
            PathBuf::from(line.replace('\\', "/"))
        };
        songs.push(if path.is_absolute() {
            path
        } else {
            folder.join(path)
        });
    }
    songs
}

/// [`parse_with`] for the system the player runs on.
pub fn parse(text: &str, folder: &Path) -> Vec<PathBuf> {
    parse_with(text, folder, cfg!(windows))
}

/// Reads a playlist file. The error is a sentence for a message box.
pub fn read(path: &Path) -> Result<Vec<PathBuf>, String> {
    let name = crate::text::base_name(path);
    let size = std::fs::metadata(path)
        .map_err(|e| format!("Couldn't read “{name}”: {e}"))?
        .len();
    if size > PLAYLIST_FILE_LIMIT {
        return Err(format!("“{name}” is too big to be a playlist."));
    }
    let data = std::fs::read(path).map_err(|e| format!("Couldn't read “{name}”: {e}"))?;
    let folder = path.parent().unwrap_or(Path::new(""));
    Ok(parse(&decode_lyrics_bytes(&data), folder))
}

/// The text of a playlist for `songs` (each with its title, if known), saved in `folder`.
///
/// Songs in `folder` or below it are written relative to it; the others with their
/// whole path. A known title goes in an `#EXTINF` line; the length there is -1
/// ("unknown"), as the player doesn't read every song's length to save a list.
pub fn to_text(songs: &[(PathBuf, Option<String>)], folder: &Path) -> String {
    let mut out = String::from("#EXTM3U\n");
    for (path, title) in songs {
        let title = title.as_deref().map(py::strip).unwrap_or_default();
        if !title.is_empty() {
            let one_line: String = title
                .chars()
                .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
                .collect();
            out.push_str(&format!("#EXTINF:-1,{one_line}\n"));
        }
        let shown = match path.strip_prefix(folder) {
            Ok(rest) if !folder.as_os_str().is_empty() && !rest.as_os_str().is_empty() => rest,
            _ => path.as_path(),
        };
        out.push_str(&shown.to_string_lossy());
        out.push('\n');
    }
    out
}

/// Saves `songs` as the playlist `path`: UTF-8, with the system's line endings.
/// The error is the short reason, such as "Permission denied".
pub fn write(path: &Path, songs: &[(PathBuf, Option<String>)]) -> Result<(), String> {
    let folder = path.parent().unwrap_or(Path::new(""));
    write_lrc(path, &to_text(songs, folder))
}

/// The name a playlist is saved under: `.m3u8` is added unless the name already ends
/// like a playlist.
pub fn with_playlist_ext(path: PathBuf) -> PathBuf {
    if is_playlist(&path.to_string_lossy()) {
        return path;
    }
    let mut name = path.into_os_string();
    name.push(".m3u8");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_playlists_by_name() {
        assert!(is_playlist("Road Trip.M3U8"));
        assert!(is_playlist("old.m3u"));
        assert!(!is_playlist("song.mp3"));
        assert!(!is_playlist("m3u8"));
    }

    #[test]
    fn reads_paths_comments_and_addresses() {
        let text = "\u{feff}#EXTM3U\r\n#EXTINF:215,Artist - One\r\nOne.flac\r\n\r\n  sub/Two.mp3  \r\n/abs/Three.ogg\nhttp://radio.example/stream\nfile:///abs/Four%20Song.wav\nfile://localhost/abs/Five.opus\nwin\\Six.m4a\n# a note\n";
        let songs = parse_with(text, Path::new("/lists"), false);
        assert_eq!(
            songs,
            [
                "/lists/One.flac",
                "/lists/sub/Two.mp3",
                "/abs/Three.ogg",
                "/abs/Four Song.wav",
                "/abs/Five.opus",
                "/lists/win/Six.m4a",
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            parse_with("", Path::new("/lists"), false),
            Vec::<PathBuf>::new()
        );
        assert_eq!(
            parse_with("#EXTM3U\n\n", Path::new("/lists"), false),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn windows_file_addresses_lose_their_leading_slash() {
        let songs = parse_with(
            "file:///C:/Music/A%26B.mp3\nFILE:///D:/x.flac\n",
            Path::new("lists"),
            true,
        );
        // Only Windows takes a drive for the start of a whole path; elsewhere this test
        // sees them joined to the playlist's folder.
        let folder = if cfg!(windows) { "" } else { "lists/" };
        assert_eq!(
            songs
                .iter()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .collect::<Vec<_>>(),
            [
                format!("{folder}C:/Music/A&B.mp3"),
                format!("{folder}D:/x.flac")
            ]
        );
        // A stray % is kept as it is.
        assert_eq!(percent_decode("100%.mp3"), "100%.mp3");
        assert_eq!(percent_decode("a%zzb%4"), "a%zzb%4");
        assert_eq!(percent_decode("%E3%81%82"), "あ");
    }

    #[test]
    fn writes_relative_paths_and_titles() {
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("Music");
        let elsewhere = dir.path().join("Elsewhere");
        let songs = vec![
            (
                music.join("Album").join("01.flac"),
                Some("First".to_owned()),
            ),
            (music.join("loose.mp3"), None),
            (elsewhere.join("far.ogg"), Some("  Far\nAway ".to_owned())),
            (music.join("empty title.wav"), Some(String::new())),
        ];
        let text = to_text(&songs, &music);
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            text,
            format!(
                "#EXTM3U\n#EXTINF:-1,First\nAlbum{sep}01.flac\nloose.mp3\n#EXTINF:-1,Far Away\n{}\nempty title.wav\n",
                elsewhere.join("far.ogg").display()
            )
        );
        // What is written reads back as the same songs.
        let paths: Vec<PathBuf> = songs.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(parse(&text, &music), paths);
        assert_eq!(to_text(&[], &music), "#EXTM3U\n");
    }

    #[test]
    fn saves_and_reads_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let list = dir.path().join("mix.m3u8");
        let songs = vec![
            (dir.path().join("a.mp3"), Some("Ä song".to_owned())),
            (dir.path().join("sub").join("b.flac"), None),
        ];
        write(&list, &songs).unwrap();
        let paths: Vec<PathBuf> = songs.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(read(&list).unwrap(), paths);
        let missing = read(&dir.path().join("none.m3u8")).unwrap_err();
        assert!(
            missing.starts_with("Couldn't read “none.m3u8”"),
            "{missing}"
        );
    }

    #[test]
    fn saved_names_end_like_a_playlist() {
        assert_eq!(
            with_playlist_ext("/m/mix".into()),
            PathBuf::from("/m/mix.m3u8")
        );
        assert_eq!(
            with_playlist_ext("/m/mix.M3U".into()),
            PathBuf::from("/m/mix.M3U")
        );
        assert_eq!(
            with_playlist_ext("/m/v1.2".into()),
            PathBuf::from("/m/v1.2.m3u8")
        );
    }
}
