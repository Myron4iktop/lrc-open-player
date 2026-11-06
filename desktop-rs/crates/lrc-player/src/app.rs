//! The main window's behaviour: `AudioLRCPlayer` in lrcplayer.py, minus what later
//! milestones bring (romaji, editors).
//!
//! The rules live in `lrc-core` (`queue`, `window`, `lrc`, `theme`) and are tested there;
//! this file wires them to the Slint window and the audio engine.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lrc_core::library::{SUPPORTED_EXTS, scan_songs};
use lrc_core::lrc::{self, LyricLine};
use lrc_core::lrclib::{self, FetchResult, Query};
use lrc_core::queue::{Previous, Queue, Repeat, Source};
use lrc_core::session::{
    self as memory, PLACEMENT_KEY, Placement, PlaybackSnapshot, Remembered, RestorePlan, Snapshot,
    WindowSnapshot,
};
use lrc_core::system::{self as sys, TrayTexts};
use lrc_core::tags::{
    TrackInfo, extract_metadata_and_cover, read_track_tags, write_embedded_lyrics,
};
use lrc_core::text::{read_lyrics_file, write_lrc};
use lrc_core::theme::{ACCENTS, Palette, build_palette, known_accent, known_theme, parse_color};
use lrc_core::visualizer as viz;
use lrc_core::window::{self as rules, AfterFetch, DropAction, SaveOffer, lrc_path_for};
use slint::{
    Color, ComponentHandle, Image, Model, ModelRc, PhysicalPosition, PhysicalSize, Rgba8Pixel,
    SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel,
};

use crate::audio::{Event, Player, State};
use crate::media::{MediaCommand, MediaControls};
use crate::session::Store;
use crate::tray::{Tray, TrayCommand, TrayItem};
use crate::visualizer::Visualizer;
use crate::{
    DetailRow, DetailsDialog, LyricLineData, MainWindow, QuestionDialog, ShortcutRow,
    ShortcutsDialog, Theme,
};

thread_local! {
    /// The app, for answers that come back from worker threads.
    static APP: RefCell<Weak<RefCell<App>>> = const { RefCell::new(Weak::new()) };
}

/// What to do with an album scan's songs once they're read.
enum ScanThen {
    /// Play this album folder (`open_album_folder`).
    Album { folder: PathBuf, name: String },
    /// Play everything that was dropped (`_run_drop` → `open_files`).
    Files,
    /// Bring back the last session's playlist, paused (`_finish_restore`).
    Restore(Remembered),
}

/// Lyrics found online, waiting for the answer to "Save these lyrics as …?".
struct PendingOffer {
    path: PathBuf,
    text: String,
    offer: SaveOffer,
}

pub struct App {
    ui: slint::Weak<MainWindow>,
    player: Player,
    queue: Queue,
    /// Titles read by the album scan, so the tracklist needn't read tags again.
    track_titles: HashMap<PathBuf, Option<String>>,
    info: Option<TrackInfo>,
    embedded_lyrics: String,

    /// The lyrics on screen, for "Embed lyrics in file…" (`current_lyrics_text`).
    shown: Vec<LyricLine>,
    lines: Rc<VecModel<LyricLineData>>,
    line_times: Vec<i64>,
    synced: bool,
    current_line: Option<usize>,
    offset_ms: i64,
    seeking: bool,

    theme: &'static str,
    accent: &'static str,
    queue_was_visible: bool,

    scan_token: u64,
    scan_cancel: Option<Arc<AtomicBool>>,

    hovered: Vec<PathBuf>,
    dropped: Vec<PathBuf>,
    drop_pending: bool,

    shortcuts: Option<ShortcutsDialog>,
    details: Option<DetailsDialog>,
    question: Option<QuestionDialog>,
    pending_offer: Option<PendingOffer>,

    store: Store,
    /// Set when the app quits: online searches still running send nothing back.
    quitting: Arc<AtomicBool>,
    /// The window is closing: nothing may overwrite the session saved on the way out.
    closing: bool,
    /// The drawer was open last time (`_queue_wanted`), shown once the playlist is back.
    queue_wanted: bool,
    /// The windowed position from before fullscreen, saved instead of the full screen.
    windowed_placement: Option<Placement>,

    visualizer: Visualizer,
    /// The visualizer's 33 ms tick; runs only while there is something to animate.
    viz_timer: Timer,
    /// The media flyout, once the window exists (`media_controls`).
    media: Option<MediaControls>,
    tray: Option<Tray>,
    /// "Keep playing in tray when closed".
    close_to_tray: bool,
    /// The window was closed to the tray.
    window_hidden: bool,
    /// Quit was asked for: closing the window really quits (`_quitting`).
    quit_requested: bool,
}

/// The timers the app runs on; keep them alive as long as the window.
pub struct Timers {
    _clock: Timer,
    _autosave: Timer,
}

/// Starts the app in `ui`, with the window state of the last session (`_restore_session_ui`).
/// Call before the window is shown, and [`App::restore_playback_session`] right after.
pub fn attach(ui: &MainWindow) -> (Rc<RefCell<App>>, Timers) {
    let lines = Rc::new(VecModel::default());
    ui.set_lines(ModelRc::from(lines.clone()));
    let app = Rc::new(RefCell::new(App {
        ui: ui.as_weak(),
        player: Player::new(),
        queue: Queue::new(),
        track_titles: HashMap::new(),
        info: None,
        embedded_lyrics: String::new(),
        shown: Vec::new(),
        lines,
        line_times: Vec::new(),
        synced: false,
        current_line: None,
        offset_ms: 0,
        seeking: false,
        theme: "dark",
        accent: "coral",
        queue_was_visible: false,
        scan_token: 0,
        scan_cancel: None,
        hovered: Vec::new(),
        dropped: Vec::new(),
        drop_pending: false,
        shortcuts: None,
        details: None,
        question: None,
        pending_offer: None,
        store: Store::open(),
        quitting: Arc::new(AtomicBool::new(false)),
        closing: false,
        queue_wanted: false,
        windowed_placement: None,
        visualizer: Visualizer::new(),
        viz_timer: Timer::default(),
        media: None,
        tray: None,
        close_to_tray: false,
        window_hidden: false,
        quit_requested: false,
    }));
    APP.with(|a| *a.borrow_mut() = Rc::downgrade(&app));
    // Before the settings come back, which turn close-to-tray on only if there is a tray.
    app.borrow_mut().init_tray(ui);
    app.borrow_mut().restore_session_ui(ui);
    connect(ui, &app);

    // Reads the clock and moves the lyrics, like Qt's positionChanged.
    let timer = Timer::default();
    let weak = Rc::downgrade(&app);
    timer.start(TimerMode::Repeated, Duration::from_millis(30), move || {
        if let Some(app) = weak.upgrade()
            && let Ok(mut app) = app.try_borrow_mut()
        {
            crate::errors::guard(|| app.tick());
        }
    });

    // Checkpoints the position now and then, in case the app is killed.
    let autosave = Timer::default();
    let weak = Rc::downgrade(&app);
    autosave.start(
        TimerMode::Repeated,
        Duration::from_millis(memory::AUTOSAVE_INTERVAL_MS),
        move || {
            if let Some(app) = weak.upgrade()
                && let Ok(mut app) = app.try_borrow_mut()
                && let Some(ui) = app.ui.upgrade()
            {
                crate::errors::guard(|| app.save_session(&ui, false));
            }
        },
    );
    (
        app,
        Timers {
            _clock: timer,
            _autosave: autosave,
        },
    )
}

/// Like [`with_app`], handing back what `f` returns.
fn with_app_ret<R>(
    app: &Weak<RefCell<App>>,
    f: impl FnOnce(&mut App, &MainWindow) -> R,
) -> Option<R> {
    let mut out = None;
    with_app(app, |a, ui| out = Some(f(a, ui)));
    out
}

/// A message box on a thread of its own (`QMessageBox`), answered asynchronously.
///
/// A box run on the UI thread would pump the windows' messages from inside Slint's own
/// event handling, which on Windows can leave the windows unable to redraw; on its own
/// thread the windows keep running behind it, and the app is never borrowed meanwhile.
async fn ask(
    level: rfd::MessageLevel,
    title: &str,
    text: &str,
    buttons: rfd::MessageButtons,
) -> rfd::MessageDialogResult {
    rfd::AsyncMessageDialog::new()
        .set_level(level)
        .set_title(title)
        .set_description(text)
        .set_buttons(buttons)
        .show()
        .await
}

