//! The lyrics editors and the AI settings window: `LyricsSyncDialog`,
//! `TranslationDialog` and `AiSettingsDialog` in lrcplayer.py.
//!
//! The rules and texts are in `lrc_core::editor`, `lrc_core::align` and `lrc_core::ai`;
//! this file connects them to the Slint windows in `ui/editors.slint` and to the player.
//! It is a child of `app`, so it works on the [`App`] directly.
//!
//! Questions that wait for an answer (message boxes, file dialogs) are asked while the
//! app is not borrowed, so the playback clock keeps running behind them.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use lrc_core::ai::{self, AlignEvent, AlignRequest, Cancel, CheckResult, Pricing};
use lrc_core::editor::{self as ed, Card, LineLook, SyncEditor, TimeLook, TranslationEditor};
use lrc_core::lrc;
use lrc_core::py;
use lrc_core::text::{read_lyrics_file, write_lrc};
use lrc_core::theme::build_palette;
use lrc_core::window::lrc_path_for;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use super::{
    App, app_call, ask, ask_yes_no, embed_lyrics_in_file, file_name, fill_theme, message_later,
    with_app_ret,
};
use crate::audio::State;
use crate::{
    AiSettingsDialog, AnswerDialog, MainWindow, SyncDialog, SyncRowData, Theme, TranslationDialog,
    TranslationRowData,
};

/// The open "Sync lyrics" window.
pub(super) struct SyncWindow {
    dialog: SyncDialog,
    ed: SyncEditor,
    track_path: PathBuf,
    rows: Rc<VecModel<SyncRowData>>,
    /// `_tick`, every 80 ms.
    _timer: Timer,
}

/// The open "Add translation" window.
pub(super) struct TranslateWindow {
    dialog: TranslationDialog,
    ed: TranslationEditor,
    track_path: PathBuf,
    rows: Rc<VecModel<TranslationRowData>>,
    /// The AI request running, if any.
    worker: Option<Arc<Cancel>>,
    /// Answers for an older request are dropped.
    token: u64,
    model_name: String,
    waiting: &'static str,
    answer: Option<AnswerDialog>,
}

/// The open "AI alignment" settings window.
pub(super) struct AiWindow {
    dialog: AiSettingsDialog,
    local_models: Vec<String>,
    or_models: Vec<String>,
    or_pricing: Pricing,
    /// Test results for an older test, or after the window closed, are dropped.
    token: u64,
}

/// Puts `rows` in a list, changing only the rows that differ. Rebuilding the whole list
/// would recreate every row, so a double-click whose first click changed the list (the
/// sync editor's cursor) would never arrive, and the list would lose its place.
fn update_rows<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.row_count() != rows.len() {
        model.set_vec(rows);
        return;
    }
    for (i, row) in rows.into_iter().enumerate() {
        if model.row_data(i).as_ref() != Some(&row) {
            model.set_row_data(i, row);
        }
    }
}

/// Something the current callback hands to the event loop to drop, once it returns.
fn drop_later<T: 'static>(value: T) {
    let mut value = Some(value);
    Timer::single_shot(Duration::ZERO, move || drop(value.take()));
}

