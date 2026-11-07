//! Themes and accent colors (`ACCENTS`, `THEMES`, `build_palette` in `lrcplayer.py`).
//!
//! Every color in the window comes from a palette built here. The Python app does its
//! HSL math with `QColor`, which stores channels as 16-bit integers and computes in
//! single precision, so the conversions below copy Qt's own steps (`QColor::toHsl`,
//! `QColor::fromHslF` and `QColor::toRgb`) to land on the very same hex codes.

/// The accent colors, in menu order: key, menu label, base color (`ACCENTS`).
pub const ACCENTS: [(&str, &str, &str); 7] = [
    ("coral", "Coral", "#E05A47"),
    ("rose", "Rose", "#E2577F"),
    ("amber", "Amber", "#E3A33B"),
    ("jade", "Jade", "#3FB27F"),
    ("aqua", "Aqua", "#34B4C8"),
    ("azure", "Azure", "#5B8FF0"),
    ("violet", "Violet", "#9B74E8"),
];

/// The themes, in menu order: key and menu label (`THEMES`).
pub const THEMES: [(&str, &str); 2] = [("dark", "Dark"), ("light", "Light")];

/// The base color of an accent, or Coral's for an unknown key.
pub fn accent_hex(key: &str) -> &'static str {
    ACCENTS.iter().find(|a| a.0 == key).unwrap_or(&ACCENTS[0]).2
}

/// An accent key the app knows, or "coral" (what `apply_theme` falls back to).
pub fn known_accent(key: &str) -> &'static str {
    ACCENTS.iter().find(|a| a.0 == key).unwrap_or(&ACCENTS[0]).0
}

/// A theme key the app knows, or "dark".
pub fn known_theme(key: &str) -> &'static str {
    THEMES.iter().find(|t| t.0 == key).unwrap_or(&THEMES[0]).0
}

// ---------------------------------------------------------------------------------------
// QColor's math
// ---------------------------------------------------------------------------------------

const MAX16: f32 = 65535.0;

/// `qRound` for floats: halves away from zero, computed in single precision.
fn q_round(d: f32) -> i32 {
    if d >= 0.0 {
        (d + 0.5) as i32
    } else {
        (d - 0.5) as i32
    }
}

/// `qFuzzyCompare` for floats.
fn fuzzy_compare(a: f32, b: f32) -> bool {
    (a - b).abs() * 100_000.0 <= a.abs().min(b.abs())
}

/// `qFuzzyIsNull` for floats.
fn fuzzy_is_null(f: f32) -> bool {
    f.abs() <= 0.00001
}

/// 16-bit to 8-bit, as `QColor::red()` and friends do: the nearest of 0..=255.
fn div_257(x: u16) -> u8 {
    ((u32::from(x) * 2 + 257) / 514) as u8
}

/// A color as `QColor` keeps it after `QColor("#rrggbb")`: three 16-bit channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb16 {
    r: u16,
    g: u16,
    b: u16,
}

impl Rgb16 {
    fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.strip_prefix('#')?;
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        let widen = |c: u8| u16::from(c) * 0x101;
        Some(Rgb16 {
            r: widen(channel(0)?),
            g: widen(channel(2)?),
            b: widen(channel(4)?),
        })
    }

    /// `QColor::name()`: lower-case `#rrggbb`.
    fn name(self) -> String {
        format!(
            "#{:02x}{:02x}{:02x}",
            div_257(self.r),
            div_257(self.g),
            div_257(self.b)
        )
    }
}

/// `QColor::toHsl()`, as hue (0..=35999 hundredths of a degree, or 65535 for grey),
/// saturation and lightness (0..=65535).
fn to_hsl(c: Rgb16) -> (u16, u16, u16) {
    let r = f32::from(c.r) / MAX16;
    let g = f32::from(c.g) / MAX16;
    let b = f32::from(c.b) / MAX16;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let delta2 = max + min;
    let lightness = 0.5 * delta2;
    let l = q_round(lightness * MAX16) as u16;
    if fuzzy_is_null(delta) {
        return (u16::MAX, 0, l);
    }
    let s = if lightness < 0.5 {
        q_round((delta / delta2) * MAX16)
    } else {
        q_round((delta / (2.0 - delta2)) * MAX16)
    } as u16;
    let mut hue = if fuzzy_compare(r, max) {
        (g - b) / delta
    } else if fuzzy_compare(g, max) {
        2.0 + (b - r) / delta
    } else {
        4.0 + (r - g) / delta
    };
    hue *= 60.0;
    if hue < 0.0 {
        hue += 360.0;
    }
    (q_round(hue * 100.0) as u16, s, l)
}

