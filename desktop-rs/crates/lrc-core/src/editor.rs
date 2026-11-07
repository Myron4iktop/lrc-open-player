//! The two lyrics editors without their windows: the tap-to-sync editor
//! (`LyricsSyncDialog`) and the translation window (`TranslationDialog`).
//!
//! The player's windows show what these hold and call them for every key and button;
//! the rules, the LRC they write and every message text are here, tested on Linux.

use std::collections::HashSet;

use crate::align::{self, LRC_LEADING, LRC_META};
use crate::lrc::{self, fmt_lrc_time};
use crate::py;

// =======================================================================================
// Sync lyrics (`LyricsSyncDialog`)
// =======================================================================================

pub const SYNC_TITLE: &str = "Sync lyrics";
pub const SYNC_HELP: &str = "Enter  stamp line   ·   Backspace  undo   ·   Space  play/pause   ·   ← →  5 s   ·   ↑ ↓  move ▶   ·   B  instrumental break   ·   [ ]  nudge selected line   ·   Ctrl+S  save";
pub const SYNC_INTRO: &str = "Paste the lyrics, one sung line per line. Empty lines are skipped.\nLines that already have [mm:ss.xx] timestamps keep them, so you can also fix an existing LRC.";
pub const SYNC_UNSAVED: &str = "You have timestamps that aren't saved yet. Close and lose them?";
pub const PASTE_FIRST: &str = "Paste some lyrics first.";
pub const NOTHING_STAMPED: &str = "Nothing is stamped yet.";
pub const OTHER_SONG_PREVIEW: &str = "The player is on another song, so it can't preview this one.";
pub const PREVIEW_SHOWN: &str = "Showing the result in the player (not saved yet).";
pub const COPIED: &str = "LRC copied to the clipboard.";

/// The warning while the player is on another song.
pub fn other_song_warning(file_name: &str) -> String {
    format!(
        "The player switched to another song. Go back to “{file_name}” to keep syncing. Saving still writes the lyrics for the right song."
    )
}

/// One line of the sync editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub text: String,
    pub time: Option<i64>,
    /// A paired line (translation) that shares the main line's timestamp.
    pub sub: String,
    /// An instrumental break (♪), added with B or read from an empty stamped line.
    pub is_break: bool,
}

/// `_parse_text`: lyrics text into entries. Stamped lines keep their stamps, and a line
/// sharing its timestamp with the previous one is that line's translation.
pub fn parse_text(text: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    for raw in py::splitlines(text) {
        let line = py::strip(raw);
        if line.is_empty() || LRC_META.is_match(line) {
            continue;
        }
        if let Some(caps) = LRC_LEADING.captures(line) {
            let mut body = py::strip(caps.get(2).map_or("", |m| m.as_str()));
            if body == "♪" {
                body = "";
            }
            let (times, _) = lrc::find_stamps(&caps[1]);
            for t in times {
                entries.push(Entry {
                    text: body.to_owned(),
                    time: Some(t),
                    sub: String::new(),
                    is_break: body.is_empty(),
                });
            }
        } else {
            entries.push(Entry {
                text: line.to_owned(),
                time: None,
                sub: String::new(),
                is_break: false,
            });
        }
    }
    if !entries.is_empty() && entries.iter().all(|e| e.time.is_some()) {
        entries.sort_by_key(|e| e.time); // stable: a translation stays after its line
    }
    // The same pairing rule the player uses, so the translation moves with its line.
    let mut paired: Vec<Entry> = Vec::new();
    for e in entries {
        if let Some(prev) = paired.last_mut()
            && let (Some(t), Some(pt)) = (e.time, prev.time)
            && (t - pt).abs() <= lrc::PAIR_WINDOW_MS
            && prev.sub.is_empty()
            && !e.text.is_empty()
            && !prev.text.is_empty()
        {
            prev.sub = e.text;
            continue;
        }
        paired.push(e);
    }
    paired
}

