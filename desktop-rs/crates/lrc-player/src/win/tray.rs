//! The tray icon (`_init_tray`, `_update_tray`, `_on_tray_activated`), drawn by Windows
//! itself through `Shell_NotifyIconW`.
//!
//! The plan named the `tray-icon` crate. It can't show the one-time "Still playing"
//! balloon and doesn't report middle clicks, so this talks to the shell directly.
//!
//! The icon's hidden window and its right-click menu live on a thread of their own, with
//! their own message loop. A menu opened from inside winit's event loop runs a second,
//! nested message loop, which is what froze the window with Slint's native menus; here
//! nothing nests inside winit. Clicks and menu choices go to the app as [`TrayCommand`]s.

use std::cell::Cell;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use lrc_core::system::TrayTexts;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFY_ICON_DATA_FLAGS, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetCursorPos,
    GetMessageW, GetSystemMetrics, GetWindowLongPtrW, HICON, HMENU, IDI_APPLICATION, IMAGE_ICON,
    LR_DEFAULTCOLOR, LoadIconW, LoadImageW, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON,
    SM_CYSMICON, SetForegroundWindow, SetWindowLongPtrW, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CLOSE,
    WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_MBUTTONUP, WM_NCCREATE, WM_NULL, WM_RBUTTONUP,
    WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

use crate::tray::{TrayCommand, TrayItem};

/// The message the shell sends about our icon.
const WM_TRAY: u32 = WM_APP + 1;
/// Sent by the UI thread: the tooltip changed.
const WM_TIP_CHANGED: u32 = WM_APP + 2;
/// Sent by the UI thread: show the balloon in `Shared::balloon`.
const WM_BALLOON: u32 = WM_APP + 3;
const ICON_ID: u32 = 1;
/// The resource number `winresource` gives the icon in `build.rs`.
const ICON_RESOURCE: u16 = 1;

/// What the UI thread and the tray thread share.
struct Shared {
    texts: TrayTexts,
    shuffle: bool,
    /// Title, text and timeout of a balloon waiting to be shown.
    balloon: Option<(String, String, u32)>,
}

/// The tray thread's state, behind its window's user data.
struct Inner {
    hwnd: HWND,
    icon: HICON,
    owns_icon: bool,
    shared: Arc<Mutex<Shared>>,
    on_command: Box<dyn Fn(TrayCommand) + Send>,
    /// Sent to every top-level window when Explorer restarts: the icon must be added again.
    taskbar_created: u32,
    /// The left button went up after a double-click; that up isn't a click of its own.
    after_double: Cell<bool>,
}

pub struct Tray {
    /// The tray window, as an integer so the handle can cross threads.
    hwnd: isize,
    shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
}

impl Tray {
    /// Puts the icon in the tray. `None` if Windows has no tray to put it in.
    pub fn new(
        texts: TrayTexts,
        on_command: impl Fn(TrayCommand) + Send + 'static,
    ) -> Option<Self> {
        let shared = Arc::new(Mutex::new(Shared {
            texts,
            shuffle: false,
            balloon: None,
        }));
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("tray".into())
            .spawn(move || run(thread_shared, Box::new(on_command), ready_tx))
            .ok()?;
        match ready_rx.recv() {
            Ok(Some(hwnd)) => Some(Tray {
                hwnd,
                shared,
                thread: Some(thread),
            }),
            _ => {
                let _ = thread.join();
                None
            }
        }
    }

    /// `_update_tray`: the menu's texts, the shuffle check and the tooltip.
    pub fn update(&self, texts: TrayTexts, shuffle: bool) {
        let Ok(mut shared) = self.shared.lock() else {
            return;
        };
        let tip_changed = shared.texts.tooltip != texts.tooltip;
        shared.texts = texts;
        shared.shuffle = shuffle;
        drop(shared);
        if tip_changed {
            self.post(WM_TIP_CHANGED);
        }
    }

