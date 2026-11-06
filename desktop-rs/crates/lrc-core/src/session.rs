//! Session memory: what the player remembers between runs and how it comes back
//! (`_restore_session_ui`, `_restore_playback_session`, `_finish_restore`,
//! `_resume_position_ms` and `_save_session` in lrcplayer.py).
//!
//! The window code gathers a [`Snapshot`] and hands it to [`save_snapshot`]; on start it
//! asks [`restore_plan`] what to reopen. Both work on [`Settings`], so the rules are
//! tested here without a window.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::queue::Source;
use crate::settings::Settings;

/// How often the position is saved while playing, in case the app is killed.
pub const AUTOSAVE_INTERVAL_MS: u64 = 5000;

/// The Rust build's own window position and size. The Python app's `window/geometry` is
/// a Qt blob and isn't imported; this key has another name so the two never mix.
pub const PLACEMENT_KEY: &str = "window/placement";

/// The window's position and size in physical pixels, when it isn't fullscreen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl Placement {
    pub fn to_json(self) -> Value {
        json!({
            "x": self.x,
            "y": self.y,
            "width": self.width,
            "height": self.height,
            "maximized": self.maximized,
        })
    }

    /// Reads a saved placement. A missing field, or a size too small to be a real window
    /// (100 px), means there's nothing usable.
    pub fn from_json(v: &Value) -> Option<Self> {
        let int = |k: &str| v.get(k).and_then(Value::as_i64);
        let p = Placement {
            x: i32::try_from(int("x")?).ok()?,
            y: i32::try_from(int("y")?).ok()?,
            width: u32::try_from(int("width")?).ok()?,
            height: u32::try_from(int("height")?).ok()?,
            maximized: v.get("maximized").and_then(Value::as_bool).unwrap_or(false),
        };
        (p.width >= 100 && p.height >= 100).then_some(p)
    }
}

/// `os.path.normcase(os.path.normpath(p))`, for Windows paths (`windows`: any case, `/`
/// or `\`) or POSIX ones.
pub fn norm_path(p: &str, windows: bool) -> String {
    if p.is_empty() {
        return ".".to_owned();
    }
    let sep = if windows { '\\' } else { '/' };
    let p = if windows {
        p.replace('/', "\\")
    } else {
        p.to_owned()
    };
    // The drive or UNC share stays as it is; the rest is cleaned.
    let (prefix, rest) = if windows {
        split_windows_prefix(&p)
    } else {
        (String::new(), p.as_str())
    };
    let rooted = rest.starts_with(sep);
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split(sep) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else if !rooted {
                    parts.push("..");
                }
            }
            _ => parts.push(part),
        }
    }
    let mut out = prefix;
    if rooted {
        out.push(sep);
    }
    out.push_str(&parts.join(&sep.to_string()));
    if out.is_empty() {
        out.push('.');
    }
    if windows { out.to_lowercase() } else { out }
}

/// `C:` or `\\server\share` at the start of a Windows path, and what follows it.
fn split_windows_prefix(p: &str) -> (String, &str) {
    if let Some(after) = p.strip_prefix("\\\\") {
        // \\server\share\rest
        let mut idx = 0;
        let mut seps = 0;
        for (i, c) in after.char_indices() {
            if c == '\\' {
                seps += 1;
                if seps == 2 {
                    idx = i;
                    break;
                }
            }
        }
        if seps == 2 {
            return (format!("\\\\{}", &after[..idx]), &after[idx..]);
        }
        return (p.to_owned(), "");
    }
    let bytes = p.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return (p[..2].to_owned(), &p[2..]);
    }
    (String::new(), p)
}

/// True if two paths name the same file, as `_finish_restore` compares them.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let windows = cfg!(windows);
    norm_path(&a.to_string_lossy(), windows) == norm_path(&b.to_string_lossy(), windows)
}

/// Where to resume a song next time (`_resume_position_ms`): a resume position that
/// hasn't been applied yet is kept; a song played to within 1.5 s of its end starts over.
pub fn resume_position_ms(pending_ms: Option<u64>, position_ms: u64, duration_ms: u64) -> u64 {
    if let Some(p) = pending_ms {
        return p;
    }
    if duration_ms > 0 && position_ms + 1500 >= duration_ms {
        return 0;
    }
    position_ms
}

