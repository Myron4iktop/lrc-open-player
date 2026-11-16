//! "Publish…" in the sync editor: sends the synced lyrics to LRCLIB (not in the Python
//! app). The rules and the requests are in `lrc_core::publish`; this file runs the
//! window, where the song's details can be corrected first, and the worker thread that
//! solves LRCLIB's anti-spam check and sends the lyrics.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lrc_core::publish::{self as pb, Outcome, PublishForm, PublishLyrics, Step};
use lrc_core::tags::extract_metadata_and_cover;
use lrc_core::theme::Palette;
use slint::ComponentHandle;

use super::{App, app_call, fill_theme};
use crate::{PublishDialog, Theme};

/// The publish window while it is open.
pub(super) struct Publishing {
    dialog: PublishDialog,
    lyrics: PublishLyrics,
    duration_sec: u64,
    /// The publish running, if any: set to stop it.
    cancel: Option<Arc<AtomicBool>>,
    /// Answers for an earlier attempt, or after the window closed, are dropped.
    token: u64,
}

impl App {
    /// The sync editor's "Publish…": opens the window with the song's details, or says
    /// in the editor why there is nothing to publish yet.
    pub(super) fn open_publish(&mut self) {
        if let Some(open) = &self.publishing {
            let _ = open.dialog.show();
            crate::platform::raise(open.dialog.window());
            return;
        }
        let (track, text) = match self.sync_publish_input() {
            Ok(input) => input,
            Err(note) => return self.sync_note(note),
        };
        let lyrics = match PublishLyrics::from_lrc(&text) {
            Ok(lyrics) => lyrics,
            Err(note) => return self.sync_note(note),
        };
        let Ok(dialog) = PublishDialog::new() else {
            return;
        };
        crate::platform::hook_repaint(&dialog, |d| {
            d.set_repaint_count(d.get_repaint_count().wrapping_add(1))
        });
        fill_theme(&dialog.global::<Theme>(), &self.palette_now());
        dialog.invoke_sync_color_scheme();

        let form = self.publish_form_for(&track);
        dialog.set_track(form.track.as_str().into());
        dialog.set_artist(form.artist.as_str().into());
        dialog.set_album(form.album.as_str().into());
        dialog.set_length_text(pb::length_text(form.duration_sec).into());
        dialog.set_summary(pb::summary_text(&lyrics).into());
        dialog.set_notes(pb::PUBLISH_NOTES.into());
        dialog.set_status_text(form.problem().unwrap_or("").into());
        dialog.set_status_error(false);
        dialog.on_publish(|| app_call(|a, _| a.publish_now()));
        dialog.on_dismiss(|| app_call(|a, _| a.close_publish()));
        dialog.window().on_close_requested(|| {
            app_call(|a, _| a.close_publish());
            slint::CloseRequestResponse::HideWindow
        });
        let _ = dialog.show();
        self.publishing = Some(Publishing {
            dialog,
            lyrics,
            duration_sec: form.duration_sec,
            cancel: None,
            token: 0,
        });
    }

    /// The details of the editor's song, from the player when it is on that song (its
    /// length is then the one the decoder measured), otherwise from the file's tags.
    fn publish_form_for(&self, track: &Path) -> PublishForm {
        let info = match &self.info {
            Some(info) if self.is_current(track) => info.clone(),
            _ => extract_metadata_and_cover(track).info,
        };
        let measured_ms = if self.is_current(track) {
            self.player.duration_ms()
        } else {
            0
        };
        // LRCLIB matches lengths to the second, as the lookup does: whole seconds.
        let duration = if measured_ms > 0 {
            (measured_ms + 500) / 1000
        } else {
            info.duration_sec
        };
        PublishForm::from_tags(
            &info.title,
            info.title_from_tags,
            &info.artist,
            &info.album,
            duration,
        )
    }

    /// "Publish": checks the details, then publishes on a worker thread.
    fn publish_now(&mut self) {
        let Some(open) = &mut self.publishing else {
            return;
        };
        if open.cancel.is_some() {
            return; // already running
        }
        let form = PublishForm {
            track: open.dialog.get_track().into(),
            artist: open.dialog.get_artist().into(),
            album: open.dialog.get_album().into(),
            duration_sec: open.duration_sec,
        };
        if let Some(problem) = form.problem() {
            open.dialog.set_status_text(problem.into());
            open.dialog.set_status_error(true);
            return;
        }
        open.token += 1;
        let token = open.token;
        let cancel = Arc::new(AtomicBool::new(false));
        open.cancel = Some(cancel.clone());
        open.dialog.set_busy(true);
        open.dialog.set_status_error(false);
        open.dialog.set_status_text(pb::SOLVING_TEXT.into());
        let lyrics = open.lyrics.clone();
        let spawned = std::thread::Builder::new()
            .name("lrclib publish".into())
            .spawn(move || {
                let step = |s: Step| {
                    let _ = slint::invoke_from_event_loop(move || {
                        app_call(|a, _| a.publish_step(token, s));
                    });
                };
                let outcome = pb::publish(&form, &lyrics, &cancel, step);
                let track = form.track.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    app_call(|a, _| a.publish_done(token, outcome, &track));
                });
            });
        if spawned.is_err()
            && let Some(open) = &mut self.publishing
        {
            open.cancel = None;
            open.dialog.set_busy(false);
            open.dialog
                .set_status_text("Couldn't start publishing.".into());
            open.dialog.set_status_error(true);
        }
    }

    fn publish_step(&mut self, token: u64, step: Step) {
        let Some(open) = &self.publishing else { return };
        if open.token != token {
            return;
        }
        let text = match step {
            Step::Solving => pb::SOLVING_TEXT,
            Step::Sending => pb::SENDING_TEXT,
        };
        open.dialog.set_status_text(text.into());
    }

    fn publish_done(&mut self, token: u64, outcome: Outcome, track: &str) {
        let Some(open) = &mut self.publishing else {
            return;
        };
        if open.token != token {
            return;
        }
        open.cancel = None;
        open.dialog.set_busy(false);
        open.dialog
            .set_status_text(pb::outcome_text(&outcome, track).into());
        open.dialog
            .set_status_error(matches!(outcome, Outcome::Failed(_)));
        if outcome == Outcome::Published {
            open.dialog.set_done(true);
            self.sync_note("Published to LRCLIB.");
        }
    }

    /// Cancel, Stop, Close, Escape or the window's close button. A publish still
    /// running stops before it sends anything, if it hasn't yet.
    pub(super) fn close_publish(&mut self) {
        if let Some(open) = self.publishing.take() {
            if let Some(cancel) = &open.cancel {
                cancel.store(true, Ordering::Relaxed);
            }
            let _ = open.dialog.hide();
            super::editors::drop_later(open);
        }
    }

    /// The theme changed: the window follows.
    pub(super) fn theme_publish(&self, p: &Palette) {
        if let Some(open) = &self.publishing {
            fill_theme(&open.dialog.global::<Theme>(), p);
            open.dialog.invoke_sync_color_scheme();
        }
    }

    fn palette_now(&self) -> Palette {
        lrc_core::theme::build_palette(self.theme, self.accent)
    }
}
