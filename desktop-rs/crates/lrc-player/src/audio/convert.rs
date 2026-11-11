//! Turning decoded audio into the output device's layout: its channel count and its rate.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};

use super::decode::Chunk;

/// Frames per resampler block.
const BLOCK: usize = 1024;

/// Converts chunks of any rate and channel count to `out_rate` and `out_channels`.
///
/// The output is exactly as long as the input in time (the resampler's delay is removed),
/// so frame counts on the output side map straight back to song positions.
pub struct Converter {
    out_rate: u32,
    out_channels: usize,
    in_rate: u32,
    resampler: Option<Fft<f32>>,
    /// Remapped to `out_channels`, still at `in_rate`, waiting for a full block.
    pending: Vec<f32>,
    block_out: Vec<f32>,
    /// Resampler delay still to drop from the start of the output.
    drop_frames: usize,
    in_frames: u64,
    out_frames: u64,
}

impl Converter {
    pub fn new(out_rate: u32, out_channels: usize) -> Self {
        Self {
            out_rate,
            out_channels: out_channels.max(1),
            in_rate: 0,
            resampler: None,
            pending: Vec::new(),
            block_out: Vec::new(),
            drop_frames: 0,
            in_frames: 0,
            out_frames: 0,
        }
    }

    /// Forgets buffered audio, for a seek or a new song.
    pub fn reset(&mut self) {
        self.in_rate = 0;
        self.resampler = None;
        self.pending.clear();
        self.drop_frames = 0;
        self.in_frames = 0;
        self.out_frames = 0;
    }

    /// Converts `chunk` and appends the result to `out`.
    pub fn push(&mut self, chunk: &Chunk<'_>, out: &mut Vec<f32>) {
        if chunk.rate != self.in_rate {
            // A new song, or a stream that changes rate midway (rare, but legal in MP3 and OGG).
            self.flush(out);
            self.start(chunk.rate);
        }
        let frames = chunk.samples.len() / chunk.channels;
        self.in_frames += frames as u64;
        if self.resampler.is_none() {
            remap(chunk.samples, chunk.channels, self.out_channels, out);
            self.out_frames += frames as u64;
            return;
        }
        remap(
            chunk.samples,
            chunk.channels,
            self.out_channels,
            &mut self.pending,
        );
        self.run_blocks(out);
    }

    /// Pushes out what is still buffered, at the end of a song.
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        if self.resampler.is_none() {
            self.pending.clear();
            return;
        }
        let ch = self.out_channels;
        let expected = self.in_frames * u64::from(self.out_rate) / u64::from(self.in_rate.max(1));
        // The rest of the input, then silence until the delayed tail is out.
        let mut rounds = 0;
        while self.out_frames < expected && rounds < 64 {
            rounds += 1;
            let have = (self.pending.len() / ch).min(BLOCK);
            self.pending.resize(self.pending.len().max(BLOCK * ch), 0.0);
            self.process_block(0, (have < BLOCK).then_some(have), out);
            self.pending.drain(..BLOCK * ch);
        }
        // Drop anything past the input's length.
        let extra = self.out_frames.saturating_sub(expected) as usize;
        out.truncate(out.len().saturating_sub(extra * ch));
        self.out_frames = self.out_frames.min(expected);
        self.pending.clear();
    }

    fn start(&mut self, rate: u32) {
        self.reset();
        self.in_rate = rate;
        if rate != self.out_rate
            && rate > 0
            && let Ok(r) = Fft::<f32>::new(
                rate as usize,
                self.out_rate as usize,
                BLOCK,
                self.out_channels,
                FixedSync::Input,
            )
        {
            self.drop_frames = r.output_delay();
            self.block_out = vec![0.0; r.output_frames_max() * self.out_channels];
            self.resampler = Some(r);
        }
    }

    fn run_blocks(&mut self, out: &mut Vec<f32>) {
        let block = BLOCK * self.out_channels;
        let mut start = 0;
        while self.pending.len() - start >= block {
            self.process_block(start, None, out);
            start += block;
        }
        self.pending.drain(..start);
    }

    /// Resamples the block of `pending` that starts at sample `start`. With `partial`, only
    /// that many frames of it are real input and the rest counts as silence.
    fn process_block(&mut self, start: usize, partial: Option<usize>, out: &mut Vec<f32>) {
        let ch = self.out_channels;
        let Some(r) = self.resampler.as_mut() else {
            return;
        };
        let Ok(input) = InterleavedSlice::new(&self.pending[start..start + BLOCK * ch], ch, BLOCK)
        else {
            return;
        };
        let out_max = self.block_out.len() / ch;
        let Ok(mut output) = InterleavedSlice::new_mut(&mut self.block_out[..], ch, out_max) else {
            return;
        };
        let indexing = partial.map(|n| Indexing::new().partial_len(n));
        let Ok((_, written)) = r.process_into_buffer(&input, &mut output, indexing.as_ref()) else {
            return;
        };
        let skip = self.drop_frames.min(written);
        self.drop_frames -= skip;
        out.extend_from_slice(&self.block_out[skip * ch..written * ch]);
        self.out_frames += (written - skip) as u64;
    }
}