thread_local! {
    /// An editor is waiting for an answer (a message box or a file dialog). Those run
    /// on threads of their own, so the windows keep working; a second click on Save
    /// meanwhile is ignored rather than asking twice.
    static ASKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs an editor flow that asks questions, unless one is already asking.
fn ask_flow(flow: impl std::future::Future<Output = ()> + 'static) {
    if ASKING.with(|a| a.replace(true)) {
        return;
    }
    if slint::spawn_local(async move {
        flow.await;
        ASKING.with(|a| a.set(false));
    })
    .is_err()
    {
        ASKING.with(|a| a.set(false));
    }
}

async fn warning(title: &str, text: &str) {
    ask(
        rfd::MessageLevel::Warning,
        title,
        text,
        rfd::MessageButtons::Ok,
    )
    .await;
}

/// The file dialog of "Load .txt / .lrc…", in the song's folder.
async fn pick_lyrics_file(title: &str, track: &Path, lrc_first: bool) -> Option<PathBuf> {
    let exts: &[&str] = if lrc_first {
        &["lrc", "txt"]
    } else {
        &["txt", "lrc"]
    };
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title(title)
        .add_filter("Lyrics", exts)
        .add_filter("All files", &["*"]);
    if let Some(dir) = track.parent() {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_file().await.map(|f| f.path().to_path_buf())
}

/// Writes a .lrc next to the song, asking first if one is there: `save` in both
/// editors. `question` gets the existing file's name, if any, and returns the question
/// to ask (or `None`). Returns the path written.
async fn save_lrc_asking(
    track: &Path,
    text: &str,
    question: impl FnOnce(Option<&str>) -> Option<String>,
) -> Option<PathBuf> {
    let path = lrc_path_for(track);
    let name = file_name(&path);
    let existing = path.exists().then_some(name.as_str());
    if let Some(q) = question(existing)
        && !ask_yes_no("Save lyrics", &q).await
    {
        return None;
    }
    if let Err(e) = write_lrc(&path, text) {
        warning("Save lyrics", &ed::save_failed(&e.to_string())).await;
        return None;
    }
    Some(path)
}

impl App {
    /// `_display_for`: what a line shows. Romaji (M5) will change this; until then it
    /// is the line itself.
    fn display_for(&self, main: &str, sub: &str) -> (String, String) {
        (main.to_owned(), sub.to_owned())
    }

    /// `_track_ok`: the player is still on the editor's song.
    fn track_ok(&self, path: &Path) -> bool {
        self.is_current(path)
    }

    /// `_editor_open`: a song whose lyrics are being edited doesn't move on at its end.
    pub(super) fn editor_open(&self) -> bool {
        self.sync.is_some() || self.translate.is_some()
    }

    fn palette(&self) -> lrc_core::theme::Palette {
        build_palette(self.theme, self.accent)
    }

    /// The song's title for the window title.
    fn editor_title(&self, path: &Path) -> String {
        self.info
            .as_ref()
            .map(|i| i.title.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| file_name(path))
    }

    /// The theme changed: every editor window follows (`apply_theme` of the dialogs).
    pub(super) fn theme_editors(&self, p: &lrc_core::theme::Palette) {
        if let Some(w) = &self.sync {
            fill_theme(&w.dialog.global::<Theme>(), p);
            w.dialog.invoke_sync_color_scheme();
        }
        if let Some(w) = &self.translate {
            fill_theme(&w.dialog.global::<Theme>(), p);
            w.dialog.invoke_sync_color_scheme();
            if let Some(a) = &w.answer {
                fill_theme(&a.global::<Theme>(), p);
                a.invoke_sync_color_scheme();
            }
        }
        if let Some(w) = &self.ai_window {
            fill_theme(&w.dialog.global::<Theme>(), p);
            w.dialog.invoke_sync_color_scheme();
        }
    }

    /// The main window closes: the editors go with it, and an AI request stops.
    pub(super) fn close_editors(&mut self) {
        if let Some(w) = self.sync.take() {
            let _ = w.dialog.hide();
            drop_later(w);
        }
        if let Some(w) = self.translate.take() {
            if let Some(c) = &w.worker {
                c.cancel();
            }
            let _ = w.dialog.hide();
            if let Some(a) = &w.answer {
                let _ = a.hide();
            }
            drop_later(w);
        }
        if let Some(w) = self.ai_window.take() {
            let _ = w.dialog.hide();
            drop_later(w);
        }
    }

    // =====================================================================================
    // Sync lyrics
    // =====================================================================================

    /// Ctrl+T (`open_sync_dialog`).
    pub(super) fn open_sync_dialog(&mut self, _ui: &MainWindow) {
        if let Some(w) = &self.sync {
            let _ = w.dialog.show();
            crate::platform::raise(w.dialog.window());
            return;
        }
        let Some(path) = self.queue.current_path().cloned() else {
            return;
        };
        let Ok(dialog) = SyncDialog::new() else {
            return;
        };
        fill_theme(&dialog.global::<Theme>(), &self.palette());
        dialog.invoke_sync_color_scheme();
        dialog.set_window_title(format!("Sync lyrics — {}", self.editor_title(&path)).into());
        dialog.set_intro_text(ed::SYNC_INTRO.into());
        dialog.set_help_text(ed::SYNC_HELP.into());
        // Starts from whatever the player shows (e.g. an LRC that is out of sync).
        let shown = self.shown_lyrics_text(&path);
        dialog.set_use_shown_enabled(!shown.is_empty());
        dialog.set_input_text(shown.into());
        dialog.set_page(0);
        let rows = Rc::new(VecModel::default());
        dialog.set_rows(ModelRc::from(rows.clone()));
        connect_sync(&dialog);

        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(80), || {
            app_call(|a, _ui| a.sync_tick());
        });
        let _ = dialog.show();
        self.sync = Some(SyncWindow {
            dialog,
            ed: SyncEditor::default(),
            track_path: path,
            rows,
            _timer: timer,
        });
        self.sync_tick();
    }

    /// `_shown_lyrics_text`: the lyrics in the player, as text (with timestamps if they
    /// are synced), if the player is on `path`.
    fn shown_lyrics_text(&self, path: &Path) -> String {
        if !self.track_ok(path) {
            return String::new();
        }
        lrc::to_text(&self.shown, self.synced)
            .trim_end_matches('\n')
            .to_owned()
    }

    fn sync_tick(&mut self) {
        let Some(w) = &self.sync else { return };
        let ok = self.track_ok(&w.track_path);
        let d = &w.dialog;
        d.set_playing(self.player.state() == State::Playing);
        d.set_time_text(
            if ok {
                lrc::fmt_lrc_time(self.player.position_ms() as i64)
            } else {
                "--:--.--".to_owned()
            }
            .into(),
        );
        d.set_track_ok(ok);
        if !ok {
            d.set_warn_text(ed::other_song_warning(&file_name(&w.track_path)).into());
        }
    }

    fn sync_status(&self, text: &str) {
        if let Some(w) = &self.sync {
            w.dialog.set_status_text(text.into());
        }
    }

    /// `_refresh` (and `_rebuild_rows`): the list from the entries.
    fn sync_refresh(&mut self, scroll: bool) {
        let Some(w) = &self.sync else { return };
        let rows: Vec<SyncRowData> =
            w.ed.rows()
                .into_iter()
                .zip(&w.ed.entries)
                .enumerate()
                .map(|(i, (row, e))| {
                    let (main, sub) = self.display_for(&e.text, &e.sub);
                    SyncRowData {
                        cursor: row.cursor,
                        time: row.time.into(),
                        time_look: match row.time_look {
                            TimeLook::None => 0,
                            TimeLook::Ok => 1,
                            TimeLook::Error => 2,
                        },
                        main: main.into(),
                        sub: sub.into(),
                        look: match row.line_look {
                            LineLook::Cursor => 0,
                            LineLook::Open => 1,
                            LineLook::Stamped => 2,
                        },
                        selected: w.ed.selected == Some(i),
                    }
                })
                .collect();
        w.dialog.set_progress_text(w.ed.progress_text().into());
        let target = if scroll {
            w.ed.scroll_row().map(|r| (r, rows.clone()))
        } else {
            None
        };
        update_rows(&w.rows, rows);
        if let Some((row, rows)) = target {
            // `scrollToItem(…, PositionAtCenter)`, from the rows' fixed heights.
            let heights: Vec<f32> = rows
                .iter()
                .map(|r| if r.sub.is_empty() { 42.0 } else { 62.0 })
                .collect();
            let top: f32 = 1.0 + heights[..row].iter().map(|h| h + 2.0).sum::<f32>();
            let total: f32 = 2.0 + heights.iter().sum::<f32>() + 2.0 * (heights.len() as f32 - 1.0);
            let visible = w.dialog.get_list_height();
            let max = (total - visible).max(0.0);
            let y = (top + heights[row] / 2.0 - visible / 2.0).clamp(0.0, max);
            w.dialog.set_list_scroll(-y);
        }
    }

    fn sync_start(&mut self) {
        let Some(w) = &mut self.sync else { return };
        let text = w.dialog.get_input_text().to_string();
        match w.ed.start(&text) {
            Err(message) => {
                message_later(rfd::MessageLevel::Info, ed::SYNC_TITLE, message.to_owned());
            }
            Ok(status) => {
                w.dialog.set_page(1);
                w.dialog.set_status_text(status.into());
                w.dialog.invoke_focus_keys();
                // After the list has its size, so the ▶ line can be centred.
                Timer::single_shot(Duration::ZERO, || app_call(|a, _ui| a.sync_refresh(true)));
                self.sync_refresh(false);
            }
        }
    }

    fn sync_back_to_text(&mut self) {
        let Some(w) = &self.sync else { return };
        w.dialog
            .set_input_text(ed::entries_to_text(&w.ed.entries).into());
        w.dialog.set_page(0);
    }

    fn sync_seek_relative(&mut self, delta: i64) {
        let Some(w) = &self.sync else { return };
        if self.track_ok(&w.track_path) {
            let pos = self.player.position_ms() as i64;
            self.seek_to((pos + delta).max(0) as u64);
        }
    }

    fn sync_stamp(&mut self) {
        let pos = self.player.position_ms() as i64;
        let Some(w) = &mut self.sync else { return };
        if w.ed.entries.is_empty()
            || !self
                .queue
                .current_path()
                .is_some_and(|p| *p == w.track_path)
        {
            return;
        }
        if let Some(status) = w.ed.stamp(pos) {
            w.dialog.set_status_text(status.into());
            self.sync_refresh(true);
        }
    }

    fn sync_undo(&mut self) {
        let pos = self.player.position_ms() as i64;
        let Some(w) = &mut self.sync else { return };
        let Some(rewind) = w.ed.undo(pos) else { return };
        let path = w.track_path.clone();
        w.dialog.set_status_text(SyncEditor::UNDONE.into());
        self.sync_refresh(true);
        if self.track_ok(&path) {
            // Rewinds a little before the line so it can be stamped again.
            self.seek_to(rewind as u64);
        }
    }

    fn sync_add_break(&mut self) {
        let pos = self.player.position_ms() as i64;
        let ok = self
            .sync
            .as_ref()
            .is_some_and(|w| self.track_ok(&w.track_path));
        let Some(w) = &mut self.sync else { return };
        if ok && w.ed.insert_break(pos) {
            w.dialog.set_status_text(SyncEditor::BREAK_ADDED.into());
            self.sync_refresh(true);
        }
    }

    fn sync_nudge(&mut self, delta: i64) {
        let Some(w) = &mut self.sync else { return };
        match w.ed.nudge(delta) {
            Ok(()) => self.sync_refresh(false),
            Err(message) => w.dialog.set_status_text(message.into()),
        }
    }

    fn sync_move_cursor(&mut self, delta: i64) {
        let Some(w) = &mut self.sync else { return };
        if w.ed.move_cursor(delta) {
            self.sync_refresh(true);
        }
    }

    fn sync_row_clicked(&mut self, row: usize) {
        let Some(w) = &mut self.sync else { return };
        w.ed.click(row);
        self.sync_refresh(false);
    }

    fn sync_row_double_clicked(&mut self, row: usize) {
        let Some(w) = &self.sync else { return };
        if let Some(from) = w.ed.replay_from(row)
            && self.track_ok(&w.track_path)
        {
            self.seek_to(from as u64);
            if self.player.state() != State::Playing {
                self.player.play();
            }
        }
    }

    fn sync_preview(&mut self, ui: &MainWindow) {
        let Some(w) = &self.sync else { return };
        let (text, _) = ed::build_sync_lrc(&w.ed.entries);
        if text.is_empty() {
            return self.sync_status(ed::NOTHING_STAMPED);
        }
        if !self.track_ok(&w.track_path) {
            return self.sync_status(ed::OTHER_SONG_PREVIEW);
        }
        self.parse_and_display_lyrics(ui, &text);
        self.sync_status(ed::PREVIEW_SHOWN);
    }

    fn sync_copy(&mut self) {
        let Some(w) = &self.sync else { return };
        let (text, _) = ed::build_sync_lrc(&w.ed.entries);
        if !text.is_empty() {
            w.dialog.invoke_copy_text(text.into());
            self.sync_status(ed::COPIED);
        }
    }

    /// After a save or an embed: nothing is unsaved, and the player shows the result.
    fn sync_written(&mut self, ui: &MainWindow, text: &str, status: String) {
        let Some(w) = &mut self.sync else { return };
        w.ed.dirty = false;
        let path = w.track_path.clone();
        if self.track_ok(&path) {
            self.parse_and_display_lyrics(ui, text);
        }
        self.sync_status(&status);
    }

    // =====================================================================================
    // Add translation
    // =====================================================================================

    /// Ctrl+Shift+T (`open_translation_dialog`).
    pub(super) fn open_translation_dialog(&mut self, _ui: &MainWindow) {
        if let Some(w) = &self.translate {
            let _ = w.dialog.show();
            crate::platform::raise(w.dialog.window());
            return;
        }
        let Some(path) = self.queue.current_path().cloned() else {
            return;
        };
        if !self
            .shown
            .iter()
            .any(|c| !c.main.is_empty() && c.main != lrc::EMPTY_LINE_MARK)
        {
            message_later(
                rfd::MessageLevel::Info,
                ed::TRANSLATE_TITLE,
                ed::NO_LYRICS_TO_TRANSLATE.to_owned(),
            );
            return;
        }
        if !self.synced {
            message_later(
                rfd::MessageLevel::Info,
                ed::TRANSLATE_TITLE,
                ed::NOT_SYNCED_TO_TRANSLATE.to_owned(),
            );
            return;
        }
        let Ok(dialog) = TranslationDialog::new() else {
            return;
        };
        fill_theme(&dialog.global::<Theme>(), &self.palette());
        dialog.invoke_sync_color_scheme();
        dialog.set_window_title(format!("Add translation — {}", self.editor_title(&path)).into());
        dialog.set_intro_text(ed::TRANSLATE_INTRO.into());
        dialog.set_hint_text(ed::TRANSLATE_HINT.into());
        let rows = Rc::new(VecModel::default());
        dialog.set_rows(ModelRc::from(rows.clone()));
        connect_translation(&dialog);

        let cards = self
            .shown
            .iter()
            .map(|l| Card {
                time_ms: l.time_ms,
                main: l.main.clone(),
                sub: l.sub.clone(),
            })
            .collect();
        let mut editor = TranslationEditor::new(cards);
        let existing = editor.existing_translation();
        let _ = dialog.show();
        self.translate = Some(TranslateWindow {
            dialog,
            ed: editor,
            track_path: path,
            rows,
            worker: None,
            token: 0,
            model_name: String::new(),
            waiting: ed::waiting_text(false),
            answer: None,
        });
        self.update_ai_buttons();
        match existing {
            // These lyrics already carry a translation: start in the review table.
            Some(text) => {
                if let Some(w) = &self.translate {
                    w.dialog.set_input_text(text.into());
                    w.dialog.set_page(1);
                }
                self.translate_fill_table();
                self.translate_status(ed::ALREADY_TRANSLATED, false);
            }
            None => {
                if let Some(w) = &self.translate {
                    w.dialog.set_page(0);
                }
                self.translate_counts();
            }
        }
    }

    fn translate_status(&self, text: &str, error: bool) {
        if let Some(w) = &self.translate {
            w.dialog.set_status_text(text.into());
            w.dialog.set_status_error(error);
        }
    }

    fn translate_counts(&self) {
        if let Some(w) = &self.translate {
            let text = w.dialog.get_input_text();
            w.dialog.set_counts_text(w.ed.counts_text(&text).into());
        }
    }

    /// `update_ai_buttons`: "Align with AI" when AI is set up, "Set up AI…" otherwise.
    fn update_ai_buttons(&self) {
        let Some(w) = &self.translate else { return };
        let cfg = ai::config(&ai::raw_settings(
            &self.store.settings,
            crate::platform::cipher(),
        ));
        w.dialog.set_ai_ready(cfg.ready);
        if cfg.ready {
            let model = if cfg.model.is_empty() {
                "the model"
            } else {
                &cfg.model
            };
            w.dialog
                .set_ai_tip(format!("Uses {model} on {}", cfg.where_).into());
        }
    }

    /// `_fill_table`.
    fn translate_fill_table(&mut self) {
        let Some(w) = &self.translate else { return };
        let rows: Vec<TranslationRowData> = (0..w.ed.n_rows())
            .map(|r| {
                let sung = w.ed.sung(r);
                TranslationRowData {
                    time: ed::row_time_text(w.ed.row_time(r)).into(),
                    sung: self.display_for(sung, "").0.into(),
                    sung_tip: sung.into(),
                    translation: w.ed.texts[r].as_str().into(),
                    empty: w.ed.is_empty(r),
                    repeat: w.ed.is_repeat(r),
                    tip: w.ed.translation_tip(r).into(),
                }
            })
            .collect();
        update_rows(&w.rows, rows);
        let (cover, tip) = w.ed.coverage();
        w.dialog.set_cover_text(cover.into());
        w.dialog.set_cover_tip(tip.into());
    }

    fn translate_set_busy(&self, busy: bool) {
        if let Some(w) = &self.translate {
            w.dialog.set_busy(busy);
        }
    }

    fn translate_match(&mut self) {
        let Some(w) = &mut self.translate else { return };
        let text = w.dialog.get_input_text().to_string();
        match w.ed.match_lines(&text) {
            Err(message) => self.translate_status(message, true),
            Ok(status) => {
                w.dialog.set_raw_visible(false);
                w.dialog.set_page(1);
                w.dialog.set_selected(-1);
                self.translate_fill_table();
                self.translate_status(&status, false);
            }
        }
    }

    /// `align_with_ai`: sends the lyrics and the translation to the model on a worker
    /// thread.
    fn translate_align(&mut self) {
        let cfg = ai::config(&ai::raw_settings(
            &self.store.settings,
            crate::platform::cipher(),
        ));
        if !cfg.ready {
            self.update_ai_buttons();
            return self.translate_status(ed::AI_NOT_SET_UP, true);
        }
        let Some(w) = &mut self.translate else { return };
        let text = w.dialog.get_input_text().to_string();
        let messages = match w.ed.prepare_ai(&text) {
            Ok(m) => m,
            Err(message) => return self.translate_status(message, true),
        };
        w.dialog.set_raw_visible(false);
        let request = AlignRequest {
            base_url: cfg.url.clone(),
            model: cfg.model.clone(),
            key: cfg.key.clone(),
            messages,
            total_rows: w.ed.n_rows(),
            headers: cfg.headers.clone(),
        };
        let cancel = Cancel::new();
        w.token += 1;
        let token = w.token;
        w.worker = Some(cancel.clone());
        w.model_name = if cfg.model.is_empty() {
            "the model".to_owned()
        } else {
            cfg.model.clone()
        };
        w.waiting = ed::waiting_text(cfg.provider == "openrouter");
        let quitting = self.quitting.clone();
        let _ = std::thread::Builder::new()
            .name("ai-align".into())
            .spawn(move || {
                ai::run_alignment(&request, &cancel, |event| {
                    if quitting.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    let _ = slint::invoke_from_event_loop(move || {
                        app_call(|a, ui| a.translate_ai_event(ui, token, event));
                    });
                });
            });
        self.translate_set_busy(true);
        self.translate_status("", false);
        self.translate_progress("waiting", 0);
    }

    fn translate_progress(&self, phase: &str, answered: usize) {
        let Some(w) = &self.translate else { return };
        let (text, bar) = w.ed.progress(phase, answered, w.waiting);
        w.dialog.set_busy_text(text.into());
        match bar {
            Some((value, max)) => {
                w.dialog.set_progress_known(true);
                w.dialog.set_progress_value(if max == 0 {
                    0.0
                } else {
                    value as f32 / max as f32
                });
            }
            None => w.dialog.set_progress_known(false),
        }
    }

    fn translate_ai_event(&mut self, _ui: &MainWindow, token: u64, event: AlignEvent) {
        let Some(w) = &mut self.translate else { return };
        if w.token != token || w.worker.is_none() {
            return; // cancelled, or an older request
        }
        match event {
            AlignEvent::Progress { phase, answered } => self.translate_progress(phase, answered),
            AlignEvent::Done { answer, finish } => {
                w.worker = None;
                let name = w.model_name.clone();
                let (status, error) = w.ed.apply_ai(&answer, &finish, &name);
                w.dialog.set_raw_visible(true);
                self.translate_set_busy(false);
                if !error && let Some(w) = &self.translate {
                    w.dialog.set_page(1);
                    w.dialog.set_selected(-1);
                }
                if !error {
                    self.translate_fill_table();
                }
                self.translate_status(&status, error);
            }
            AlignEvent::Failed { message, raw } => {
                w.worker = None;
                w.dialog.set_raw_visible(!py::strip(&raw).is_empty());
                w.ed.raw_answer = raw;
                self.translate_set_busy(false);
                self.translate_status(&message, true);
            }
        }
    }

    fn translate_cancel_ai(&mut self) {
        let Some(w) = &mut self.translate else { return };
        if let Some(c) = w.worker.take() {
            c.cancel();
        }
        self.translate_set_busy(false);
        self.translate_status(ed::CANCELLED, false);
    }

    /// `_selected_row`: the clicked row, or a note to click one first.
    fn translate_selected(&self) -> Option<usize> {
        let w = self.translate.as_ref()?;
        match usize::try_from(w.dialog.get_selected()) {
            Ok(r) if r < w.ed.texts.len() => Some(r),
            _ => {
                self.translate_status(ed::CLICK_A_LINE, true);
                None
            }
        }
    }

    /// Shift down, Shift up and Clear (`_after_edit`).
    fn translate_edit_rows(&mut self, op: fn(&mut TranslationEditor, usize) -> usize) {
        let Some(row) = self.translate_selected() else {
            return;
        };
        let Some(w) = &mut self.translate else { return };
        let select = op(&mut w.ed, row);
        w.dialog.set_selected(select as i32);
        self.translate_fill_table();
        self.translate_status("", false);
    }

    fn translate_commit(&mut self, row: usize, text: &str) {
        let Some(w) = &mut self.translate else { return };
        if row >= w.ed.texts.len()
            || w.ed.texts[row] == py::strip(text) && !w.ed.skipped.contains(&row)
        {
            return; // unchanged
        }
        w.ed.edit(row, text);
        self.translate_fill_table();
    }

    fn translate_replay(&mut self, row: usize) {
        let Some(w) = &self.translate else { return };
        if row >= w.ed.n_rows() || !self.track_ok(&w.track_path) {
            return;
        }
        let from = w.ed.replay_from(row, self.offset_ms);
        self.seek_to(from as u64);
        if self.player.state() != State::Playing {
            self.player.play();
        }
    }

    fn translate_back_to_text(&mut self) {
        let Some(w) = &self.translate else { return };
        w.dialog.set_page(0);
        w.dialog.set_editing(-1);
        self.translate_status("", false);
        self.translate_counts();
    }

    fn translate_preview(&mut self, ui: &MainWindow) {
        let Some(w) = &self.translate else { return };
        if !self.track_ok(&w.track_path) {
            return self.translate_status(ed::OTHER_SONG_PREVIEW, true);
        }
        let text = w.ed.build_lrc();
        self.parse_and_display_lyrics(ui, &text);
        self.translate_status(ed::PREVIEW_SHOWN, false);
    }

    fn translate_written(&mut self, ui: &MainWindow, text: &str, status: String) {
        let Some(w) = &mut self.translate else { return };
        w.ed.dirty = false;
        let path = w.track_path.clone();
        if self.track_ok(&path) {
            self.parse_and_display_lyrics(ui, text);
        }
        self.translate_status(&status, false);
    }

    fn translate_show_raw(&mut self) {
        let palette = self.palette();
        let Some(w) = &mut self.translate else { return };
        if w.answer.is_none() {
            let Ok(dialog) = AnswerDialog::new() else {
                return;
            };
            dialog.set_window_title(ed::RAW_ANSWER_TITLE.into());
            dialog.set_message(ed::RAW_ANSWER_TEXT.into());
            let weak = dialog.as_weak();
            dialog.on_dismiss(move || {
                if let Some(d) = weak.upgrade() {
                    let _ = d.hide();
                }
            });
            w.answer = Some(dialog);
        }
        let Some(dialog) = &w.answer else { return };
        fill_theme(&dialog.global::<Theme>(), &palette);
        dialog.invoke_sync_color_scheme();
        let raw = if w.ed.raw_answer.is_empty() {
            "(empty)".to_owned()
        } else {
            w.ed.raw_answer.clone()
        };
        dialog.set_details(raw.into());
        dialog.set_details_shown(false);
        let _ = dialog.show();
    }

    // =====================================================================================
    // AI alignment settings
    // =====================================================================================

    /// View ▾ → AI alignment… and "Set up AI…" (`open_ai_settings`).
    pub(super) fn open_ai_settings(&mut self, _ui: &MainWindow) {
        if let Some(w) = &self.ai_window {
            let _ = w.dialog.show();
            crate::platform::raise(w.dialog.window());
            return;
        }
        let Ok(dialog) = AiSettingsDialog::new() else {
            return;
        };
        fill_theme(&dialog.global::<Theme>(), &self.palette());
        dialog.invoke_sync_color_scheme();
        dialog.set_intro_text("Optional help for Lyrics ▾ → Add translation: a language model works out which translated line belongs to which sung line. It only matches lines; it never writes or changes the translation.".into());
        dialog.set_keys_url(ai::OPENROUTER_KEYS_URL.into());
        let raw = ai::raw_settings(&self.store.settings, crate::platform::cipher());
        dialog.set_ai_enabled(raw.enabled);
        dialog.set_url(raw.url.into());
        dialog.set_model(raw.model.into());
        dialog.set_key(raw.key.into());
        dialog.set_or_key(raw.or_key.into());
        dialog.set_or_model(raw.or_model.into());
        dialog.set_provider(raw.provider.into());
        connect_ai_settings(&dialog);
        let _ = dialog.show();
        self.ai_window = Some(AiWindow {
            dialog,
            local_models: Vec::new(),
            or_models: Vec::new(),
            or_pricing: Pricing::new(),
            token: 0,
        });
        self.ai_provider_changed();
    }

    fn ai_test_status(&self, text: &str, error: bool) {
        if let Some(w) = &self.ai_window {
            w.dialog.set_test_text(text.into());
            w.dialog.set_test_error(error);
        }
    }

    /// `_on_provider`.
    fn ai_provider_changed(&mut self) {
        let Some(w) = &self.ai_window else { return };
        let hint = if w.dialog.get_provider() == "openrouter" {
            ai::openrouter_hint(ai::secrets_are_encrypted(crate::platform::cipher()))
        } else {
            ai::LOCAL_HINT.to_owned()
        };
        w.dialog.set_hint_text(hint.into());
        self.ai_test_status("", false);
        self.ai_update_cost();
    }

    /// `_update_cost`, and the list of matching models under the field.
    fn ai_update_cost(&self) {
        let Some(w) = &self.ai_window else { return };
        let typed = w.dialog.get_or_model().to_string();
        let (text, error) = if w.dialog.get_provider() == "openrouter" {
            ai::cost_text(&typed, &w.or_models, &w.or_pricing)
        } else {
            (String::new(), false)
        };
        w.dialog.set_cost_text(text.into());
        w.dialog.set_cost_error(error);
        let matches: Vec<SharedString> = if typed.is_empty() {
            Vec::new()
        } else {
            ai::matching_models(&typed, &w.or_models)
                .into_iter()
                .map(|m| m.as_str().into())
                .collect()
        };
        w.dialog
            .set_suggestions(ModelRc::from(Rc::new(VecModel::from(matches))));
    }

    /// `test_connection`.
    fn ai_test(&mut self) {
        let Some(w) = &mut self.ai_window else { return };
        let provider = w.dialog.get_provider().to_string();
        let (url, key, headers) = if provider == "openrouter" {
            let key = py::strip(&w.dialog.get_or_key()).to_owned();
            if key.is_empty() {
                return self.ai_test_status("Paste your OpenRouter API key first.", true);
            }
            (
                ai::OPENROUTER_BASE.to_owned(),
                key,
                ai::openrouter_headers(),
            )
        } else {
            let url = ai::normalize_ai_url(&w.dialog.get_url());
            if url.is_empty() {
                return self.ai_test_status("Enter the server address first.", true);
            }
            w.dialog.set_url(url.as_str().into());
            (url, py::strip(&w.dialog.get_key()).to_owned(), Vec::new())
        };
        w.dialog.set_testing(true);
        w.token += 1;
        let token = w.token;
        self.ai_test_status("Connecting…", false);
        let _ = std::thread::Builder::new()
            .name("ai-check".into())
            .spawn(move || {
                let info = ai::run_check(&provider, &url, &key, &headers);
                let _ = slint::invoke_from_event_loop(move || {
                    app_call(|a, _ui| a.ai_test_result(token, info));
                });
            });
    }

    /// `_on_test_result`.
    fn ai_test_result(&mut self, token: u64, info: CheckResult) {
        let Some(w) = &mut self.ai_window else { return };
        if w.token != token {
            return;
        }
        w.dialog.set_testing(false);
        if info.provider != w.dialog.get_provider().as_str() {
            return; // switched provider while testing
        }
        if !info.error.is_empty() {
            return self.ai_test_status(&info.error, true);
        }
        if info.provider == "openrouter" {
            let status = ai::openrouter_test_result(&info.key_note, info.ids.len());
            w.or_models = info.ids;
            w.or_pricing = info.pricing;
            self.ai_test_status(&status, false);
            self.ai_update_cost();
            return;
        }
        let current = w.dialog.get_model().to_string();
        let (status, error, fill) = ai::local_test_result(&info.ids, &current);
        w.dialog
            .set_local_models(ModelRc::from(Rc::new(VecModel::from(
                info.ids
                    .iter()
                    .map(|m| SharedString::from(m.as_str()))
                    .collect::<Vec<_>>(),
            ))));
        w.local_models = info.ids;
        if let Some(m) = fill {
            w.dialog.set_model(m.into());
        }
        self.ai_test_status(&status, error);
    }

    /// `save`.
    fn ai_save(&mut self) {
        let Some(w) = &self.ai_window else { return };
        let d = &w.dialog;
        let enabled = d.get_ai_enabled();
        let provider: &'static str = if d.get_provider() == "openrouter" {
            "openrouter"
        } else {
            "local"
        };
        let url = ai::normalize_ai_url(&d.get_url());
        let or_key = py::strip(&d.get_or_key()).to_owned();
        let or_model = py::strip(&d.get_or_model()).to_owned();
        if enabled && provider == "local" && url.is_empty() {
            return self
                .ai_test_status("Enter the server address, or turn AI alignment off.", true);
        }
        if enabled && provider == "openrouter" {
            if or_key.is_empty() {
                return self.ai_test_status(
                    "Paste your OpenRouter API key, or turn AI alignment off.",
                    true,
                );
            }
            if or_model.is_empty() {
                return self.ai_test_status(
                    "Choose a model: run Test connection, then type in Model to search.",
                    true,
                );
            }
        }
        let raw = ai::RawSettings {
            enabled,
            provider,
            url,
            model: py::strip(&d.get_model()).to_owned(),
            key: py::strip(&d.get_key()).to_owned(),
            or_model,
            or_key,
        };
        if ai::save_settings(&mut self.store.settings, &raw, crate::platform::cipher()) {
            self.store.save();
        }
        self.ai_close();
    }

    /// Closes the settings window (Cancel, Escape, the close button, or after Save).
    fn ai_close(&mut self) {
        if let Some(mut w) = self.ai_window.take() {
            w.token += 1; // a test still running reports nothing
            let _ = w.dialog.hide();
            drop_later(w);
        }
        self.update_ai_buttons();
    }
}

