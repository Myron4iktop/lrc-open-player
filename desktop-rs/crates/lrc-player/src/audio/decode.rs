//! Opening a song and decoding it into interleaved `f32` samples, with accurate seeking.

use std::fs::File;
use std::path::Path;
use std::sync::OnceLock;

use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{TimeBase, Timestamp};

use super::mp4_edit;

/// Symphonia's own decoders plus libopus, which Symphonia lacks.
fn codecs() -> &'static CodecRegistry {
    static CODECS: OnceLock<CodecRegistry> = OnceLock::new();
    CODECS.get_or_init(|| {
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
        registry
    })
}

/// A block of decoded audio: interleaved samples at `rate` with `channels` channels.
pub struct Chunk<'a> {
    pub samples: &'a [f32],
    pub rate: u32,
    pub channels: usize,
}

/// One open song.
///
/// Positions are song time: 0 is where the music starts, after any encoder priming that the
/// file marks (the M4A edit list; MP3 and Opus are trimmed by Symphonia itself).
pub struct Source {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: TimeBase,
    /// Song time 0, in track ticks.
    zero_ts: i64,
    /// The end of the song in track ticks, when the file marks padding after it.
    end_ts: Option<i64>,
    duration_ms: u64,
    /// Frames before this timestamp are dropped: after a seek, so playback starts exactly
    /// where it was asked to, not at the packet or key frame before it.
    skip_until: Option<i64>,
    /// Opus pre-skip that the decoder will drop from the next packet it decodes. It does
    /// that once in its life, seek or not.
    pre_skip_pending: i64,
    /// Set by a seek past the end.
    exhausted: bool,
    buf: Vec<f32>,
}

impl Source {
    /// Opens a song. The error is a short reason, for the error event.
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if let Some(ext) = &ext {
            hint.with_extension(ext);
        }
        let reader = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(describe)?;
        let track = reader
            .default_track(TrackType::Audio)
            .ok_or("The file has no audio track.")?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or("The audio format is not supported.")?;
        let decoder = codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(describe)?;
        let time_base = track
            .time_base
            .or_else(|| params.sample_rate.and_then(TimeBase::try_from_recip))
            .ok_or("The audio format is not supported.")?;
        let track_id = track.id;
        let total = track.duration.map(|d| d.get() as i64);

        let edit = match ext.as_deref() {
            Some("m4a" | "mp4" | "m4b" | "aac") => mp4_edit::read(path),
            _ => None,
        };
        // Opus: the Ogg reader leaves the pre-skip in the timeline, and the libopus adapter
        // drops it from the first packet it decodes.
        let pre_skip = opus_pre_skip(params);
        let zero_ts = edit.map_or(0, |e| e.start as i64) + pre_skip;
        let end_ts = edit.and_then(|e| e.length).map(|n| zero_ts + n as i64);
        let length = match (end_ts, total) {
            (Some(end), _) => Some(end - zero_ts),
            (None, Some(total)) => Some(total - zero_ts),
            _ => None,
        };
        let duration_ms = length.map_or(0, |ticks| ticks_to_ms(time_base, ticks));