/// Yes/No question (`QMessageBox.question`); true for Yes.
async fn ask_yes_no(title: &str, text: &str) -> bool {
    ask(
        rfd::MessageLevel::Info,
        title,
        text,
        rfd::MessageButtons::YesNo,
    )
    .await
        == rfd::MessageDialogResult::Yes
}

/// A message box that doesn't wait for an answer (QMessageBox.information / .warning).
fn message_later(level: rfd::MessageLevel, title: &str, text: String) {
    let title = title.to_owned();
    let _ = slint::spawn_local(async move {
        ask(level, &title, &text, rfd::MessageButtons::Ok).await;
    });
}

/// Runs `f` on the app and the window, unless either is gone or the app is busy (a
/// modal dialog is open further up the stack). A panic in `f` is reported and the app
/// carries on (`install_error_handler`).
fn with_app(app: &Weak<RefCell<App>>, f: impl FnOnce(&mut App, &MainWindow)) {
    let Some(app) = app.upgrade() else { return };
    let Ok(mut app) = app.try_borrow_mut() else {
        return;
    };
    let Some(ui) = app.ui.upgrade() else { return };
    crate::errors::guard(|| f(&mut app, &ui));
}

fn connect(ui: &MainWindow, app: &Rc<RefCell<App>>) {
    macro_rules! on {
        ($setter:ident, |$a:ident, $ui:ident $(, $arg:ident)*| $body:expr) => {{
            let weak = Rc::downgrade(app);
            ui.$setter(move |$($arg),*| with_app(&weak, |$a, $ui| {
                $body;
            }));
        }};
    }

    // File dialogs run on a thread of their own and are awaited, like message boxes:
    // the window keeps running and redrawing behind them, and the app isn't borrowed.
    let weak = Rc::downgrade(app);
    ui.on_open_file(move || {
        let Some(start) = weak.upgrade().map(|a| a.borrow().dialog_start_dir()) else {
            return;
        };
        let weak = weak.clone();
        let _ = slint::spawn_local(async move {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title("Select Audio File")
                .add_filter("Audio Files", &["flac", "mp3", "wav", "m4a", "ogg", "opus"]);
            if let Some(dir) = start {
                dialog = dialog.set_directory(dir);
            }
            if let Some(file) = dialog.pick_file().await {
                let path = file.path().to_path_buf();
                with_app(&weak, |a, ui| a.open_file(ui, path));
            }
        });
    });

    let weak = Rc::downgrade(app);
    ui.on_open_album(move || {
        let Some(start) = weak.upgrade().map(|a| a.borrow().dialog_start_dir()) else {
            return;
        };
        let weak = weak.clone();
        let _ = slint::spawn_local(async move {
            let mut dialog = rfd::AsyncFileDialog::new().set_title("Select Album Folder");
            if let Some(dir) = start {
                dialog = dialog.set_directory(dir);
            }
            if let Some(folder) = dialog.pick_folder().await {
                let folder = folder.path().to_path_buf();
                with_app(&weak, |a, _ui| a.open_album_folder(folder));
            }
        });
    });

    let weak = Rc::downgrade(app);
    ui.on_load_lrc(move || {
        let weak = weak.clone();
        let _ = slint::spawn_local(async move {
            let file = rfd::AsyncFileDialog::new()
                .set_title("Select LRC File")
                .add_filter("Lyrics Files", &["lrc", "txt"])
                .pick_file()
                .await;
            if let Some(file) = file {
                show_lyrics_file(&weak, file.path(), "Load LRC file");
            }
        });
    });

    on!(on_search_online, |a, ui| a.search_lyrics_online(ui, true));
    on!(on_show_details, |a, ui| a.show_details_dialog(ui));
    let weak = Rc::downgrade(app);
    ui.on_embed_lyrics(move || {
        let weak = weak.clone();
        let _ = slint::spawn_local(async move { embed_shown_lyrics(&weak).await });
    });
    on!(on_show_shortcuts, |a, _ui| a.show_shortcuts());
    on!(on_set_theme, |a, ui, theme| {
        let accent = a.accent;
        a.apply_theme(ui, &theme, accent, true)
    });
    on!(on_set_accent, |a, ui, accent| {
        let theme = a.theme;
        a.apply_theme(ui, theme, &accent, true)
    });
    on!(on_toggle_play, |a, _ui| a.toggle_play());
    on!(on_previous, |a, ui| a.play_previous(ui));
    on!(on_next, |a, ui| a.play_next(ui));
    on!(on_toggle_shuffle, |a, ui| {
        let on = !a.queue.shuffle();
        a.set_shuffle(ui, on, true)
    });
    on!(on_cycle_repeat, |a, ui| {
        let r = a.queue.repeat().next();
        a.set_repeat(ui, r, true)
    });
    on!(on_toggle_mute, |a, ui| a.toggle_mute(ui));
    on!(on_volume_changed, |a, ui, value| a
        .volume_changed(ui, value));
    on!(on_seek_pressed, |a, _ui| a.seeking = true);
    on!(on_seek_released, |a, _ui, value| {
        a.seeking = false;
        a.seek_to(value.max(0.0) as u64)
    });
    on!(on_offset_step, |a, ui, delta| {
        let ms = a.offset_ms + i64::from(delta);
        a.set_lyrics_offset(ui, ms, true)
    });
    on!(on_offset_reset, |a, ui| a.set_lyrics_offset(ui, 0, true));
    on!(on_queue_play, |a, ui, row| {
        if let Ok(i) = usize::try_from(row) {
            a.play_index(ui, i)
        }
    });
    on!(on_line_clicked, |a, _ui, row| a.line_clicked(row));
    on!(on_toggle_fullscreen, |a, ui| a.toggle_fullscreen(ui));
    on!(on_set_visualizer, |a, ui, mode| a
        .set_visualizer(ui, &mode, true));
    on!(on_cycle_visualizer, |a, ui| {
        let mode = viz::next_mode(a.visualizer.mode());
        a.set_visualizer(ui, mode, true)
    });
    on!(on_set_close_to_tray, |a, ui, on| {
        a.close_to_tray = on && a.tray.is_some();
        ui.set_close_to_tray(a.close_to_tray);
        a.store.set("ui/close_to_tray", a.close_to_tray)
    });

    let weak = Rc::downgrade(app);
    ui.window().on_close_requested(move || {
        // Either way the window goes; only a real close stops the event loop.
        let mut to_tray = false;
        with_app(&weak, |a, ui| {
            to_tray = a.close_to_tray();
            if to_tray {
                a.close_to_tray_now(ui);
            } else {
                a.close();
            }
        });
        if !to_tray {
            let _ = slint::quit_event_loop();
        }
        slint::CloseRequestResponse::HideWindow
    });
}

/// "Embed lyrics in file…" (`embed_shown_lyrics`). Runs without the app borrowed, since
/// its message boxes wait for an answer.
async fn embed_shown_lyrics(app: &Weak<RefCell<App>>) {
    let Some(Some((path, text))) = with_app_ret(app, |a, _ui| {
        let path = a.queue.current_path()?.clone();
        Some((path, lrc::to_text(&a.shown, a.synced)))
    }) else {
        return;
    };
    if lrc_core::py::strip(&text).is_empty() {
        message_later(
            rfd::MessageLevel::Info,
            rules::EMBED_TITLE,
            rules::NOTHING_TO_EMBED_TEXT.to_owned(),
        );
        return;
    }
    if embed_lyrics_in_file(app, &path, &text).await {
        message_later(
            rfd::MessageLevel::Info,
            rules::EMBED_TITLE,
            rules::embed_done_text(&file_name(&path)),
        );
    }
}

/// `embed_lyrics_in_file`: asks first, then writes the lyrics into the song's tags.
async fn embed_lyrics_in_file(app: &Weak<RefCell<App>>, path: &Path, text: &str) -> bool {
    let name = file_name(path);
    if !ask_yes_no(rules::EMBED_TITLE, &rules::embed_question(&name)).await {
        return false;
    }
    match with_app_ret(app, |a, _ui| a.write_lyrics_into(path, text)) {
        Some(Ok(())) => true,
        Some(Err(error)) => {
            message_later(
                rfd::MessageLevel::Warning,
                rules::EMBED_TITLE,
                rules::embed_failed_text(&name, &error),
            );
            false
        }
        None => false,
    }
}