/// `_entries_to_text`: the entries as text again, for "← Edit text".
pub fn entries_to_text(entries: &[Entry]) -> String {
    let mut out = Vec::new();
    for e in entries {
        match e.time {
            None => {
                out.push(if e.text.is_empty() {
                    "♪".to_owned()
                } else {
                    e.text.clone()
                });
                if !e.sub.is_empty() {
                    out.push(e.sub.clone());
                }
            }
            Some(t) => {
                let stamp = format!("[{}]", fmt_lrc_time(t));
                out.push(format!("{stamp}{}", e.text));
                if !e.sub.is_empty() {
                    out.push(format!("{stamp}{}", e.sub));
                }
            }
        }
    }
    out.join("\n")
}

/// `build_lrc`: the LRC of the stamped lines in time order, and how many lines have no
/// timestamp (left out).
pub fn build_sync_lrc(entries: &[Entry]) -> (String, usize) {
    let mut stamped: Vec<&Entry> = entries.iter().filter(|e| e.time.is_some()).collect();
    stamped.sort_by_key(|e| e.time);
    let mut lines = Vec::new();
    for e in stamped {
        let stamp = format!("[{}]", fmt_lrc_time(e.time.unwrap_or(0)));
        lines.push(format!("{stamp}{}", e.text));
        if !e.sub.is_empty() {
            lines.push(format!("{stamp}{}", e.sub));
        }
    }
    let missing = entries.iter().filter(|e| e.time.is_none()).count();
    if lines.is_empty() {
        (String::new(), missing)
    } else {
        (lines.join("\n") + "\n", missing)
    }
}

/// The question before saving, or `None` when nothing needs asking.
pub fn sync_save_question(missing: usize, existing: Option<&str>) -> Option<String> {
    let mut notes = Vec::new();
    if missing > 0 {
        notes.push(format!(
            "{missing} line(s) have no timestamp and will be left out."
        ));
    }
    if let Some(name) = existing {
        notes.push(format!("{name} already exists and will be replaced."));
    }
    (!notes.is_empty()).then(|| notes.join("\n") + "\n\nSave anyway?")
}

/// The question before embedding lines without a timestamp.
pub fn sync_embed_question(missing: usize) -> String {
    format!("{missing} line(s) have no timestamp and will be left out.\n\nContinue?")
}

/// How a row's time is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeLook {
    /// "--:--.--", not stamped yet.
    None,
    /// Stamped in order.
    Ok,
    /// Earlier than a line above it: probably a mis-tap.
    Error,
}

/// How a row's line is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineLook {
    /// The ▶ line, the next to stamp.
    Cursor,
    /// Not stamped yet.
    Open,
    Stamped,
}

/// One row of the sync list, as `_refresh` draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRow {
    pub cursor: bool,
    pub time: String,
    pub time_look: TimeLook,
    pub line_look: LineLook,
}

/// The sync editor's state: the entries, the ▶ line and the selected row.
#[derive(Debug, Clone, Default)]
pub struct SyncEditor {
    pub entries: Vec<Entry>,
    /// The next line to stamp (the ▶ line).
    pub cursor: usize,
    /// The list's current row (what [ and ] nudge).
    pub selected: Option<usize>,
    pub dirty: bool,
}

impl SyncEditor {
    /// `start_syncing`: Err with the message when there is nothing to sync.
    pub fn start(&mut self, text: &str) -> Result<&'static str, &'static str> {
        let entries = parse_text(text);
        if entries.is_empty() {
            return Err(PASTE_FIRST);
        }
        self.cursor = entries.iter().position(|e| e.time.is_none()).unwrap_or(0);
        self.entries = entries;
        self.selected = None;
        Ok("Press Space to play, then Enter whenever the ▶ line starts.")
    }