// =========================================================================================
// Callbacks
// =========================================================================================

fn connect_sync(d: &SyncDialog) {
    macro_rules! on {
        ($setter:ident, |$a:ident, $ui:ident $(, $arg:ident)*| $body:expr) => {
            d.$setter(move |$($arg),*| app_call(|$a, $ui| { let _ = &$ui; $body; }));
        };
    }
    d.on_load_file(|| {
        let Some(Some(track)) = with_sync(|w| w.track_path.clone()) else {
            return;
        };
        ask_flow(async move {
            if let Some(path) = pick_lyrics_file("Load lyrics", &track, true).await {
                match read_lyrics_file(&path) {
                    Ok(text) => app_call(|a, _| {
                        if let Some(w) = &a.sync {
                            w.dialog.set_input_text(text.into());
                        }
                    }),
                    Err(e) => warning("Load lyrics", &e.to_string()).await,
                }
            }
        });
    });
    on!(on_use_shown, |a, ui| {
        if let Some(w) = &a.sync {
            let text = a.shown_lyrics_text(&w.track_path);
            w.dialog.set_input_text(text.into());
        }
    });
    on!(on_start_syncing, |a, ui| a.sync_start());
    on!(on_restart, |a, ui| a.seek_to(0));
    on!(on_seek_relative, |a, ui, delta| a
        .sync_seek_relative(i64::from(delta)));
    on!(on_toggle_play, |a, ui| a.toggle_play());
    on!(on_stamp, |a, ui| a.sync_stamp());
    on!(on_undo, |a, ui| a.sync_undo());
    on!(on_edit_text, |a, ui| a.sync_back_to_text());
    on!(on_add_break, |a, ui| a.sync_add_break());
    on!(on_preview, |a, ui| a.sync_preview(ui));
    on!(on_copy_lrc, |a, ui| a.sync_copy());
    on!(on_move_cursor, |a, ui, delta| a
        .sync_move_cursor(i64::from(delta)));
    on!(on_nudge, |a, ui, delta| a.sync_nudge(i64::from(delta)));
    on!(on_row_clicked, |a, ui, row| {
        if let Ok(r) = usize::try_from(row) {
            a.sync_row_clicked(r)
        }
    });
    on!(on_row_double_clicked, |a, ui, row| {
        if let Ok(r) = usize::try_from(row) {
            a.sync_row_double_clicked(r)
        }
    });
    d.on_save(|| ask_flow(sync_save()));
    d.on_embed(|| ask_flow(sync_embed()));
    d.on_close_asked(|| {
        ask_flow(async {
            try_close_sync().await;
        })
    });
    d.window().on_close_requested(|| {
        // The window goes once any question about unsaved work is answered.
        ask_flow(async {
            try_close_sync().await;
        });
        slint::CloseRequestResponse::KeepWindowShown
    });
}

