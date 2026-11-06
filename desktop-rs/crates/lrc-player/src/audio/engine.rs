//! The decode thread: opens songs, decodes and resamples them into the output's buffer, and
//! keeps the output on the system's default device.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use super::convert::Converter;
use super::decode::Source;
use super::output::{Output, Segment, Shared, default_device_id};

/// Seconds of audio the buffer between decoder and output holds.
const BUFFER_SECONDS: usize = 1;
/// How often the default output device is checked.
const DEVICE_POLL: Duration = Duration::from_secs(1);
/// The second look after a change: some systems announce a new device before they make it
/// the default (`_on_audio_outputs_changed` in the Python app).
const DEVICE_RECHECK: Duration = Duration::from_millis(400);
/// How long to wait before trying again when no output could be opened.
const OUTPUT_RETRY: Duration = Duration::from_secs(1);

pub enum Cmd {
    Load {
        path: PathBuf,
        generation: u64,
    },
    Seek {
        ms: u64,
        generation: u64,
    },
    Stop {
        generation: u64,
    },
    SetLooping(bool),
    /// From the output callback: the stream broke and needs rebuilding.
    StreamFailed,
    Shutdown,
}

/// What the decode thread tells the UI handle.
pub enum EngineEvent {
    Loaded { generation: u64, duration_ms: u64 },
    Error { generation: u64, message: String },
}

pub struct Engine {
    cmds: Receiver<Cmd>,
    cmd_tx: Sender<Cmd>,
    events: Sender<EngineEvent>,
    shared: Arc<Shared>,
    output: Option<Output>,
    output_failed_at: Option<Instant>,
    next_device_poll: Instant,
    device_recheck_at: Option<Instant>,

    source: Option<Source>,
    generation: u64,
    /// Where the newest segment starts, for a rebuild before the output has picked it up.
    segment_ms: u64,
    looping: bool,
    converter: Converter,
    /// Converted samples that didn't fit in the buffer yet.
    pending: Vec<f32>,
    pending_pos: usize,
    producer: Option<rtrb::Producer<f32>>,
    done: Arc<AtomicBool>,
    /// The decoder reached the end of the song (the buffer may still be playing).
    decoded_all: bool,
}

impl Engine {
    pub fn new(
        cmds: Receiver<Cmd>,
        cmd_tx: Sender<Cmd>,
        events: Sender<EngineEvent>,
        shared: Arc<Shared>,
    ) -> Self {
        Self {
            cmds,
            cmd_tx,
            events,
            shared,
            output: None,
            output_failed_at: None,
            next_device_poll: Instant::now() + DEVICE_POLL,
            device_recheck_at: None,
            source: None,
            generation: 0,
            segment_ms: 0,
            looping: false,
            converter: Converter::new(48_000, 2),
            pending: Vec::new(),
            pending_pos: 0,
            producer: None,
            done: Arc::new(AtomicBool::new(false)),
            decoded_all: false,
        }
    }