    pub fn rows(&self) -> Vec<SyncRow> {
        let mut latest = -1;
        self.entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let (time, time_look) = match e.time {
                    None => ("--:--.--".to_owned(), TimeLook::None),
                    Some(t) => {
                        let look = if t < latest {
                            TimeLook::Error
                        } else {
                            TimeLook::Ok
                        };
                        latest = latest.max(t);
                        (fmt_lrc_time(t), look)
                    }
                };
                let line_look = if i == self.cursor {
                    LineLook::Cursor
                } else if e.time.is_none() {
                    LineLook::Open
                } else {
                    LineLook::Stamped
                };
                SyncRow {
                    cursor: i == self.cursor,
                    time,
                    time_look,
                    line_look,
                }
            })
            .collect()
    }

    /// "3 / 20 stamped".
    pub fn progress_text(&self) -> String {
        let done = self.entries.iter().filter(|e| e.time.is_some()).count();
        format!("{done} / {} stamped", self.entries.len())
    }

    /// The row to scroll to the middle.
    pub fn scroll_row(&self) -> Option<usize> {
        (!self.entries.is_empty()).then(|| self.cursor.min(self.entries.len() - 1))
    }

    fn select(&mut self, row: usize) {
        if row < self.entries.len() {
            self.selected = Some(row);
        }
    }

    /// `stamp`: the ▶ line starts at `pos_ms`. The new status, or `None` if nothing
    /// happened.
    pub fn stamp(&mut self, pos_ms: i64) -> Option<&'static str> {
        if self.entries.is_empty() {
            return None;
        }
        if self.cursor >= self.entries.len() {
            return Some("Every line is stamped. Save when you're happy with it.");
        }
        self.entries[self.cursor].time = Some(pos_ms);
        self.select(self.cursor);
        self.cursor += 1;
        self.dirty = true;
        Some(if self.cursor >= self.entries.len() {
            "That was the last line. Preview it in the player, then save."
        } else {
            ""
        })
    }

    /// `undo`: takes the last stamp back (a break goes away). Returns where to rewind
    /// to, from the undone stamp (or `pos_ms`) minus 3 s.
    pub fn undo(&mut self, pos_ms: i64) -> Option<i64> {
        if self.cursor == 0 || self.entries.is_empty() {
            return None;
        }
        let i = self.cursor - 1;
        let t = self.entries[i].time;
        if self.entries[i].is_break {
            self.entries.remove(i);
        } else {
            self.entries[i].time = None;
        }
        self.cursor = i;
        self.select(self.cursor);
        self.dirty = true;
        Some((t.unwrap_or(pos_ms) - 3000).max(0))
    }

    pub const UNDONE: &'static str =
        "Undone. Playback jumped back 3 s so you can stamp the line again.";

    /// `insert_break`: an instrumental break at `pos_ms`, before the ▶ line.
    pub fn insert_break(&mut self, pos_ms: i64) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        self.entries.insert(
            self.cursor,
            Entry {
                text: String::new(),
                time: Some(pos_ms),
                sub: String::new(),
                is_break: true,
            },
        );
        self.cursor += 1;
        self.dirty = true;
        self.select(self.cursor - 1);
        true
    }

    pub const BREAK_ADDED: &'static str = "Added an instrumental break (shown as ♪ in the player).";

    /// `nudge_selected`: moves the selected line by `delta_ms`. Err with the message when
    /// no stamped line is selected.
    pub fn nudge(&mut self, delta_ms: i64) -> Result<(), &'static str> {
        match self.selected {
            Some(row) if row < self.entries.len() => {
                if let Some(t) = self.entries[row].time {
                    self.entries[row].time = Some((t + delta_ms).max(0));
                    self.dirty = true;
                    return Ok(());
                }
            }
            _ => {}
        }
        Err("Click a stamped line first to nudge it.")
    }

    /// `move_cursor`: ↑ and ↓.
    pub fn move_cursor(&mut self, delta: i64) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let max = self.entries.len() as i64 - 1;
        self.cursor = (self.cursor as i64 + delta).clamp(0, max) as usize;
        self.select(self.cursor);
        true
    }

    /// `_on_item_clicked`: the clicked row becomes the ▶ line.
    pub fn click(&mut self, row: usize) {
        if row < self.entries.len() {
            self.cursor = row;
            self.selected = Some(row);
        }
    }

    /// `_on_item_double_clicked`: where to replay from, just before the line.
    pub fn replay_from(&self, row: usize) -> Option<i64> {
        self.entries
            .get(row)
            .and_then(|e| e.time)
            .map(|t| (t - 1500).max(0))
    }
}