/// `os.path.basename`.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `_show_lyrics_file`: shows a lyrics file, or says why it can't be read.
fn show_lyrics_file(app: &Weak<RefCell<App>>, path: &Path, title: &str) {
    match read_lyrics_file(path) {
        Ok(text) => with_app(app, |a, ui| a.parse_and_display_lyrics(ui, &text)),
        Err(e) => message_later(rfd::MessageLevel::Warning, title, e.to_string()),
    }
}

/// A palette color for Slint.
fn color(p: &Palette, name: &str) -> Color {
    let [r, g, b, a] = parse_color(p.get(name)).unwrap_or([255, 0, 255, 255]);
    Color::from_argb_u8(a, r, g, b)
}

/// Puts a palette into a window's `Theme` global. Every Slint window has its own copy of
/// the global, so each window is filled.
pub(crate) fn fill_theme(t: &Theme<'_>, p: &Palette) {
    t.set_dark(p.dark);
    t.set_inactive_opacity(p.inactive_opacity as f32);
    macro_rules! set {
        ($($setter:ident = $name:literal),* $(,)?) => { $( t.$setter(color(p, $name)); )* };
    }
    set!(
        set_bg = "bg",
        set_dialog_bg = "dialog_bg",
        set_surface = "surface",
        set_surface2 = "surface2",
        set_surface_border = "surface_border",
        set_btn = "btn",
        set_btn_hover = "btn_hover",
        set_btn_border = "btn_border",
        set_btn_disabled = "btn_disabled",
        set_btn_disabled_border = "btn_disabled_border",
        set_menu = "menu",
        set_menu_border = "menu_border",
        set_divider = "divider",
        set_hover = "hover",
        set_groove = "groove",
        set_badge_bg = "badge_bg",
        set_text = "text",
        set_text_soft = "text_soft",
        set_title = "title",
        set_list_text = "list_text",
        set_muted = "muted",
        set_dim = "dim",
        set_faint = "faint",
        set_faint2 = "faint2",
        set_text_disabled = "text_disabled",
        set_handle = "handle",
        set_handle_border = "handle_border",
        set_icon = "icon",
        set_icon_muted = "icon_muted",
        set_accent = "accent",
        set_artist = "artist",
        set_badge_text = "badge_text",
        set_sel = "sel",
        set_sel_text = "sel_text",
        set_toggle_bg = "toggle_bg",
        set_toggle_border = "toggle_border",
        set_toggle_text = "toggle_text",
        set_primary = "primary",
        set_primary_hover = "primary_hover",
        set_primary_border = "primary_border",
        set_primary_text = "primary_text",
        set_primary_disabled = "primary_disabled",
        set_primary_disabled_text = "primary_disabled_text",
        set_primary_disabled_border = "primary_disabled_border",
        set_lyric_active = "lyric_active",
        set_lyric_active_sub = "lyric_active_sub",
        set_lyric_inactive = "lyric_inactive",
        set_lyric_inactive_sub = "lyric_inactive_sub",
        set_lyric_plain = "lyric_plain",
        set_time_ok = "time_ok",
        set_status = "status",
        set_error = "error",
        set_warn_text = "warn_text",
        set_warn_bg = "warn_bg",
        set_overlay = "overlay",
    );
}

/// The accent dot of the View menu (`make_swatch_icon`): a filled circle, with a ring
/// around it for the accent in use. Drawn at 48 px, as the Python app does (16 px × 3).
fn swatch_icon(hex: &str, selected: bool, ring: [u8; 4]) -> Image {
    const SIZE: u32 = 48;
    let [r, g, b, _] = parse_color(hex).unwrap_or([0, 0, 0, 255]);
    let scale = SIZE as f32 / 24.0;
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(SIZE, SIZE);
    let (cx, cy) = (12.0 * scale, 12.0 * scale);
    // Coverage of a pixel by a disc or ring, softened over one pixel for smooth edges.
    let cover = |d: f32, radius: f32| (radius - d + 0.5).clamp(0.0, 1.0);
    for (i, px) in buf.make_mut_slice().iter_mut().enumerate() {
        let x = (i as u32 % SIZE) as f32 + 0.5;
        let y = (i as u32 / SIZE) as f32 + 0.5;
        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        let (dot, ring_alpha) = if selected {
            let ring_r = 9.5 * scale;
            let half = 1.0 * scale;
            let ring_alpha = cover(d, ring_r + half) * (1.0 - cover(d, ring_r - half));
            (cover(d, 5.5 * scale), ring_alpha)
        } else {
            (cover(d, 8.0 * scale), 0.0)
        };
        *px = if dot > 0.0 {
            Rgba8Pixel::new(r, g, b, (dot * 255.0) as u8)
        } else if ring_alpha > 0.0 {
            Rgba8Pixel::new(ring[0], ring[1], ring[2], (ring_alpha * 255.0) as u8)
        } else {
            Rgba8Pixel::new(0, 0, 0, 0)
        };
    }
    Image::from_rgba8(buf)
}

impl App {
    // -------------------------------------------------------------------------
    // Look
    // -------------------------------------------------------------------------

    /// `apply_theme`: switches theme and accent; everything re-reads the palette.
    pub fn apply_theme(&mut self, ui: &MainWindow, theme: &str, accent: &str, persist: bool) {
        self.theme = known_theme(theme);
        self.accent = known_accent(accent);
        if persist {
            self.store.set("ui/theme", self.theme);
            self.store.set("ui/accent", self.accent);
        }
        let p = build_palette(self.theme, self.accent);
        fill_theme(&ui.global::<Theme>(), &p);
        if let Some(dialog) = &self.shortcuts {
            fill_theme(&dialog.global::<Theme>(), &p);
        }
        if let Some(dialog) = &self.details {
            fill_theme(&dialog.global::<Theme>(), &p);
        }
        if let Some(dialog) = &self.question {
            fill_theme(&dialog.global::<Theme>(), &p);
        }
        ui.invoke_sync_color_scheme();
        ui.set_theme_key(self.theme.into());
        ui.set_accent_key(self.accent.into());
        let ring = parse_color(p.get("text_soft")).unwrap_or([255; 4]);
        let icons: Vec<Image> = ACCENTS
            .iter()
            .map(|(key, _, hex)| swatch_icon(hex, *key == self.accent, ring))
            .collect();
        ui.set_accent_icons(ModelRc::from(Rc::new(VecModel::from(icons))));
        ui.set_viz_accent(color(&p, "accent"));
        ui.set_viz_alpha(p.viz_alpha as f32);
        crate::errors::set_palette(p.clone());
        // `_rescale_cards`: the playing line goes back to the middle.
        ui.set_rescale_count(ui.get_rescale_count().wrapping_add(1));
        crate::platform::apply_title_bar(ui, p.dark);
    }

    // -------------------------------------------------------------------------
    // Opening songs
    // -------------------------------------------------------------------------

