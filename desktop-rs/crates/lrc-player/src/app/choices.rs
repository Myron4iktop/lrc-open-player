//! Choosing between the versions of a song's lyrics LRCLIB has (not in the Python app).
//!
//! LRCLIB often holds several entries for one song: other releases of it, a live take,
//! or the same words timed by someone else. A search normally takes the likeliest one.
//! With Lyrics ▾ → Let me choose the version on, a manual search lists them all in a
//! window of its own. Picking a row puts that version in the player at once, where it
//! follows the song as it plays, so the one whose lines keep time is easy to find.
//! "Use these lyrics" then carries on as a search does (keeps them as a .lrc, or asks);
//! Cancel puts back what was shown before.
//!
//! The rules are in `lrc_core::lrclib` ([`lrclib::candidates`]) and `lrc_core::window`
//! ([`rules::choice_row`]); this file runs the window.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use lrc_core::lrc;
use lrc_core::lrclib::{self, Candidate, Query};
use lrc_core::theme::Palette;
use lrc_core::window as rules;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::{App, app_call, fill_theme};
use crate::{ChoiceRowData, LyricsChoicesDialog, MainWindow, Theme};

/// The list of versions, while its window is open.
pub(super) struct Choosing {
    /// The song they are versions of.
    path: PathBuf,
    candidates: Vec<Candidate>,
    /// The lyrics on screen before the search, as text; `None` when there were none.
    before: Option<String>,
    /// `lyrics_shown` when the picked version went on screen. If other lyrics were shown
    /// since (a file loaded, an edit previewed), Cancel leaves those alone.
    shown: u64,
}

impl App {
    /// A manual search with "Let me choose the version" on: asks LRCLIB for every
    /// version on a worker thread.
    pub(super) fn search_lyrics_versions(&mut self, ui: &MainWindow, path: PathBuf, query: Query) {
        // Searching again while the list is open: Cancel still goes back to the lyrics
        // from before the first search, not to the version that was being tried.
        let before = match self.choosing.take() {
            Some(open) if open.shown == self.lyrics_shown && open.path == path => open.before,
            _ => (!self.shown.is_empty()).then(|| lrc::to_text(&self.shown, self.synced)),
        };
        self.hide_choices();
        self.cancel_lyrics_search();
        self.show_lyrics_message(ui, rules::SEARCHING_TEXT, "status");
        let cancel = Arc::new(AtomicBool::new(false));
        self.fetch_cancel = Some(cancel.clone());
        let shown = self.lyrics_shown;
        let _ = std::thread::Builder::new()
            .name("lrclib".into())
            .spawn(move || {
                if let Some(found) = lrclib::fetch_candidates(&query, &cancel) {
                    let _ = slint::invoke_from_event_loop(move || {
                        app_call(|a, ui| a.versions_fetched(ui, found, path, shown, before));
                    });
                }
            });
    }

    fn versions_fetched(
        &mut self,
        ui: &MainWindow,
        mut found: Vec<Candidate>,
        path: PathBuf,
        shown: u64,
        before: Option<String>,
    ) {
        if self.closing || !self.is_current(&path) {
            return; // a late answer for a song that is no longer current
        }
        if shown != self.lyrics_shown {
            return; // other lyrics were put on screen while this search ran; they stay
        }
        match found.len() {
            0 => self.nothing_found_online(ui, &path, true),
            // Nothing to choose between: as a search that found it.
            1 => {
                let only = found.remove(0).to_result();
                self.lyrics_found_online(ui, only, path, true);
            }
            _ => self.show_choices(ui, found, path, before),
        }
    }

