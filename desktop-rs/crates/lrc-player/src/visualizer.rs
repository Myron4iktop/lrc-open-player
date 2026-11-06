//! The visualizer behind the lyrics (`VisualizerBackdrop`): the feed from the audio
//! output, and the drawing. The analysis itself is `lrc_core::visualizer`.
//!
//! The picture is painted into an image with tiny-skia (antialiased shapes and the same
//! accent gradient as the Python app's QPainter), and Slint shows that image at the
//! bottom of the lyrics area, under the lyric cards.

use lrc_core::visualizer::{self as viz, Analyzer, Chunks, HEIGHT_SHARE};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, Shader,
    SpreadMode, Stroke, Transform,
};

use crate::audio::{Player, VizFeed};

pub struct Visualizer {
    mode: &'static str,
    feed: Option<VizFeed>,
    chunks: Chunks,
    analyzer: Analyzer,
}

impl Default for Visualizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Visualizer {
    pub fn new() -> Self {
        Self {
            mode: "off",
            feed: None,
            chunks: Chunks::default(),
            analyzer: Analyzer::new(),
        }
    }

    pub fn mode(&self) -> &'static str {
        self.mode
    }

    /// `set_mode`: nothing is captured or computed while it is off.
    pub fn set_mode(&mut self, mode: &str, player: &Player) {
        self.mode = viz::known_mode(mode);
        if self.mode == "off" {
            player.detach_visualizer();
            self.feed = None;
            self.chunks.clear();
            self.analyzer.reset();
        } else if self.feed.is_none() {
            self.feed = Some(player.attach_visualizer());
        }
    }

    /// A new song (`sourceChanged`): what was buffered belongs to the old one.
    pub fn clear(&mut self) {
        self.chunks.clear();
        if let Some(feed) = &mut self.feed {
            while feed.blocks.pop().is_ok() {}
            let n = feed.samples.slots();
            if let Ok(chunk) = feed.samples.read_chunk(n) {
                chunk.commit_all();
            }
        }
    }

    /// `_tick`, every 33 ms: takes in what was played, and eases the bars towards the
    /// spectrum at `pos_ms`, the position being heard. False once paused and flat, when
    /// the timer can stop.
    pub fn tick(&mut self, playing: bool, pos_ms: u64) -> bool {
        if let Some(feed) = &mut self.feed {
            while let Ok(block) = feed.blocks.pop() {
                let Ok(chunk) = feed.samples.read_chunk(block.frames) else {
                    break;
                };
                let (a, b) = chunk.as_slices();
                let data = [a, b].concat();
                chunk.commit_all();
                self.chunks
                    .push(block.start_us as f64 / 1000.0, block.rate, data);
            }
        }
        let target = if playing {
            self.chunks
                .window_at(pos_ms as f64)
                .map(|w| self.analyzer.analyze(&w))
        } else {
            None
        };
        self.analyzer.step(target, playing)
    }

    /// `paintEvent`: the bottom band of a lyrics area `width` × `height` (logical px) at
    /// `scale` physical px per logical px, or an empty image when there is nothing to draw.
    pub fn render(
        &self,
        width: f32,
        height: f32,
        scale: f32,
        accent: [u8; 4],
        alpha: f64,
    ) -> Image {
        if self.mode == "off" || width < 2.0 || height < 2.0 {
            return Image::default();
        }
        let Some(disp) = self.analyzer.display() else {
            return Image::default();
        };
        let (w, h) = (f64::from(width), f64::from(height));
        let (base_y, area_h) = viz::area(h);
        // Only the part the spectrum can reach (`_update_band_area`).
        let band = (h * HEIGHT_SHARE).floor() + 12.0;
        let top = (h - band).max(0.0);
        let px_w = (w * f64::from(scale)).ceil() as u32;
        let px_h = ((h - top) * f64::from(scale)).ceil() as u32;
        let Some(mut pixmap) = Pixmap::new(px_w.max(1), px_h.max(1)) else {
            return Image::default();
        };
        let to_px = Transform::from_translate(0.0, -top as f32).post_scale(scale, scale);

        let [r, g, b, _] = accent;
        let rgba = |a: f64| Color::from_rgba8(r, g, b, (a.clamp(0.0, 1.0) * 255.0).round() as u8);
        let Some(gradient) = LinearGradient::new(
            Point::from_xy(0.0, (base_y - area_h) as f32),
            Point::from_xy(0.0, base_y as f32),
            vec![
                GradientStop::new(0.0, rgba(0.0)),
                GradientStop::new(0.55, rgba(alpha * 0.45)),
                GradientStop::new(1.0, rgba(alpha)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        ) else {
            return Image::default();
        };
        let fill = Paint {
            shader: gradient,
            anti_alias: true,
            ..Paint::default()
        };

        let mut pb = PathBuilder::new();
        if self.mode == "bars" {
            for bar in viz::bar_rects(&disp, w, h) {
                push_rounded_rect(&mut pb, bar);
            }
            if let Some(path) = pb.finish() {
                pixmap.fill_path(&path, &fill, FillRule::Winding, to_px, None);
            }
        } else {
            let (first, curve) = viz::wave_curve(&disp, w, h);
            let push_curve = |pb: &mut PathBuilder| {
                for [c1, c2, p] in &curve {
                    pb.cubic_to(
                        c1.0 as f32,
                        c1.1 as f32,
                        c2.0 as f32,
                        c2.1 as f32,
                        p.0 as f32,
                        p.1 as f32,
                    );
                }
            };
            pb.move_to(0.0, base_y as f32);
            pb.line_to(first.0 as f32, first.1 as f32);
            push_curve(&mut pb);
            pb.line_to(w as f32, base_y as f32);
            pb.close();
            if let Some(path) = pb.finish() {
                pixmap.fill_path(&path, &fill, FillRule::Winding, to_px, None);
            }
            let mut crest = PathBuilder::new();
            crest.move_to(first.0 as f32, first.1 as f32);
            push_curve(&mut crest);
            if let Some(path) = crest.finish() {
                let line = Paint {
                    shader: Shader::SolidColor(rgba((alpha * 1.35).min(1.0))),
                    anti_alias: true,
                    ..Paint::default()
                };
                let stroke = Stroke {
                    width: 2.0,
                    ..Stroke::default()
                };
                pixmap.stroke_path(&path, &line, &stroke, to_px, None);
            }
        }
        let (pw, ph) = (pixmap.width(), pixmap.height());
        let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(pixmap.data(), pw, ph);
        // tiny-skia keeps premultiplied alpha.
        Image::from_rgba8_premultiplied(buf)
    }
}

/// `QPainter.drawRoundedRect`: a rectangle with quarter-circle corners.
fn push_rounded_rect(pb: &mut PathBuilder, bar: viz::Bar) {
    let (x, y, w, h) = (bar.x as f32, bar.y as f32, bar.w as f32, bar.h as f32);
    let r = (bar.radius as f32).min(w / 2.0).min(h / 2.0).max(0.0);
    // How far a cubic's handles sit along the tangent for a quarter circle.
    let k = r * 0.552_284_8;
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_only_while_something_moves() {
        let mut v = Visualizer::new();
        v.mode = "bars";
        assert!(
            v.render(400.0, 300.0, 1.0, [255, 0, 0, 255], 0.34)
                .size()
                .width
                == 0
        );
        v.analyzer.step(Some([0.8; viz::BANDS]), true);
        for mode in ["bars", "wave"] {
            v.mode = mode;
            let img = v.render(400.0, 300.0, 1.5, [255, 0, 0, 255], 0.34);
            let size = img.size();
            assert_eq!(size.width, 600);
            // The bottom 36% plus 12 px, at 1.5 physical px per px.
            assert_eq!(
                size.height,
                ((300.0f64 * 0.36).floor() + 12.0) as u32 * 3 / 2
            );
            let px = img.to_rgba8_premultiplied().unwrap();
            let painted = px.as_slice().iter().filter(|p| p.a > 0).count();
            assert!(painted > 1000, "{mode}: {painted} pixels");
        }
    }
}
