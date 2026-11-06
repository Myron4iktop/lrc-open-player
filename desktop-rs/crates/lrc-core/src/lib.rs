//! The parts of LRC Open Player that need neither a window nor Windows.
//!
//! Everything here builds and is tested on Linux (`cargo test -p lrc-core`). Each module
//! ports functions from `desktop/lrcplayer.py`, which stays the reference: same rules,
//! same edge cases and the same messages.
//!
//! - [`text`]: reading lyrics files in any encoding.
//! - [`lrc`]: LRC timestamps, and parsing lyrics into lines with translations.
//! - [`tags`]: tags, covers and embedded lyrics of a song, and writing lyrics into it.
//! - [`library`]: album scans, natural sort and search text.
//! - [`lrclib`]: the LRCLIB lookup and the rules that pick an answer.
//! - [`queue`]: the playlist, shuffle, repeat, next and previous.
//! - [`window`]: the main window's texts and rules, such as which lyric line is playing.
//! - [`theme`]: the dark and light palettes for every accent color.
//! - [`settings`]: the remembered settings file, and the one-time import from the
//!   Python app's registry values.
//! - [`session`]: what is saved between runs and how the last playlist comes back.
//!
//! The AI alignment module (`align`) arrives in M7.

mod codepages;
pub mod library;
pub mod lrc;
pub mod lrclib;
mod mpeg;
mod ogg;
pub mod py;
pub mod queue;
pub mod session;
pub mod settings;
pub mod tags;
pub mod text;
pub mod theme;
pub mod window;

/// The name shown in the title bar and in message boxes (`APP_TITLE` in `lrcplayer.py`).
pub const APP_TITLE: &str = "LRC Open Player";

/// The Windows taskbar identity of the Rust build.
///
/// The Python app uses `myron.lrcplayer.app.2_3`. The Rust build has its own, so both
/// versions can be pinned and tell apart in the media flyout.
pub const APP_USER_MODEL_ID: &str = "myron.lrcopenplayer.rust";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_matches_python_app() {
        assert_eq!(APP_TITLE, "LRC Open Player");
    }

    #[test]
    fn app_id_differs_from_python_app() {
        assert_ne!(APP_USER_MODEL_ID, "myron.lrcplayer.app.2_3");
        // SetCurrentProcessExplicitAppUserModelID allows at most 128 characters, no spaces.
        assert!(APP_USER_MODEL_ID.len() <= 128);
        assert!(!APP_USER_MODEL_ID.contains(' '));
    }
}
