//! What the window needs from the system: the title bar color, the taskbar identity and
//! icons, and files dropped from Explorer. All of it goes through Slint's winit window.

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};

use crate::MainWindow;
use crate::app::App;

/// Watches the window's system events: files dragged over it and the window coming back
/// from the taskbar.
///
/// Drops go to the app (`dragEnterEvent`, `dropEvent`). winit reports each file on its
/// own: `HoveredFile` for every file being dragged, then `DroppedFile` for each one, or
/// `HoveredFileCancelled` when the drag leaves.
pub fn hook_window_events(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    let weak_app = Rc::downgrade(app);
    let weak_ui = ui.as_weak();
    ui.window().on_winit_window_event(move |_, event| {
        let (Some(app), Some(ui)) = (weak_app.upgrade(), weak_ui.upgrade()) else {
            return EventResult::Propagate;
        };
        match event {
            winit::event::WindowEvent::Occluded(false)
            | winit::event::WindowEvent::Focused(true) => {
                repaint_all(&ui);
                // Once more after Windows has finished restoring the window.
                let weak = ui.as_weak();
                slint::Timer::single_shot(std::time::Duration::from_millis(150), move || {
                    if let Some(ui) = weak.upgrade() {
                        repaint_all(&ui);
                    }
                });
            }
            winit::event::WindowEvent::Resized(_) => repaint_all(&ui),
            _ => {}
        }
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

/// Makes the next frame repaint the whole window (see `repaint-count` in main.slint).
///
/// After the window is restored from the taskbar, the renderer sometimes drew only the
/// parts it thought had changed into a fresh buffer, and the rest showed the desktop.
fn repaint_all(ui: &MainWindow) {
    ui.set_repaint_count(ui.get_repaint_count().wrapping_add(1));
    ui.window().request_redraw();
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

    /// Keeps a Start Menu shortcut to this .exe that carries the player's taskbar identity.
    /// That shortcut is where Windows finds the name for the media panel by the clock, which
    /// otherwise says "Unknown app". A portable .exe has no installer to make one, so it is
    /// made at start-up, and made again when the .exe has moved. Failing is harmless.
    pub fn ensure_start_menu_shortcut() {
        if let Err(e) = try_start_menu_shortcut() {
            eprintln!("Start Menu shortcut: {e}");
        }
    }

    fn try_start_menu_shortcut() -> windows::core::Result<()> {
        use windows::Win32::Foundation::{E_FAIL, MAX_PATH};
        use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
        use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
        use windows::Win32::System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            IPersistFile, STGM_READ,
        };
        use windows::Win32::System::Variant::VT_LPWSTR;
        use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
        use windows::Win32::UI::Shell::{IShellLinkW, SHStrDupW, ShellLink};
        use windows::core::Interface;

        let fail = || windows::core::Error::from(E_FAIL);
        let exe = std::env::current_exe().map_err(|_| fail())?;
        let folder = std::path::PathBuf::from(std::env::var_os("APPDATA").ok_or_else(fail)?)
            .join(r"Microsoft\Windows\Start Menu\Programs");
        let shortcut = folder.join(format!("{}.lnk", lrc_core::APP_TITLE));
        let shortcut_w = HSTRING::from(shortcut.as_os_str());

        // SAFETY: plain COM calls on this thread, with valid, NUL-terminated strings and
        // buffers that outlive each call. The window toolkit later asks for the same
        // threading model.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let file: IPersistFile = link.cast()?;

            if shortcut.exists() && file.Load(&shortcut_w, STGM_READ).is_ok() {
                let mut target = [0u16; MAX_PATH as usize];
                if link.GetPath(&mut target, std::ptr::null_mut(), 0).is_ok() {
                    let len = target.iter().position(|&c| c == 0).unwrap_or(target.len());
                    let target = String::from_utf16_lossy(&target[..len]);
                    if std::path::Path::new(&target) == exe {
                        return Ok(()); // already there and pointing here
                    }
                }
            }

            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(exe.as_os_str()))?;
            link.SetDescription(&HSTRING::from(lrc_core::APP_TITLE))?;

            let store: IPropertyStore = link.cast()?;
            let mut id = PROPVARIANT::default();
            {
                let inner = &mut *id.Anonymous.Anonymous;
                inner.vt = VT_LPWSTR;
                inner.Anonymous.pwszVal = SHStrDupW(&HSTRING::from(lrc_core::APP_USER_MODEL_ID))?;
            }
            let set = store
                .SetValue(&PKEY_AppUserModel_ID, &id)
                .and_then(|()| store.Commit());
            let _ = PropVariantClear(&mut id);
            set?;

            std::fs::create_dir_all(&folder).map_err(|_| fail())?;
            let file: IPersistFile = link.cast()?;
            file.Save(&shortcut_w, true)?;
        }
        Ok(())
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
pub use win::{ensure_start_menu_shortcut, set_app_user_model_id, set_icons_from_exe};

/// Moves the window back onto a screen if the remembered position is off every one
/// (a monitor unplugged since), as Qt's `restoreGeometry` does.
pub fn keep_on_screen(ui: &MainWindow) {
    ui.window().with_winit_window(|w| {
        let Ok(pos) = w.outer_position() else { return };
        let size = w.outer_size();
        let monitors: Vec<_> = w.available_monitors().collect();
        if monitors.is_empty() {
            return;
        }
        // At least a 100 px corner of the title bar must be on some monitor.
        let visible = monitors.iter().any(|m| {
            let (mp, ms) = (m.position(), m.size());
            let left = pos.x.max(mp.x);
            let right = (pos.x + size.width as i32).min(mp.x + ms.width as i32);
            let top = pos.y.max(mp.y);
            let bottom = (pos.y + 100).min(mp.y + ms.height as i32);
            right - left >= 100 && bottom > top
        });
        if !visible {
            let target = w.primary_monitor().or_else(|| monitors.first().cloned());
            if let Some(m) = target {
                let mp = m.position();
                w.set_outer_position(winit::dpi::PhysicalPosition::new(mp.x + 40, mp.y + 40));
            }
        }
    });
}

/// `raise_()` and `activateWindow()`: brings the window back in front of the others.
pub fn bring_to_front(ui: &MainWindow) {
    ui.window().with_winit_window(|w| w.focus_window());
}