    /// Where file dialogs open: the album folder, the current song's folder or the
    /// remembered one, if it still exists (`_dialog_start_dir`).
    fn dialog_start_dir(&self) -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Source::Folder(folder) = self.queue.source() {
            candidates.push(folder.clone());
        }
        if let Some(parent) = self.queue.current_path().and_then(|p| p.parent()) {
            candidates.push(parent.to_path_buf());
        }
        let remembered = self.store.settings.get_str("playback/source_path", "");
        if !remembered.is_empty() {
            candidates.push(remembered.into());
        }
        candidates.into_iter().find_map(|c| {
            if c.is_dir() {
                Some(c)
            } else if c.is_file() {
                c.parent().map(Path::to_path_buf)
            } else {
                None
            }
        })
    }

    pub fn open_file(&mut self, ui: &MainWindow, path: PathBuf) {
        self.cancel_scan();
        self.set_playlist(ui, vec![path.clone()], Source::File(path));
        self.load_track_by_index(ui, 0, true, 0);
    }

    /// Plays an album folder, with its subfolders, once its songs have been read.
    pub fn open_album_folder(&mut self, folder: PathBuf) {
        let trimmed = folder.to_string_lossy();
        let trimmed = trimmed.trim_end_matches(['/', '\\']);
        let name = Path::new(trimmed)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| folder.to_string_lossy().into_owned());
        let label = format!("“{name}”");
        self.scan(
            vec![folder.clone()],
            Vec::new(),
            true,
            ScanThen::Album { folder, name },
            label,
        );
    }

    /// Opens a folder as an album, or a song (the debug-only `LRC_DEV_OPEN`).
    #[cfg(debug_assertions)]
    pub fn open_path(&mut self, ui: &MainWindow, path: PathBuf) {
        if path.is_dir() {
            self.open_album_folder(path);
        } else {
            self.open_file(ui, path);
        }
    }

    /// Several songs and folders dropped at once (`open_files`).
    fn open_files(&mut self, ui: &MainWindow, files: Vec<PathBuf>) {
        self.cancel_scan();
        if files.is_empty() {
            return;
        }
        if files.len() == 1 {
            let file = files.into_iter().next().unwrap();
            self.open_file(ui, file);
            return;
        }
        let dir = files[0].parent().map(Path::to_path_buf).unwrap_or_default();
        self.set_playlist(ui, files, Source::Files(dir));
        self.show_queue_panel(ui);
        let start = self.queue.start_index();
        self.load_track_by_index(ui, start, true, 0);
    }

    /// Reads songs' tags on a worker thread, then carries on with `then` (`_scan`).
    /// Opening something else first makes this result be ignored.
    fn scan(
        &mut self,
        folders: Vec<PathBuf>,
        files: Vec<PathBuf>,
        sort_files: bool,
        then: ScanThen,
        label: String,
    ) {
        self.cancel_scan();
        let token = self.scan_token;
        let cancel = Arc::new(AtomicBool::new(false));
        self.scan_cancel = Some(cancel.clone());
        let label = if label.is_empty() {
            "songs".to_owned()
        } else {
            label
        };
        let spawned = std::thread::Builder::new()
            .name("album scan".into())
            .spawn(move || {
                let cancelled = || cancel.load(Ordering::Relaxed);
                let mut progress = |count: usize| {
                    let label = label.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        app_call(|a, ui| a.scan_progress(ui, token, count, &label));
                    });
                };
                let songs = scan_songs(
                    &folders,
                    &files,
                    &SUPPORTED_EXTS,
                    sort_files,
                    &cancelled,
                    &mut progress,
                );
                if let Some(songs) = songs {
                    let _ = slint::invoke_from_event_loop(move || {
                        let entries = songs.into_iter().map(|s| (s.path, s.title)).collect();
                        app_call(|a, ui| a.scan_done(ui, token, entries, then));
                    });
                }
            });
        if spawned.is_err() {
            self.scan_cancel = None;
        }
    }

    /// Whatever is opened now wins over an album that is still being read.
    fn cancel_scan(&mut self) {
        self.scan_token += 1;
        if let Some(cancel) = self.scan_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    fn scan_progress(&mut self, ui: &MainWindow, token: u64, count: usize, label: &str) {
        if token == self.scan_token && count >= 50 {
            self.show_lyrics_message(
                ui,
                &format!("Reading {label}…   {count} songs so far"),
                "status",
            );
        }
    }

    fn scan_done(
        &mut self,
        ui: &MainWindow,
        token: u64,
        entries: Vec<(PathBuf, Option<String>)>,
        then: ScanThen,
    ) {
        if token != self.scan_token {
            return; // something else was opened in the meantime
        }
        self.scan_cancel = None;
        self.track_titles = entries.iter().cloned().collect();
        let paths: Vec<PathBuf> = entries.into_iter().map(|(p, _)| p).collect();
        match then {
            ScanThen::Album { folder, name } => {
                if paths.is_empty() {
                    message_later(
                        rfd::MessageLevel::Info,
                        "Open album",
                        format!("No audio files found in “{name}”."),
                    );
                    return;
                }
                self.set_playlist(ui, paths, Source::Folder(folder));
                self.show_queue_panel(ui);
                let start = self.queue.start_index();
                self.load_track_by_index(ui, start, true, 0);
            }
            ScanThen::Files => self.open_files(ui, paths),
            ScanThen::Restore(remembered) => self.finish_restore(ui, paths, remembered),
        }
    }

    fn set_playlist(&mut self, ui: &MainWindow, files: Vec<PathBuf>, source: Source) {
        self.queue.set_playlist(files, source);
        self.populate_queue_list(ui);
        self.update_navigation();
    }

    /// `_update_navigation`: whether the media flyout's Previous and Next work.
    fn update_navigation(&mut self) {
        let has_playlist = !self.queue.playlist().is_empty();
        let has_next = self.queue.has_next();
        if let Some(media) = &mut self.media {
            media.set_navigation(has_playlist, has_next);
        }
    }

    /// `_populate_queue_list`.
    fn populate_queue_list(&mut self, ui: &MainWindow) {
        let rows: Vec<SharedString> = self
            .queue
            .playlist()
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let title = match self.track_titles.get(path) {
                    Some(title) => title.clone(),
                    None => read_track_tags(path).title,
                };
                rules::queue_row_text(i + 1, title.as_deref(), path).into()
            })
            .collect();
        let count = rows.len();
        ui.set_queue_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
        ui.set_queue_current(-1);
        ui.set_queue_title(format!("Tracklist ({count})").into());
    }

    fn show_queue_panel(&mut self, ui: &MainWindow) {
        if ui.get_fullscreen() {
            self.queue_was_visible = true;
        } else {
            ui.set_queue_visible(true);
        }
    }

    /// A song the user picked in the tracklist (`play_index`).
    fn play_index(&mut self, ui: &MainWindow, index: usize) {
        self.queue.pick(index);
        self.update_navigation();
        self.load_track_by_index(ui, index, true, 0);
    }

    /// `load_track_by_index`: opens a playlist song, playing or paused, at `start_ms`,
    /// and shows its details and lyrics.
    fn load_track_by_index(
        &mut self,
        ui: &MainWindow,
        index: usize,
        autoplay: bool,
        start_ms: u64,
    ) {
        if !self.queue.set_current(index) {
            return;
        }
        let path = self.queue.playlist()[index].clone();
        ui.set_queue_current(index as i32);
        self.player.load(&path);
        self.visualizer.clear(); // `sourceChanged`
        if start_ms > 0 {
            self.player.seek(start_ms);
        }
        if autoplay {
            self.player.play();
        } else {
            self.player.pause();
        }

        let meta = extract_metadata_and_cover(&path);
        self.embedded_lyrics = meta.lyrics.unwrap_or_default();
        ui.set_has_track(true);
        ui.set_track_title(meta.info.title.as_str().into());
        ui.set_track_artist(meta.info.artist.as_str().into());
        ui.set_specs_text(rules::specs_text(&meta.info).into());
        match &meta.cover {
            Some(cover) => {
                let (w, h) = (cover.image.width(), cover.image.height());
                let buf =
                    SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(cover.image.as_raw(), w, h);
                ui.set_cover(Image::from_rgba8(buf));
                ui.set_has_cover(true);
            }
            None => {
                ui.set_cover(Image::default());
                ui.set_has_cover(false);
            }
        }
        let has_next = self.queue.has_next();
        if let Some(media) = &mut self.media {
            // "Previous" restarts the song when it isn't the first one.
            media.set_track(
                &meta.info,
                meta.cover.as_ref().map(|c| &c.image),
                true,
                has_next,
            );
            media.update_timeline(0, meta.info.duration_sec * 1000, true);
        }
        self.info = Some(meta.info);
        self.update_tray(ui);

        if !self.show_own_lyrics(ui, &path) {
            self.search_lyrics_online(ui, false);
        }
    }

    /// `_show_own_lyrics`: a .lrc next to the song wins over lyrics in its tags. False
    /// if the song has neither.
    fn show_own_lyrics(&mut self, ui: &MainWindow, path: &Path) -> bool {
        let lrc_path = lrc_path_for(path);
        if lrc_path.exists() {
            match read_lyrics_file(&lrc_path) {
                Ok(text) => {
                    self.parse_and_display_lyrics(ui, &text);
                    return true;
                }
                Err(problem) => {
                    if self.embedded_lyrics.is_empty() {
                        // Say what's wrong instead of quietly showing other lyrics.
                        self.show_lyrics_message(ui, &problem.to_string(), "empty");
                        return true;
                    }
                    eprintln!("{problem}");
                }
            }
        }
        if !self.embedded_lyrics.is_empty() {
            let text = self.embedded_lyrics.clone();
            self.parse_and_display_lyrics(ui, &text);
            return true;
        }
        false
    }

    // -------------------------------------------------------------------------
    // Lyrics
    // -------------------------------------------------------------------------

    /// `_clear_lyrics`.
    fn clear_lyrics(&mut self, ui: &MainWindow) {
        ui.invoke_stop_lyrics_scroll();
        self.lines.set_vec(Vec::new());
        self.shown.clear();
        self.line_times.clear();
        self.current_line = None;
        self.synced = false;
        ui.set_synced(false);
        ui.set_lyrics_message(SharedString::new());
        ui.set_lyrics_scroll(0.0);
    }

    /// `_show_lyrics_message`: a one-line note in the lyrics area instead of lyrics.
    fn show_lyrics_message(&mut self, ui: &MainWindow, text: &str, kind: &str) {
        self.clear_lyrics(ui);
        ui.set_lyrics_message(text.into());
        ui.set_lyrics_message_kind(kind.into());
    }

    /// `parse_and_display_lyrics`.
    pub fn parse_and_display_lyrics(&mut self, ui: &MainWindow, content: &str) {
        self.clear_lyrics(ui);
        let parsed = lrc::parse(content);
        self.synced = parsed.synced;
        ui.set_synced(parsed.synced);
        self.line_times = parsed.lines.iter().map(|l| l.time_ms).collect();
        let rows: Vec<LyricLineData> = parsed
            .lines
            .iter()
            .map(|LyricLine { main, sub, .. }| LyricLineData {
                main: main.into(),
                sub: sub.into(),
                active: false,
            })
            .collect();
        self.shown = parsed.lines;
        self.lines.set_vec(rows);
        // `_reset_scroll_and_sync`: back to the top, then the line for the position.
        let pos = self.player.position_ms();
        if pos > 0 {
            self.sync_lyrics_to_position(pos);
        }
    }

    /// `_sync_lyrics_to_position`: lights up the line for `pos_ms`.
    fn sync_lyrics_to_position(&mut self, pos_ms: u64) {
        if self.line_times.is_empty() || !self.synced {
            return;
        }
        let new = rules::active_line(&self.line_times, pos_ms, self.offset_ms);
        if new == self.current_line {
            return;
        }
        if let Some(old) = self.current_line {
            self.set_line_active(old, false);
        }
        if let Some(new) = new {
            // The card scrolls itself to the middle once it's active.
            self.set_line_active(new, true);
        }
        self.current_line = new;
    }

    fn set_line_active(&self, row: usize, active: bool) {
        if let Some(mut data) = self.lines.row_data(row) {
            data.active = active;
            self.lines.set_row_data(row, data);
        }
    }

    /// `LyricCard.mousePressEvent`: a click on a synced line seeks to it.
    fn line_clicked(&mut self, row: i32) {
        if !self.synced {
            return;
        }
        if let Some(&t) = usize::try_from(row)
            .ok()
            .and_then(|r| self.line_times.get(r))
        {
            self.seek_to(rules::line_seek_ms(t, self.offset_ms));
        }
    }

    /// `set_lyrics_offset`: ±30 s at most, and remembered.
    fn set_lyrics_offset(&mut self, ui: &MainWindow, ms: i64, persist: bool) {
        self.offset_ms = rules::clamp_offset(ms);
        if persist {
            self.store.set("lyrics/offset_ms", self.offset_ms);
        }
        ui.set_offset_text(rules::offset_text(self.offset_ms).into());
        let pos = self.player.position_ms();
        self.sync_lyrics_to_position(pos);
    }

    // -------------------------------------------------------------------------
    // Transport
    // -------------------------------------------------------------------------

    fn toggle_play(&mut self) {
        if self.player.state() == State::Playing {
            self.player.pause();
        } else {
            self.player.play();
        }
    }

    fn seek_to(&mut self, ms: u64) {
        self.player.seek(ms);
        let duration = self.player.duration_ms();
        if let Some(media) = &mut self.media {
            media.update_timeline(ms, duration, true);
        }
    }

    fn play_previous(&mut self, ui: &MainWindow) {
        let pos = self.player.position_ms();
        match self.queue.previous_track(pos) {
            Some(Previous::Restart) => self.seek_to(0),
            Some(Previous::Play(i)) => self.load_track_by_index(ui, i, true, 0),
            None => {}
        }
    }

    fn play_next(&mut self, ui: &MainWindow) {
        if let Some(i) = self.queue.next_track() {
            self.load_track_by_index(ui, i, true, 0);
        }
    }

    fn set_shuffle(&mut self, ui: &MainWindow, on: bool, persist: bool) {
        self.queue.set_shuffle(on);
        ui.set_shuffle(on);
        if persist {
            self.store.set("playback/shuffle", on);
        }
        self.update_navigation();
        self.update_tray(ui);
    }

    /// `set_repeat`: repeat-track loops inside the player, which is gapless.
    fn set_repeat(&mut self, ui: &MainWindow, repeat: Repeat, persist: bool) {
        self.queue.set_repeat(repeat);
        if persist {
            self.store.set("playback/repeat", repeat.key());
        }
        self.player.set_looping(repeat == Repeat::Track);
        ui.set_repeat(repeat.key().into());
        ui.set_repeat_tip(repeat.tooltip().into());
        self.update_tray(ui);
        self.update_navigation();
    }

    fn toggle_mute(&mut self, ui: &MainWindow) {
        let muted = !self.player.is_muted();
        self.player.set_muted(muted);
        ui.set_muted(muted);
    }

    /// `_on_volume_changed`: linear, and moving the slider up unmutes.
    fn volume_changed(&mut self, ui: &MainWindow, value: f32) {
        self.player.set_volume(value / 100.0);
        if self.player.is_muted() && value > 0.0 {
            self.player.set_muted(false);
            ui.set_muted(false);
        }
    }

    // -------------------------------------------------------------------------
    // Window
    // -------------------------------------------------------------------------

    /// `toggle_fullscreen`: the tracklist hides while fullscreen and comes back after.
    fn toggle_fullscreen(&mut self, ui: &MainWindow) {
        if ui.get_fullscreen() {
            ui.set_fullscreen(false);
            if self.queue_was_visible {
                ui.set_queue_visible(true);
            }
        } else {
            self.windowed_placement = self.current_placement(ui);
            self.queue_was_visible = ui.get_queue_visible();
            ui.set_queue_visible(false);
            ui.set_fullscreen(true);
        }
        ui.set_rescale_count(ui.get_rescale_count().wrapping_add(1));
    }

    /// `show_shortcuts`: the F1 list.
    fn show_shortcuts(&mut self) {
        if self.shortcuts.is_none() {
            let Ok(dialog) = ShortcutsDialog::new() else {
                return;
            };
            let rows: Vec<ShortcutRow> = rules::SHORTCUTS
                .iter()
                .map(|(keys, action)| ShortcutRow {
                    keys: (*keys).into(),
                    action: (*action).into(),
                })
                .collect();
            dialog.set_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
            dialog.set_footer(rules::SHORTCUTS_FOOTER.into());
            fill_theme(
                &dialog.global::<Theme>(),
                &build_palette(self.theme, self.accent),
            );
            let weak = dialog.as_weak();
            dialog.on_dismiss(move || {
                if let Some(d) = weak.upgrade() {
                    let _ = d.hide();
                }
            });
            self.shortcuts = Some(dialog);
        }
        if let Some(dialog) = &self.shortcuts {
            let _ = dialog.show();
        }
    }

    // -------------------------------------------------------------------------
    // Lyrics online (`search_lyrics_online`, `_on_online_lyrics_fetched`)
    // -------------------------------------------------------------------------

    fn is_current(&self, path: &Path) -> bool {
        self.queue.current_path().is_some_and(|p| p == path)
    }

    /// Looks the song up on LRCLIB on a worker thread. `manual`: the user picked
    /// Lyrics ▾ → Search online.
    ///
    /// Lyrics found are saved as a .lrc next to the song only when that can't overwrite
    /// anything and the match is reliable. Otherwise a manual search asks first, and an
    /// automatic one only shows them.
    fn search_lyrics_online(&mut self, ui: &MainWindow, manual: bool) {
        let (Some(path), Some(info)) = (self.queue.current_path().cloned(), self.info.as_ref())
        else {
            return;
        };
        let query = Query::new(
            &info.title,
            &info.artist,
            i64::try_from(info.duration_sec).unwrap_or(i64::MAX),
            info.title_from_tags,
        );
        self.show_lyrics_message(ui, rules::SEARCHING_TEXT, "status");
        let quitting = self.quitting.clone();
        // A result for a song that is no longer current is dropped when it arrives.
        let _ = std::thread::Builder::new()
            .name("lrclib".into())
            .spawn(move || {
                if let Some(found) = lrclib::fetch(&query, &quitting) {
                    let _ = slint::invoke_from_event_loop(move || {
                        app_call(|a, ui| a.online_lyrics_fetched(ui, found, path, manual));
                    });
                }
            });
    }

    fn online_lyrics_fetched(
        &mut self,
        ui: &MainWindow,
        found: FetchResult,
        path: PathBuf,
        manual: bool,
    ) {
        if self.closing || !self.is_current(&path) {
            return; // a late answer for a song that is no longer current
        }
        if found.lyrics.is_empty() {
            if manual && self.show_own_lyrics(ui, &path) {
                message_later(
                    rfd::MessageLevel::Info,
                    rules::SEARCH_TITLE,
                    rules::LRCLIB_HAS_NONE_TEXT.to_owned(),
                );
                return;
            }
            self.show_lyrics_message(ui, rules::NO_LYRICS_TEXT, "empty");
            return;
        }

        self.parse_and_display_lyrics(ui, &found.lyrics);
        let lrc_path = lrc_path_for(&path);
        let has_file = lrc_path.exists();
        let has_embedded = !self.embedded_lyrics.is_empty();
        match rules::after_fetch(self.synced, found.trusted, has_file, has_embedded, manual) {
            AfterFetch::ShowOnly => {}
            AfterFetch::Cache => {
                // Kept, so the song opens instantly next time.
                if let Err(error) = write_lrc(&lrc_path, &found.lyrics) {
                    eprintln!("Could not cache LRC file: {error}");
                }
            }
            AfterFetch::Offer => {
                let mine = self
                    .info
                    .as_ref()
                    .map_or(0, |i| i64::try_from(i.duration_sec).unwrap_or(0));
                let offer = rules::save_offer(
                    &file_name(&lrc_path),
                    found.trusted,
                    found.lrclib_duration_sec,
                    mine,
                    has_file,
                    has_embedded,
                );
                self.offer_to_save(path, found.lyrics, offer);
            }
        }
    }

    /// `_offer_to_save_online_lyrics`: "Save these lyrics as …?" or "Replace …?".
    fn offer_to_save(&mut self, path: PathBuf, text: String, offer: SaveOffer) {
        if self.question.is_none() {
            let Ok(dialog) = QuestionDialog::new() else {
                return;
            };
            fill_theme(
                &dialog.global::<Theme>(),
                &build_palette(self.theme, self.accent),
            );
            let weak = dialog.as_weak();
            dialog.on_answered(move |yes| {
                if let Some(d) = weak.upgrade() {
                    let _ = d.hide();
                }
                app_call(|a, ui| a.offer_answered(ui, yes));
            });
            // Closing the box is "Don't save" / "Keep mine", as Escape is.
            dialog.window().on_close_requested(|| {
                app_call(|a, ui| a.offer_answered(ui, false));
                slint::CloseRequestResponse::HideWindow
            });
            self.question = Some(dialog);
        }
        let Some(dialog) = &self.question else { return };
        dialog.set_window_title(rules::SEARCH_TITLE.into());
        dialog.set_question(offer.question.as_str().into());
        dialog.set_notes(offer.notes.join("\n\n").into());
        dialog.set_yes_text(offer.yes.into());
        dialog.set_no_text(offer.no.into());
        dialog.set_default_no(offer.default_no);
        self.pending_offer = Some(PendingOffer { path, text, offer });
        let _ = dialog.show();
    }

    fn offer_answered(&mut self, ui: &MainWindow, yes: bool) {
        let Some(PendingOffer { path, text, offer }) = self.pending_offer.take() else {
            return;
        };
        let lrc_path = lrc_path_for(&path);
        if yes {
            if let Err(error) = write_lrc(&lrc_path, &text) {
                message_later(
                    rfd::MessageLevel::Warning,
                    rules::SEARCH_TITLE,
                    rules::save_failed_text(&file_name(&lrc_path), &error),
                );
            }
        } else if offer.no_keeps_mine() && self.is_current(&path) {
            self.show_own_lyrics(ui, &path); // back to the user's own lyrics
        }
    }

    /// The writing part of `embed_lyrics_in_file`. Windows won't let a file be replaced
    /// while it is playing, so the player lets go of it for a moment and then carries on
    /// at the same position, playing or paused as before.
    fn write_lyrics_into(&mut self, path: &Path, text: &str) -> Result<(), String> {
        let is_current = self.is_current(path);
        let resume_ms = if is_current {
            self.player.position_ms()
        } else {
            0
        };
        let was_playing = is_current && self.player.state() == State::Playing;
        if is_current {
            self.player.unload(); // releases the file
        }

        let mut error = None;
        for _ in 0..6 {
            match write_embedded_lyrics(path, text) {
                Ok(()) => {
                    error = None;
                    break;
                }
                // The file may take a moment to be released.
                Err(e) if e.is_busy() => {
                    error = Some(e.to_string());
                    std::thread::sleep(Duration::from_millis(250));
                }
                Err(e) => {
                    error = Some(e.to_string());
                    break;
                }
            }
        }

        if is_current {
            self.player.load(path);
            if resume_ms > 0 {
                self.player.seek(resume_ms);
            }
            if was_playing {
                self.player.play();
            } else {
                self.player.pause();
            }
        }
        if let Some(error) = error {
            return Err(error);
        }
        if is_current {
            // The song now carries these lyrics, so a later online search treats them as
            // its own (won't quietly save a .lrc over them, and "Keep mine" restores them).
            self.embedded_lyrics = text.to_owned();
        }
        Ok(())
    }

    /// Ctrl+I: the Track Details window (`show_details_dialog`).
    fn show_details_dialog(&mut self, ui: &MainWindow) {
        let Some(info) = &self.info else { return };
        if self.details.is_none() {
            let Ok(dialog) = DetailsDialog::new() else {
                return;
            };
            fill_theme(
                &dialog.global::<Theme>(),
                &build_palette(self.theme, self.accent),
            );
            let weak = dialog.as_weak();
            dialog.on_dismiss(move || {
                if let Some(d) = weak.upgrade() {
                    let _ = d.hide();
                }
            });
            self.details = Some(dialog);
        }
        let Some(dialog) = &self.details else { return };
        let rows: Vec<DetailRow> = rules::details_fields(info)
            .into_iter()
            .map(|(label, value)| DetailRow {
                label: label.into(),
                value: value.into(),
            })
            .collect();
        dialog.set_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
        dialog.set_cover(ui.get_cover());
        dialog.set_has_cover(ui.get_has_cover());
        let _ = dialog.show();
    }

    // -------------------------------------------------------------------------
    // Session memory (`_restore_session_ui`, `_restore_playback_session`,
    // `_save_session`)
    // -------------------------------------------------------------------------

    /// Theme, volume, lyrics offset, shuffle, repeat and window position. Runs before
    /// the window is shown.
    fn restore_session_ui(&mut self, ui: &MainWindow) {
        let s = &self.store.settings;
        let theme = s.get_str("ui/theme", "dark");
        let accent = s.get_str("ui/accent", "coral");
        let volume = s.get_int("ui/volume", 80).clamp(0, 100);
        let offset = s.get_int("lyrics/offset_ms", 0);
        let shuffle = s.get_bool("playback/shuffle", false);
        let repeat = Repeat::from_key(&s.get_str("playback/repeat", "off"));
        let placement = s.raw(PLACEMENT_KEY).and_then(Placement::from_json);
        self.queue_wanted = s.get_bool("ui/queue_open", false);

        self.apply_theme(ui, &theme, &accent, false);
        ui.set_volume(volume as f32);
        self.player.set_volume(volume as f32 / 100.0);
        self.set_lyrics_offset(ui, offset, false);
        self.set_shuffle(ui, shuffle, false);
        self.set_repeat(ui, repeat, false);
        let mode = self.store.settings.get_str("ui/visualizer", "off");
        self.close_to_tray =
            self.store.settings.get_bool("ui/close_to_tray", true) && self.tray.is_some();
        ui.set_close_to_tray(self.close_to_tray);
        self.set_visualizer(ui, &mode, false);
        // The romaji setting is kept for M5.

        if let Some(p) = placement {
            let window = ui.window();
            window.set_size(PhysicalSize::new(p.width, p.height));
            window.set_position(PhysicalPosition::new(p.x, p.y));
            if p.maximized {
                window.set_maximized(true);
            }
        }
    }

    /// Fullscreen, playlist, song and position. Runs right after the window appears.
    pub fn restore_playback_session(&mut self, ui: &MainWindow) {
        let s = &self.store.settings;
        if s.get_bool("window/fullscreen", false) && !ui.get_fullscreen() {
            self.toggle_fullscreen(ui);
        }
        let remembered = memory::restore_plan(&self.store.settings);
        match remembered.plan.clone() {
            RestorePlan::Nothing => {}
            // The album is read in the background.
            RestorePlan::Folder(folder) => self.scan(
                vec![folder],
                Vec::new(),
                true,
                ScanThen::Restore(remembered),
                String::new(),
            ),
            RestorePlan::File(file) => self.finish_restore(ui, vec![file], remembered),
            // Kept in the order they were played.
            RestorePlan::Files(files) => self.scan(
                Vec::new(),
                files,
                false,
                ScanThen::Restore(remembered),
                String::new(),
            ),
        }
    }

    /// `_finish_restore`: the playlist is back; open the remembered song, paused.
    fn finish_restore(&mut self, ui: &MainWindow, files: Vec<PathBuf>, remembered: Remembered) {
        if files.is_empty() {
            return;
        }
        let source = remembered.source(&self.store.settings);
        self.set_playlist(ui, files, source);
        let start = self.queue.start_index();
        let (index, position, found) = remembered.start(self.queue.playlist(), start);
        if found {
            // Under shuffle the rest of the playlist is shuffled to come after it.
            self.queue.pick(index);
        }
        // In fullscreen the drawer is hidden, and shown again on leaving fullscreen.
        if ui.get_fullscreen() {
            self.queue_was_visible = self.queue_wanted;
        } else {
            ui.set_queue_visible(self.queue_wanted);
        }
        // Restored paused: never start blasting audio on launch.
        self.load_track_by_index(ui, index, false, position);
    }

    /// The window's position and size now, unless it is fullscreen. A maximized window
    /// keeps the size it had before it was maximized.
    fn current_placement(&self, ui: &MainWindow) -> Option<Placement> {
        let window = ui.window();
        if window.is_fullscreen() || ui.get_fullscreen() || window.is_minimized() {
            return None;
        }
        let pos = window.position();
        let size = window.size();
        let mut p = Placement {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            maximized: window.is_maximized(),
        };
        if p.maximized
            && let Some(old) = self
                .store
                .settings
                .raw(PLACEMENT_KEY)
                .and_then(Placement::from_json)
        {
            p = Placement {
                maximized: true,
                ..old
            };
        }
        Some(p)
    }

    /// `_save_session`: `full` when the window closes; the 5 s autosave only keeps the
    /// volume, offset and playback.
    fn save_session(&mut self, ui: &MainWindow, full: bool) {
        if self.closing {
            return;
        }
        let playback = match (self.queue.current_path(), self.queue.source()) {
            (Some(current), source) if *source != Source::None => {
                // A resume position not reached yet is what the player reads as its
                // position, so nothing needs to be kept aside for it.
                Some(PlaybackSnapshot {
                    source: source.clone(),
                    playlist: self.queue.playlist().to_vec(),
                    current_track: current.clone(),
                    position_ms: memory::resume_position_ms(
                        None,
                        self.player.position_ms(),
                        self.player.duration_ms(),
                    ),
                })
            }
            _ => None,
        };
        let window = full.then(|| {
            let fullscreen = ui.get_fullscreen();
            WindowSnapshot {
                fullscreen,
                placement: if fullscreen {
                    self.windowed_placement
                } else {
                    self.current_placement(ui)
                },
                queue_open: if fullscreen {
                    self.queue_was_visible || ui.get_queue_visible()
                } else {
                    ui.get_queue_visible()
                },
            }
        });
        let snap = Snapshot {
            volume: ui.get_volume().round() as i64,
            offset_ms: self.offset_ms,
            playback,
            window,
        };
        if memory::save_snapshot(&mut self.store.settings, &snap) {
            self.store.save();
        }
    }

    /// The window is closing (`closeEvent`): searches still running are told to stop
    /// and their answers ignored, the session is saved, and the music stops.
    fn close(&mut self) {
        self.quitting.store(true, Ordering::Relaxed);
        self.viz_timer.stop();
        self.cancel_scan();
        if let Some(ui) = self.ui.upgrade() {
            self.save_session(&ui, true);
        }
        self.closing = true; // nothing below may overwrite the snapshot just saved
        if let Some(media) = &mut self.media {
            media.shutdown();
        }
        self.player.stop();
        self.tray = None; // takes the icon out of the tray
        if let Some(dialog) = &self.shortcuts {
            let _ = dialog.hide();
        }
        if let Some(dialog) = &self.details {
            let _ = dialog.hide();
        }
        if let Some(dialog) = &self.question {
            let _ = dialog.hide();
        }
    }

    // -------------------------------------------------------------------------
    // Drops (`dragEnterEvent`, `dropEvent`, `_run_drop`)
    // -------------------------------------------------------------------------

    pub fn drag_hovered(&mut self, ui: &MainWindow, path: PathBuf) {
        self.hovered.push(path);
        match rules::drop_plan(&self.hovered, self.queue.current().is_some()) {
            Some(plan) => {
                ui.set_drop_headline(plan.headline.into());
                ui.set_drop_detail(plan.detail.into());
                ui.set_drop_visible(true);
            }
            None => ui.set_drop_visible(false),
        }
    }

    pub fn drag_left(&mut self, ui: &MainWindow) {
        self.hovered.clear();
        ui.set_drop_visible(false);
    }

    /// A file was dropped. Several arrive one after another; they're handled together
    /// once the drop is over, so Explorer isn't left waiting while an album is read.
    pub fn file_dropped(&mut self, path: PathBuf) {
        self.dropped.push(path);
        if self.drop_pending {
            return;
        }
        self.drop_pending = true;
        Timer::single_shot(Duration::ZERO, || app_call(|a, ui| a.run_drop(ui)));
    }

    fn run_drop(&mut self, ui: &MainWindow) {
        self.drop_pending = false;
        let paths = std::mem::take(&mut self.dropped);
        self.hovered.clear();
        ui.set_drop_visible(false);
        let Some(plan) = rules::drop_plan(&paths, self.queue.current().is_some()) else {
            return;
        };
        match plan.action {
            DropAction::Lyrics(path) => {
                let weak = APP.with(|a| a.borrow().clone());
                Timer::single_shot(Duration::ZERO, move || {
                    show_lyrics_file(&weak, &path, "Open lyrics")
                });
            }
            DropAction::Folder(folder) => self.open_album_folder(folder),
            DropAction::File(file) => self.open_file(ui, file),
            DropAction::Files { folders, audio } => {
                self.scan(folders, audio, true, ScanThen::Files, String::new());
            }
        }
    }

    // -------------------------------------------------------------------------
    // The clock
    // -------------------------------------------------------------------------

    fn tick(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        for event in self.player.poll() {
            match event {
                Event::Loaded { duration_ms } => {
                    // `_on_duration_changed`
                    ui.set_seek_max(duration_ms as f32);
                    ui.set_duration_text(rules::format_time(duration_ms).into());
                    let pos = self.player.position_ms();
                    if let Some(media) = &mut self.media {
                        media.update_timeline(pos, duration_ms, true);
                    }
                }
                Event::StateChanged(state) => self.playback_state_changed(&ui, state),
                // `_on_media_status_changed`: no editors yet (M7), so nothing holds a song.
                Event::EndOfMedia => {
                    if self.queue.repeat() == Repeat::Track {
                        self.player.seek(0);
                        self.player.play();
                    } else if self.queue.has_next() {
                        self.play_next(&ui);
                    }
                }
                // The Python app shows nothing when Qt can't play a file; here the
                // error message says why (checklist item 18).
                Event::Error(message) => {
                    eprintln!("playback error: {message}");
                    let name = self
                        .queue
                        .current_path()
                        .map(|p| file_name(p))
                        .unwrap_or_default();
                    crate::errors::report(message.clone(), format!("{name}\n\n{message}"));
                }
            }
        }
        if self.player.duration_ms() == 0 {
            ui.set_seek_max(0.0);
            ui.set_duration_text(rules::format_time(0).into());
        }
        let pos = self.player.position_ms();
        if !self.seeking {
            ui.set_seek_value(pos as f32);
            ui.set_position_text(rules::format_time(pos).into());
        }
        let duration = self.player.duration_ms();
        if let Some(media) = &mut self.media {
            media.update_timeline(pos, duration, false);
        }
        self.sync_lyrics_to_position(pos);
    }

    /// `_on_playback_state_changed`.
    fn playback_state_changed(&mut self, ui: &MainWindow, state: State) {
        ui.set_playing(state == State::Playing);
        let (pos, duration) = (self.player.position_ms(), self.player.duration_ms());
        if let Some(media) = &mut self.media {
            media.set_playback_state(state);
            media.update_timeline(pos, duration, true);
        }
        self.update_tray(ui);
        self.wake_visualizer();
    }

    // -------------------------------------------------------------------------
    // Visualizer (`set_visualizer`, `VisualizerBackdrop`)
    // -------------------------------------------------------------------------

    fn set_visualizer(&mut self, ui: &MainWindow, mode: &str, persist: bool) {
        self.visualizer.set_mode(mode, &self.player);
        let mode = self.visualizer.mode();
        ui.set_visualizer_mode(mode.into());
        if persist {
            self.store.set("ui/visualizer", mode);
        }
        if mode == "off" {
            self.viz_timer.stop();
            ui.set_viz_frame(Image::default());
        } else {
            self.start_viz_timer();
        }
    }

    /// `_wake`: the music started again, so the bars may move.
    fn wake_visualizer(&mut self) {
        if self.visualizer.mode() != "off" && !self.viz_timer.running() {
            self.start_viz_timer();
        }
    }

    fn start_viz_timer(&mut self) {
        self.viz_timer.start(
            TimerMode::Repeated,
            Duration::from_millis(viz::TICK_MS),
            || app_call(|a, ui| a.viz_tick(ui)),
        );
    }

    fn viz_tick(&mut self, ui: &MainWindow) {
        let playing = self.player.state() == State::Playing;
        let pos = self.player.position_ms();
        let keep_going = self.visualizer.tick(playing, pos);
        let frame = self.visualizer.render(
            ui.get_lyrics_width(),
            ui.get_lyrics_height(),
            ui.window().scale_factor(),
            {
                let c = ui.get_viz_accent();
                [c.red(), c.green(), c.blue(), c.alpha()]
            },
            f64::from(ui.get_viz_alpha()),
        );
        ui.set_viz_frame(frame);
        if !keep_going {
            self.viz_timer.stop(); // paused or stopped: nothing to animate
        }
    }

    // -------------------------------------------------------------------------
    // Media flyout (`WindowsMediaControls` and the `_smtc_*` slots)
    // -------------------------------------------------------------------------

    /// Connects the media flyout and media keys. Needs the native window, so it runs
    /// once the window is shown.
    pub fn init_media_controls(&mut self, ui: &MainWindow) {
        let Some(hwnd) = crate::platform::window_handle(ui) else {
            return;
        };
        self.media = MediaControls::new(hwnd, |command| {
            let _ = slint::invoke_from_event_loop(move || {
                app_call(|a, ui| a.media_command(ui, command));
            });
        });
        if self.media.is_some() {
            self.update_navigation();
        }
    }

    fn media_command(&mut self, ui: &MainWindow, command: MediaCommand) {
        let has_playlist = !self.queue.playlist().is_empty();
        match command {
            MediaCommand::Play => {
                if has_playlist && self.player.state() != State::Playing {
                    self.player.play();
                }
            }
            MediaCommand::Pause => {
                if self.player.state() == State::Playing {
                    self.player.pause();
                }
            }
            MediaCommand::Next => self.play_next(ui),
            MediaCommand::Previous => self.play_previous(ui),
            MediaCommand::Seek(ms) => {
                if has_playlist {
                    self.seek_to(ms);
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // Tray (`_init_tray`, `_update_tray`, `_on_tray_activated`, `closeEvent`)
    // -------------------------------------------------------------------------

    fn init_tray(&mut self, ui: &MainWindow) {
        let texts = self.tray_texts(ui);
        // The tray runs on a thread of its own; its clicks come back to this one.
        self.tray = Tray::new(texts, |command| {
            let _ = slint::invoke_from_event_loop(move || {
                app_call(|a, ui| a.tray_command(ui, command));
            });
        });
        ui.set_tray_available(self.tray.is_some());
    }

    fn tray_texts(&self, ui: &MainWindow) -> TrayTexts {
        let track = self
            .info
            .as_ref()
            .filter(|_| self.queue.current().is_some())
            .map(|i| (i.title.as_str(), i.artist.as_str()));
        let shown = !self.window_hidden && !ui.window().is_minimized();
        sys::tray_texts(
            track,
            self.player.state() == State::Playing,
            self.queue.repeat(),
            shown,
        )
    }

    fn update_tray(&mut self, ui: &MainWindow) {
        if let Some(tray) = &self.tray {
            tray.update(self.tray_texts(ui), self.queue.shuffle());
        }
    }

    fn tray_command(&mut self, ui: &MainWindow, command: TrayCommand) {
        match command {
            TrayCommand::Clicked | TrayCommand::DoubleClicked => self.toggle_window(ui),
            TrayCommand::MiddleClicked => self.toggle_play(),
            TrayCommand::Menu(item) => match item {
                TrayItem::Now => {}
                TrayItem::Play => self.toggle_play(),
                TrayItem::Next => self.play_next(ui),
                TrayItem::Previous => self.play_previous(ui),
                TrayItem::Shuffle => {
                    let on = !self.queue.shuffle();
                    self.set_shuffle(ui, on, true);
                }
                TrayItem::Repeat => {
                    let r = self.queue.repeat().next();
                    self.set_repeat(ui, r, true);
                }
                TrayItem::Window => self.toggle_window(ui),
                TrayItem::Quit => self.quit_app(ui),
            },
        }
    }

    fn toggle_window(&mut self, ui: &MainWindow) {
        if !self.window_hidden && !ui.window().is_minimized() {
            self.window_hidden = true;
            let _ = ui.hide();
        } else {
            self.show_window(ui);
        }
        self.update_tray(ui);
    }

    fn show_window(&mut self, ui: &MainWindow) {
        self.window_hidden = false;
        let _ = ui.show();
        ui.window().set_minimized(false);
        crate::platform::bring_to_front(ui);
    }

    /// `_close_to_tray`.
    fn close_to_tray(&self) -> bool {
        self.tray.is_some() && self.close_to_tray && !self.quit_requested
    }

    /// `closeEvent` with close-to-tray on: the music keeps playing and the window goes,
    /// with a hint the first time.
    fn close_to_tray_now(&mut self, ui: &MainWindow) {
        self.save_session(ui, true);
        self.window_hidden = true;
        self.update_tray(ui);
        if !self.store.settings.get_bool("ui/tray_hint_shown", false) {
            if let Some(tray) = &self.tray {
                tray.show_message(sys::TRAY_HINT_TITLE, sys::TRAY_HINT_TEXT, sys::TRAY_HINT_MS);
            }
            self.store.set("ui/tray_hint_shown", true);
        }
    }

    /// `quit_app`: Quit in the tray menu.
    fn quit_app(&mut self, ui: &MainWindow) {
        self.quit_requested = true;
        self.close(); // saves the window's place first, as `closeEvent` does
        let _ = ui.hide();
        let _ = slint::quit_event_loop();
    }
}

/// Runs `f` on the app from a callback that has no handle to it (workers, timers).
fn app_call(f: impl FnOnce(&mut App, &MainWindow)) {
    let weak = APP.with(|a| a.borrow().clone());
    with_app(&weak, f);
}
