//! The audio engine, in place of the Python app's `QMediaPlayer` and `QAudioOutput`.
//!
//! Three parts, on three threads:
//!
//! - [`Player`], on the UI thread: the commands (load, play, pause, seek, volume, mute)
//!   and the clock. The UI reads [`Player::position_ms`] on a timer and drains
//!   [`Player::poll`] for events.
//! - The decode thread (`engine`): Symphonia decodes, Rubato resamples to the device's
//!   rate, and the samples go into a buffer. It also follows the default output device.
//! - The output callback (`output`): plays from the buffer and counts the frames it plays,
//!   so the position is what is being heard, not what was decoded.
//!
//! Play, pause, volume and mute act in the callback directly, so they are instant. A seek or
//! a load starts a new buffer, which the callback switches to at once.

mod convert;
mod decode;
mod engine;
mod mp4_edit;
mod output;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use engine::{Cmd, Engine, EngineEvent};
use output::Shared;
pub use output::VizFeed;

/// Like `QMediaPlayer.PlaybackState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    Playing,
    Paused,
}

/// What happened since the last [`Player::poll`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The song is open and its length is known (`LoadedMedia` and `durationChanged`).
    Loaded {
        duration_ms: u64,
    },
    StateChanged(State),
    /// The song played to its end (`EndOfMedia`). Not sent while repeating one track.
    EndOfMedia,
    /// The song can't be played. The player is stopped.
    Error(String),
}

pub struct Player {
    cmds: Sender<Cmd>,
    events: Receiver<EngineEvent>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,

    /// The newest load, seek or stop.
    generation: u64,
    /// The newest load: events from older songs are dropped.
    load_generation: u64,
    /// A seek the output hasn't reached yet, so the position reads as the target meanwhile.
    seek_target: Option<(u64, u64)>,
    loaded: bool,
    state: State,
    duration_ms: u64,
    volume: f32,
    muted: bool,
    looping: bool,
    /// The end was reached: `play` starts over, as `QMediaPlayer` does.
    at_end: bool,
    queued: Vec<Event>,
}

impl Player {
    /// Starts the decode thread and opens the default output device.
    pub fn new() -> Self {
        let shared = Arc::new(Shared::new());
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let engine = Engine::new(cmd_rx, cmd_tx.clone(), event_tx, shared.clone());
        let thread = std::thread::Builder::new()
            .name("audio decode".into())
            .spawn(move || engine.run())
            .ok();
        let player = Self {
            cmds: cmd_tx,
            events: event_rx,
            shared,
            thread,
            generation: 0,
            load_generation: 0,
            seek_target: None,
            loaded: false,
            state: State::Stopped,
            duration_ms: 0,
            volume: 1.0,
            muted: false,
            looping: false,
            at_end: false,
            queued: Vec::new(),
        };
        // `self.audio_output.setVolume(0.8)` in the Python app.
        let mut player = player;
        player.set_volume(0.8);
        player
    }

    /// Opens a song, stopped at its start (`setSource`). Follow with [`play`](Self::play)
    /// or [`pause`](Self::pause).
    pub fn load(&mut self, path: &Path) {
        self.generation += 1;
        self.load_generation = self.generation;
        self.seek_target = None;
        self.loaded = true;
        self.at_end = false;
        self.duration_ms = 0;
        self.set_playing(false);
        self.set_state(State::Stopped);
        self.send(Cmd::Load {
            path: path.to_path_buf(),
            generation: self.generation,
        });
    }

    pub fn play(&mut self) {
        if !self.loaded {
            return;
        }
        if self.at_end {
            self.seek(0);
        }
        self.set_playing(true);
        self.set_state(State::Playing);
    }

    pub fn pause(&mut self) {
        if !self.loaded {
            return;
        }
        self.set_playing(false);
        self.set_state(State::Paused);
    }

    #[allow(dead_code, reason = "used by the main window from M3 on")]
    /// Stops and goes back to the start, keeping the song open (`stop`).
    pub fn stop(&mut self) {
        if !self.loaded {
            return;
        }
        self.set_playing(false);
        self.seek(0);
        self.set_state(State::Stopped);
    }

    #[allow(dead_code, reason = "used by the main window from M3 on")]
    /// Closes the song, so its file is free (`setSource(QUrl())`).
    pub fn unload(&mut self) {
        self.generation += 1;
        self.load_generation = self.generation;
        self.seek_target = None;
        self.loaded = false;
        self.at_end = false;
        self.duration_ms = 0;
        self.set_playing(false);
        self.set_state(State::Stopped);
        self.send(Cmd::Stop {
            generation: self.generation,
        });
    }

    /// Moves to `ms` (`setPosition`). The position reads as `ms` straight away.
    pub fn seek(&mut self, ms: u64) {
        if !self.loaded {
            return;
        }
        let ms = if self.duration_ms > 0 {
            ms.min(self.duration_ms)
        } else {
            ms
        };
        self.generation += 1;
        self.seek_target = Some((self.generation, ms));
        self.at_end = false;
        self.send(Cmd::Seek {
            ms,
            generation: self.generation,
        });
    }

    /// The position being heard, in milliseconds.
    pub fn position_ms(&mut self) -> u64 {
        if let Some((generation, ms)) = self.seek_target {
            if self.shared.seg_gen.load(Ordering::Acquire) < generation {
                return ms;
            }
            self.seek_target = None;
        }
        if self.at_end {
            return self.duration_ms;
        }
        if !self.loaded || self.shared.seg_gen.load(Ordering::Acquire) < self.load_generation {
            return 0;
        }
        let ms = self.shared.pos_us.load(Ordering::Relaxed) / 1000;
        if self.duration_ms > 0 {
            ms.min(self.duration_ms)
        } else {
            ms
        }
    }

