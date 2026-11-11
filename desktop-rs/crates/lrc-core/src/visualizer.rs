//! The spectrum behind the lyrics: `VisualizerBackdrop` and `_make_fft` in lrcplayer.py.
//!
//! The player hands over mono copies of what it plays, each stamped with its song
//! position ([`Chunks::push`]). Every tick the window takes the samples that end at the
//! position being heard ([`Chunks::window_at`]), turns them into 44 band levels
//! ([`Analyzer::analyze`]), eases the bars towards them ([`Analyzer::step`]) and draws
//! them ([`bar_rects`], [`wave_curve`]). The constants are the Python app's.

use std::collections::VecDeque;
use std::f64::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// `VisualizerBackdrop.MODES`, in the order `V` cycles through them.
pub const MODES: [&str; 3] = ["off", "bars", "wave"];
/// Samples per analysis.
pub const FFT_SIZE: usize = 1024;
/// The rate the samples are analyzed at: mono at 22 kHz is plenty for a picture.
pub const RATE: u32 = 22050;
pub const BANDS: usize = 44;
pub const F_LO: f64 = 55.0;
pub const F_HI: f64 = 9500.0;
/// The bottom part of the lyrics area the spectrum may use.
pub const HEIGHT_SHARE: f64 = 0.36;
/// How often the picture moves, in milliseconds.
pub const TICK_MS: u64 = 33;

/// A mode the app knows, or "off".
pub fn known_mode(mode: &str) -> &'static str {
    MODES.iter().find(|m| **m == mode).copied().unwrap_or("off")
}

/// `cycle_visualizer`: off → bars → wave → off.
pub fn next_mode(mode: &str) -> &'static str {
    let i = MODES.iter().position(|m| *m == mode).unwrap_or(0);
    MODES[(i + 1) % MODES.len()]
}

/// A run of mono samples that starts at a known song position.
#[derive(Clone, Debug)]
struct Chunk {
    start_ms: f64,
    rate: u32,
    data: Vec<f32>,
}

/// The last few seconds of what was played (`_chunks`).
#[derive(Default, Debug)]
pub struct Chunks {
    chunks: VecDeque<Chunk>,
}

