//! M2's throwaway debug window: open songs, play, pause, seek, volume and mute, to test the
//! audio engine on Windows until M3 replaces it with the real main window.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};

use crate::MainWindow;
use crate::audio::{Event, Player, State};

struct Debug {
    player: Player,
    playlist: Vec<PathBuf>,
    index: usize,
}

impl Debug {
    fn load(&mut self, ui: &MainWindow, index: usize, autoplay: bool) {
        let Some(path) = self.playlist.get(index).cloned() else {
            return;
        };
        self.index = index;
        self.player.load(&path);
        if autoplay {
            self.player.play();
        } else {
            self.player.pause();
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        ui.set_track(
            format!(
                "{} ({}/{})",
                name.unwrap_or_default(),
                index + 1,
                self.playlist.len()
            )
            .into(),
        );
        ui.set_status("".into());
    }

    fn tick(&mut self, ui: &MainWindow) {
        for event in self.player.poll() {
            match event {
                Event::Loaded { duration_ms } => {
                    ui.set_duration_ms(duration_ms as f32);
                    ui.set_duration_text(format_time(duration_ms).into());
                }
                Event::StateChanged(state) => {
                    ui.set_playing(state == State::Playing);
                    ui.set_state_text(format!("{state:?}").into());
                }
                Event::EndOfMedia => {
                    if ui.get_stay_at_end() {
                        ui.set_status("End of song: staying on it, as with an editor open.".into());
                    } else if self.index + 1 < self.playlist.len() {
                        self.load(ui, self.index + 1, true);
                    } else {
                        ui.set_status("End of the list.".into());
                    }
                }
                Event::Error(message) => ui.set_status(format!("Error: {message}").into()),
            }
        }
        let pos = self.player.position_ms();
        if !ui.get_seeking() {
            ui.set_seek_value(pos as f32);
            ui.set_position_text(format_time(pos).into());
        }
        if self.player.duration_ms() == 0 {
            ui.set_duration_ms(0.0);
            ui.set_duration_text(format_time(0).into());
        }
    }
}

/// `QTime(0, 0, 0).addMSecs(ms).toString("m:ss")`, as `_format_time` shows it.
fn format_time(ms: u64) -> String {
    let secs = ms / 1000;
    format!("{}:{:02}", (secs / 60) % 60, secs % 60)
}

/// Wires the window to a new player. The returned timer must be kept alive.
pub fn attach(ui: &MainWindow) -> Timer {
    let state = Rc::new(RefCell::new(Debug {
        player: Player::new(),
        playlist: Vec::new(),
        index: 0,
    }));
    ui.set_volume(80.0);

    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_open_files(move || {
        let Some(ui) = weak.upgrade() else { return };
        let files = rfd::FileDialog::new()
            .set_title("Select Audio File")
            .add_filter("Audio Files", &["flac", "mp3", "wav", "m4a", "ogg", "opus"])
            .pick_files();
        if let Some(files) = files.filter(|f| !f.is_empty()) {
            let mut d = s.borrow_mut();
            d.playlist = files;
            d.load(&ui, 0, true);
        }
    });

    let s = state.clone();
    ui.on_toggle_play(move || {
        let mut d = s.borrow_mut();
        if d.player.state() == State::Playing {
            d.player.pause();
        } else {
            d.player.play();
        }
    });

    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_toggle_mute(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut d = s.borrow_mut();
        let muted = !d.player.is_muted();
        d.player.set_muted(muted);
        ui.set_muted(muted);
    });

    let s = state.clone();
    ui.on_seek_to(move |ms| s.borrow_mut().player.seek(ms.max(0.0) as u64));

    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_volume_changed(move |value| {
        let Some(ui) = weak.upgrade() else { return };
        let mut d = s.borrow_mut();
        // `_on_volume_changed`: linear, and moving the slider up unmutes.
        d.player.set_volume(value / 100.0);
        if d.player.is_muted() && value > 0.0 {
            d.player.set_muted(false);
            ui.set_muted(false);
        }
    });

    let s = state.clone();
    ui.on_repeat_changed(move |on| s.borrow_mut().player.set_looping(on));

    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_previous(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut d = s.borrow_mut();
        // `play_previous`: after 3 seconds, back to the start of the song.
        if d.player.position_ms() > 3000 || d.index == 0 {
            d.player.seek(0);
        } else {
            let i = d.index - 1;
            d.load(&ui, i, true);
        }
    });

    let weak = ui.as_weak();
    let s = state.clone();
    ui.on_next(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut d = s.borrow_mut();
        if d.index + 1 < d.playlist.len() {
            let i = d.index + 1;
            d.load(&ui, i, true);
        }
    });

    // The Python app reads the clock on Qt's position signal; M3 moves lyrics on this tick.
    let timer = Timer::default();
    let weak = ui.as_weak();
    timer.start(TimerMode::Repeated, Duration::from_millis(30), move || {
        if let Some(ui) = weak.upgrade() {
            state.borrow_mut().tick(&ui);
        }
    });
    timer
}
