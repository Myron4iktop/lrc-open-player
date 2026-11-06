//! The tray icon (`_init_tray`, `_update_tray`, `_on_tray_activated`), drawn by Windows
//! itself through `Shell_NotifyIconW`.
//!
//! The plan named the `tray-icon` crate. It can't show the one-time "Still playing"
//! balloon and doesn't report middle clicks, so this talks to the shell directly.
//!
//! A hidden window of our own receives the icon's messages. It lives on the UI thread,
//! so winit's message loop delivers them; each one is turned into a [`TrayCommand`] that
//! the app handles once the message is done.

use std::cell::{Cell, RefCell};

use lrc_core::system::TrayTexts;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, GWLP_USERDATA, GetCursorPos, GetSystemMetrics,
    GetWindowLongPtrW, HICON, HMENU, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, LoadIconW,
    LoadImageW, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON, SetForegroundWindow, SetWindowLongPtrW,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WINDOW_EX_STYLE, WM_APP,
    WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_MBUTTONUP, WM_NCCREATE, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
    WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

use crate::tray::{TrayCommand, TrayItem};

/// The message the shell sends about our icon.
const WM_TRAY: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;
/// The resource number `winresource` gives the icon in `build.rs`.
const ICON_RESOURCE: u16 = 1;

/// Everything the window procedure needs, behind the window's user data.
struct Inner {
    hwnd: Cell<HWND>,
    icon: HICON,
    owns_icon: bool,
    texts: RefCell<(TrayTexts, bool)>,
    on_command: Box<dyn Fn(TrayCommand)>,
    /// Sent to every top-level window when Explorer restarts: the icon must be added again.
    taskbar_created: u32,
    /// The left button went up after a double-click; that up isn't a click of its own.
    after_double: Cell<bool>,
}

pub struct Tray {
    inner: Box<Inner>,
}

impl Tray {
    /// Puts the icon in the tray. `None` if Windows has no tray to put it in.
    pub fn new(texts: TrayTexts, on_command: impl Fn(TrayCommand) + 'static) -> Option<Self> {
        // SAFETY: plain Win32 calls with valid arguments; the window class and window
        // stay alive for as long as the tray (the window is destroyed in `drop`).
        unsafe {
            let instance = GetModuleHandleW(None).ok()?;
            let class = w!("LRCOpenPlayerTray");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            };
            // Fails harmlessly if the class is already registered.
            RegisterClassW(&wc);

            let (icon, owns_icon) = match LoadImageW(
                Some(instance.into()),
                PCWSTR(ICON_RESOURCE as usize as *const u16),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            ) {
                Ok(h) => (HICON(h.0), true),
                Err(_) => (LoadIconW(None, IDI_APPLICATION).ok()?, false),
            };

            let mut inner = Box::new(Inner {
                hwnd: Cell::new(HWND::default()),
                icon,
                owns_icon,
                texts: RefCell::new((texts, false)),
                on_command: Box::new(on_command),
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                after_double: Cell::new(false),
            });
            // A hidden top-level window (not message-only, so it hears TaskbarCreated).
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                w!("LRC Open Player tray"),
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                Some(std::ptr::from_mut::<Inner>(&mut inner).cast()),
            )
            .ok()?;
            inner.hwnd.set(hwnd);
            let tray = Tray { inner };
            if !tray.inner.add() {
                return None;
            }
            Some(tray)
        }
    }

    /// `_update_tray`: the menu's texts, the shuffle check and the tooltip.
    pub fn update(&self, texts: TrayTexts, shuffle: bool) {
        let tip_changed = self.inner.texts.borrow().0.tooltip != texts.tooltip;
        *self.inner.texts.borrow_mut() = (texts, shuffle);
        if tip_changed {
            let data = self.inner.data(NIF_TIP);
            // SAFETY: `data` is a valid NOTIFYICONDATAW for our icon.
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            }
        }
    }

    /// `QSystemTrayIcon.showMessage` with the Information icon.
    pub fn show_message(&self, title: &str, text: &str, timeout_ms: u32) {
        let mut data = self.inner.data(NIF_INFO);
        copy_wide(&mut data.szInfoTitle, title);
        copy_wide(&mut data.szInfo, text);
        data.dwInfoFlags = NIIF_INFO;
        data.Anonymous.uTimeout = timeout_ms;
        // SAFETY: as in `update`.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = self.inner.data(Default::default());
        // SAFETY: removes our own icon and window, which nothing uses afterwards.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            SetWindowLongPtrW(self.inner.hwnd.get(), GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.inner.hwnd.get());
            if self.inner.owns_icon {
                let _ = DestroyIcon(self.inner.icon);
            }
        }
    }
}