/// `QColor::fromHslF(h, s, l)` followed by `toRgb()`.
fn from_hsl_f(h: f32, s: f32, l: f32) -> Rgb16 {
    let hue = if h == -1.0 {
        u16::MAX
    } else {
        q_round(h * 36000.0) as u16
    };
    let sat = q_round(s * MAX16) as u16;
    let light = q_round(l * MAX16) as u16;

    if sat == 0 || hue == u16::MAX {
        return Rgb16 {
            r: light,
            g: light,
            b: light,
        };
    }
    if light == 0 {
        return Rgb16 { r: 0, g: 0, b: 0 };
    }
    let h = if hue == 36000 {
        0.0
    } else {
        f32::from(hue) / 36000.0
    };
    let s = f32::from(sat) / MAX16;
    let l = f32::from(light) / MAX16;
    let temp2 = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - (l * s)
    };
    let temp1 = (2.0 * l) - temp2;
    let mut temp3 = [h + (1.0 / 3.0), h, h - (1.0 / 3.0)];
    let mut out = [0u16; 3];
    for i in 0..3 {
        if temp3[i] < 0.0 {
            temp3[i] += 1.0;
        } else if temp3[i] > 1.0 {
            temp3[i] -= 1.0;
        }
        let sixtemp3 = temp3[i] * 6.0;
        let v = if sixtemp3 < 1.0 {
            temp1 + (temp2 - temp1) * sixtemp3
        } else if temp3[i] * 2.0 < 1.0 {
            temp2
        } else if temp3[i] * 3.0 < 2.0 {
            temp1 + (temp2 - temp1) * (2.0 / 3.0 - temp3[i]) * 6.0
        } else {
            temp1
        };
        out[i] = q_round(v * MAX16) as u16;
    }
    Rgb16 {
        r: out[0],
        g: out[1],
        b: out[2],
    }
}

/// `_hsl(h, s, l)`: hue in degrees (any value), saturation and lightness clamped to 0..1.
fn hsl(h: f64, s: f64, l: f64) -> String {
    let h = h.rem_euclid(360.0) / 360.0;
    from_hsl_f(h as f32, s.clamp(0.0, 1.0) as f32, l.clamp(0.0, 1.0) as f32).name()
}

// ---------------------------------------------------------------------------------------
// build_palette
// ---------------------------------------------------------------------------------------

/// Every UI color for a theme and an accent (`build_palette`).
///
/// Colors are strings as the Python app writes them: `#rrggbb` (lower case when
/// computed, as given otherwise) or `rgba(r, g, b, a)` for the drop overlay.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// Name and color, in the Python dict's order.
    pub colors: Vec<(&'static str, String)>,
    /// Opacity of the lyric lines that aren't playing.
    pub inactive_opacity: f64,
    /// Opacity of the visualizer (M6).
    pub viz_alpha: f64,
}

impl Palette {
    /// A color by its name in `build_palette`. Panics on a name the palette doesn't have,
    /// which is a typo in the caller.
    pub fn get(&self, name: &str) -> &str {
        self.colors
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, c)| c.as_str())
            .unwrap_or_else(|| panic!("no color named {name:?} in the palette"))
    }
}