/// What the window looks like and what is playing, for [`save_snapshot`].
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub volume: i64,
    pub offset_ms: i64,
    /// The playlist and what it was built from; `None` while nothing is loaded.
    pub playback: Option<PlaybackSnapshot>,
    /// Only for a full save (closing the window), not the 5 s autosave.
    pub window: Option<WindowSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackSnapshot {
    pub source: Source,
    pub playlist: Vec<PathBuf>,
    pub current_track: PathBuf,
    pub position_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowSnapshot {
    pub fullscreen: bool,
    /// The windowed placement (the one from before fullscreen, while fullscreen).
    pub placement: Option<Placement>,
    pub queue_open: bool,
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// The `playback/source_kind` and `playback/source_path` of a playlist.
pub fn source_keys(source: &Source) -> Option<(&'static str, &Path)> {
    match source {
        Source::None => None,
        Source::File(p) => Some(("file", p)),
        Source::Files(p) => Some(("files", p)),
        Source::Folder(p) => Some(("folder", p)),
    }
}

/// Writes a snapshot into the settings (`_save_session`). Returns true if anything changed.
///
/// The remembered playlist is only replaced while something is actually loaded, so a
/// restore that failed (a drive unplugged, say) never erases it.
pub fn save_snapshot(settings: &mut Settings, snap: &Snapshot) -> bool {
    let mut changed = false;
    changed |= settings.set("ui/volume", snap.volume);
    changed |= settings.set("lyrics/offset_ms", snap.offset_ms);
    if let Some(pb) = &snap.playback
        && let Some((kind, source_path)) = source_keys(&pb.source)
    {
        changed |= settings.set("playback/source_kind", kind);
        changed |= settings.set("playback/source_path", path_str(source_path));
        if kind == "files" {
            let files: Vec<Value> = pb.playlist.iter().map(|p| path_str(p).into()).collect();
            changed |= settings.set("playback/files", files);
        }
        changed |= settings.set("playback/last_track", path_str(&pb.current_track));
        changed |= settings.set("playback/last_position_ms", pb.position_ms);
    }
    if let Some(w) = &snap.window {
        if let Some(p) = w.placement {
            changed |= settings.set(PLACEMENT_KEY, p.to_json());
        }
        changed |= settings.set("window/fullscreen", w.fullscreen);
        changed |= settings.set("ui/queue_open", w.queue_open);
    }
    changed
}

/// What to reopen at start (`_restore_playback_session`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestorePlan {
    Nothing,
    /// Read this album folder in the background, then carry on.
    Folder(PathBuf),
    /// This one song.
    File(PathBuf),
    /// These songs, in the order they were played (only the ones still there).
    Files(Vec<PathBuf>),
}

/// The remembered playback: the plan, the song to resume and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remembered {
    pub plan: RestorePlan,
    pub last_track: PathBuf,
    pub last_position_ms: u64,
}

/// Reads what to reopen. Folders and songs that are gone give [`RestorePlan::Nothing`]
/// (and the settings stay as they are).
pub fn restore_plan(settings: &Settings) -> Remembered {
    let kind = settings.get_str("playback/source_kind", "");
    let source = settings.get_str("playback/source_path", "");
    let last_track = settings.get_str("playback/last_track", "");
    let last_pos = settings.get_int("playback/last_position_ms", 0).max(0) as u64;
    let plan = match kind.as_str() {
        "folder" if !source.is_empty() && Path::new(&source).is_dir() => {
            RestorePlan::Folder(source.into())
        }
        "file" if !last_track.is_empty() && Path::new(&last_track).is_file() => {
            RestorePlan::File(last_track.clone().into())
        }
        "files" => {
            let files: Vec<PathBuf> = settings
                .get_list("playback/files")
                .into_iter()
                .filter(|f| Path::new(f).is_file())
                .map(PathBuf::from)
                .collect();
            if files.is_empty() {
                RestorePlan::Nothing
            } else {
                RestorePlan::Files(files)
            }
        }
        _ => RestorePlan::Nothing,
    };
    Remembered {
        plan,
        last_track: last_track.into(),
        last_position_ms: last_pos,
    }
}