fn with_sync<R>(f: impl FnOnce(&SyncWindow) -> R) -> Option<Option<R>> {
    let weak = super::APP.with(|a| a.borrow().clone());
    with_app_ret(&weak, |a, _| a.sync.as_ref().map(f))
}

fn with_translate<R>(f: impl FnOnce(&TranslateWindow) -> R) -> Option<Option<R>> {
    let weak = super::APP.with(|a| a.borrow().clone());
    with_app_ret(&weak, |a, _| a.translate.as_ref().map(f))
}

/// `save` of the sync editor.
async fn sync_save() {
    let Some(Some((track, (text, missing)))) =
        with_sync(|w| (w.track_path.clone(), ed::build_sync_lrc(&w.ed.entries)))
    else {
        return;
    };
    if text.is_empty() {
        return app_call(|a, _| a.sync_status(ed::NOTHING_STAMPED));
    }
    if let Some(path) = save_lrc_asking(&track, &text, |existing| {
        ed::sync_save_question(missing, existing)
    })
    .await
    {
        let status = format!("Saved {}", file_name(&path));
        app_call(|a, ui| a.sync_written(ui, &text, status));
    }
}

/// `embed` of the sync editor.
async fn sync_embed() {
    let Some(Some((track, (text, missing)))) =
        with_sync(|w| (w.track_path.clone(), ed::build_sync_lrc(&w.ed.entries)))
    else {
        return;
    };
    if text.is_empty() {
        return app_call(|a, _| a.sync_status(ed::NOTHING_STAMPED));
    }
    if missing > 0 && !ask_yes_no("Embed lyrics", &ed::sync_embed_question(missing)).await {
        return;
    }
    let weak = super::APP.with(|a| a.borrow().clone());
    if embed_lyrics_in_file(&weak, &track, &text).await {
        let status = format!("Lyrics embedded in {}", file_name(&track));
        app_call(|a, ui| a.sync_written(ui, &text, status));
    }
}

