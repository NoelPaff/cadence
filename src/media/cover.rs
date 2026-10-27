use crate::config::CoverRenderMode;
use crate::media::palette::{hsl_to_rgb, ThemePalette};
use crate::model::Track;
use base64::Engine;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::io::{Cursor, Write};

const ASCII_RAMP: &[u8] = b" .:-=+*#%@";

/// Prepared album artwork with cached palette and terminal graphics state.
pub struct CoverArt {
    pub image: DynamicImage,
    pub extracted_palette: ThemePalette,
    pub is_procedural: bool,
    kitty_active_rect: Option<Rect>,
    cached_resize: Option<(u32, u32, RgbaImage)>,
}

impl CoverArt {
    pub fn from_image(image: DynamicImage) -> Self {
        let extracted_palette = ThemePalette::from_album_image(&image);
        Self {
            image,
            extracted_palette,
            is_procedural: false,
            kitty_active_rect: None,
            cached_resize: None,
        }
    }

    /// Generates an artistic procedural vinyl/gradient sleeve cover seeded by Artist + Album.
    pub fn procedural_for_track(track: &Track) -> Self {
        let mut hash: u64 = 14695981039346656037;
        for b in format!("{}::{}", track.display_artist(), track.display_album()).bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(1099511628211);
        }
        let hue1 = ((hash & 0xFFFF) as f32) / 65535.0;
        let hue2 = (hue1 + 0.19).fract();
        let size = 96u32;
        let mut img = RgbaImage::new(size, size);
        let center = (size as f32 - 1.0) * 0.5;

        for y in 0..size {
            for x in 0..size {
                let u = x as f32 / size as f32;
                let v = y as f32 / size as f32;
                let dx = x as f32 - center;
                let dy = y as f32 - center;
                let dist = (dx * dx + dy * dy).sqrt() / center;

                let (r, g, b) = if dist < 0.84 {
                    if dist < 0.08 {
                        // Spindle hole
                        (0.08, 0.08, 0.11)
                    } else if dist < 0.30 {
                        // Center vinyl label in album hue
                        let blend = (u * 0.6 + v * 0.4).clamp(0.0, 1.0);
                        hsl_to_rgb((hue1 + blend * 0.12).fract(), 0.78, 0.58)
                    } else {
                        // Vinyl grooves with diagonal sheen highlight
                        let groove = ((dist * 54.0).sin() * 0.5 + 0.5) * 0.045;
                        let angle = dy.atan2(dx);
                        let sheen = ((angle * 2.0 + 0.8).cos().max(0.0)).powi(5) * 0.22;
                        let base = 0.11 + groove + sheen;
                        let (hr, hg, hb) = hsl_to_rgb(hue2, 0.65, 0.55);
                        (
                            base + sheen * hr * 0.35,
                            base + sheen * hg * 0.35,
                            base + sheen * hb * 0.42,
                        )
                    }
                } else {
                    // Sleeve background gradient
                    let diag = (u * 0.55 + v * 0.45).clamp(0.0, 1.0);
                    hsl_to_rgb((hue1 + diag * 0.18).fract(), 0.62, 0.24 + diag * 0.18)
                };

                img.put_pixel(
                    x,
                    y,
                    Rgba([
                        (r.clamp(0.0, 1.0) * 255.0) as u8,
                        (g.clamp(0.0, 1.0) * 255.0) as u8,
                        (b.clamp(0.0, 1.0) * 255.0) as u8,
                        255,
                    ]),
                );
            }
        }