    /// The song's length in milliseconds, or 0 before it has loaded.
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// Linear volume, 0.0 to 1.0, like `QAudioOutput.setVolume` (the UI passes `slider / 100`).
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        self.apply_gain();
    }

    #[allow(dead_code, reason = "used by the main window from M3 on")]
    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.apply_gain();
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Repeat of one track, without a gap (`setLoops(Infinite)`).
    pub fn set_looping(&mut self, looping: bool) {
        if looping != self.looping {
            self.looping = looping;
            self.send(Cmd::SetLooping(looping));
        }
    }

    /// Starts sending mono copies of what is played to the visualizer
    /// (`setAudioBufferOutput`). Replaces any earlier feed.
    pub fn attach_visualizer(&self) -> VizFeed {
        let (tap, feed) = output::viz_channel();
        if let Ok(mut slot) = self.shared.viz.lock() {
            *slot = Some(tap);
        }
        self.shared.viz_on.store(true, Ordering::Relaxed);
        feed
    }

    /// Stops the visualizer's feed (`setAudioBufferOutput(None)`).
    pub fn detach_visualizer(&self) {
        self.shared.viz_on.store(false, Ordering::Relaxed);
        if let Ok(mut slot) = self.shared.viz.lock() {
            *slot = None;
        }
    }

    /// Everything that happened since the last call, oldest first.
    pub fn poll(&mut self) -> Vec<Event> {
        while let Ok(event) = self.events.try_recv() {
            match event {
                EngineEvent::Loaded {
                    generation,
                    duration_ms,
                } if generation == self.load_generation => {
                    self.duration_ms = duration_ms;
                    self.queued.push(Event::Loaded { duration_ms });
                }
                EngineEvent::Error {
                    generation,
                    message,
                } if generation >= self.load_generation && self.loaded => {
                    self.loaded = false;
                    self.set_playing(false);
                    self.set_state(State::Stopped);
                    self.queued.push(Event::Error(message));
                }
                _ => {}
            }
        }
        let ended = self.shared.ended_gen.load(Ordering::Acquire);
        if self.loaded
            && !self.at_end
            && self.state == State::Playing
            && self.seek_target.is_none()
            && ended == self.generation
        {
            self.at_end = true;
            self.set_playing(false);
            self.set_state(State::Stopped);
            self.queued.push(Event::EndOfMedia);
        }
        std::mem::take(&mut self.queued)
    }

    fn set_state(&mut self, state: State) {
        if state != self.state {
            self.state = state;
            self.queued.push(Event::StateChanged(state));
        }
    }

    fn set_playing(&self, playing: bool) {
        self.shared.playing.store(playing, Ordering::Relaxed);
    }

    fn apply_gain(&self) {
        let gain = if self.muted { 0.0 } else { self.volume };
        self.shared.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.cmds.send(cmd);
    }
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.set_playing(false);
        let _ = self.cmds.send(Cmd::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    //! Needs a sound device, so it only runs by hand: `cargo test -p lrc-player -- --ignored`
    //! with `LRC_MEDIA_FIXTURES` set. On Linux without sound hardware, an ALSA `null` device
    //! as the default works.

    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for(player: &mut Player, what: impl Fn(&Event) -> bool, secs: u64) -> Vec<Event> {
        let mut seen = Vec::new();
        let until = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < until {
            let events = player.poll();
            let hit = events.iter().any(&what);
            seen.extend(events);
            if hit {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out; saw {seen:?}");
    }

    #[test]
    #[ignore]
    fn plays_seeks_and_ends() {
        let dir = std::path::PathBuf::from(std::env::var_os("LRC_MEDIA_FIXTURES").unwrap());
        let mut player = Player::new();
        player.load(&dir.join("full.m4a"));
        player.play();
        let events = wait_for(&mut player, |e| matches!(e, Event::Loaded { .. }), 5);
        assert!(
            events.contains(&Event::StateChanged(State::Playing)),
            "{events:?}"
        );
        assert_eq!(player.duration_ms(), 2500);

        // The clock runs at real time.
        std::thread::sleep(Duration::from_millis(500));
        let pos = player.position_ms();
        assert!((300..=700).contains(&pos), "position {pos} after 0.5 s");

        // A pause holds the clock.
        player.pause();
        std::thread::sleep(Duration::from_millis(100));
        let held = player.position_ms();
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            player.position_ms().abs_diff(held) <= 30,
            "clock ran while paused"
        );

        // A seek reads as its target at once, and plays on from there.
        player.seek(2000);
        assert_eq!(player.position_ms(), 2000);
        player.play();
        let events = wait_for(&mut player, |e| *e == Event::EndOfMedia, 5);
        assert!(
            events.contains(&Event::StateChanged(State::Stopped)),
            "{events:?}"
        );
        assert_eq!(player.position_ms(), 2500);

        // Play after the end starts over.
        player.play();
        std::thread::sleep(Duration::from_millis(200));
        assert!(player.position_ms() < 1000, "did not start over");

        // Repeat of one track never ends.
        player.set_looping(true);
        player.seek(2300);
        std::thread::sleep(Duration::from_millis(600));
        assert!(player.poll().iter().all(|e| *e != Event::EndOfMedia));
        let pos = player.position_ms();
        assert!(pos < 1000, "did not loop: {pos}");

        // A file that can't be played is an error, and the player stops.
        player.load(&dir.join("broken.mp3"));
        player.play();
        wait_for(&mut player, |e| matches!(e, Event::Error(_)), 5);
        assert_eq!(player.state(), State::Stopped);
    }
}