// =======================================================================================
// Add translation (`TranslationDialog`)
// =======================================================================================

pub const TRANSLATE_TITLE: &str = "Add translation";
pub const TRANSLATE_INTRO: &str = "Paste a translation of these lyrics, one line per line. Each translated line will appear under the sung line it belongs to. Section labels like [Chorus] are fine: they're skipped.";
pub const TRANSLATE_HINT: &str = "Double-click a time to hear that line   ·   Double-click a translation to edit it   ·   Tinted rows have no translation";
pub const NO_LYRICS_TO_TRANSLATE: &str = "This song has no lyrics to add a translation to yet.";
pub const NOT_SYNCED_TO_TRANSLATE: &str = "A translation is added line by line to synced lyrics, and this song's lyrics aren't synced yet.\n\nSync them first with Lyrics ▾ → Sync lyrics…";
pub const TRANSLATE_UNSAVED: &str = "The translation isn't saved yet. Close and lose it?";
pub const MODEL_STILL_WORKING: &str = "The model is still working. Stop it and close?";
pub const PASTE_TRANSLATION_FIRST: &str = "Paste a translation first.";
pub const AI_NOT_SET_UP: &str = "AI alignment isn't set up. Use Set up AI…";
pub const CLICK_A_LINE: &str = "Click a line first.";
pub const ALREADY_TRANSLATED: &str = "Loaded the translation that's already in these lyrics.";
pub const CANCELLED: &str = "Cancelled.";
pub const RAW_ANSWER_TITLE: &str = "Model's answer";
pub const RAW_ANSWER_TEXT: &str =
    "This is what the model sent back. It helps when the alignment looks wrong.";
pub const SAME_AS_SUNG_TIP: &str = "Same as the sung line, so it won't be saved as a translation";
pub const SKIPPED_TIP: &str = "The model gave no answer for this line";

/// The question before replacing a .lrc.
pub fn replace_question(file_name: &str) -> String {
    format!("{file_name} already exists and will be replaced.\n\nSave anyway?")
}

/// `QMessageBox.warning(…, f"Couldn't save the file:\n{e}")`.
pub fn save_failed(error: &str) -> String {
    format!("Couldn't save the file:\n{error}")
}

/// A lyric line as the player holds it (`LyricCard`): time, line and translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub time_ms: i64,
    pub main: String,
    pub sub: String,
}

/// The time column: "1:02.5".
pub fn row_time_text(t_ms: i64) -> String {
    format!(
        "{}:{:04.1}",
        t_ms.div_euclid(60000),
        t_ms.rem_euclid(60000) as f64 / 1000.0
    )
}

/// The waiting text while the model hasn't answered yet.
pub fn waiting_text(openrouter: bool) -> &'static str {
    if openrouter {
        "Waiting for OpenRouter…"
    } else {
        "Waiting for the model… (a big model can take a minute to start)"
    }
}

/// The translation window's state.
#[derive(Debug, Clone, Default)]
pub struct TranslationEditor {
    /// Every card of the lyrics; "rows" are the sung ones (♪ breaks stay as they are).
    pub cards: Vec<Card>,
    pub row_cards: Vec<usize>,
    /// The translation per row.
    pub texts: Vec<String>,
    /// Cleaned translation lines (what T1, T2 … refer to).
    pub trans_lines: Vec<String>,
    /// Rows the model gave no answer for.
    pub skipped: HashSet<usize>,
    pub dirty: bool,
    pub raw_answer: String,
}