/// Appends `src` (interleaved, `from` channels) to `out` as `to` channels.
///
/// Mono goes to the two front speakers; 5.1 and up is folded down to stereo with the usual
/// −3 dB centre and surrounds, scaled so it cannot clip (as FFmpeg's default downmix does).
fn remap(src: &[f32], from: usize, to: usize, out: &mut Vec<f32>) {
    if from == to {
        out.extend_from_slice(src);
        return;
    }
    out.reserve(src.len() / from * to);
    for frame in src.chunks_exact(from) {
        if to == 1 {
            out.push(frame.iter().sum::<f32>() / from as f32);
            continue;
        }
        let (l, r) = if from == 1 {
            (frame[0], frame[0])
        } else if from >= 6 {
            // FL FR FC LFE BL BR (the WAVE channel order Symphonia uses).
            const H: f32 = std::f32::consts::FRAC_1_SQRT_2;
            let norm = 1.0 / (1.0 + 2.0 * H);
            (
                (frame[0] + H * frame[2] + H * frame[4]) * norm,
                (frame[1] + H * frame[2] + H * frame[5]) * norm,
            )
        } else {
            (frame[0], frame[1])
        };
        out.push(l);
        out.push(r);
        if from > 2 && from < 6 && to > 2 {
            // Same layout prefix: keep what fits, silence the rest.
            out.extend((2..to).map(|i| frame.get(i).copied().unwrap_or(0.0)));
        } else {
            out.extend(std::iter::repeat_n(0.0, to - 2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, channels: usize, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v = (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5;
                std::iter::repeat_n(v, channels)
            })
            .collect()
    }

    fn convert(
        rate: u32,
        channels: usize,
        frames: usize,
        out_rate: u32,
        out_ch: usize,
    ) -> Vec<f32> {
        let input = sine(rate, channels, frames);
        let mut c = Converter::new(out_rate, out_ch);
        let mut out = Vec::new();
        // Feed in uneven pieces, like a decoder does.
        for piece in input.chunks(577 * channels) {
            c.push(
                &Chunk {
                    samples: piece,
                    rate,
                    channels,
                },
                &mut out,
            );
        }
        c.flush(&mut out);
        out
    }

    #[test]
    fn same_rate_passes_through() {
        let out = convert(48_000, 2, 10_000, 48_000, 2);
        assert_eq!(out, sine(48_000, 2, 10_000));
    }

    #[test]
    fn resampled_length_matches_time() {
        for (from, to) in [
            (44_100, 48_000),
            (48_000, 44_100),
            (96_000, 48_000),
            (22_050, 48_000),
        ] {
            let frames = 30_000;
            let out = convert(from, 2, frames, to, 2);
            let expected = frames as u64 * u64::from(to) / u64::from(from);
            assert_eq!((out.len() / 2) as u64, expected, "{from} -> {to}");
        }
    }

    #[test]
    fn resampling_keeps_timing() {
        // A click at a known time must come out at the same time, not shifted by the delay.
        let (from, to) = (44_100u32, 48_000u32);
        let mut input = vec![0.0f32; 20_000];
        input[10_000] = 1.0;
        let mut c = Converter::new(to, 1);
        let mut out = Vec::new();
        c.push(
            &Chunk {
                samples: &input,
                rate: from,
                channels: 1,
            },
            &mut out,
        );
        c.flush(&mut out);
        let peak = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(i, _)| i)
            .unwrap();
        let expected = 10_000 * to as usize / from as usize;
        assert!(
            peak.abs_diff(expected) <= 2,
            "peak at {peak}, expected {expected}"
        );
    }

    #[test]
    fn mono_goes_to_both_speakers() {
        let mut out = Vec::new();
        remap(&[0.25, -0.5], 1, 2, &mut out);
        assert_eq!(out, [0.25, 0.25, -0.5, -0.5]);
        out.clear();
        remap(&[0.25], 1, 4, &mut out);
        assert_eq!(out, [0.25, 0.25, 0.0, 0.0]);
    }

    #[test]
    fn surround_folds_down_without_clipping() {
        let mut out = Vec::new();
        remap(&[1.0; 6], 6, 2, &mut out);
        assert!(out.iter().all(|&v| (v - 1.0).abs() < 1e-6), "{out:?}");
        out.clear();
        remap(&[0.2, 0.4], 2, 1, &mut out);
        assert!((out[0] - 0.3).abs() < 1e-6);
    }
}
