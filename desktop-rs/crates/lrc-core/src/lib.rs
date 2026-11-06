//! The parts of LRC Open Player that need neither a window nor Windows.
//!
//! Everything here builds and is tested on Linux (`cargo test -p lrc-core`).
//! The modules from the port plan (`text`, `lrc`, `tags`, `library`, `lrclib`,
//! `align`) arrive in M1 and M7.

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
