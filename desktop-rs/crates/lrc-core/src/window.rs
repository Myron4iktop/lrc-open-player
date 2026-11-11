//! The main window's rules that need no window: the texts it shows, which lyric line is
//! playing, what a drop would do, and the keyboard shortcut list.

use std::path::{Path, PathBuf};

use crate::library::is_supported_audio;
use crate::tags::TrackInfo;
use crate::text::base_name;

/// How far the lyrics offset can go either way (`OFFSET_LIMIT_MS`).
pub const OFFSET_LIMIT_MS: i64 = 30_000;

/// `_format_time`: `QTime(0, 0, 0).addMSecs(ms).toString("m:ss")`. Minutes don't
/// count hours, as in the Python app (1:02:05 shows as 2:05).
pub fn format_time(ms: u64) -> String {
    let secs = (ms % 86_400_000) / 1000;
    format!("{}:{:02}", (secs / 60) % 60, secs % 60)
}

/// An offset clamped to ±30 s (`set_lyrics_offset`).
pub fn clamp_offset(ms: i64) -> i64 {
    ms.clamp(-OFFSET_LIMIT_MS, OFFSET_LIMIT_MS)
}

/// The offset label: `f"{ms / 1000:+.1f} s"`, such as "+0.0 s" or "-1.2 s".
pub fn offset_text(ms: i64) -> String {
    // Rust rounds the exact binary value half to even, as Python's format does.
    format!("{:+.1} s", ms as f64 / 1000.0)
}

/// The audio badge under the seek bar, such as "HD  24bit / 48.0kHz / 2ch / 1919kbps".
pub fn specs_text(info: &TrackInfo) -> String {
    let hd = if info.is_hd { "HD  " } else { "" };
    format!(
        "{hd}{}bit / {}kHz / {}ch / {}kbps",
        info.bits_per_sample,
        py_float(info.sample_rate_khz),
        info.channels,
        info.bitrate_kbps
    )
}

/// A float as Python's `str()` shows it: "44.1", "48.0".
fn py_float(x: f64) -> String {
    // Debug prints the shortest text that reads back as the same number, with ".0" for
    // whole numbers, like Python's repr for the values a sample rate can have.
    format!("{x:?}")
}

/// A tracklist row: "01. Title" (`_populate_queue_list`). Without a title from the
/// tags, the file name without its extension.
pub fn queue_row_text(number: usize, title: Option<&str>, path: &Path) -> String {
    let title = match title.filter(|t| !t.is_empty()) {
        Some(t) => t.to_owned(),
        None => path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    };
    format!("{number:02}. {title}")
}

/// The lyric line for a playback position (`_sync_lyrics_to_position`): the last line
/// that started at or before `pos_ms - offset_ms`, or `None` before the first.
pub fn active_line(times_ms: &[i64], pos_ms: u64, offset_ms: i64) -> Option<usize> {
    let adjusted = pos_ms as i64 - offset_ms;
    let mut found = None;
    for (i, &t) in times_ms.iter().enumerate() {
        if adjusted >= t {
            found = Some(i);
        } else {
            break;
        }
    }
    found
}

/// Where clicking a synced line seeks to: its time plus the offset, never below 0.
pub fn line_seek_ms(time_ms: i64, offset_ms: i64) -> u64 {
    (time_ms + offset_ms).max(0) as u64
}

/// How the window spells a lyric card for its state (`LyricCard.update_style`).
///
/// Sizes are in pixels at 96 DPI. The Python app sets them in points, so 22 pt is
/// 29.33 px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardStyle {
    pub main_px: f32,
    pub sub_px: f32,
    pub main_weight: i32,
    pub sub_weight: i32,
    pub padding_y: f32,
    pub spacing: f32,
}

/// Points to pixels at 96 DPI, as Qt converts a font's point size.
pub fn pt(points: f32) -> f32 {
    points * 96.0 / 72.0
}

pub fn card_style(active: bool, fullscreen: bool) -> CardStyle {
    if active {
        CardStyle {
            main_px: pt(if fullscreen { 26.0 } else { 22.0 }),
            sub_px: pt(if fullscreen { 17.0 } else { 15.0 }),
            main_weight: 700,
            sub_weight: 600,
            padding_y: if fullscreen { 30.0 } else { 22.0 },
            spacing: 8.0,
        }
    } else {
        CardStyle {
            main_px: pt(if fullscreen { 19.0 } else { 16.0 }),
            sub_px: pt(if fullscreen { 14.0 } else { 12.0 }),
            main_weight: 400,
            sub_weight: 400,
            padding_y: if fullscreen { 8.0 } else { 5.0 },
            spacing: 4.0,
        }
    }
}

// ---------------------------------------------------------------------------------------
// Drops
// ---------------------------------------------------------------------------------------