        Ok(Self {
            reader,
            decoder,
            track_id,
            time_base,
            zero_ts,
            end_ts,
            duration_ms,
            skip_until: (zero_ts - pre_skip > 0).then_some(zero_ts),
            pre_skip_pending: pre_skip,
            exhausted: false,
            buf: Vec::new(),
        })
    }

    /// The length in milliseconds, or 0 when the file doesn't say.
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    /// Moves to `ms`. The next chunk starts exactly there, or is the end if `ms` is past it.
    pub fn seek(&mut self, ms: u64) -> Result<(), String> {
        let ts = self.zero_ts + ms_to_ticks(self.time_base, ms);
        self.exhausted = false;
        if self.end_ts.is_some_and(|end| ts >= end) {
            self.exhausted = true;
            return Ok(());
        }
        let to = SeekTo::Timestamp {
            ts: Timestamp::new(ts),
            track_id: self.track_id,
        };
        match self.reader.seek(SeekMode::Accurate, to) {
            Ok(_) => {
                self.skip_until = Some(ts);
                self.decoder.reset();
                Ok(())
            }
            // Past the end: the next chunk is the end of the song.
            Err(Error::SeekError(_)) | Err(Error::IoError(_)) if ms > 0 => {
                self.exhausted = true;
                Ok(())
            }
            Err(e) => Err(describe(e)),
        }
    }

    /// Decodes the next block of audio. `Ok(None)` is the end of the song.
    ///
    /// Damaged packets are skipped, as FFmpeg (behind Qt in the Python app) does.
    pub fn next_chunk(&mut self) -> Result<Option<Chunk<'_>>, String> {
        if self.exhausted {
            return Ok(None);
        }
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(None),
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(Error::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(describe(e)),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            // The decoder drops the packet's `trim_start` frames, so what it returns begins here.
            let start = packet.pts.get()
                + packet.trim_start.get() as i64
                + std::mem::take(&mut self.pre_skip_pending);
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(Error::DecodeError(_)) | Err(Error::IoError(_)) => continue,
                Err(e) => return Err(describe(e)),
            };
            let rate = decoded.spec().rate();
            let channels = decoded.spec().channels().count().max(1);
            if decoded.frames() == 0 {
                continue;
            }
            decoded.copy_to_vec_interleaved(&mut self.buf);
            let frames = self.buf.len() / channels;

            let mut first = 0;
            if let Some(until) = self.skip_until {
                first = self.ticks_to_frames(until - start, rate).min(frames);
            }
            let mut last = frames;
            if let Some(end) = self.end_ts {
                if start >= end {
                    return Ok(None);
                }
                last = last.min(self.ticks_to_frames(end - start, rate));
            }
            if first >= last {
                continue;
            }
            self.skip_until = None;
            return Ok(Some(Chunk {
                samples: &self.buf[first * channels..last * channels],
                rate,
                channels,
            }));
        }
    }

    /// Track ticks to frames at `rate` (ticks are usually frames already).
    fn ticks_to_frames(&self, ticks: i64, rate: u32) -> usize {
        let ticks = ticks.max(0) as u128;
        let tb = self.time_base;
        let frames =
            ticks * u128::from(tb.numer.get()) * u128::from(rate) / u128::from(tb.denom.get());
        usize::try_from(frames).unwrap_or(usize::MAX)
    }
}

/// The pre-skip of an Opus stream, in 48 kHz frames, from its `OpusHead`.
fn opus_pre_skip(params: &AudioCodecParameters) -> i64 {
    if params.codec != CODEC_ID_OPUS {
        return 0;
    }
    match params.extra_data.as_deref() {
        Some(head) if head.len() >= 12 && head.starts_with(b"OpusHead") => {
            i64::from(u16::from_le_bytes([head[10], head[11]]))
        }
        _ => 0,
    }
}

fn ticks_to_ms(tb: TimeBase, ticks: i64) -> u64 {
    let ticks = ticks.max(0) as u128;
    (ticks * 1000 * u128::from(tb.numer.get()) / u128::from(tb.denom.get())) as u64
}

fn ms_to_ticks(tb: TimeBase, ms: u64) -> i64 {
    (u128::from(ms) * u128::from(tb.denom.get()) / (1000 * u128::from(tb.numer.get()))) as i64
}