impl Chunks {
    pub fn clear(&mut self) {
        self.chunks.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// `_on_buffer`: keeps a block of mono samples that starts at `start_ms`.
    pub fn push(&mut self, start_ms: f64, rate: u32, data: Vec<f32>) {
        if data.is_empty() {
            return;
        }
        let rate = if rate == 0 { RATE } else { rate };
        if self
            .chunks
            .back()
            .is_some_and(|last| start_ms < last.start_ms - 500.0)
        {
            self.chunks.clear(); // jumped backwards (seek / new song)
        }
        self.chunks.push_back(Chunk {
            start_ms,
            rate,
            data,
        });
        while self.chunks.len() > 2
            && self.chunks.back().unwrap().start_ms - self.chunks.front().unwrap().start_ms > 3000.0
        {
            self.chunks.pop_front();
        }
    }

    /// `_window_at`: [`FFT_SIZE`] samples at [`RATE`] ending at `pos_ms`, or `None` if that
    /// part of the song isn't buffered.
    ///
    /// The Python app asks Qt for 22,050 Hz. Here the samples come at the output's rate,
    /// so the same stretch of time is taken and then averaged down to [`RATE`].
    pub fn window_at(&self, pos_ms: f64) -> Option<Vec<f32>> {
        let idx = self.chunks.iter().rposition(|c| c.start_ms <= pos_ms)?;
        let Chunk {
            start_ms: mut start,
            rate,
            ref data,
        } = self.chunks[idx];
        let end = ((pos_ms - start) * f64::from(rate) / 1000.0) as usize;
        if end > data.len() + rate as usize / 4 {
            return None; // nothing decoded for this moment (just seeked, or the song ended)
        }
        let end = end.min(data.len());
        let need = (FFT_SIZE as f64 * f64::from(rate) / f64::from(RATE)).round() as usize;
        let mut parts: Vec<&[f32]> = vec![&data[end.saturating_sub(need)..end]];
        let mut have = parts[0].len();
        let mut j = idx;
        while have < need && j > 0 {
            j -= 1;
            let prev = &self.chunks[j];
            let prev_end = prev.start_ms + prev.data.len() as f64 * 1000.0 / f64::from(prev.rate);
            if prev.rate != rate || (prev_end - start).abs() > 60.0 {
                break; // not continuous with the newer chunk
            }
            let take = (need - have).min(prev.data.len());
            parts.push(&prev.data[prev.data.len() - take..]);
            have += take;
            start = prev.start_ms;
        }
        let mut out = vec![0.0f32; need - have];
        for part in parts.iter().rev() {
            out.extend_from_slice(part);
        }
        Some(downsample(&out, FFT_SIZE))
    }
}

/// Averages `input` down to `len` samples (or stretches it, if shorter): each output
/// sample is the mean of the input samples in its slot, which also keeps the treble
/// above the new rate from folding back into the picture.
fn downsample(input: &[f32], len: usize) -> Vec<f32> {
    if input.len() == len {
        return input.to_vec();
    }
    let step = input.len() as f64 / len as f64;
    (0..len)
        .map(|i| {
            let lo = (i as f64 * step) as usize;
            let hi = (((i + 1) as f64 * step) as usize).clamp(lo + 1, input.len().max(1));
            let slot = &input[lo.min(input.len().saturating_sub(1))..hi.min(input.len())];
            if slot.is_empty() {
                0.0
            } else {
                slot.iter().sum::<f32>() / slot.len() as f32
            }
        })
        .collect()
}

/// `_band_bins`: the FFT bins of each band, `lo..hi`.
pub fn band_bins() -> Vec<(usize, usize)> {
    let bin_hz = f64::from(RATE) / FFT_SIZE as f64;
    let ratio = F_HI / F_LO;
    let edges: Vec<f64> = (0..=BANDS)
        .map(|i| F_LO * ratio.powf(i as f64 / BANDS as f64))
        .collect();
    let half = FFT_SIZE / 2;
    (0..BANDS)
        .map(|i| {
            let lo = ((edges[i] / bin_hz) as usize).max(1);
            let hi = half.min((lo + 1).max((edges[i + 1] / bin_hz).ceil() as usize));
            (lo, hi)
        })
        .collect()
}

/// The FFT, the band levels and their easing (the analysis half of `VisualizerBackdrop`).
pub struct Analyzer {
    fft: Arc<dyn Fft<f64>>,
    window: Vec<f64>,
    bands: Vec<(usize, usize)>,
    tilt: Vec<f64>,
    ref_db: f64,
    levels: [f64; BANDS],
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Self {
        let n = FFT_SIZE;
        Self {
            fft: FftPlanner::new().plan_fft_forward(n),
            // Hann window.
            window: (0..n)
                .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / (n - 1) as f64).cos())
                .collect(),
            bands: band_bins(),
            // Music has less energy up high; lift the treble so the bars look even.
            tilt: (0..BANDS)
                .map(|i| 10.0 * i as f64 / (BANDS - 1) as f64)
                .collect(),
            ref_db: -30.0,
            levels: [0.0; BANDS],
        }
    }

    /// Back to silence, as switching the visualizer off does.
    pub fn reset(&mut self) {
        self.levels = [0.0; BANDS];
    }

    pub fn levels(&self) -> &[f64; BANDS] {
        &self.levels
    }

    /// The loudness reference the levels are measured against, in dB.
    pub fn ref_db(&self) -> f64 {
        self.ref_db
    }

    /// `_analyze`: the target level of each band, 0 to 1, for [`FFT_SIZE`] samples.
    pub fn analyze(&mut self, samples: &[f32]) -> [f64; BANDS] {
        let n = FFT_SIZE;
        let mut spectrum: Vec<Complex<f64>> = (0..n)
            .map(|i| {
                Complex::new(
                    f64::from(samples.get(i).copied().unwrap_or(0.0)) * self.window[i],
                    0.0,
                )
            })
            .collect();
        self.fft.process(&mut spectrum);
        let norm = 4.0 / n as f64; // a full-scale sine wave reads as 1.0
        let power: Vec<f64> = spectrum[..n / 2]
            .iter()
            .map(|c| (c.norm() * norm).powi(2))
            .collect();
        let mut dbs = [0.0; BANDS];
        for (i, &(lo, hi)) in self.bands.iter().enumerate() {
            let peak = power[lo..hi]
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            dbs[i] = 10.0 * (peak + 1e-12).log10() + self.tilt[i];
        }
        // Auto-gain: follow loud passages quickly, relax slowly, never amplify silence.
        let loudest = dbs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        self.ref_db = loudest.max(self.ref_db - 0.25).max(-34.0);
        let span = 46.0;
        let floor = self.ref_db - span;
        dbs.map(|db| ((db - floor) / span).clamp(0.0, 1.0).powf(1.7))
    }

    /// The easing half of `_tick`: moves the bars towards `target` (silence if `None`).
    /// Returns false once there is nothing left to animate while not `playing`, so the
    /// timer can stop.
    pub fn step(&mut self, target: Option<[f64; BANDS]>, playing: bool) -> bool {
        let target = target.unwrap_or([0.0; BANDS]);
        for (v, t) in self.levels.iter_mut().zip(target) {
            *v += (t - *v) * if t > *v { 0.55 } else { 0.14 }; // quick rise, gentle fall
        }
        if !playing && self.peak() < 0.002 {
            self.levels = [0.0; BANDS];
            return false; // paused or stopped: nothing to animate
        }
        true
    }

    /// The highest level, to skip drawing when everything is (almost) flat.
    pub fn peak(&self) -> f64 {
        self.levels.iter().copied().fold(0.0, f64::max)
    }

    /// What `paintEvent` draws: each level smoothed with its neighbours, which looks
    /// calmer than the raw bins. `None` when there is nothing to draw.
    pub fn display(&self) -> Option<[f64; BANDS]> {
        if self.peak() < 0.002 {
            return None;
        }
        let lv = &self.levels;
        let n = BANDS;
        Some(std::array::from_fn(|i| {
            (lv[i.saturating_sub(1)] + 2.0 * lv[i] + lv[(i + 1).min(n - 1)]) / 4.0
        }))
    }
}