/// All UI colors for a theme and an accent (`build_palette`).
///
/// Neutrals carry a faint tint of the accent's hue, so each accent feels like one
/// coherent theme. An unknown accent is Coral; any theme but "light" is dark.
pub fn build_palette(theme: &str, accent: &str) -> Palette {
    let base = Rgb16::from_hex(accent_hex(accent)).expect("accent colors are valid");
    let (hue, sat16, light16) = to_hsl(base);
    // hslHueF() is -1 for grey; the Python app takes max(0, ...).
    let hue_f = if hue == u16::MAX {
        -1.0
    } else {
        f32::from(hue) / 36000.0
    };
    let h = f64::from(hue_f.max(0.0)) * 360.0;
    let sat = f64::from(f32::from(sat16) / MAX16);
    let lig = f64::from(f32::from(light16) / MAX16);
    let t = |s: f64, l: f64| hsl(h, s, l);
    let fixed = |c: &str| c.to_owned();

    if theme == "light" {
        let pl = lig.min(0.44);
        let colors = vec![
            ("bg", t(0.20, 0.962)),
            ("dialog_bg", t(0.22, 0.972)),
            ("surface", t(0.30, 0.990)),
            ("surface2", t(0.16, 0.925)),
            ("surface_border", t(0.14, 0.86)),
            ("btn", t(0.30, 0.992)),
            ("btn_hover", t(0.22, 0.935)),
            ("btn_border", t(0.13, 0.84)),
            ("btn_disabled", t(0.15, 0.945)),
            ("btn_disabled_border", t(0.12, 0.895)),
            ("menu", t(0.30, 0.992)),
            ("menu_border", t(0.13, 0.84)),
            ("divider", t(0.13, 0.89)),
            ("hover", t(0.20, 0.935)),
            ("groove", t(0.13, 0.855)),
            ("badge_bg", t(0.18, 0.925)),
            ("text", t(0.14, 0.10)),
            ("text_soft", t(0.12, 0.17)),
            ("title", t(0.20, 0.08)),
            ("list_text", t(0.10, 0.26)),
            ("muted", t(0.08, 0.42)),
            ("dim", t(0.07, 0.47)),
            ("faint", t(0.06, 0.56)),
            ("faint2", t(0.06, 0.55)),
            ("text_disabled", t(0.06, 0.70)),
            ("handle", fixed("#FFFFFF")),
            ("handle_border", t(0.12, 0.74)),
            ("icon", t(0.14, 0.20)),
            ("icon_muted", t(0.07, 0.52)),
            ("accent", hsl(h, sat, lig.min(0.50))),
            ("artist", t(0.35, 0.40)),
            ("badge_text", t(0.22, 0.38)),
            ("sel", t(0.55, 0.905)),
            ("sel_text", t(0.20, 0.10)),
            ("toggle_bg", t(0.70, 0.90)),
            ("toggle_border", t(0.45, 0.72)),
            ("toggle_text", t(0.60, 0.30)),
            ("primary", hsl(h, sat * 0.9, pl)),
            ("primary_hover", hsl(h, sat * 0.9, pl - 0.05)),
            ("primary_border", hsl(h, sat * 0.9, pl - 0.08)),
            ("primary_text", fixed("#FFFFFF")),
            ("primary_disabled", t(0.25, 0.88)),
            ("primary_disabled_text", t(0.06, 0.60)),
            ("primary_disabled_border", t(0.20, 0.82)),
            ("lyric_active", t(0.20, 0.08)),
            ("lyric_active_sub", t(0.60, 0.40)),
            ("lyric_inactive", t(0.12, 0.17)),
            ("lyric_inactive_sub", t(0.25, 0.35)),
            ("lyric_plain", t(0.12, 0.20)),
            ("time_ok", t(0.55, 0.38)),
            ("status", t(0.30, 0.38)),
            ("error", fixed("#C7362B")),
            ("warn_text", fixed("#8A1F16")),
            ("warn_bg", fixed("#FBE3DF")),
            ("overlay", fixed("rgba(255, 255, 255, 215)")),
        ];
        return Palette {
            dark: false,
            colors,
            inactive_opacity: 0.40,
            viz_alpha: 0.26,
        };
    }

    let colors = vec![
        ("bg", t(0.11, 0.071)),
        ("dialog_bg", t(0.13, 0.063)),
        ("surface", t(0.13, 0.090)),
        ("surface2", t(0.11, 0.108)),
        ("surface_border", t(0.14, 0.155)),
        ("btn", t(0.10, 0.118)),
        ("btn_hover", t(0.10, 0.175)),
        ("btn_border", t(0.09, 0.194)),
        ("btn_disabled", t(0.11, 0.092)),
        ("btn_disabled_border", t(0.12, 0.130)),
        ("menu", t(0.13, 0.108)),
        ("menu_border", t(0.14, 0.200)),
        ("divider", t(0.12, 0.180)),
        ("hover", t(0.14, 0.137)),
        ("groove", t(0.13, 0.157)),
        ("badge_bg", t(0.14, 0.114)),
        ("text", fixed("#FFFFFF")),
        ("text_soft", t(0.22, 0.895)),
        ("title", t(1.0, 0.967)),
        ("list_text", t(0.17, 0.80)),
        ("muted", t(0.10, 0.61)),
        ("dim", t(0.08, 0.51)),
        ("faint", t(0.09, 0.45)),
        ("faint2", t(0.07, 0.52)),
        ("text_disabled", t(0.09, 0.34)),
        ("handle", fixed("#FFFFFF")),
        ("handle_border", fixed("#FFFFFF")),
        ("icon", t(0.33, 0.92)),
        ("icon_muted", t(0.08, 0.52)),
        ("accent", base.name()),
        ("artist", t(0.28, 0.69)),
        ("badge_text", t(0.20, 0.69)),
        ("sel", t(0.22, 0.185)),
        ("sel_text", t(1.0, 0.967)),
        ("toggle_bg", t(0.35, 0.26)),
        ("toggle_border", t(0.34, 0.40)),
        ("toggle_text", t(1.0, 0.94)),
        ("primary", t(0.49, 0.32)),
        ("primary_hover", t(0.47, 0.38)),
        ("primary_border", t(0.43, 0.44)),
        ("primary_text", t(1.0, 0.97)),
        ("primary_disabled", t(0.22, 0.185)),
        ("primary_disabled_text", t(0.08, 0.52)),
        ("primary_disabled_border", t(0.18, 0.24)),
        ("lyric_active", fixed("#FFFFFF")),
        ("lyric_active_sub", t(1.0, 0.87)),
        ("lyric_inactive", t(0.22, 0.895)),
        ("lyric_inactive_sub", t(0.20, 0.69)),
        ("lyric_plain", t(0.18, 0.87)),
        ("time_ok", t(0.58, 0.79)),
        ("status", t(0.25, 0.72)),
        ("error", fixed("#FF7A6E")),
        ("warn_text", fixed("#FFB4A8")),
        ("warn_bg", fixed("#3A1F1C")),
        ("overlay", fixed("rgba(20, 16, 16, 225)")),
    ];
    Palette {
        dark: true,
        colors,
        inactive_opacity: 0.32,
        viz_alpha: 0.34,
    }
}