fn describe(e: Error) -> String {
    match e {
        Error::Unsupported(_) => "The audio format is not supported.".to_owned(),
        Error::IoError(e) => e.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    //! Run on the songs from `make_fixtures.py media` (see `lrc-core/tests/media.rs`).

    use super::*;
    use std::path::PathBuf;

    fn media(name: &str) -> Option<PathBuf> {
        let Some(dir) = std::env::var_os("LRC_MEDIA_FIXTURES") else {
            assert!(
                std::env::var_os("LRC_REQUIRE_MEDIA_FIXTURES").is_none(),
                "LRC_MEDIA_FIXTURES is not set"
            );
            eprintln!("skipped: set LRC_MEDIA_FIXTURES to a folder from make_fixtures.py media");
            return None;
        };
        Some(PathBuf::from(dir).join(name))
    }

    /// Every playable test song and its real length (`make_fixtures.py` writes sine tones).
    const SONGS: [(&str, u64); 9] = [
        ("full.flac", 2500),
        ("full.mp3", 2500),
        ("v23.mp3", 2500),
        ("full.m4a", 2500),
        ("zero.m4a", 2500),
        ("full.ogg", 2500),
        ("full.opus", 2500),
        ("full.wav", 1500),
        ("plain24.wav", 1250),
    ];

    /// Decodes the rest of the song: (frames, rate, the samples).
    fn decode_rest(source: &mut Source) -> (u64, u32, Vec<f32>) {
        let (mut frames, mut rate, mut all) = (0u64, 0u32, Vec::new());
        while let Some(chunk) = source.next_chunk().expect("decodes") {
            frames += (chunk.samples.len() / chunk.channels) as u64;
            rate = chunk.rate;
            all.extend_from_slice(chunk.samples);
        }
        (frames, rate, all)
    }

    #[test]
    fn every_format_decodes_to_its_length() {
        for (name, length_ms) in SONGS {
            let Some(path) = media(name) else { return };
            let mut source = Source::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            let duration = source.duration_ms();
            assert!(
                duration.abs_diff(length_ms) <= 40,
                "{name}: duration {duration} ms"
            );
            let (frames, rate, _) = decode_rest(&mut source);
            let decoded_ms = frames * 1000 / u64::from(rate);
            // To the millisecond: encoder priming and padding must not shift the clock.
            assert!(
                decoded_ms.abs_diff(length_ms) <= 1,
                "{name}: decoded {decoded_ms} ms"
            );
        }
    }

    #[test]
    fn seeking_lands_on_the_asked_position() {
        for (name, length_ms) in SONGS {
            let Some(path) = media(name) else { return };
            let mut whole = Source::open(&path).unwrap();
            let (total, rate, reference) = decode_rest(&mut whole);
            for target in [0, 700, 1000, length_ms - 300] {
                let mut source = Source::open(&path).unwrap();
                source
                    .seek(target)
                    .unwrap_or_else(|e| panic!("{name} @ {target}: {e}"));
                let (frames, _, samples) = decode_rest(&mut source);
                // What is left must be the song from `target` on, to the frame or very close.
                let expected = total - target * u64::from(rate) / 1000;
                let off_ms = frames.abs_diff(expected) * 1000 / u64::from(rate);
                assert!(off_ms <= 5, "{name} @ {target}: {off_ms} ms off");
                if name.ends_with(".flac") || name.ends_with(".wav") {
                    // Lossless: the very same samples.
                    let start = reference.len() - samples.len();
                    assert_eq!(
                        samples[..256],
                        reference[start..start + 256],
                        "{name} @ {target}"
                    );
                }
            }
            // Past the end: nothing left, and no error.
            let mut source = Source::open(&path).unwrap();
            source.seek(length_ms + 5000).unwrap();
            assert_eq!(decode_rest(&mut source).0, 0, "{name} past the end");
        }
    }

    #[test]
    fn a_broken_file_is_an_error_not_a_crash() {
        let Some(path) = media("broken.mp3") else {
            return;
        };
        let result = Source::open(&path).and_then(|mut s| {
            while s.next_chunk()?.is_some() {}
            Ok(())
        });
        assert!(result.is_err(), "broken.mp3 played");
    }

    #[test]
    fn resampled_to_48k_keeps_the_length() {
        use super::super::convert::Converter;
        for (name, length_ms) in SONGS {
            let Some(path) = media(name) else { return };
            let mut source = Source::open(&path).unwrap();
            let mut conv = Converter::new(48_000, 2);
            let mut out = Vec::new();
            while let Some(chunk) = source.next_chunk().unwrap() {
                conv.push(&chunk, &mut out);
            }
            conv.flush(&mut out);
            let ms = (out.len() / 2) as u64 * 1000 / 48_000;
            assert!(
                ms.abs_diff(length_ms) <= 40,
                "{name}: {ms} ms after resampling"
            );
        }
    }
}
