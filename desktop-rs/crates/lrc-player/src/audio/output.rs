//! The output stream: the cpal callback that plays the decoded samples and keeps the clock.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, OutputCallbackInfo, SampleFormat, SizedSample, StreamConfig};

use super::engine::Cmd;

/// State shared by the UI handle, the decode thread and the output callback.
#[derive(Default)]
pub struct Shared {
    /// Set by the UI: whether the callback plays or sends silence.
    pub playing: AtomicBool,
    /// Set by the UI: the gain as `f32` bits (0 while muted).
    pub gain: AtomicU32,
    /// Written by the callback: the position being heard, in microseconds.
    pub pos_us: AtomicU64,
    /// Written by the callback: the generation of the segment it is playing.
    pub seg_gen: AtomicU64,
    /// Written by the callback: a generation whose audio ran out with nothing after it.
    pub ended_gen: AtomicU64,
}

impl Shared {
    pub fn new() -> Self {
        let s = Self::default();
        s.gain.store(1.0f32.to_bits(), Ordering::Relaxed);
        s
    }
}

/// A run of audio that starts at a known song position: everything after a load or a seek.
pub struct Segment {
    /// Bumped by every load, seek and stop, so stale answers can be told apart.
    pub generation: u64,
    /// The song position of the first sample, in microseconds.
    pub base_us: u64,
    /// The samples, at the device's rate and channel count. `None` stops playback.
    pub samples: Option<rtrb::Consumer<f32>>,
    /// Set by the decode thread once every sample of this segment is in the buffer.
    pub done: Arc<AtomicBool>,
    /// Replace the current segment at once (seek, load). Otherwise it follows the current
    /// one without a gap (repeat of one track).
    pub immediate: bool,
}

/// An open output stream on the default device.
pub struct Output {
    _stream: cpal::Stream,
    pub segments: rtrb::Producer<Segment>,
    pub rate: u32,
    pub channels: usize,
    /// The device the stream plays on, to notice when the default changes.
    pub device_id: Option<String>,
}

/// The id of the system's current default output, if there is one.
pub fn default_device_id() -> Option<String> {
    let device = cpal::default_host().default_output_device()?;
    device.id().ok().map(|id| id.to_string())
}

impl Output {
    /// Opens the default output device. Stream errors are sent to the decode thread.
    pub fn open(shared: Arc<Shared>, cmds: Sender<Cmd>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("No audio output device was found.")?;
        let device_id = device.id().ok().map(|id| id.to_string());
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let rate = config.sample_rate;
        let channels = usize::from(config.channels).max(1);
        let (segments, segment_rx) = rtrb::RingBuffer::new(8);
        let callback = Callback {
            segments: segment_rx,
            current: None,
            next: None,
            played: 0,
            last_gen: 0,
            rate,
            channels,
            shared,
        };
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, config, callback, cmds),
            SampleFormat::F64 => build::<f64>(&device, config, callback, cmds),
            SampleFormat::I16 => build::<i16>(&device, config, callback, cmds),
            SampleFormat::I32 => build::<i32>(&device, config, callback, cmds),
            SampleFormat::U16 => build::<u16>(&device, config, callback, cmds),
            SampleFormat::U8 => build::<u8>(&device, config, callback, cmds),
            SampleFormat::I8 => build::<i8>(&device, config, callback, cmds),
            other => return Err(format!("The output format {other} is not supported.")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: stream,
            segments,
            rate,
            channels,
            device_id,
        })
    }
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut callback: Callback,
    cmds: Sender<Cmd>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], info: &OutputCallbackInfo| {
                scratch.resize(data.len(), 0.0);
                let ts = info.timestamp();
                let latency = ts.playback.saturating_duration_since(ts.callback);
                callback.fill(&mut scratch, latency.as_micros() as u64);
                for (out, &s) in data.iter_mut().zip(&scratch) {
                    *out = T::from_sample(s);
                }
            },
            move |err: cpal::Error| {
                use cpal::ErrorKind::*;
                // These keep playing (or were rerouted already); everything else needs a new stream.
                if !matches!(err.kind(), Xrun | DeviceChanged | RealtimeDenied) {
                    let _ = cmds.send(Cmd::StreamFailed);
                }
            },
            None,
        )
        .map_err(|e| e.to_string())
}

struct Callback {
    segments: rtrb::Consumer<Segment>,
    current: Option<Segment>,
    next: Option<Segment>,
    /// Frames of `current` handed to the device so far.
    played: u64,
    /// The generation of the last segment that replaced the current one.
    last_gen: u64,
    rate: u32,
    channels: usize,
    shared: Arc<Shared>,
}

