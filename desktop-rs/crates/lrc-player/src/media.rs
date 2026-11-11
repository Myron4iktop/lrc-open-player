//! The system's media controls: on Windows the media flyout, lock screen and media keys
//! (`win::media`). Elsewhere the app works without them, as the Python app does when
//! the WinRT packages are missing.

/// A button pressed in the media flyout, or a media key.
/// Only Windows sends these.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaCommand {
    Play,
    Pause,
    Next,
    Previous,
    /// The flyout's timeline was dragged to this position, in milliseconds.
    Seek(u64),
}

#[cfg(windows)]
pub use crate::win::media::MediaControls;

#[cfg(not(windows))]
mod none {
    use image::RgbaImage;
    use lrc_core::tags::TrackInfo;

    use super::MediaCommand;
    use crate::audio::State;

    /// No media controls on this system: every call does nothing.
    pub struct MediaControls;

    impl MediaControls {
        pub fn new(_on_command: impl Fn(MediaCommand) + Send + Sync + 'static) -> Option<Self> {
            None
        }
        pub fn set_track(&mut self, _: &TrackInfo, _: Option<&RgbaImage>, _: bool, _: bool) {}
        pub fn set_navigation(&mut self, _: bool, _: bool) {}
        pub fn set_playback_state(&mut self, _: State) {}
        pub fn update_timeline(&mut self, _: u64, _: u64, _: bool) {}
        pub fn shutdown(&mut self) {}
    }
}

#[cfg(not(windows))]
pub use none::MediaControls;