impl Remembered {
    /// The playlist source to rebuild (`_set_playlist(files, kind, source)`). For one
    /// song it is the song itself; for dropped songs, the remembered folder.
    pub fn source(&self, settings: &Settings) -> Source {
        match &self.plan {
            RestorePlan::Nothing => Source::None,
            RestorePlan::Folder(f) => Source::Folder(f.clone()),
            RestorePlan::File(f) => Source::File(f.clone()),
            RestorePlan::Files(_) => {
                Source::Files(settings.get_str("playback/source_path", "").into())
            }
        }
    }

    /// Which song to start and where (`_finish_restore`): the remembered one at its
    /// position, or, when it's gone, `start_index` from the beginning.
    pub fn start(&self, files: &[PathBuf], start_index: usize) -> (usize, u64, bool) {
        let found = (!self.last_track.as_os_str().is_empty())
            .then(|| files.iter().position(|f| same_path(f, &self.last_track)))
            .flatten();
        match found {
            Some(i) => (i, self.last_position_ms, true),
            None => (start_index, 0, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn normalizes_like_python() {
        // ntpath
        assert_eq!(
            norm_path(r"C:\Music\.\Album\..\Song.FLAC", true),
            r"c:\music\song.flac"
        );
        assert_eq!(norm_path("C:/Music//Album/", true), r"c:\music\album");
        assert_eq!(
            norm_path(r"\\NAS\Share\a\..\b.mp3", true),
            r"\\nas\share\b.mp3"
        );
        assert_eq!(norm_path(r"C:\..\x", true), r"c:\x");
        assert_eq!(norm_path(r"..\x", true), r"..\x");
        assert_eq!(norm_path("", true), ".");
        // posixpath: case is kept
        assert_eq!(
            norm_path("/Music/./A/../Song.flac", false),
            "/Music/Song.flac"
        );
        assert_eq!(norm_path("/a//b/", false), "/a/b");
        assert_eq!(norm_path("a/../..", false), "..");
    }

    #[test]
    fn resume_position() {
        assert_eq!(resume_position_ms(Some(42_000), 0, 0), 42_000);
        assert_eq!(resume_position_ms(None, 10_000, 200_000), 10_000);
        assert_eq!(resume_position_ms(None, 198_600, 200_000), 0);
        assert_eq!(resume_position_ms(None, 198_400, 200_000), 198_400);
        assert_eq!(resume_position_ms(None, 5000, 0), 5000);
    }

    #[test]
    fn placement_round_trip() {
        let p = Placement {
            x: -8,
            y: 20,
            width: 1150,
            height: 780,
            maximized: true,
        };
        assert_eq!(Placement::from_json(&p.to_json()), Some(p));
        assert_eq!(Placement::from_json(&json!({"x": 1})), None);
        assert_eq!(
            Placement::from_json(&json!({"x": 0, "y": 0, "width": 10, "height": 10})),
            None
        );
    }

    fn snapshot(playback: Option<PlaybackSnapshot>) -> Snapshot {
        Snapshot {
            volume: 55,
            offset_ms: -200,
            playback,
            window: None,
        }
    }

    #[test]
    fn a_failed_restore_never_erases_the_remembered_playlist() {
        let mut s = Settings::new();
        s.set("playback/source_kind", "folder");
        s.set("playback/source_path", "E:\\Album");
        s.set("playback/last_track", "E:\\Album\\03.flac");
        s.set("playback/last_position_ms", 61_000);
        let before = s.clone();
        save_snapshot(&mut s, &snapshot(None));
        for key in [
            "playback/source_kind",
            "playback/source_path",
            "playback/last_track",
            "playback/last_position_ms",
        ] {
            assert_eq!(s.raw(key), before.raw(key), "{key}");
        }
        assert_eq!(s.get_int("ui/volume", 0), 55);
        assert_eq!(s.get_int("lyrics/offset_ms", 0), -200);
    }

    #[test]
    fn saves_playback_and_window() {
        let mut s = Settings::new();
        let mut snap = snapshot(Some(PlaybackSnapshot {
            source: Source::Files("/m".into()),
            playlist: vec!["/m/b.mp3".into(), "/m/a.mp3".into()],
            current_track: "/m/a.mp3".into(),
            position_ms: 1234,
        }));
        snap.window = Some(WindowSnapshot {
            fullscreen: true,
            placement: Some(Placement {
                x: 1,
                y: 2,
                width: 800,
                height: 600,
                maximized: false,
            }),
            queue_open: true,
        });
        assert!(save_snapshot(&mut s, &snap));
        assert!(!save_snapshot(&mut s, &snap), "nothing new the second time");
        assert_eq!(s.get_str("playback/source_kind", ""), "files");
        assert_eq!(s.get_str("playback/source_path", ""), "/m");
        assert_eq!(s.get_list("playback/files"), ["/m/b.mp3", "/m/a.mp3"]);
        assert_eq!(s.get_str("playback/last_track", ""), "/m/a.mp3");
        assert_eq!(s.get_int("playback/last_position_ms", 0), 1234);
        assert!(s.get_bool("window/fullscreen", false));
        assert!(s.get_bool("ui/queue_open", false));
        assert_eq!(
            Placement::from_json(s.raw(PLACEMENT_KEY).unwrap())
                .unwrap()
                .width,
            800
        );

        // A single song doesn't touch the remembered list of dropped songs.
        let snap = snapshot(Some(PlaybackSnapshot {
            source: Source::File("/m/c.mp3".into()),
            playlist: vec!["/m/c.mp3".into()],
            current_track: "/m/c.mp3".into(),
            position_ms: 0,
        }));
        save_snapshot(&mut s, &snap);
        assert_eq!(s.get_str("playback/source_kind", ""), "file");
        assert_eq!(s.get_str("playback/source_path", ""), "/m/c.mp3");
        assert_eq!(s.get_list("playback/files"), ["/m/b.mp3", "/m/a.mp3"]);
    }

    #[test]
    fn plans_what_to_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let album = dir.path().join("Album");
        fs::create_dir(&album).unwrap();
        let a = album.join("a.flac");
        let b = album.join("b.flac");
        fs::write(&a, b"x").unwrap();
        fs::write(&b, b"x").unwrap();
        let gone = album.join("gone.flac");
        let p = |p: &Path| p.to_string_lossy().into_owned();

        let mut s = Settings::new();
        assert_eq!(restore_plan(&s).plan, RestorePlan::Nothing);

        s.set("playback/source_kind", "folder");
        s.set("playback/source_path", p(&album));
        s.set("playback/last_track", p(&b));
        s.set("playback/last_position_ms", "9000");
        let r = restore_plan(&s);
        assert_eq!(r.plan, RestorePlan::Folder(album.clone()));
        assert_eq!(r.source(&s), Source::Folder(album.clone()));
        assert_eq!(r.start(&[a.clone(), b.clone()], 0), (1, 9000, true));
        // The remembered song is gone: start over at the playlist's start.
        assert_eq!(r.start(std::slice::from_ref(&a), 0), (0, 0, false));

        s.set("playback/source_path", p(&dir.path().join("missing")));
        assert_eq!(restore_plan(&s).plan, RestorePlan::Nothing);

        s.set("playback/source_kind", "file");
        s.set("playback/last_track", p(&a));
        let r = restore_plan(&s);
        assert_eq!(r.plan, RestorePlan::File(a.clone()));
        assert_eq!(r.source(&s), Source::File(a.clone()));
        s.set("playback/last_track", p(&gone));
        assert_eq!(restore_plan(&s).plan, RestorePlan::Nothing);

        s.set("playback/source_kind", "files");
        s.set("playback/source_path", p(&album));
        s.set("playback/files", json!([p(&b), p(&gone), p(&a)]));
        let r = restore_plan(&s);
        assert_eq!(r.plan, RestorePlan::Files(vec![b.clone(), a.clone()]));
        assert_eq!(r.source(&s), Source::Files(album.clone()));
        s.set("playback/files", json!([p(&gone)]));
        assert_eq!(restore_plan(&s).plan, RestorePlan::Nothing);
    }

    #[test]
    fn matches_the_remembered_song_loosely() {
        assert!(same_path(Path::new("/m/./a.flac"), Path::new("/m/a.flac")));
        if cfg!(windows) {
            assert!(same_path(
                Path::new(r"C:\M\A.flac"),
                Path::new("c:/m/a.FLAC")
            ));
        }
    }
}