/// A palette color as red, green, blue and alpha bytes: `#rrggbb` or `rgba(r, g, b, a)`.
pub fn parse_color(text: &str) -> Option<[u8; 4]> {
    if let Some(hex) = text.strip_prefix('#') {
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        let c = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return Some([c(0)?, c(2)?, c(4)?, 255]);
    }
    let inner = text.strip_prefix("rgba(")?.strip_suffix(')')?;
    let parts: Vec<u8> = inner
        .split(',')
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    match parts[..] {
        [r, g, b, a] => Some([r, g, b, a]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_through_qcolor() {
        for (_, _, hex) in ACCENTS {
            let c = Rgb16::from_hex(hex).unwrap();
            assert_eq!(c.name(), hex.to_lowercase());
        }
    }

    #[test]
    fn unknown_keys_fall_back() {
        assert_eq!(accent_hex("nope"), "#E05A47");
        assert_eq!(known_accent("nope"), "coral");
        assert_eq!(known_theme("blue"), "dark");
        assert_eq!(known_theme("light"), "light");
        assert!(!build_palette("light", "jade").dark);
        assert!(build_palette("other", "jade").dark);
    }

    #[test]
    fn colors_parse() {
        assert_eq!(parse_color("#ff0080"), Some([255, 0, 128, 255]));
        assert_eq!(
            parse_color("rgba(20, 16, 16, 225)"),
            Some([20, 16, 16, 225])
        );
        assert_eq!(parse_color("red"), None);
        for theme in ["dark", "light"] {
            for (accent, _, _) in ACCENTS {
                for (name, color) in &build_palette(theme, accent).colors {
                    assert!(
                        parse_color(color).is_some(),
                        "{theme}/{accent} {name} {color}"
                    );
                }
            }
        }
    }
}