    /// Opens the list with its first version, the one a search would have taken,
    /// already in the player.
    fn show_choices(
        &mut self,
        ui: &MainWindow,
        candidates: Vec<Candidate>,
        path: PathBuf,
        before: Option<String>,
    ) {
        if self.choices_dialog.is_none() {
            let Ok(dialog) = LyricsChoicesDialog::new() else {
                // No window to choose in: carry on as a search that took the likeliest.
                let first = candidates[0].to_result();
                self.lyrics_found_online(ui, first, path, true);
                return;
            };
            crate::platform::hook_repaint(&dialog, |d| {
                d.set_repaint_count(d.get_repaint_count().wrapping_add(1))
            });
            dialog.set_window_title(rules::CHOICES_TITLE.into());
            dialog.set_hint(rules::CHOICES_HINT.into());
            dialog.on_picked(|row| app_call(|a, ui| a.choice_picked(ui, row)));
            dialog.on_accepted(|| app_call(|a, ui| a.choice_accepted(ui)));
            dialog.on_dismiss(|| app_call(|a, ui| a.choices_cancelled(ui)));
            // Closing the window is Cancel, as Escape is.
            dialog.window().on_close_requested(|| {
                app_call(|a, ui| a.choices_cancelled(ui));
                slint::CloseRequestResponse::HideWindow
            });
            self.choices_dialog = Some(dialog);
        }
        let (title, artist, my_length) = self.info.as_ref().map_or_else(
            || (String::new(), String::new(), 0),
            |i| {
                let length = i64::try_from(i.duration_sec).unwrap_or(0);
                (i.title.clone(), i.artist.clone(), length)
            },
        );
        let rows: Vec<ChoiceRowData> = candidates
            .iter()
            .map(|c| {
                let row = rules::choice_row(c, my_length);
                ChoiceRowData {
                    title: row.title.into(),
                    detail: row.detail.into(),
                    kind: row.kind.into(),
                    synced: row.synced,
                }
            })
            .collect();
        let palette = lrc_core::theme::build_palette(self.theme, self.accent);

        self.parse_and_display_lyrics(ui, &candidates[0].lyrics);
        let heading = rules::choices_heading(candidates.len(), &title, &artist);
        self.choosing = Some(Choosing {
            path,
            candidates,
            before,
            shown: self.lyrics_shown,
        });
        let Some(dialog) = &self.choices_dialog else {
            return;
        };
        fill_theme(&dialog.global::<Theme>(), &palette);
        dialog.invoke_sync_color_scheme();
        dialog.set_heading(heading.into());
        dialog.set_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
        dialog.set_current(0);
        let _ = dialog.show();
        crate::platform::raise(dialog.window());
    }

    /// A row was picked: that version goes into the player, to be tried against the song.
    fn choice_picked(&mut self, ui: &MainWindow, row: i32) {
        let Some(open) = &self.choosing else { return };
        let Some(lyrics) = usize::try_from(row)
            .ok()
            .and_then(|r| open.candidates.get(r))
            .map(|c| c.lyrics.clone())
        else {
            return;
        };
        if !self.is_current(&open.path) {
            return;
        }
        self.parse_and_display_lyrics(ui, &lyrics);
        if let Some(open) = &mut self.choosing {
            open.shown = self.lyrics_shown;
        }
    }

    /// "Use these lyrics": the picked version is the search's answer.
    fn choice_accepted(&mut self, ui: &MainWindow) {
        let row = self.choices_dialog.as_ref().map_or(0, |d| d.get_current());
        self.hide_choices();
        let Some(open) = self.choosing.take() else {
            return;
        };
        let picked = usize::try_from(row)
            .ok()
            .and_then(|r| open.candidates.get(r));
        if let Some(picked) = picked
            && self.is_current(&open.path)
        {
            self.lyrics_found_online(ui, picked.to_result(), open.path, true);
        }
    }

    /// Cancel, Escape or the window's close button: back to the lyrics from before the
    /// search, unless other lyrics were shown in the meantime.
    fn choices_cancelled(&mut self, ui: &MainWindow) {
        self.hide_choices();
        let Some(open) = self.choosing.take() else {
            return;
        };
        if !self.is_current(&open.path) || open.shown != self.lyrics_shown {
            return;
        }
        match open.before {
            Some(text) => self.parse_and_display_lyrics(ui, &text),
            None => {
                if !self.show_own_lyrics(ui, &open.path) {
                    self.show_lyrics_message(ui, rules::NO_LYRICS_TEXT, "empty");
                }
            }
        }
    }

    /// The list no longer applies (another song, another search, or the app closing):
    /// its window goes, and the lyrics on screen stay as they are.
    pub(super) fn close_choices(&mut self) {
        self.choosing = None;
        self.hide_choices();
    }

    fn hide_choices(&self) {
        if let Some(dialog) = &self.choices_dialog {
            let _ = dialog.hide();
        }
    }

    /// The theme changed: the list follows.
    pub(super) fn theme_choices(&self, p: &Palette) {
        if let Some(dialog) = &self.choices_dialog {
            fill_theme(&dialog.global::<Theme>(), p);
            dialog.invoke_sync_color_scheme();
        }
    }
}