    /// `QSystemTrayIcon.showMessage` with the Information icon.
    pub fn show_message(&self, title: &str, text: &str, timeout_ms: u32) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.balloon = Some((title.to_owned(), text.to_owned(), timeout_ms));
        }
        self.post(WM_BALLOON);
    }

    fn post(&self, msg: u32) {
        // SAFETY: posting to our tray window; if it is gone the call just fails.
        unsafe {
            let _ = PostMessageW(Some(HWND(self.hwnd as *mut _)), msg, WPARAM(0), LPARAM(0));
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // The tray thread removes the icon and ends its loop.
        self.post(WM_CLOSE);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The tray thread: makes the window and the icon, then runs its message loop until
/// the window is closed.
fn run(
    shared: Arc<Mutex<Shared>>,
    on_command: Box<dyn Fn(TrayCommand) + Send>,
    ready: mpsc::Sender<Option<isize>>,
) {
    // SAFETY: plain Win32 calls with valid arguments, all on this thread. `inner` lives
    // on this stack frame until the message loop ends, after the window is destroyed.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            let _ = ready.send(None);
            return;
        };
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
            Err(_) => match LoadIconW(None, IDI_APPLICATION) {
                Ok(h) => (h, false),
                Err(_) => {
                    let _ = ready.send(None);
                    return;
                }
            },
        };

        let mut inner = Box::new(Inner {
            hwnd: HWND::default(),
            icon,
            owns_icon,
            shared,
            on_command,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            after_double: Cell::new(false),
        });
        // A hidden top-level window (not message-only, so it hears TaskbarCreated).
        let Ok(hwnd) = CreateWindowExW(
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
        ) else {
            let _ = ready.send(None);
            return;
        };
        inner.hwnd = hwnd;
        if !inner.add() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(hwnd);
            let _ = ready.send(None);
            return;
        }
        let _ = ready.send(Some(hwnd.0 as isize));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if inner.owns_icon {
            let _ = DestroyIcon(inner.icon);
        }
    }
}

impl Inner {
    fn data(&self, flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            uFlags: flags,
            ..Default::default()
        }
    }

    fn tooltip(&self) -> String {
        self.shared
            .lock()
            .map(|s| s.texts.tooltip.clone())
            .unwrap_or_default()
    }

    fn add(&self) -> bool {
        let mut data = self.data(NIF_ICON | NIF_MESSAGE | NIF_TIP);
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = self.icon;
        copy_wide(&mut data.szTip, &self.tooltip());
        // SAFETY: `data` is a valid NOTIFYICONDATAW for our window.
        unsafe { Shell_NotifyIconW(NIM_ADD, &data).as_bool() }
    }

    fn update_tip(&self) {
        let mut data = self.data(NIF_TIP);
        copy_wide(&mut data.szTip, &self.tooltip());
        // SAFETY: as in `add`.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    fn show_balloon(&self) {
        let Some((title, text, timeout_ms)) =
            self.shared.lock().ok().and_then(|mut s| s.balloon.take())
        else {
            return;
        };
        let mut data = self.data(NIF_INFO);
        copy_wide(&mut data.szInfoTitle, &title);
        copy_wide(&mut data.szInfo, &text);
        data.dwInfoFlags = NIIF_INFO;
        data.Anonymous.uTimeout = timeout_ms;
        // SAFETY: as in `add`.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    fn remove(&self) {
        let data = self.data(NOTIFY_ICON_DATA_FLAGS::default());
        // SAFETY: as in `add`.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
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
        let (texts, shuffle) = {
            let shared = self.shared.lock().ok()?;
            (shared.texts.clone(), shared.shuffle)
        };
        // SAFETY: the menu is created, shown and destroyed here, on the tray thread; the
        // strings outlive the calls that use them.
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
            // Without this the menu doesn't close when clicking elsewhere (KB135788).
            let _ = SetForegroundWindow(self.hwnd);
            let chosen = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
                pt.x,
                pt.y,
                Some(0),
                self.hwnd,
                None,
            );
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            TrayItem::from_id(chosen.0 as usize)
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the user data is the `Inner` that `run` passed at creation, which outlives
    // the window (`run` keeps it until the message loop has ended).
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        let inner = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Inner;
        if let Some(inner) = inner.as_ref() {
            // A panic must not unwind into Windows.
            let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match msg {
                WM_TRAY => {
                    inner.on_tray_message(lparam.0 as u32 & 0xFFFF);
                    true
                }
                WM_TIP_CHANGED => {
                    inner.update_tip();
                    true
                }
                WM_BALLOON => {
                    inner.show_balloon();
                    true
                }
                WM_CLOSE => {
                    inner.remove();
                    let _ = DestroyWindow(hwnd);
                    true
                }
                WM_DESTROY => {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    PostQuitMessage(0);
                    true
                }
                _ if msg == inner.taskbar_created && msg != 0 => {
                    inner.add();
                    true
                }
                _ => false,
            }));
            if matches!(handled, Ok(true) | Err(_)) {
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