impl Callback {
    /// Fills `data` (interleaved) and updates the clock. `latency_us` is how long until
    /// what is written now reaches the speakers.
    fn fill(&mut self, data: &mut [f32], latency_us: u64) {
        while let Ok(seg) = self.segments.pop() {
            if seg.immediate || self.current.is_none() {
                self.last_gen = seg.generation;
                self.shared.pos_us.store(seg.base_us, Ordering::Relaxed);
                self.current = seg.samples.is_some().then_some(seg);
                self.next = None;
                self.played = 0;
            } else {
                self.next = Some(seg);
            }
        }

        let mut written = 0;
        if self.shared.playing.load(Ordering::Relaxed) {
            let gain = f32::from_bits(self.shared.gain.load(Ordering::Relaxed));
            while written < data.len() {
                let Some(seg) = self.current.as_mut() else {
                    break;
                };
                let Some(samples) = seg.samples.as_mut() else {
                    break;
                };
                let want = (data.len() - written).min(samples.slots());
                // Only whole frames, so the channels never slip.
                let want = want - want % self.channels;
                if want > 0 {
                    if let Ok(chunk) = samples.read_chunk(want) {
                        let (a, b) = chunk.as_slices();
                        for (o, &s) in data[written..].iter_mut().zip(a.iter().chain(b)) {
                            *o = s * gain;
                        }
                        chunk.commit_all();
                        written += want;
                        self.played += (want / self.channels) as u64;
                    }
                    continue;
                }
                // Nothing buffered: either the decoder is behind (play silence) or the
                // segment is over.
                if seg.done.load(Ordering::Acquire) && samples.is_empty() {
                    let finished = self.current.take().map_or(0, |s| s.generation);
                    self.played = 0;
                    match self.next.take() {
                        Some(next) => self.current = Some(next),
                        None => self.shared.ended_gen.store(finished, Ordering::Release),
                    }
                    continue;
                }
                break;
            }
        }
        data[written..].fill(0.0);

        if let Some(seg) = &self.current {
            let played_us = self.played * 1_000_000 / u64::from(self.rate.max(1));
            let pos = seg.base_us + played_us.saturating_sub(latency_us);
            self.shared.pos_us.store(pos, Ordering::Relaxed);
        }
        let generation = self
            .current
            .as_ref()
            .map_or(self.last_gen, |s| s.generation);
        self.shared.seg_gen.store(generation, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 1000; // one frame per millisecond keeps the numbers readable

    fn callback() -> (Callback, rtrb::Producer<Segment>, Arc<Shared>) {
        let shared = Arc::new(Shared::new());
        let (tx, rx) = rtrb::RingBuffer::new(8);
        let cb = Callback {
            segments: rx,
            current: None,
            next: None,
            played: 0,
            last_gen: 0,
            rate: RATE,
            channels: 2,
            shared: shared.clone(),
        };
        (cb, tx, shared)
    }

    /// A segment holding `frames` stereo frames of `value`, already complete.
    fn segment(
        generation: u64,
        base_ms: u64,
        frames: usize,
        value: f32,
        immediate: bool,
    ) -> Segment {
        let (mut p, c) = rtrb::RingBuffer::new(frames * 2 + 2);
        for _ in 0..frames * 2 {
            p.push(value).unwrap();
        }
        Segment {
            generation,
            base_us: base_ms * 1000,
            samples: Some(c),
            done: Arc::new(AtomicBool::new(true)),
            immediate,
        }
    }

    fn pos_ms(shared: &Shared) -> u64 {
        shared.pos_us.load(Ordering::Relaxed) / 1000
    }

    #[test]
    fn clock_counts_played_frames_minus_latency() {
        let (mut cb, mut tx, shared) = callback();
        tx.push(segment(1, 5000, 100, 0.5, true)).ok().unwrap();
        shared.playing.store(true, Ordering::Relaxed);
        let mut buf = vec![0.0; 40]; // 20 frames
        cb.fill(&mut buf, 0);
        assert_eq!(pos_ms(&shared), 5020);
        assert!(buf.iter().all(|&s| s == 0.5));
        cb.fill(&mut buf, 15_000);
        assert_eq!(
            pos_ms(&shared),
            5025,
            "40 frames played, 15 still on the way"
        );
        assert_eq!(shared.seg_gen.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn pause_sends_silence_and_holds_the_clock() {
        let (mut cb, mut tx, shared) = callback();
        tx.push(segment(1, 0, 100, 0.5, true)).ok().unwrap();
        let mut buf = vec![1.0; 40];
        cb.fill(&mut buf, 0);
        assert!(buf.iter().all(|&s| s == 0.0));
        assert_eq!(pos_ms(&shared), 0);
        shared.playing.store(true, Ordering::Relaxed);
        cb.fill(&mut buf, 0);
        shared.playing.store(false, Ordering::Relaxed);
        cb.fill(&mut buf, 0);
        assert_eq!(pos_ms(&shared), 20);
    }

    #[test]
    fn volume_and_mute_scale_the_samples() {
        let (mut cb, mut tx, shared) = callback();
        tx.push(segment(1, 0, 100, 0.5, true)).ok().unwrap();
        shared.playing.store(true, Ordering::Relaxed);
        shared.gain.store(0.8f32.to_bits(), Ordering::Relaxed);
        let mut buf = vec![0.0; 4];
        cb.fill(&mut buf, 0);
        assert!(buf.iter().all(|&s| (s - 0.4).abs() < 1e-6));
        shared.gain.store(0.0f32.to_bits(), Ordering::Relaxed);
        cb.fill(&mut buf, 0);
        assert!(buf.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn a_seek_replaces_what_was_buffered_at_once() {
        let (mut cb, mut tx, shared) = callback();
        shared.playing.store(true, Ordering::Relaxed);
        tx.push(segment(1, 0, 100, 0.5, true)).ok().unwrap();
        let mut buf = vec![0.0; 20];
        cb.fill(&mut buf, 0);
        tx.push(segment(2, 60_000, 100, -0.5, true)).ok().unwrap();
        cb.fill(&mut buf, 0);
        assert!(
            buf.iter().all(|&s| s == -0.5),
            "old samples played after the seek"
        );
        assert_eq!(pos_ms(&shared), 60_010);
        assert_eq!(shared.seg_gen.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn the_end_is_reported_once_the_buffer_runs_dry() {
        let (mut cb, mut tx, shared) = callback();
        shared.playing.store(true, Ordering::Relaxed);
        tx.push(segment(3, 0, 10, 0.5, true)).ok().unwrap();
        let mut buf = vec![9.0; 40];
        cb.fill(&mut buf, 0);
        assert!(buf[..20].iter().all(|&s| s == 0.5) && buf[20..].iter().all(|&s| s == 0.0));
        assert_eq!(shared.ended_gen.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn an_unfinished_segment_waits_instead_of_ending() {
        let (mut cb, mut tx, shared) = callback();
        shared.playing.store(true, Ordering::Relaxed);
        let seg = segment(3, 0, 10, 0.5, true);
        seg.done.store(false, Ordering::Relaxed);
        tx.push(seg).ok().unwrap();
        let mut buf = vec![0.0; 40];
        cb.fill(&mut buf, 0);
        assert_eq!(shared.ended_gen.load(Ordering::Relaxed), 0);
        assert_eq!(pos_ms(&shared), 10);
    }

    #[test]
    fn repeat_follows_on_without_a_gap() {
        let (mut cb, mut tx, shared) = callback();
        shared.playing.store(true, Ordering::Relaxed);
        tx.push(segment(4, 90, 10, 0.5, true)).ok().unwrap();
        tx.push(segment(4, 0, 100, -0.5, false)).ok().unwrap();
        let mut buf = vec![0.0; 40]; // 20 frames: the last 10 of the song, then the first 10
        cb.fill(&mut buf, 0);
        assert!(buf[..20].iter().all(|&s| s == 0.5));
        assert!(buf[20..].iter().all(|&s| s == -0.5));
        assert_eq!(pos_ms(&shared), 10);
        assert_eq!(shared.ended_gen.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn stop_goes_silent() {
        let (mut cb, mut tx, shared) = callback();
        shared.playing.store(true, Ordering::Relaxed);
        tx.push(segment(1, 0, 100, 0.5, true)).ok().unwrap();
        let mut buf = vec![0.0; 20];
        cb.fill(&mut buf, 0);
        tx.push(Segment {
            generation: 2,
            base_us: 0,
            samples: None,
            done: Arc::new(AtomicBool::new(true)),
            immediate: true,
        })
        .ok()
        .unwrap();
        cb.fill(&mut buf, 0);
        assert!(buf.iter().all(|&s| s == 0.0));
        assert_eq!(shared.seg_gen.load(Ordering::Relaxed), 2);
        assert_eq!(pos_ms(&shared), 0);
    }
}
