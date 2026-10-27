use image::{DynamicImage, GenericImageView};
use ratatui::style::Color;
use serde::Deserialize;
use std::fs;
use std::path::Path;

/// Complete semantic UI color palette (either from a named theme or extracted from album art).
#[derive(Debug, Clone, Copy)]
pub struct ThemePalette {
    pub surface_highlight: Color,
    pub fg: Color,
    pub muted: Color,
    pub border: Color,
    pub border_focused: Color,
    pub primary: Color,
    pub secondary: Color,
    pub accent: Color,
    pub playing: Color,
    pub spectrum_low: (u8, u8, u8),
    pub spectrum_mid: (u8, u8, u8),
    pub spectrum_high: (u8, u8, u8),
}

pub const BUILTIN_THEME_NAMES: &[&str] = &[
    "midnight",
    "catppuccin",
    "gruvbox",
    "nord",
    "tokyonight",
    "rosepine",
    "solarized",
    "amber",
];

#[derive(Debug, Deserialize)]
struct CustomThemeToml {
    primary: Option<String>,
    secondary: Option<String>,
    accent: Option<String>,
    playing: Option<String>,
    fg: Option<String>,
    muted: Option<String>,
    border: Option<String>,
}

impl ThemePalette {
    pub fn by_name(name: &str, custom_themes_dir: Option<&Path>) -> Self {
        let lower = name.trim().to_ascii_lowercase();
        if let Some(dir) = custom_themes_dir {
            let candidate = dir.join(format!("{}.toml", lower));
            if let Ok(raw) = fs::read_to_string(&candidate) {
                if let Ok(parsed) = toml::from_str::<CustomThemeToml>(&raw) {
                    let mut base = Self::midnight();
                    if let Some((r, g, b)) = parsed.primary.as_deref().and_then(parse_hex_rgb) {
                        base.primary = Color::Rgb(r, g, b);
                        base.border_focused = Color::Rgb(r, g, b);
                        base.spectrum_low = (r, g, b);
                    }
                    if let Some((r, g, b)) = parsed.secondary.as_deref().and_then(parse_hex_rgb) {
                        base.secondary = Color::Rgb(r, g, b);
                        base.spectrum_mid = (r, g, b);
                    }
                    if let Some((r, g, b)) = parsed.accent.as_deref().and_then(parse_hex_rgb) {
                        base.accent = Color::Rgb(r, g, b);
                        base.spectrum_high = (r, g, b);
                    }
                    if let Some((r, g, b)) = parsed.playing.as_deref().and_then(parse_hex_rgb) {
                        base.playing = Color::Rgb(r, g, b);
                    }
                    if let Some((r, g, b)) = parsed.fg.as_deref().and_then(parse_hex_rgb) {
                        base.fg = Color::Rgb(r, g, b);
                    }
                    if let Some((r, g, b)) = parsed.muted.as_deref().and_then(parse_hex_rgb) {
                        base.muted = Color::Rgb(r, g, b);
                    }
                    if let Some((r, g, b)) = parsed.border.as_deref().and_then(parse_hex_rgb) {
                        base.border = Color::Rgb(r, g, b);
                    }
                    return base;
                }
            }
        }

        match lower.as_str() {
            "catppuccin" | "mocha" => Self {
                surface_highlight: Color::Rgb(49, 50, 68),
                fg: Color::Rgb(205, 214, 244),
                muted: Color::Rgb(127, 132, 156),
                border: Color::Rgb(88, 91, 112),
                border_focused: Color::Rgb(203, 166, 247),
                primary: Color::Rgb(203, 166, 247),
                secondary: Color::Rgb(137, 180, 250),
                accent: Color::Rgb(250, 179, 135),
                playing: Color::Rgb(166, 227, 161),
                spectrum_low: (137, 180, 250),
                spectrum_mid: (203, 166, 247),
                spectrum_high: (243, 139, 168),
            },
            "gruvbox" => Self {
                surface_highlight: Color::Rgb(60, 56, 54),
                fg: Color::Rgb(235, 219, 178),
                muted: Color::Rgb(146, 131, 116),
                border: Color::Rgb(102, 92, 84),
                border_focused: Color::Rgb(254, 128, 25),
                primary: Color::Rgb(254, 128, 25),
                secondary: Color::Rgb(250, 189, 47),
                accent: Color::Rgb(142, 192, 124),
                playing: Color::Rgb(184, 187, 38),
                spectrum_low: (142, 192, 124),
                spectrum_mid: (250, 189, 47),
                spectrum_high: (251, 73, 52),
            },
            "nord" => Self {
                surface_highlight: Color::Rgb(59, 66, 82),
                fg: Color::Rgb(236, 239, 244),
                muted: Color::Rgb(129, 161, 193),
                border: Color::Rgb(76, 86, 106),
                border_focused: Color::Rgb(136, 192, 208),
                primary: Color::Rgb(136, 192, 208),
                secondary: Color::Rgb(129, 161, 193),
                accent: Color::Rgb(180, 142, 173),
                playing: Color::Rgb(163, 190, 140),
                spectrum_low: (94, 129, 172),
                spectrum_mid: (136, 192, 208),
                spectrum_high: (180, 142, 173),
            },
            "tokyonight" | "tokyo" => Self {
                surface_highlight: Color::Rgb(41, 46, 66),
                fg: Color::Rgb(192, 202, 245),
                muted: Color::Rgb(115, 122, 162),
                border: Color::Rgb(65, 72, 104),
                border_focused: Color::Rgb(122, 162, 247),
                primary: Color::Rgb(122, 162, 247),
                secondary: Color::Rgb(187, 154, 247),
                accent: Color::Rgb(125, 207, 255),
                playing: Color::Rgb(158, 206, 106),
                spectrum_low: (122, 162, 247),
                spectrum_mid: (187, 154, 247),
                spectrum_high: (247, 118, 142),
            },
            "rosepine" | "rose" => Self {
                surface_highlight: Color::Rgb(38, 35, 58),
                fg: Color::Rgb(224, 222, 244),
                muted: Color::Rgb(144, 140, 170),
                border: Color::Rgb(82, 79, 103),
                border_focused: Color::Rgb(235, 188, 186),
                primary: Color::Rgb(235, 188, 186),
                secondary: Color::Rgb(196, 167, 231),
                accent: Color::Rgb(246, 193, 119),
                playing: Color::Rgb(156, 207, 216),
                spectrum_low: (156, 207, 216),
                spectrum_mid: (196, 167, 231),
                spectrum_high: (235, 111, 146),
            },
            "solarized" => Self {
                surface_highlight: Color::Rgb(7, 54, 66),
                fg: Color::Rgb(238, 232, 213),
                muted: Color::Rgb(131, 148, 150),
                border: Color::Rgb(88, 110, 117),
                border_focused: Color::Rgb(42, 161, 152),
                primary: Color::Rgb(42, 161, 152),
                secondary: Color::Rgb(38, 139, 210),
                accent: Color::Rgb(181, 137, 0),
                playing: Color::Rgb(133, 153, 0),
                spectrum_low: (38, 139, 210),
                spectrum_mid: (42, 161, 152),
                spectrum_high: (203, 75, 22),
            },
            "amber" => Self {
                surface_highlight: Color::Rgb(52, 36, 12),
                fg: Color::Rgb(255, 216, 140),
                muted: Color::Rgb(166, 124, 56),
                border: Color::Rgb(115, 80, 28),
                border_focused: Color::Rgb(255, 176, 0),
                primary: Color::Rgb(255, 176, 0),
                secondary: Color::Rgb(255, 208, 70),
                accent: Color::Rgb(255, 132, 36),
                playing: Color::Rgb(255, 210, 80),
                spectrum_low: (196, 115, 0),
                spectrum_mid: (255, 176, 0),
                spectrum_high: (255, 232, 135),
            },
            _ => Self::midnight(),
        }
    }

