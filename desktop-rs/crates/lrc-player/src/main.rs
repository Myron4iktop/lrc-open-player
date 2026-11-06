// No console window behind the player in release builds, like PyInstaller's `--windowed`.
// Debug builds keep the console so `println!` and panics are visible while developing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    #[cfg(windows)]
    win::set_app_user_model_id();

    let window = MainWindow::new()?;
    window.show()?;
    #[cfg(windows)]
    win::set_icons_from_exe(&window);
    slint::run_event_loop()?;
    window.hide()
}

#[cfg(windows)]
mod win {
    use slint::ComponentHandle;
    use slint::winit_030::WinitWindowAccessor;
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    use windows::core::HSTRING;
    use winit::platform::windows::{IconExtWindows, WindowExtWindows};
    use winit::window::Icon;

    /// The resource number `winresource` gives the icon in `build.rs`.
    const ICON_RESOURCE: u16 = 1;

    /// Gives the player its own taskbar identity, as the Python app does at start-up.
    /// Failing is harmless (the taskbar then groups by .exe path), so the error is ignored.
    pub fn set_app_user_model_id() {
        let id = HSTRING::from(lrc_core::APP_USER_MODEL_ID);
        // SAFETY: `id` is a valid, NUL-terminated wide string that outlives the call.
        let _ = unsafe { SetCurrentProcessExplicitAppUserModelID(&id) };
    }

    /// Slint only sets the small (title bar) icon. The taskbar and Alt+Tab use the big one,
    /// so both are loaded from the .exe's own icon, which Windows picks at the right size.
    pub fn set_icons_from_exe(window: &crate::MainWindow) {
        window.window().with_winit_window(|w| {
            if let Ok(icon) = Icon::from_resource(ICON_RESOURCE, None) {
                w.set_window_icon(Some(icon));
            }
            if let Ok(icon) = Icon::from_resource(ICON_RESOURCE, None) {
                w.set_taskbar_icon(Some(icon));
            }
        });
    }
}
