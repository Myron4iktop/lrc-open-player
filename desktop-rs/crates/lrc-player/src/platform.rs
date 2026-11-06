//! What the window needs from the system: the title bar color, the taskbar identity and
//! icons, and files dropped from Explorer. All of it goes through Slint's winit window.

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};

use crate::MainWindow;
use crate::app::App;

/// Passes files dragged over the window to the app (`dragEnterEvent`, `dropEvent`).
///
/// winit reports each file on its own: `HoveredFile` for every file being dragged, then
/// `DroppedFile` for each one, or `HoveredFileCancelled` when the drag leaves.
pub fn hook_file_drops(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let weak_app = Rc::downgrade(app);
    let weak_ui = ui.as_weak();
    ui.window().on_winit_window_event(move |_, event| {
        let (Some(app), Some(ui)) = (weak_app.upgrade(), weak_ui.upgrade()) else {
            return EventResult::Propagate;
        };
        let Ok(mut app) = app.try_borrow_mut() else {
            return EventResult::Propagate;
        };
        match event {
            winit::event::WindowEvent::HoveredFile(path) => app.drag_hovered(&ui, path.clone()),
            winit::event::WindowEvent::HoveredFileCancelled => app.drag_left(&ui),
            winit::event::WindowEvent::DroppedFile(path) => app.file_dropped(path.clone()),
            _ => {}
        }
        EventResult::Propagate
    });
}

/// Matches the Windows title bar to the theme (`_apply_title_bar`; Windows 10 2004+ and 11).
#[cfg(windows)]
pub fn apply_title_bar(ui: &MainWindow, dark: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    ui.window().with_winit_window(|w| {
        let Ok(handle) = w.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(h) = handle.as_raw() else {
            return;
        };
        let hwnd = HWND(h.hwnd.get() as *mut _);
        let value: i32 = dark.into();
        // SAFETY: `hwnd` is this window's live handle, and `value` outlives the call.
        // Failing is harmless (older Windows keeps its own title bar), so it's ignored.
        let _ = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                std::ptr::from_ref(&value).cast(),
                size_of::<i32>() as u32,
            )
        };
    });
}

#[cfg(not(windows))]
pub fn apply_title_bar(_ui: &MainWindow, _dark: bool) {}

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

#[cfg(windows)]
pub use win::{set_app_user_model_id, set_icons_from_exe};