/// `closeEvent` of the sync editor: asks about unsaved timestamps. True when it closed
/// (or wasn't open).
pub(super) async fn try_close_sync() -> bool {
    let Some(Some(dirty)) = with_sync(|w| w.ed.dirty) else {
        return true;
    };
    if dirty && !ask_yes_no(ed::SYNC_TITLE, ed::SYNC_UNSAVED).await {
        return false;
    }
    app_call(|a, _| {
        if let Some(w) = a.sync.take() {
            let _ = w.dialog.hide();
            drop_later(w);
        }
    });
    true
}

fn connect_translation(d: &TranslationDialog) {
    macro_rules! on {
        ($setter:ident, |$a:ident, $ui:ident $(, $arg:ident)*| $body:expr) => {
            d.$setter(move |$($arg),*| app_call(|$a, $ui| { let _ = &$ui; $body; }));
        };
    }
    on!(on_text_edited, |a, ui, _text| a.translate_counts());
    d.on_load_file(|| {
        let Some(Some(track)) = with_translate(|w| w.track_path.clone()) else {
            return;
        };
        ask_flow(async move {
            if let Some(path) = pick_lyrics_file("Load translation", &track, false).await {
                match read_lyrics_file(&path) {
                    Ok(text) => app_call(|a, _| {
                        if let Some(w) = &a.translate {
                            w.dialog.set_input_text(text.into());
                        }
                        a.translate_counts();
                    }),
                    Err(e) => app_call(|a, _| a.translate_status(&e.to_string(), true)),
                }
            }
        });
    });
    on!(on_setup_ai, |a, ui| a.open_ai_settings(ui));
    on!(on_match_lines, |a, ui| a.translate_match());
    on!(on_align_ai, |a, ui| a.translate_align());
    on!(on_cancel_ai, |a, ui| a.translate_cancel_ai());
    on!(on_shift_down, |a, ui| a
        .translate_edit_rows(TranslationEditor::shift_down));
    on!(on_shift_up, |a, ui| a
        .translate_edit_rows(TranslationEditor::shift_up));
    on!(on_clear_line, |a, ui| a
        .translate_edit_rows(TranslationEditor::clear));
    on!(on_back_to_text, |a, ui| a.translate_back_to_text());
    on!(on_preview, |a, ui| a.translate_preview(ui));
    on!(on_show_raw, |a, ui| a.translate_show_raw());
    on!(on_replay, |a, ui, row| {
        if let Ok(r) = usize::try_from(row) {
            a.translate_replay(r)
        }
    });
    on!(on_commit_edit, |a, ui, row, text| {
        if let Ok(r) = usize::try_from(row) {
            a.translate_commit(r, &text)
        }
    });
    d.on_save(|| ask_flow(translate_save()));
    d.on_embed(|| ask_flow(translate_embed()));
    d.on_close_asked(|| {
        ask_flow(async {
            try_close_translate().await;
        })
    });
    d.window().on_close_requested(|| {
        // The window goes once any question about unsaved work is answered.
        ask_flow(async {
            try_close_translate().await;
        });
        slint::CloseRequestResponse::KeepWindowShown
    });
}