    pub fn midnight() -> Self {
        Self {
            surface_highlight: Color::Rgb(34, 42, 62),
            fg: Color::Rgb(228, 234, 246),
            muted: Color::Rgb(126, 138, 164),
            border: Color::Rgb(70, 82, 110),
            border_focused: Color::Rgb(88, 214, 245),
            primary: Color::Rgb(88, 214, 245),
            secondary: Color::Rgb(164, 132, 255),
            accent: Color::Rgb(255, 134, 178),
            playing: Color::Rgb(106, 236, 168),
            spectrum_low: (72, 198, 239),
            spectrum_mid: (164, 132, 255),
            spectrum_high: (255, 118, 168),
        }
    }

    pub fn next_theme_name(current: &str) -> &'static str {
        let lower = current.trim().to_ascii_lowercase();
        let idx = BUILTIN_THEME_NAMES
            .iter()
            .position(|n| *n == lower)
            .unwrap_or(0);
        BUILTIN_THEME_NAMES[(idx + 1) % BUILTIN_THEME_NAMES.len()]
    }

    /// Interpolates the 3-stop spectrum gradient for normalized height `t` in `[0.0, 1.0]`.
    pub fn spectrum_color(&self, t: f32) -> Color {
        let clamped = t.clamp(0.0, 1.0);
        let (r, g, b) = if clamped < 0.5 {
            lerp_rgb(self.spectrum_low, self.spectrum_mid, clamped * 2.0)
        } else {
            lerp_rgb(self.spectrum_mid, self.spectrum_high, (clamped - 0.5) * 2.0)
        };
        Color::Rgb(r, g, b)
    }

    /// Extracts a vibrant, contrast-safe UI palette directly from an album cover image.
    pub fn from_album_image(img: &DynamicImage) -> Self {
        let small = img.thumbnail(48, 48);
        #[derive(Clone, Copy, Default)]
        struct HueBin {
            weight: f32,
            r_sum: f32,
            g_sum: f32,
            b_sum: f32,
        }

        let mut bins = [HueBin::default(); 24];
        let mut fallback_r = 0.0f32;
        let mut fallback_g = 0.0f32;
        let mut fallback_b = 0.0f32;
        let mut count = 0.0f32;

        for (_, _, px) in small.pixels() {
            let [r, g, b, a] = px.0;
            if a < 128 {
                continue;
            }
            let rf = r as f32 / 255.0;
            let gf = g as f32 / 255.0;
            let bf = b as f32 / 255.0;
            fallback_r += rf;
            fallback_g += gf;
            fallback_b += bf;
            count += 1.0;

            let (h, s, l) = rgb_to_hsl(rf, gf, bf);
            // Ignore near-black, near-white, and near-achromatic grays when looking for vibrant hues
            if l < 0.12 || l > 0.90 || s < 0.16 {
                continue;
            }
            let bin_idx = ((h * 24.0) as usize).min(23);
            let vibrancy = s * (1.0 - (l - 0.52).abs() * 1.3).max(0.15);
            bins[bin_idx].weight += vibrancy;
            bins[bin_idx].r_sum += rf * vibrancy;
            bins[bin_idx].g_sum += gf * vibrancy;
            bins[bin_idx].b_sum += bf * vibrancy;
        }

        let mut ranked: Vec<(usize, HueBin)> = bins
            .into_iter()
            .enumerate()
            .filter(|(_, b)| b.weight > 0.01)
            .collect();
        ranked.sort_by(|a, b| b.1.weight.partial_cmp(&a.1.weight).unwrap_or(std::cmp::Ordering::Equal));

        if ranked.is_empty() {
            // Monochrome/B&W album cover -> crisp silver-ivory & warm accent palette
            let avg = if count > 0.0 {
                ((fallback_r + fallback_g + fallback_b) / (3.0 * count)).clamp(0.45, 0.85)
            } else {
                0.75
            };
            let v = (avg * 255.0) as u8;
            return Self {
                surface_highlight: Color::Rgb(46, 48, 56),
                fg: Color::Rgb(236, 238, 242),
                muted: Color::Rgb(140, 144, 154),
                border: Color::Rgb(92, 96, 108),
                border_focused: Color::Rgb(v, v, v.saturating_add(16)),
                primary: Color::Rgb(220, 224, 236),
                secondary: Color::Rgb(168, 178, 202),
                accent: Color::Rgb(242, 198, 138),
                playing: Color::Rgb(154, 226, 180),
                spectrum_low: (148, 158, 178),
                spectrum_mid: (206, 212, 226),
                spectrum_high: (246, 218, 170),
            };
        }

        let primary_bin = ranked[0];
        let p_rgb = ensure_readable_accent((
            primary_bin.1.r_sum / primary_bin.1.weight,
            primary_bin.1.g_sum / primary_bin.1.weight,
            primary_bin.1.b_sum / primary_bin.1.weight,
        ));

        // Pick secondary hue at least 3 hue bins away for rich visual contrast
        let secondary_rgb = ranked
            .iter()
            .skip(1)
            .find(|(idx, _)| hue_bin_dist(*idx, primary_bin.0) >= 3)
            .map(|(_, b)| ensure_readable_accent((b.r_sum / b.weight, b.g_sum / b.weight, b.b_sum / b.weight)))
            .unwrap_or_else(|| shift_hue_rgb(p_rgb, 0.14));

        let accent_rgb = ranked
            .iter()
            .skip(1)
            .find(|(idx, _)| hue_bin_dist(*idx, primary_bin.0) >= 6)
            .map(|(_, b)| ensure_readable_accent((b.r_sum / b.weight, b.g_sum / b.weight, b.b_sum / b.weight)))
            .unwrap_or_else(|| shift_hue_rgb(p_rgb, 0.78));

        let surface = (
            (p_rgb.0 as u16 / 5 + 22) as u8,
            (p_rgb.1 as u16 / 5 + 24) as u8,
            (p_rgb.2 as u16 / 5 + 30) as u8,
        );
        let border = (
            (p_rgb.0 as u16 / 2 + 34) as u8,
            (p_rgb.1 as u16 / 2 + 36) as u8,
            (p_rgb.2 as u16 / 2 + 42) as u8,
        );

        Self {
            surface_highlight: Color::Rgb(surface.0, surface.1, surface.2),
            fg: Color::Rgb(234, 238, 245),
            muted: Color::Rgb(142, 148, 164),
            border: Color::Rgb(border.0, border.1, border.2),
            border_focused: Color::Rgb(p_rgb.0, p_rgb.1, p_rgb.2),
            primary: Color::Rgb(p_rgb.0, p_rgb.1, p_rgb.2),
            secondary: Color::Rgb(secondary_rgb.0, secondary_rgb.1, secondary_rgb.2),
            accent: Color::Rgb(accent_rgb.0, accent_rgb.1, accent_rgb.2),
            playing: Color::Rgb(p_rgb.0, p_rgb.1, p_rgb.2),
            spectrum_low: secondary_rgb,
            spectrum_mid: p_rgb,
            spectrum_high: accent_rgb,
        }
    }
}

