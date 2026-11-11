//! Unexpected errors (`install_error_handler`): one unreadable file or a bug in a button
//! handler shouldn't stop the music. A panic in a UI callback is caught ([`guard`]) and
//! reported in a message box, one at a time and not the same one again within 10 s, and
//! the player keeps running. A panic on a background thread is only written to stderr,
//! as in the Python app.

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use lrc_core::system::{ErrorGate, error_summary};
use lrc_core::theme::Palette;
use slint::{ComponentHandle, Timer};

use crate::{ErrorDialog, Theme};

static MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();
/// The last panic on the UI thread, as (summary, details), for [`guard`] to show.
static LAST_PANIC: Mutex<Option<(String, String)>> = Mutex::new(None);

thread_local! {
    static GATE: RefCell<ErrorGate> = RefCell::new(ErrorGate::default());
    static DIALOG: RefCell<Option<ErrorDialog>> = const { RefCell::new(None) };
    static PALETTE: RefCell<Option<Palette>> = const { RefCell::new(None) };
}

/// Installs the panic hook. Call once, from the UI thread, before anything else.
pub fn install() {
    let _ = MAIN_THREAD.set(std::thread::current().id());
    panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown error".to_owned());
        let thread = std::thread::current();
        let place = info
            .location()
            .map(|l| format!(" at {}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let details = format!(
            "thread '{}' panicked{place}:\n{message}\n\n{}",
            thread.name().unwrap_or("<unnamed>"),
            std::backtrace::Backtrace::force_capture()
        );
        eprintln!("{details}");
        if MAIN_THREAD.get() == Some(&thread.id())
            && let Ok(mut last) = LAST_PANIC.lock()
        {
            *last = Some((format!("panic: {message}"), details));
        }
    }));
}

/// Runs `f`, and if it panics, reports the panic and returns `None`.
pub fn guard<R>(f: impl FnOnce() -> R) -> Option<R> {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => Some(r),
        Err(_) => {
            let (summary, details) = LAST_PANIC
                .lock()
                .ok()
                .and_then(|mut last| last.take())
                .unwrap_or_else(|| ("panic: unknown error".to_owned(), String::new()));
            report(summary, details);
            None
        }
    }
}

/// The palette the message box uses, kept in step with the main window's.
pub fn set_palette(palette: Palette) {
    DIALOG.with(|d| {
        if let Some(dialog) = d.borrow().as_ref() {
            crate::app::fill_theme(&dialog.global::<Theme>(), &palette);
        }
    });
    PALETTE.with(|p| *p.borrow_mut() = Some(palette));
}

/// Shows "Something went wrong, but the player is still running." with `summary` and,
/// behind "Show Details…", `details`. Shown once the current callback has returned.
pub fn report(summary: String, details: String) {
    let summary = error_summary(&summary);
    if !GATE.with(|g| g.borrow_mut().open(&summary, Instant::now())) {
        return; // one message at a time, and not the same one over and over
    }
    Timer::single_shot(Duration::ZERO, move || {
        let shown = DIALOG.with(|d| {
            let mut d = d.borrow_mut();
            if d.is_none() {
                let dialog = ErrorDialog::new().ok()?;
                let weak = dialog.as_weak();
                dialog.on_dismiss(move || {
                    if let Some(dialog) = weak.upgrade() {
                        let _ = dialog.hide();
                    }
                    GATE.with(|g| g.borrow_mut().closed());
                });
                dialog.window().on_close_requested(|| {
                    GATE.with(|g| g.borrow_mut().closed());
                    slint::CloseRequestResponse::HideWindow
                });
                *d = Some(dialog);
            }
            let dialog = d.as_ref()?;
            if let Some(p) = PALETTE.with(|p| p.borrow().clone()) {
                crate::app::fill_theme(&dialog.global::<Theme>(), &p);
            }
            dialog.set_summary(summary.as_str().into());
            dialog.set_details(details.as_str().into());
            dialog.set_details_shown(false);
            dialog.show().ok()
        });
        if shown.is_none() {
            GATE.with(|g| g.borrow_mut().closed());
        }
    });
}