/// Where the spectrum sits in a lyrics area `height` tall: the baseline and how high
/// the bars may reach.
pub fn area(height: f64) -> (f64, f64) {
    (height - 4.0, height * HEIGHT_SHARE)
}

/// A rounded bar: x, y, width, height and corner radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub radius: f64,
}

/// The "bars" mode in an area `width` × `height`.
pub fn bar_rects(disp: &[f64], width: f64, height: f64) -> Vec<Bar> {
    let (base_y, area_h) = area(height);
    let n = disp.len().max(1) as f64;
    let slot = width / n;
    let bw = slot * 0.6;
    let radius = (bw / 2.0).min(4.0);
    disp.iter()
        .enumerate()
        .map(|(i, v)| {
            let h = (v * area_h).max(3.0);
            Bar {
                x: i as f64 * slot + (slot - bw) / 2.0,
                y: base_y - h,
                w: bw,
                h,
                radius,
            }
        })
        .collect()
}

/// One cubic Bézier piece: two control points and the end point.
pub type Cubic = [(f64, f64); 3];

/// The "wave" mode: the first point, then a Catmull-Rom curve through all of them.
pub fn wave_curve(disp: &[f64], width: f64, height: f64) -> ((f64, f64), Vec<Cubic>) {
    let (base_y, area_h) = area(height);
    let n = disp.len();
    if n < 2 {
        return ((0.0, base_y), Vec::new());
    }
    let pts: Vec<(f64, f64)> = disp
        .iter()
        .enumerate()
        .map(|(i, v)| (i as f64 * width / (n - 1) as f64, base_y - v * area_h))
        .collect();
    let curve = (0..n - 1)
        .map(|i| {
            let p0 = pts[i.saturating_sub(1)];
            let (p1, p2) = (pts[i], pts[i + 1]);
            let p3 = pts[(i + 2).min(n - 1)];
            [
                (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0),
                (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0),
                p2,
            ]
        })
        .collect();
    (pts[0], curve)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freqs: &[(f64, f64)], rate: u32, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                freqs
                    .iter()
                    .map(|(f, a)| a * (2.0 * PI * f * i as f64 / f64::from(rate)).sin())
                    .sum::<f64>() as f32
            })
            .collect()
    }

    #[test]
    fn modes_cycle_like_the_python_app() {
        assert_eq!(next_mode("off"), "bars");
        assert_eq!(next_mode("bars"), "wave");
        assert_eq!(next_mode("wave"), "off");
        assert_eq!(known_mode("rainbow"), "off");
        assert_eq!(known_mode("wave"), "wave");
    }

    #[test]
    fn band_bins_match_python() {
        // Printed by `_band_bins` in lrcplayer.py.
        let expected = [
            (2, 3),
            (2, 4),
            (3, 4),
            (3, 5),
            (4, 5),
            (4, 6),
            (5, 6),
            (5, 7),
            (6, 8),
            (7, 9),
            (8, 10),
            (9, 11),
            (10, 12),
            (11, 14),
            (13, 15),
            (14, 17),
            (16, 19),
            (18, 22),
            (21, 24),
            (23, 27),
            (26, 30),
            (29, 34),
            (33, 38),
            (37, 43),
            (42, 48),
            (47, 54),
            (53, 61),
            (60, 68),
            (67, 77),
            (76, 86),
            (85, 97),
            (96, 109),
            (108, 122),
            (121, 137),
            (136, 154),
            (153, 173),
            (172, 195),
            (194, 219),
            (218, 246),
            (245, 277),
            (276, 311),
            (310, 350),
            (349, 393),
            (392, 442),
        ];
        assert_eq!(band_bins(), expected);
    }

    #[test]
    fn analysis_matches_python() {
        // `_analyze` in lrcplayer.py, on 0.5 × 1 kHz + 0.1 × 200 Hz, then on a near-silent
        // 440 Hz (which the auto-gain must not lift).
        let expected = [
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.223364, 0.467035, 0.473334, 0.414283,
            0.053372, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.982974, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ];
        let mut a = Analyzer::new();
        let got = a.analyze(&sine(&[(1000.0, 0.5), (200.0, 0.1)], RATE, FFT_SIZE));
        for (i, (g, e)) in got.iter().zip(expected).enumerate() {
            assert!((g - e).abs() < 1e-5, "band {i}: {g} vs {e}");
        }
        assert!((a.ref_db() - -1.543347).abs() < 1e-5, "{}", a.ref_db());
        let quiet = a.analyze(&sine(&[(440.0, 0.001)], RATE, FFT_SIZE));
        assert!(quiet.iter().all(|&v| v == 0.0), "{quiet:?}");
        assert!((a.ref_db() - -1.793347).abs() < 1e-5);
    }

    #[test]
    fn easing_rises_fast_falls_slowly_and_stops_when_paused() {
        let mut a = Analyzer::new();
        assert!(a.step(Some([1.0; BANDS]), true));
        assert!((a.levels()[0] - 0.55).abs() < 1e-9);
        assert!(a.step(None, true));
        assert!((a.levels()[0] - 0.55 * 0.86).abs() < 1e-9);
        assert!(a.display().is_some());
        // Paused: it falls until flat, then says stop.
        let mut ticks = 0;
        while a.step(None, false) {
            ticks += 1;
            assert!(ticks < 100);
        }
        assert!(a.display().is_none());
        assert_eq!(a.peak(), 0.0);
    }

    #[test]
    fn window_follows_the_position_at_any_output_rate() {
        // 48 kHz output, 1 s of a 1 kHz tone in 100 ms blocks starting at 10 s.
        let rate = 48_000;
        let tone = sine(&[(1000.0, 0.5)], rate, 48_000);
        let mut chunks = Chunks::default();
        for (k, block) in tone.chunks(4800).enumerate() {
            chunks.push(10_000.0 + k as f64 * 100.0, rate, block.to_vec());
        }
        assert!(
            chunks.window_at(9_000.0).is_none(),
            "before anything was played"
        );
        assert!(
            chunks.window_at(12_000.0).is_none(),
            "well past what was played"
        );
        let w = chunks.window_at(10_500.0).unwrap();
        assert_eq!(w.len(), FFT_SIZE);
        let levels = Analyzer::new().analyze(&w);
        let loudest = (0..BANDS)
            .max_by(|&a, &b| levels[a].total_cmp(&levels[b]))
            .unwrap();
        let (lo, hi) = band_bins()[loudest];
        let hz = |bin: usize| bin as f64 * f64::from(RATE) / FFT_SIZE as f64;
        assert!(hz(lo) <= 1000.0 && 1000.0 <= hz(hi), "band {loudest}");

        // Right at the start, the missing part is silence in front.
        let w = chunks.window_at(10_010.0).unwrap();
        assert!(w[..FFT_SIZE / 2].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn a_jump_backwards_forgets_what_was_played() {
        let mut chunks = Chunks::default();
        chunks.push(60_000.0, RATE, vec![0.5; 2205]);
        chunks.push(1_000.0, RATE, vec![0.25; 2205]);
        let w = chunks.window_at(1_100.0).unwrap();
        assert!(w.iter().all(|&s| s == 0.25 || s == 0.0));
        // A seek forwards: the old chunk isn't continuous, so it isn't used to fill up.
        chunks.push(30_000.0, RATE, vec![-0.5; 2205]);
        let w = chunks.window_at(30_010.0).unwrap();
        assert!(w.iter().all(|&s| s == -0.5 || s == 0.0));
    }

    #[test]
    fn only_three_seconds_are_kept() {
        let mut chunks = Chunks::default();
        for k in 0..100 {
            chunks.push(k as f64 * 100.0, RATE, vec![0.1; 2205]);
        }
        assert!(chunks.chunks.len() <= 32);
        assert!(chunks.window_at(100.0).is_none());
    }

    #[test]
    fn bars_and_wave_geometry() {
        let disp = [0.0, 1.0, 0.5];
        let bars = bar_rects(&disp, 300.0, 100.0);
        assert_eq!(bars.len(), 3);
        // A silent band still shows a 3 px stub; a full one reaches 36% of the height.
        assert!((bars[0].h - 3.0).abs() < 1e-9);
        assert!((bars[1].h - 36.0).abs() < 1e-9);
        assert!((bars[1].y + bars[1].h - 96.0).abs() < 1e-9);
        assert!((bars[0].w - 60.0).abs() < 1e-9 && (bars[0].x - 20.0).abs() < 1e-9);
        assert_eq!(bars[0].radius, 4.0);

        let (start, curve) = wave_curve(&disp, 200.0, 100.0);
        assert_eq!(start, (0.0, 96.0));
        assert_eq!(curve.len(), 2);
        assert_eq!(curve[1][2], (200.0, 96.0 - 18.0));
    }

    #[test]
    fn downsampling_keeps_the_level() {
        let out = downsample(&vec![0.5; 2229], FFT_SIZE);
        assert_eq!(out.len(), FFT_SIZE);
        assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }
}