impl TranslationEditor {
    pub fn new(cards: Vec<Card>) -> Self {
        let row_cards: Vec<usize> = cards
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.main.is_empty() && c.main != "♪")
            .map(|(i, _)| i)
            .collect();
        let texts = row_cards.iter().map(|&i| cards[i].sub.clone()).collect();
        TranslationEditor {
            cards,
            row_cards,
            texts,
            ..Default::default()
        }
    }

    /// The translation already in the lyrics, if any: the text for the box, after which
    /// the window starts in the review table.
    pub fn existing_translation(&mut self) -> Option<String> {
        if !self.texts.iter().any(|t| !t.is_empty()) {
            return None;
        }
        let text = self
            .texts
            .iter()
            .filter(|t| !t.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        self.trans_lines = align::clean_translation_lines(&text);
        Some(text)
    }

    pub fn originals(&self) -> Vec<String> {
        self.row_cards
            .iter()
            .map(|&i| self.cards[i].main.clone())
            .collect()
    }

    pub fn n_rows(&self) -> usize {
        self.row_cards.len()
    }

    /// The sung line of a row.
    pub fn sung(&self, row: usize) -> &str {
        &self.cards[self.row_cards[row]].main
    }

    pub fn row_time(&self, row: usize) -> i64 {
        self.cards[self.row_cards[row]].time_ms
    }

    /// `_update_counts`: the line under the text box.
    pub fn counts_text(&self, text: &str) -> String {
        let lines = align::clean_translation_lines(text);
        let content = lines
            .iter()
            .filter(|l| !align::is_section_header(l))
            .count();
        let n = self.n_rows();
        if content == 0 {
            return format!("{n} sung lines");
        }
        let mut s = format!("{n} sung lines   ·   {content} translation lines");
        if content == n {
            s.push_str("   ·   same count, so matching line by line should work");
        } else {
            s.push_str("   ·   different counts: some lines are split, joined or repeated");
        }
        s
    }

    /// `match_lines`: Err with the message when there's no translation.
    pub fn match_lines(&mut self, text: &str) -> Result<String, &'static str> {
        let lines = align::clean_translation_lines(text);
        let content = lines
            .iter()
            .filter(|l| !align::is_section_header(l))
            .count();
        if content == 0 {
            return Err(PASTE_TRANSLATION_FIRST);
        }
        self.texts = align::match_in_order(self.n_rows(), &lines);
        self.trans_lines = lines;
        self.skipped.clear();
        self.dirty = true;
        let n = self.n_rows();
        Ok(if content == n {
            "Matched line by line. Play a few lines to check they line up.".to_owned()
        } else {
            format!(
                "Matched line by line, but the counts differ ({n} sung, {content} translated), so later lines may be off. Fix them with Shift down / up, or use AI alignment."
            )
        })
    }

    /// The first half of `align_with_ai`: the lines to send, or the message when there
    /// is no translation.
    pub fn prepare_ai(&mut self, text: &str) -> Result<Vec<align::Message>, &'static str> {
        let lines = align::clean_translation_lines(text);
        if !lines.iter().any(|l| !align::is_section_header(l)) {
            return Err(PASTE_TRANSLATION_FIRST);
        }
        self.trans_lines = lines;
        self.raw_answer.clear();
        Ok(align::build_alignment_messages(
            &self.originals(),
            &self.trans_lines,
        ))
    }

    /// `_on_ai_done`: reads the model's answer. The status, and whether it's an error
    /// (then the table is left as it was).
    pub fn apply_ai(&mut self, answer: &str, finish: &str, model_name: &str) -> (String, bool) {
        self.raw_answer = answer.to_owned();
        let n = self.n_rows();
        let mapping = align::parse_alignment(answer, n, &self.trans_lines);
        let cut_short = finish == "length";
        if mapping.len() < (n / 2).max(1) {
            let why = if cut_short {
                "It stopped before finishing (it ran out of room). "
            } else {
                "Its answer didn't use the expected “O1: T1” format. "
            };
            return (
                format!(
                    "The model didn't produce a usable alignment. {why}Try again, or use Match line by line."
                ),
                true,
            );
        }
        let texts = align::alignment_to_texts(&mapping, n, &self.trans_lines);
        self.skipped = texts
            .iter()
            .enumerate()
            .filter(|(_, t)| t.is_none())
            .map(|(i, _)| i)
            .collect();
        self.texts = texts.into_iter().map(Option::unwrap_or_default).collect();
        self.dirty = true;
        if self.skipped.is_empty() {
            (
                format!("Aligned by {model_name}. Play a few lines to check, then save."),
                false,
            )
        } else {
            let k = self.skipped.len();
            let s = if k != 1 { "s" } else { "" };
            let extra = if cut_short { " It stopped early." } else { "" };
            (
                format!(
                    "Aligned by {model_name}, but it skipped {k} line{s} (tinted).{extra} Check the result before saving."
                ),
                false,
            )
        }
    }

    /// The AI progress line and the progress bar (`None` = busy indicator).
    pub fn progress(
        &self,
        phase: &str,
        answered: usize,
        waiting: &str,
    ) -> (String, Option<(usize, usize)>) {
        let n = self.n_rows();
        if phase == "answering" {
            let a = answered.min(n);
            (format!("Matching lines… {a} of {n}"), Some((a, n)))
        } else if phase == "thinking" {
            ("The model is thinking…".to_owned(), None)
        } else {
            (waiting.to_owned(), None)
        }
    }

    /// The translation of a row was edited (`_on_item_changed`).
    pub fn edit(&mut self, row: usize, text: &str) {
        if row < self.texts.len() {
            self.texts[row] = py::strip(text).to_owned();
            self.skipped.remove(&row);
            self.dirty = true;
        }
    }

    /// `shift_down`: pushes this translation and the ones after it one line down. The
    /// row to select after.
    pub fn shift_down(&mut self, row: usize) -> usize {
        self.texts.insert(row, String::new());
        self.texts.pop();
        self.skipped.clear();
        self.dirty = true;
        (row + 1).min(self.texts.len().saturating_sub(1))
    }

    /// `shift_up`: removes this line's translation and pulls the ones after it up.
    pub fn shift_up(&mut self, row: usize) -> usize {
        self.texts.remove(row);
        self.texts.push(String::new());
        self.skipped.clear();
        self.dirty = true;
        row
    }

    /// `clear_line`.
    pub fn clear(&mut self, row: usize) -> usize {
        self.texts[row].clear();
        self.dirty = true;
        row
    }

    /// The translation only repeats the sung line ("Spinning, spinning"), so it isn't
    /// saved (`_mark_repeat`).
    pub fn is_repeat(&self, row: usize) -> bool {
        let t = &self.texts[row];
        !py::strip(t).is_empty() && lrc::same_text(t, self.sung(row))
    }

    /// The row has no translation (tinted).
    pub fn is_empty(&self, row: usize) -> bool {
        py::strip(&self.texts[row]).is_empty()
    }

    /// The translation cell's tooltip (`_mark_repeat`, `_style_row`).
    pub fn translation_tip(&self, row: usize) -> &'static str {
        if self.is_repeat(row) {
            SAME_AS_SUNG_TIP
        } else if self.skipped.contains(&row) && self.is_empty(row) {
            SKIPPED_TIP
        } else {
            ""
        }
    }

    /// `_refresh_coverage`: the line under the table, and its tooltip.
    pub fn coverage(&self) -> (String, String) {
        let n = self.n_rows();
        let filled = self
            .texts
            .iter()
            .filter(|t| !py::strip(t).is_empty())
            .count();
        let unused: Vec<&String> = self
            .trans_lines
            .iter()
            .filter(|l| !align::is_section_header(l))
            .filter(|l| !align::line_is_used(l, &self.texts))
            .collect();
        let mut text = format!("{filled} of {n} lines have a translation");
        if unused.is_empty() {
            return (text, String::new());
        }
        let s = if unused.len() != 1 { "s" } else { "" };
        text.push_str(&format!(
            "   ·   {} translation line{s} not used",
            unused.len()
        ));
        let tip = std::iter::once("Not used anywhere:".to_owned())
            .chain(unused.iter().take(25).map(|l| (*l).clone()))
            .collect::<Vec<_>>()
            .join("\n");
        (text, tip)
    }

    /// `build_lrc`: every card with its stamp, and the translation as a second line with
    /// the same stamp.
    pub fn build_lrc(&self) -> String {
        let mut lines = Vec::new();
        let mut row = 0;
        for (ci, card) in self.cards.iter().enumerate() {
            let stamp = format!("[{}]", fmt_lrc_time(card.time_ms));
            if self.row_cards.get(row) != Some(&ci) {
                lines.push(stamp); // instrumental break
                continue;
            }
            lines.push(format!("{stamp}{}", card.main));
            let tr = py::strip(&self.texts[row]);
            if !tr.is_empty() && !lrc::same_text(tr, &card.main) {
                lines.push(format!("{stamp}{tr}"));
            }
            row += 1;
        }
        lines.join("\n") + "\n"
    }

    /// Where double-clicking a row's time plays from (`_on_cell_double_clicked`).
    pub fn replay_from(&self, row: usize, offset_ms: i64) -> i64 {
        (self.row_time(row) + offset_ms - 300).max(0)
    }
}