async fn translate_save() {
    let Some(Some((track, text))) = with_translate(|w| (w.track_path.clone(), w.ed.build_lrc()))
    else {
        return;
    };
    if let Some(path) =
        save_lrc_asking(&track, &text, |existing| existing.map(ed::replace_question)).await
    {
        let status = format!("Saved {}", file_name(&path));
        app_call(|a, ui| a.translate_written(ui, &text, status));
    }
}

async fn translate_embed() {
    let Some(Some((track, text))) = with_translate(|w| (w.track_path.clone(), w.ed.build_lrc()))
    else {
        return;
    };
    let weak = super::APP.with(|a| a.borrow().clone());
    if embed_lyrics_in_file(&weak, &track, &text).await {
        let status = ed::translation_embedded(&file_name(&track));
        app_call(|a, ui| a.translate_written(ui, &text, status));
    }
}

/// `closeEvent` of the translation window: a running model is stopped, and unsaved
/// work is asked about. True when it closed (or wasn't open).
pub(super) async fn try_close_translate() -> bool {
    let Some(Some((working, dirty))) = with_translate(|w| (w.worker.is_some(), w.ed.dirty)) else {
        return true;
    };
    if working {
        if !ask_yes_no(ed::TRANSLATE_TITLE, ed::MODEL_STILL_WORKING).await {
            return false;
        }
        app_call(|a, _| a.translate_cancel_ai());
    }
    if dirty && !ask_yes_no(ed::TRANSLATE_TITLE, ed::TRANSLATE_UNSAVED).await {
        return false;
    }
    app_call(|a, _| {
        if let Some(w) = a.translate.take() {
            if let Some(c) = &w.worker {
                c.cancel();
            }
            let _ = w.dialog.hide();
            if let Some(answer) = &w.answer {
                let _ = answer.hide();
            }
            drop_later(w);
        }
    });
    true
}