impl Inner {
    fn data(&self, flags: windows::Win32::UI::Shell::NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd.get(),
            uID: ICON_ID,
            uFlags: flags,
            ..Default::default()
        }
    }

    fn add(&self) -> bool {
        let mut data = self.data(NIF_ICON | NIF_MESSAGE | NIF_TIP);
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = self.icon;
        copy_wide(&mut data.szTip, &self.texts.borrow().0.tooltip);
        // SAFETY: `data` is a valid NOTIFYICONDATAW for our window.
        unsafe { Shell_NotifyIconW(NIM_ADD, &data).as_bool() }
    }

    fn on_tray_message(&self, event: u32) {
        match event {
            WM_LBUTTONUP if self.after_double.replace(false) => {}
            WM_LBUTTONUP => (self.on_command)(TrayCommand::Clicked),
            WM_LBUTTONDBLCLK => {
                self.after_double.set(true);
                (self.on_command)(TrayCommand::DoubleClicked);
            }
            WM_MBUTTONUP => (self.on_command)(TrayCommand::MiddleClicked),
            WM_RBUTTONUP => {
                if let Some(item) = self.show_menu() {
                    (self.on_command)(TrayCommand::Menu(item));
                }
            }
            _ => {}
        }
    }

    /// The right-click menu, built from the latest texts each time it opens (the
    /// Python app refreshes it on `aboutToShow`).
    fn show_menu(&self) -> Option<TrayItem> {
        let (texts, shuffle) = self.texts.borrow().clone();
        // SAFETY: the menu is created, shown and destroyed here; the strings outlive
        // the calls that use them.
        unsafe {
            let menu = CreatePopupMenu().ok()?;
            let add = |id: TrayItem, text: &str, flags| {
                // A lone & marks a shortcut key in menus.
                let wide = wide(&text.replace('&', "&&"));
                let _ = AppendMenuW(menu, MF_STRING | flags, id as usize, PCWSTR(wide.as_ptr()));
            };
            let separator = |menu: HMENU| {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            };
            add(TrayItem::Now, &texts.now, MF_GRAYED);
            separator(menu);
            add(TrayItem::Play, texts.play, Default::default());
            add(TrayItem::Next, "Next", Default::default());
            add(TrayItem::Previous, "Previous", Default::default());
            separator(menu);
            add(
                TrayItem::Shuffle,
                "Shuffle",
                if shuffle {
                    MF_CHECKED
                } else {
                    Default::default()
                },
            );
            add(TrayItem::Repeat, texts.repeat, Default::default());
            separator(menu);
            add(TrayItem::Window, texts.window, Default::default());
            add(TrayItem::Quit, "Quit", Default::default());

            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let hwnd = self.hwnd.get();
            // Without this the menu doesn't close when clicking elsewhere (KB135788).
            let _ = SetForegroundWindow(hwnd);
            let chosen = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
                pt.x,
                pt.y,
                Some(0),
                hwnd,
                None,
            );
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            TrayItem::from_id(chosen.0 as usize)
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the user data is the `Inner` that `Tray::new` passed at creation, which
    // outlives the window (`Tray::drop` clears it before destroying the window).
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let inner = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Inner;
        if let Some(inner) = inner.as_ref() {
            if msg == WM_TRAY {
                // A panic must not unwind into Windows.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    inner.on_tray_message(lparam.0 as u32 & 0xFFFF)
                }));
                return LRESULT(0);
            }
            if msg == inner.taskbar_created && msg != 0 {
                inner.add();
                return LRESULT(0);
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/// A NUL-terminated UTF-16 copy of `text`.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copies `text` into a fixed-size UTF-16 field, cut to fit and NUL-terminated.
fn copy_wide(field: &mut [u16], text: &str) {
    let mut n = 0;
    for unit in text.encode_utf16().take(field.len().saturating_sub(1)) {
        field[n] = unit;
        n += 1;
    }
    field[n] = 0;
}