    pub fn run(mut self) {
        self.open_output();
        loop {
            let wait = if self.has_work() {
                Duration::ZERO
            } else {
                Duration::from_millis(10)
            };
            match self.cmds.recv_timeout(wait) {
                Ok(Cmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(cmd) => {
                    self.handle(cmd);
                    // Act on the newest of a burst (a seek bar being dragged) right away.
                    while let Ok(cmd) = self.cmds.try_recv() {
                        if matches!(cmd, Cmd::Shutdown) {
                            return;
                        }
                        self.handle(cmd);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.watch_device();
            self.pump();
        }
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Load { path, generation } => {
                self.generation = generation;
                self.source = None;
                match Source::open(&path) {
                    Ok(source) => {
                        let duration_ms = source.duration_ms();
                        self.source = Some(source);
                        let _ = self.events.send(EngineEvent::Loaded {
                            generation,
                            duration_ms,
                        });
                        self.start_segment(0, true);
                    }
                    Err(message) => {
                        let _ = self.events.send(EngineEvent::Error {
                            generation,
                            message,
                        });
                        self.stop_output();
                    }
                }
            }
            Cmd::Seek { ms, generation } => {
                self.generation = generation;
                self.seek_and_restart(ms);
            }
            Cmd::Stop { generation } => {
                self.generation = generation;
                self.source = None;
                self.stop_output();
            }
            Cmd::SetLooping(on) => {
                self.looping = on;
                // The song may have been decoded to the end already, with its tail still
                // playing: queue the repeat now.
                if on && self.decoded_all && self.pending_left() == 0 {
                    self.queue_loop();
                }
            }
            Cmd::StreamFailed => self.rebuild_output(),
            Cmd::Shutdown => {}
        }
    }

    /// Seeks the song to `ms` and starts a new segment there. A failed seek is an error, like
    /// a song that can't be read.
    fn seek_and_restart(&mut self, ms: u64) {
        let Some(source) = self.source.as_mut() else {
            self.stop_output();
            return;
        };
        if let Err(message) = source.seek(ms) {
            let _ = self.events.send(EngineEvent::Error {
                generation: self.generation,
                message,
            });
            self.source = None;
            self.stop_output();
            return;
        }
        self.start_segment(ms, true);
    }

    /// Starts a new buffer at song position `ms` and hands it to the output.
    fn start_segment(&mut self, ms: u64, immediate: bool) {
        self.converter.reset();
        self.pending.clear();
        self.pending_pos = 0;
        self.decoded_all = false;
        self.segment_ms = ms;
        let Some(output) = self.output.as_mut() else {
            self.producer = None;
            return;
        };
        let capacity = output.rate as usize * output.channels * BUFFER_SECONDS;
        let (producer, consumer) = rtrb::RingBuffer::new(capacity);
        self.done = Arc::new(AtomicBool::new(false));
        let segment = Segment {
            generation: self.generation,
            base_us: ms * 1000,
            samples: Some(consumer),
            done: self.done.clone(),
            immediate,
        };
        if output.segments.push(segment).is_ok() {
            self.producer = Some(producer);
        } else {
            // The callback isn't running (the device is gone): rebuild instead.
            self.producer = None;
            self.rebuild_output();
        }
    }

    fn stop_output(&mut self) {
        self.producer = None;
        self.pending.clear();
        self.pending_pos = 0;
        self.decoded_all = false;
        if let Some(output) = self.output.as_mut() {
            let _ = output.segments.push(Segment {
                generation: self.generation,
                base_us: 0,
                samples: None,
                done: Arc::new(AtomicBool::new(true)),
                immediate: true,
            });
        }
    }

    /// Repeat of one track: the song starts again right after the current buffer runs out.
    fn queue_loop(&mut self) {
        let Some(source) = self.source.as_mut() else {
            return;
        };
        if source.seek(0).is_ok() {
            self.start_segment(0, false);
        }
    }

    fn pending_left(&self) -> usize {
        self.pending.len() - self.pending_pos
    }

    fn has_work(&self) -> bool {
        let Some(p) = &self.producer else {
            return false;
        };
        if self.decoded_all && self.pending_left() == 0 {
            return !self.done.load(Ordering::Relaxed);
        }
        p.slots() >= 4096.min(p.buffer().capacity())
    }

    /// Decodes until the buffer is full or the song is over.
    fn pump(&mut self) {
        for _ in 0..64 {
            let left = self.pending_left();
            let Some(producer) = self.producer.as_mut() else {
                return;
            };
            if left > 0 {
                let n = producer.slots().min(left);
                if n == 0 {
                    return;
                }
                if let Ok(chunk) = producer.write_chunk_uninit(n) {
                    let from = self.pending_pos;
                    chunk.fill_from_iter(self.pending[from..from + n].iter().copied());
                }
                self.pending_pos += n;
                if self.pending_left() > 0 {
                    return;
                }
                self.pending.clear();
                self.pending_pos = 0;
                continue;
            }
            if self.decoded_all {
                if !self.done.swap(true, Ordering::Release) && self.looping {
                    self.queue_loop();
                }
                return;
            }
            let Some(source) = self.source.as_mut() else {
                return;
            };
            match source.next_chunk() {
                Ok(Some(chunk)) => self.converter.push(&chunk, &mut self.pending),
                Ok(None) => {
                    self.converter.flush(&mut self.pending);
                    self.decoded_all = true;
                }
                Err(message) => {
                    let _ = self.events.send(EngineEvent::Error {
                        generation: self.generation,
                        message,
                    });
                    self.converter.flush(&mut self.pending);
                    self.decoded_all = true;
                    self.source = None;
                }
            }
        }
    }

    fn open_output(&mut self) {
        match Output::open(self.shared.clone(), self.cmd_tx.clone()) {
            Ok(output) => {
                self.converter = Converter::new(output.rate, output.channels);
                self.output = Some(output);
                self.output_failed_at = None;
            }
            Err(_) => {
                self.output = None;
                self.output_failed_at = Some(Instant::now());
            }
        }
    }

    /// Opens a new stream on the current default device, and carries on from the position
    /// that was being heard.
    fn rebuild_output(&mut self) {
        let heard_ms = if self.shared.seg_gen.load(Ordering::Acquire) == self.generation {
            self.shared.pos_us.load(Ordering::Relaxed) / 1000
        } else {
            self.segment_ms
        };
        let ended = self.shared.ended_gen.load(Ordering::Acquire) == self.generation;
        self.output = None; // closes the old stream first
        self.producer = None;
        self.open_output();
        if self.output.is_none() {
            return;
        }
        if self.source.is_some() && !ended {
            self.seek_and_restart(heard_ms);
        }
    }

    /// Follows the default device: plugging headphones in or out moves the sound there.
    fn watch_device(&mut self) {
        let now = Instant::now();
        if self.output.is_none() {
            if self
                .output_failed_at
                .is_none_or(|t| now.duration_since(t) >= OUTPUT_RETRY)
            {
                self.rebuild_output();
            }
            return;
        }
        let recheck = self.device_recheck_at.is_some_and(|t| now >= t);
        if now < self.next_device_poll && !recheck {
            return;
        }
        self.next_device_poll = now + DEVICE_POLL;
        if recheck {
            self.device_recheck_at = None;
        }
        let current = default_device_id();
        let playing_on = self.output.as_ref().and_then(|o| o.device_id.clone());
        if current.is_some() && current != playing_on {
            self.rebuild_output();
            if !recheck {
                self.device_recheck_at = Some(now + DEVICE_RECHECK);
            }
        }
    }
}
