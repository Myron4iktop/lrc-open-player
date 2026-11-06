//! The tray icon and its menu (`_init_tray`): on Windows the notification area
//! (`win::tray`). Elsewhere there is no tray, and "Keep playing in tray when closed" is
//! greyed out, as in the Python app when `QSystemTrayIcon.isSystemTrayAvailable()` is false.

/// Something done to the tray icon.
/// Only Windows has a tray to send these.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    /// A left click (`ActivationReason.Trigger`).
    Clicked,
    DoubleClicked,
    MiddleClicked,
    Menu(TrayItem),
}

/// The entries of the tray menu, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum TrayItem {
    Now = 1,
    Play,
    Next,
    Previous,
    Shuffle,
    Repeat,
    Window,
    Quit,
}

impl TrayItem {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn from_id(id: usize) -> Option<Self> {
        use TrayItem::*;
        [Now, Play, Next, Previous, Shuffle, Repeat, Window, Quit]
            .into_iter()
            .find(|item| *item as usize == id)
    }
}

#[cfg(windows)]
pub use crate::win::tray::Tray;

#[cfg(not(windows))]
mod none {
    use lrc_core::system::TrayTexts;

    use super::TrayCommand;

    /// No tray on this system.
    pub struct Tray;

    impl Tray {
        pub fn new(_texts: TrayTexts, _on_command: impl Fn(TrayCommand) + 'static) -> Option<Self> {
            None
        }
        pub fn update(&self, _: TrayTexts, _: bool) {}
        pub fn show_message(&self, _: &str, _: &str, _: u32) {}
    }
}

#[cfg(not(windows))]
pub use none::Tray;