/// What dropping files on the window does (`_drop_plan`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropAction {
    /// Show this .lrc or .txt as the lyrics.
    Lyrics(PathBuf),
    /// Play this album folder.
    Folder(PathBuf),
    /// Play this song.
    File(PathBuf),
    /// Read these folders and songs, then play them all.
    Files {
        folders: Vec<PathBuf>,
        audio: Vec<PathBuf>,
    },
}

/// A drop's action and the two lines the overlay shows while dragging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropPlan {
    pub action: DropAction,
    pub headline: String,
    pub detail: String,
}

/// The file extensions shown as lyrics when dropped (`LYRICS_EXTS`).
pub const LYRICS_EXTS: [&str; 2] = [".lrc", ".txt"];

fn lower_name(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// `os.path.basename(p.rstrip("/\\"))`.
fn folder_name(path: &Path) -> String {
    let s = path.to_string_lossy();
    base_name(Path::new(s.trim_end_matches(['/', '\\'])))
}

/// What a drop of `paths` would do, or `None` to refuse it (`_drop_plan`). Cheap: it
/// looks at each path but never scans a folder. Lyrics only make sense while a song
/// is open (`has_track`).
pub fn drop_plan(paths: &[PathBuf], has_track: bool) -> Option<DropPlan> {
    let paths: Vec<&PathBuf> = paths.iter().filter(|p| !p.as_os_str().is_empty()).collect();
    if paths.is_empty() {
        return None;
    }
    if paths.len() == 1 {
        let p = paths[0];
        let name = lower_name(p);
        if LYRICS_EXTS.iter().any(|e| name.ends_with(e)) && p.is_file() {
            if !has_track {
                return None;
            }
            return Some(DropPlan {
                action: DropAction::Lyrics(p.clone()),
                headline: "Drop to show these lyrics".into(),
                detail: base_name(p),
            });
        }
    }
    let folders: Vec<PathBuf> = paths
        .iter()
        .filter(|p| p.is_dir())
        .map(|p| (*p).clone())
        .collect();
    let audio: Vec<PathBuf> = paths
        .iter()
        .filter(|p| p.is_file() && is_supported_audio(&lower_name(p)))
        .map(|p| (*p).clone())
        .collect();
    if folders.len() == 1 && audio.is_empty() {
        let detail = folder_name(&folders[0]);
        return Some(DropPlan {
            action: DropAction::Folder(folders.into_iter().next().unwrap()),
            headline: "Drop to play this album".into(),
            detail,
        });
    }
    if audio.len() == 1 && folders.is_empty() {
        let detail = base_name(&audio[0]);
        return Some(DropPlan {
            action: DropAction::File(audio.into_iter().next().unwrap()),
            headline: "Drop to play this song".into(),
            detail,
        });
    }
    if !folders.is_empty() || !audio.is_empty() {
        let count = folders.len() + audio.len();
        return Some(DropPlan {
            action: DropAction::Files { folders, audio },
            headline: "Drop to play".into(),
            detail: format!("{count} items"),
        });
    }
    None
}

// ---------------------------------------------------------------------------------------
// Shortcuts
// ---------------------------------------------------------------------------------------

/// The F1 list (`SHORTCUTS_HTML`): keys in bold, then what they do.
pub const SHORTCUTS: [(&str, &str); 17] = [
    ("Space", "Play / pause"),
    ("Ctrl+← / Ctrl+→", "Previous / next track"),
    ("F11 or double-click", "Fullscreen (Esc to leave)"),
    ("[ / ]", "Show lyrics 0.1 s earlier / later"),
    ("\\", "Reset lyrics offset"),
    ("R", "Romaji on / off"),
    ("S", "Shuffle on / off"),
    ("L", "Repeat: off → track → album"),
    ("M", "Mute / unmute"),
    ("V", "Visualizer: off → bars → wave"),
    ("Ctrl+O", "Open audio file"),
    ("Ctrl+Shift+O", "Open album folder"),
    ("Ctrl+L", "Load LRC file"),
    ("Ctrl+T", "Sync lyrics (tap-to-time editor)"),
    ("Ctrl+Shift+T", "Add translation"),
    ("Ctrl+I", "Track details"),
    ("F1", "This list"),
];

/// The line under the shortcut list.
pub const SHORTCUTS_FOOTER: &str =
    "Drop songs, album folders or an .lrc file onto the window to open them.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_line_follows_position_and_offset() {
        let times = [1000, 2000, 3000];
        assert_eq!(active_line(&times, 0, 0), None);
        assert_eq!(active_line(&times, 1000, 0), Some(0));
        assert_eq!(active_line(&times, 2999, 0), Some(1));
        assert_eq!(active_line(&times, 9999, 0), Some(2));
        // Positive offset: lyrics appear later.
        assert_eq!(active_line(&times, 2000, 500), Some(0));
        // Negative offset: earlier.
        assert_eq!(active_line(&times, 1500, -500), Some(1));
        assert_eq!(active_line(&[], 5000, 0), None);
    }

    #[test]
    fn seeking_to_a_line_respects_the_offset() {
        assert_eq!(line_seek_ms(1000, 200), 1200);
        assert_eq!(line_seek_ms(100, -500), 0);
    }

    #[test]
    fn offset_is_clamped() {
        assert_eq!(clamp_offset(40_000), 30_000);
        assert_eq!(clamp_offset(-40_000), -30_000);
        assert_eq!(clamp_offset(1200), 1200);
    }

    #[test]
    fn card_sizes_are_points() {
        let s = card_style(true, false);
        assert!((s.main_px - 29.333).abs() < 0.01);
        assert_eq!(s.padding_y, 22.0);
        assert_eq!(card_style(false, true).padding_y, 8.0);
        assert_eq!(card_style(true, true).padding_y, 30.0);
    }

    #[test]
    fn specs_badge() {
        let mut info = TrackInfo::defaults(Path::new("a.flac"));
        info.is_hd = true;
        info.bits_per_sample = 24;
        info.sample_rate_khz = 48.0;
        info.bitrate_kbps = 1919;
        assert_eq!(specs_text(&info), "HD  24bit / 48.0kHz / 2ch / 1919kbps");
        info.is_hd = false;
        info.sample_rate_khz = 44.1;
        info.bits_per_sample = 16;
        assert_eq!(specs_text(&info), "16bit / 44.1kHz / 2ch / 1919kbps");
    }

    #[test]
    fn queue_rows() {
        let p = Path::new("/m/03 Song.flac");
        assert_eq!(queue_row_text(3, Some("Surfacing"), p), "03. Surfacing");
        assert_eq!(queue_row_text(12, None, p), "12. 03 Song");
        assert_eq!(queue_row_text(100, Some(""), p), "100. 03 Song");
    }

    #[test]
    fn drop_plans() {
        let dir = tempfile::tempdir().unwrap();
        let album = dir.path().join("Album");
        std::fs::create_dir(&album).unwrap();
        let song = dir.path().join("a.MP3");
        let lrc = dir.path().join("a.lrc");
        let other = dir.path().join("notes.pdf");
        for f in [&song, &lrc, &other] {
            std::fs::write(f, b"x").unwrap();
        }

        let plan = drop_plan(std::slice::from_ref(&lrc), true).unwrap();
        assert_eq!(plan.action, DropAction::Lyrics(lrc.clone()));
        assert_eq!(plan.headline, "Drop to show these lyrics");
        assert_eq!(plan.detail, "a.lrc");
        assert_eq!(drop_plan(std::slice::from_ref(&lrc), false), None);

        let plan = drop_plan(std::slice::from_ref(&album), false).unwrap();
        assert_eq!(plan.action, DropAction::Folder(album.clone()));
        assert_eq!(plan.detail, "Album");

        let plan = drop_plan(std::slice::from_ref(&song), false).unwrap();
        assert_eq!(plan.headline, "Drop to play this song");
        assert_eq!(plan.detail, "a.MP3");

        let plan = drop_plan(&[album.clone(), song.clone(), other.clone()], false).unwrap();
        assert_eq!(plan.headline, "Drop to play");
        assert_eq!(plan.detail, "2 items");

        assert_eq!(drop_plan(std::slice::from_ref(&other), true), None);
        // An .lrc among other files isn't lyrics, and on its own it's not audio either.
        assert_eq!(drop_plan(&[lrc.clone(), other], true), None);
        assert_eq!(drop_plan(&[], true), None);
    }

    #[test]
    fn shortcut_list_matches_python() {
        let html = include_str!("../../../../desktop/lrcplayer.py");
        let start = html.find("SHORTCUTS_HTML = \"\"\"").unwrap() + 20;
        let block = &html[start..start + html[start..].find("\"\"\"").unwrap()];
        for (keys, what) in SHORTCUTS {
            let keys_html = keys
                .replace("\\", "\\\\")
                .replace(" or double-click", "</b> or double-click");
            let wanted = if keys_html.contains("</b>") {
                format!("<td><b>{keys_html}</td><td>{what}</td>")
            } else {
                format!("<td><b>{keys_html}</b></td><td>{what}</td>")
            };
            assert!(block.contains(&wanted), "missing {wanted}");
        }
        assert_eq!(block.matches("<tr>").count(), SHORTCUTS.len());
        assert!(block.contains(SHORTCUTS_FOOTER));
    }
}