/// The main window is closing or quitting: each editor may ask about unsaved work
/// first. False when the user chose to keep editing.
pub(super) async fn try_close_editors() -> bool {
    try_close_sync().await && try_close_translate().await
}

/// An editor has unsaved work or a model still answering, so closing asks first.
pub(super) fn editors_need_asking() -> bool {
    with_sync(|w| w.ed.dirty).flatten().unwrap_or(false)
        || with_translate(|w| w.worker.is_some() || w.ed.dirty)
            .flatten()
            .unwrap_or(false)
}

fn connect_ai_settings(d: &AiSettingsDialog) {
    macro_rules! on {
        ($setter:ident, |$a:ident $(, $arg:ident)*| $body:expr) => {
            d.$setter(move |$($arg),*| app_call(|$a, _ui| { $body; }));
        };
    }
    on!(on_provider_changed, |a| a.ai_provider_changed());
    d.on_get_key(|| crate::platform::open_url(ai::OPENROUTER_KEYS_URL));
    on!(on_or_model_edited, |a, _text| a.ai_update_cost());
    on!(on_test, |a| a.ai_test());
    on!(on_cancel, |a| a.ai_close());
    on!(on_save, |a| a.ai_save());
    d.window().on_close_requested(|| {
        app_call(|a, _ui| a.ai_close());
        slint::CloseRequestResponse::HideWindow
    });
}