        let dyn_img = DynamicImage::ImageRgba8(img);
        let extracted_palette = ThemePalette::from_album_image(&dyn_img);
        Self {
            image: dyn_img,
            extracted_palette,
            is_procedural: true,
            kitty_active_rect: None,
            cached_resize: None,
        }
    }

    /// Computes a centered square-aspect `Rect` inside `area` (1 terminal cell ~ 1:2 width:height).
    pub fn square_aspect_rect(area: Rect) -> Rect {
        if area.width < 4 || area.height < 2 {
            return area;
        }
        // A square image needs `cols ≈ rows * 2`
        let max_rows_by_width = area.width / 2;
        let rows = area.height.min(max_rows_by_width).max(2);
        let cols = (rows * 2).min(area.width);
        let x = area.x + (area.width.saturating_sub(cols)) / 2;
        let y = area.y + (area.height.saturating_sub(rows)) / 2;
        Rect::new(x, y, cols, rows)
    }

    fn resized(&mut self, w: u32, h: u32) -> &RgbaImage {
        let needs_new = self
            .cached_resize
            .as_ref()
            .map(|(cw, ch, _)| *cw != w || *ch != h)
            .unwrap_or(true);
        if needs_new {
            let scaled = self
                .image
                .resize_exact(w.max(1), h.max(1), FilterType::Triangle)
                .to_rgba8();
            self.cached_resize = Some((w, h, scaled));
        }
        &self.cached_resize.as_ref().unwrap().2
    }

    /// Renders the cover art inside `area` using the chosen `CoverRenderMode`.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, mode: CoverRenderMode) {
        let target = Self::square_aspect_rect(area);
        if target.width < 2 || target.height < 2 {
            return;
        }

        if mode != CoverRenderMode::Kitty && self.kitty_active_rect.is_some() {
            clear_kitty_graphics();
            self.kitty_active_rect = None;
        }

        match mode {
            CoverRenderMode::HalfBlock => self.render_half_block(target, buf),
            CoverRenderMode::Ascii => self.render_ascii(target, buf),
            CoverRenderMode::Braille => self.render_braille(target, buf),
            CoverRenderMode::Kitty => {
                // Render half-block underneath as clean fallback if terminal ignores Kitty APC,
                // or blank cells & transmit Kitty placement when target rect changes.
                if kitty_supported_env() {
                    for y in target.y..target.y + target.height {
                        for x in target.x..target.x + target.width {
                            if let Some(cell) = buf.cell_mut((x, y)) {
                                cell.set_symbol(" ");
                                cell.set_bg(Color::Reset);
                            }
                        }
                    }
                    if self.kitty_active_rect != Some(target) {
                        self.transmit_kitty(target);
                        self.kitty_active_rect = Some(target);
                    }
                } else {
                    self.render_half_block(target, buf);
                }
            }
        }
    }

    pub fn clear_kitty_if_active(&mut self) {
        if self.kitty_active_rect.take().is_some() {
            clear_kitty_graphics();
        }
    }

    fn render_half_block(&mut self, target: Rect, buf: &mut Buffer) {
        let pw = target.width as u32;
        let ph = (target.height as u32) * 2;
        let img = self.resized(pw, ph).clone();

        for row in 0..target.height {
            for col in 0..target.width {
                let top = img.get_pixel(col as u32, (row as u32) * 2).0;
                let bot = img.get_pixel(col as u32, (row as u32) * 2 + 1).0;
                if let Some(cell) = buf.cell_mut((target.x + col, target.y + row)) {
                    cell.set_symbol("▀");
                    cell.set_fg(Color::Rgb(top[0], top[1], top[2]));
                    cell.set_bg(Color::Rgb(bot[0], bot[1], bot[2]));
                }
            }
        }
    }

    fn render_ascii(&mut self, target: Rect, buf: &mut Buffer) {
        let pw = target.width as u32;
        let ph = target.height as u32;
        let img = self.resized(pw, ph).clone();

        for row in 0..target.height {
            for col in 0..target.width {
                let [r, g, b, _] = img.get_pixel(col as u32, row as u32).0;
                let luma = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0;
                let idx = ((luma * (ASCII_RAMP.len() - 1) as f32).round() as usize)
                    .min(ASCII_RAMP.len() - 1);
                let ch = ASCII_RAMP[idx] as char;
                // Slightly boost brightness so colored ASCII pops on dark backgrounds
                let br = r.saturating_add(24);
                let bg = g.saturating_add(24);
                let bb = b.saturating_add(24);
                if let Some(cell) = buf.cell_mut((target.x + col, target.y + row)) {
                    let mut s = [0u8; 4];
                    cell.set_symbol(ch.encode_utf8(&mut s));
                    cell.set_fg(Color::Rgb(br, bg, bb));
                    cell.set_bg(Color::Reset);
                }
            }
        }
    }

    fn render_braille(&mut self, target: Rect, buf: &mut Buffer) {
        // Each Braille cell encodes a 2x4 subpixel matrix!
        let pw = (target.width as u32) * 2;
        let ph = (target.height as u32) * 4;
        let img = self.resized(pw, ph).clone();

        const DOT_BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

        for row in 0..target.height {
            for col in 0..target.width {
                let bx = (col as u32) * 2;
                let by = (row as u32) * 4;
                let mut lumas = [[0.0f32; 4]; 2];
                let mut mean_luma = 0.0f32;
                let mut r_acc = 0u32;
                let mut g_acc = 0u32;
                let mut b_acc = 0u32;

                for dx in 0..2u32 {
                    for dy in 0..4u32 {
                        let [r, g, b, _] = img.get_pixel(bx + dx, by + dy).0;
                        let l = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0;
                        lumas[dx as usize][dy as usize] = l;
                        mean_luma += l;
                        r_acc += r as u32;
                        g_acc += g as u32;
                        b_acc += b as u32;
                    }
                }
                mean_luma /= 8.0;
                // Blend global threshold with local contrast so edges & details pop crisply
                let threshold = (mean_luma * 0.65 + 0.16).clamp(0.14, 0.78);
                let mut mask = 0u8;
                for dx in 0..2 {
                    for dy in 0..4 {
                        if lumas[dx][dy] >= threshold {
                            mask |= DOT_BITS[dx][dy];
                        }
                    }
                }
                let ch = char::from_u32(0x2800 + mask as u32).unwrap_or('⠀');
                let r_avg = ((r_acc / 8) as u8).saturating_add(28);
                let g_avg = ((g_acc / 8) as u8).saturating_add(28);
                let b_avg = ((b_acc / 8) as u8).saturating_add(28);

                if let Some(cell) = buf.cell_mut((target.x + col, target.y + row)) {
                    let mut s = [0u8; 4];
                    cell.set_symbol(ch.encode_utf8(&mut s));
                    cell.set_fg(Color::Rgb(r_avg, g_avg, b_avg));
                    cell.set_bg(Color::Reset);
                }
            }
        }
    }

    fn transmit_kitty(&self, target: Rect) {
        let thumb = self.image.resize_exact(
            (target.width as u32 * 10).min(512),
            (target.height as u32 * 20).min(512),
            FilterType::Lanczos3,
        );
        let mut png_bytes = Vec::new();
        if thumb
            .write_to(&mut Cursor::new(&mut png_bytes), ImageFormat::Png)
            .is_err()
        {
            return;
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
        let mut stdout = std::io::stdout().lock();
        // Delete previous placement ID 42, move cursor to target top-left (1-based), send chunks
        let _ = write!(
            stdout,
            "\x1b_Ga=d,d=i,i=42,q=2\x1b\\\x1b[{};{}H",
            target.y + 1,
            target.x + 1
        );
        let bytes = encoded.as_bytes();
        let total_chunks = bytes.chunks(4096).len();
        for (idx, chunk) in bytes.chunks(4096).enumerate() {
            let more = if idx + 1 < total_chunks { 1 } else { 0 };
            let chunk_str = std::str::from_utf8(chunk).unwrap_or_default();
            if idx == 0 {
                let _ = write!(
                    stdout,
                    "\x1b_Ga=T,f=100,i=42,c={},r={},C=1,q=2,m={};{}\x1b\\",
                    target.width, target.height, more, chunk_str
                );
            } else {
                let _ = write!(stdout, "\x1b_Gm={};{}\x1b\\", more, chunk_str);
            }
        }
        let _ = stdout.flush();
    }
}

pub fn kitty_supported_env() -> bool {
    if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        return true;
    }
    let term = std::env::var("TERM").unwrap_or_default().to_ascii_lowercase();
    let prog = std::env::var("TERM_PROGRAM")
        .unwrap_or_default()
        .to_ascii_lowercase();
    term.contains("kitty") || prog.contains("kitty") || prog.contains("ghostty") || prog.contains("wezterm")
}

pub fn clear_kitty_graphics() {
    if kitty_supported_env() {
        let mut stdout = std::io::stdout().lock();
        let _ = write!(stdout, "\x1b_Ga=d,d=i,i=42,q=2\x1b\\");
        let _ = stdout.flush();
    }
}