fn hue_bin_dist(a: usize, b: usize) -> usize {
    let diff = a.abs_diff(b);
    diff.min(24 - diff)
}

fn ensure_readable_accent((r, g, b): (f32, f32, f32)) -> (u8, u8, u8) {
    let (h, s, l) = rgb_to_hsl(r, g, b);
    let boosted_s = s.clamp(0.48, 0.92);
    let boosted_l = l.clamp(0.62, 0.78);
    let (nr, ng, nb) = hsl_to_rgb(h, boosted_s, boosted_l);
    (
        (nr * 255.0).round() as u8,
        (ng * 255.0).round() as u8,
        (nb * 255.0).round() as u8,
    )
}

fn shift_hue_rgb((r, g, b): (u8, u8, u8), delta_hue: f32) -> (u8, u8, u8) {
    let (h, s, l) = rgb_to_hsl(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let (nr, ng, nb) = hsl_to_rgb((h + delta_hue).fract(), s.clamp(0.5, 0.9), l.clamp(0.64, 0.78));
    (
        (nr * 255.0).round() as u8,
        (ng * 255.0).round() as u8,
        (nb * 255.0).round() as u8,
    )
}

fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t).round() as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t).round() as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t).round() as u8,
    )
}

fn parse_hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let hex = s.trim().strip_prefix('#').unwrap_or(s.trim());
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}

pub fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    if (max - min).abs() < 1e-5 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < 1e-5 {
        let mut seg = (g - b) / d;
        if g < b {
            seg += 6.0;
        }
        seg / 6.0
    } else if (max - g).abs() < 1e-5 {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h.clamp(0.0, 1.0), s.clamp(0.0, 1.0), l.clamp(0.0, 1.0))
}

pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s.abs() < 1e-5 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    fn hue_to_channel(p: f32, q: f32, mut t: f32) -> f32 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    }
    (
        hue_to_channel(p, q, h + 1.0 / 3.0),
        hue_to_channel(p, q, h),
        hue_to_channel(p, q, h - 1.0 / 3.0),
    )
}