/// The status after embedding the translation.
pub fn translation_embedded(file_name: &str) -> String {
    format!("Lyrics with translation embedded in {file_name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_round_trip() {
        let mut ed = SyncEditor::default();
        assert_eq!(ed.start(""), Err(PASTE_FIRST));
        ed.start("one\n\ntwo\nthree").unwrap();
        assert_eq!(ed.cursor, 0);
        ed.stamp(1000);
        ed.stamp(2000);
        assert!(ed.insert_break(2500));
        assert_eq!(ed.undo(9000), Some(0)); // the break goes away, back 3 s from it
        assert_eq!(ed.entries.len(), 3);
        assert_eq!(
            ed.nudge(-100),
            Err("Click a stamped line first to nudge it.")
        );
        ed.click(1);
        ed.nudge(-100).unwrap();
        assert_eq!(
            build_sync_lrc(&ed.entries),
            ("[00:01.00]one\n[00:01.90]two\n".to_owned(), 1)
        );
        assert_eq!(ed.stamp(5000), Some(""));
        ed.cursor = 2;
        assert_eq!(
            ed.stamp(5000),
            Some("That was the last line. Preview it in the player, then save.")
        );
    }

    #[test]
    fn translation_rows() {
        let cards = vec![
            Card {
                time_ms: 1000,
                main: "a".into(),
                sub: String::new(),
            },
            Card {
                time_ms: 2000,
                main: "♪".into(),
                sub: String::new(),
            },
            Card {
                time_ms: 3000,
                main: "b".into(),
                sub: String::new(),
            },
        ];
        let mut ed = TranslationEditor::new(cards);
        assert_eq!(
            ed.counts_text("[Chorus]\nA"),
            "2 sung lines   ·   1 translation lines   ·   different counts: some lines are split, joined or repeated"
        );
        ed.match_lines("X\nY").unwrap();
        assert_eq!(
            ed.build_lrc(),
            "[00:01.00]a\n[00:01.00]X\n[00:02.00]\n[00:03.00]b\n[00:03.00]Y\n"
        );
        assert_eq!(ed.shift_down(0), 1);
        assert_eq!(ed.texts, ["", "X"]);
        assert_eq!(
            ed.coverage().0,
            "1 of 2 lines have a translation   ·   1 translation line not used"
        );
        assert_eq!(row_time_text(61_250), "1:01.2");
    }
}
