use crate::audio::sink::ENGINE_SAMPLE_RATE;
use crate::config::VisualizerStyle;
use crate::media::palette::ThemePalette;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::{Arc, Mutex};

const FFT_SIZE: usize = 2048;
const RING_FRAMES: usize = 4096;
const BAR_BLOCKS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

struct RingInner {
    mono: [f32; RING_FRAMES],
    write_pos: usize,
}

/// Lock-light circular audio tap shared between the audio output thread and the TUI analyzer.
#[derive(Clone)]
pub struct VisualizerTap {
    inner: Arc<Mutex<RingInner>>,
}

impl Default for VisualizerTap {
    fn default() -> Self {
        Self::new()
    }
}

impl VisualizerTap {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RingInner {
                mono: [0.0; RING_FRAMES],
                write_pos: 0,
            })),
        }
    }

    /// Pushes interleaved stereo `[L, R, ...]` samples into the circular visual buffer.
    pub fn push_stereo(&self, samples: &[f32]) {
        let Ok(mut guard) = self.inner.try_lock() else {
            return;
        };
        let mut wp = guard.write_pos;
        for pair in samples.chunks_exact(2) {
            guard.mono[wp] = (pair[0] + pair[1]) * 0.5;
            wp = (wp + 1) % RING_FRAMES;
        }
        guard.write_pos = wp;
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.mono.fill(0.0);
        }
    }

    pub fn read_latest(&self, out: &mut [f32; FFT_SIZE]) {
        if let Ok(guard) = self.inner.lock() {
            let start = (guard.write_pos + RING_FRAMES - FFT_SIZE) % RING_FRAMES;
            for (i, dst) in out.iter_mut().enumerate() {
                *dst = guard.mono[(start + i) % RING_FRAMES];
            }
        }
    }
}

/// Real-time 2048-point FFT spectrum analyzer with Hann windowing, log-frequency binning,
/// pink-noise treble tilt compensation, smooth decay, and falling peak caps.
pub struct SpectrumAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    hann_window: Vec<f32>,
    time_buf: [f32; FFT_SIZE],
    fft_buf: Vec<Complex<f32>>,
    smoothed_bars: Vec<f32>,
    peaks: Vec<f32>,
    peak_vel: Vec<f32>,
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl SpectrumAnalyzer {
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let hann_window: Vec<f32> = (0..FFT_SIZE)
            .map(|n| {
                0.5 * (1.0
                    - (2.0 * std::f32::consts::PI * n as f32 / (FFT_SIZE - 1) as f32).cos())
            })
            .collect();

        Self {
            fft,
            hann_window,
            time_buf: [0.0; FFT_SIZE],
            fft_buf: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            smoothed_bars: vec![0.0; 128],
            peaks: vec![0.0; 128],
            peak_vel: vec![0.0; 128],
        }
    }

    /// Updates spectrum magnitudes for `num_bars` columns. When `paused`, bars gently decay to rest.
    pub fn update(&mut self, tap: &VisualizerTap, num_bars: usize, active: bool) {
        let n = num_bars.clamp(4, 160);
        if self.smoothed_bars.len() != n {
            self.smoothed_bars.resize(n, 0.0);
            self.peaks.resize(n, 0.0);
            self.peak_vel.resize(n, 0.0);
        }

        if !active {
            for i in 0..n {
                self.smoothed_bars[i] *= 0.80;
                if self.smoothed_bars[i] < 0.005 {
                    self.smoothed_bars[i] = 0.0;
                }
                self.peak_vel[i] += 0.006;
                self.peaks[i] = (self.peaks[i] - self.peak_vel[i]).max(self.smoothed_bars[i]);
            }
            return;
        }

        tap.read_latest(&mut self.time_buf);
        for i in 0..FFT_SIZE {
            self.fft_buf[i] = Complex::new(self.time_buf[i] * self.hann_window[i], 0.0);
        }
        self.fft.process(&mut self.fft_buf);

        let nyquist_bins = FFT_SIZE / 2;
        let min_freq = 32.0f32;
        let max_freq = 16_500.0f32;
        let bin_hz = ENGINE_SAMPLE_RATE as f32 / FFT_SIZE as f32;
        let norm = 2.0 / FFT_SIZE as f32;

        for bar in 0..n {
            let t0 = bar as f32 / n as f32;
            let t1 = (bar + 1) as f32 / n as f32;
            let f0 = min_freq * (max_freq / min_freq).powf(t0);
            let f1 = min_freq * (max_freq / min_freq).powf(t1);

            let b0 = ((f0 / bin_hz).floor() as usize).clamp(1, nyquist_bins - 1);
            let b1 = ((f1 / bin_hz).ceil() as usize).clamp(b0 + 1, nyquist_bins);

            let mut energy = 0.0f32;
            for b in b0..b1 {
                let mag = self.fft_buf[b].norm() * norm;
                if mag > energy {
                    energy = mag;
                }
            }

            // Perceptual pink-noise tilt compensation so mids/highs stay lively
            let center_f = (f0 * f1).sqrt();
            let tilt = (center_f / 220.0).powf(0.30).clamp(0.75, 3.6);
            let scaled = (energy * tilt).max(1e-6);
            let db = 20.0 * scaled.log10();
            let target = ((db + 52.0) / 46.0).clamp(0.0, 1.0);

            let prev = self.smoothed_bars[bar];
            let next = if target > prev {
                prev + (target - prev) * 0.62
            } else {
                prev * 0.78 + target * 0.22
            };
            self.smoothed_bars[bar] = next;

            if next >= self.peaks[bar] {
                self.peaks[bar] = next;
                self.peak_vel[bar] = 0.002;
            } else {
                self.peak_vel[bar] = (self.peak_vel[bar] + 0.0045).min(0.06);
                self.peaks[bar] = (self.peaks[bar] - self.peak_vel[bar]).max(next);
            }
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, style: VisualizerStyle, palette: &ThemePalette) {
        if area.width < 4 || area.height < 2 || self.smoothed_bars.is_empty() {
            return;
        }
        match style {
            VisualizerStyle::Bars => self.render_bars(area, buf, palette),
            VisualizerStyle::Mirrored => self.render_mirrored(area, buf, palette),
            VisualizerStyle::Braille => self.render_braille(area, buf, palette),
            VisualizerStyle::Led => self.render_led(area, buf, palette),
        }
    }

    fn sample_bar(&self, col: usize, total_cols: usize) -> (f32, f32) {
        let len = self.smoothed_bars.len();
        let idx = ((col * len) / total_cols.max(1)).min(len - 1);
        (self.smoothed_bars[idx], self.peaks[idx])
    }

    fn render_bars(&self, area: Rect, buf: &mut Buffer, palette: &ThemePalette) {
        let cols = (area.width as usize) / 2;
        if cols == 0 {
            return;
        }
        let x_pad = (area.width.saturating_sub((cols * 2) as u16)) / 2;
        let sub_rows = (area.height as f32) * 8.0;

        for c in 0..cols {
            let (val, peak) = self.sample_bar(c, cols);
            let filled_eighths = (val * sub_rows).round() as u16;
            let peak_row_from_bottom = ((peak * (area.height as f32 - 0.01)).floor() as u16)
                .min(area.height.saturating_sub(1));
            let x = area.x + x_pad + (c as u16) * 2;

            for r_from_bottom in 0..area.height {
                let y = area.y + area.height - 1 - r_from_bottom;
                let row_base = r_from_bottom * 8;
                let h_ratio = (r_from_bottom as f32 + 0.5) / area.height as f32;
                let color = palette.spectrum_color(h_ratio);

                let sym = if filled_eighths >= row_base + 8 {
                    "█"
                } else if filled_eighths > row_base {
                    BAR_BLOCKS[(filled_eighths - row_base) as usize]
                } else if peak > 0.06 && r_from_bottom == peak_row_from_bottom {
                    "▔"
                } else {
                    " "
                };

                if sym != " " {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_symbol(sym);
                        cell.set_fg(if sym == "▔" { palette.accent } else { color });
                        cell.set_bg(Color::Reset);
                    }
                }
            }
        }
    }

    fn render_mirrored(&self, area: Rect, buf: &mut Buffer, palette: &ThemePalette) {
        let width = area.width as usize;
        let mid_y = area.y + area.height / 2;
        let half_h = (area.height / 2).max(1);

        for col in 0..width {
            // Symmetric low-in-center -> highs-at-edges mapping
            let center = (width as f32 - 1.0) * 0.5;
            let dist_norm = ((col as f32 - center).abs() / center.max(1.0)).clamp(0.0, 1.0);
            let (val, _) = self.sample_bar((dist_norm * 63.0) as usize, 64);
            let span = (val * half_h as f32 * 8.0).round() as u16;
            let x = area.x + col as u16;

            for step in 0..half_h {
                let base = step * 8;
                let sym_up = if span >= base + 8 {
                    "█"
                } else if span > base {
                    BAR_BLOCKS[(span - base) as usize]
                } else {
                    " "
                };
                let color = palette.spectrum_color((step as f32 + 0.4) / half_h as f32);

                if sym_up != " " {
                    let y_up = mid_y.saturating_sub(1 + step);
                    if y_up >= area.y {
                        if let Some(cell) = buf.cell_mut((x, y_up)) {
                            cell.set_symbol(sym_up);
                            cell.set_fg(color);
                        }
                    }
                    let y_dn = mid_y + step;
                    if y_dn < area.y + area.height {
                        if let Some(cell) = buf.cell_mut((x, y_dn)) {
                            cell.set_symbol(if sym_up == "█" { "█" } else { "▀" });
                            cell.set_fg(color);
                        }
                    }
                }
            }
        }
    }

    fn render_braille(&self, area: Rect, buf: &mut Buffer, palette: &ThemePalette) {
        const DOT_BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
        let sub_w = (area.width as usize) * 2;
        let sub_h = (area.height as usize) * 4;

        for cx in 0..area.width as usize {
            let (v0, _) = self.sample_bar(cx * 2, sub_w);
            let (v1, _) = self.sample_bar(cx * 2 + 1, sub_w);
            let h0 = (v0 * sub_h as f32).round() as usize;
            let h1 = (v1 * sub_h as f32).round() as usize;

            for cy in 0..area.height as usize {
                let mut mask = 0u8;
                for dy in 0..4 {
                    let global_from_bottom = (area.height as usize - 1 - cy) * 4 + (3 - dy);
                    if global_from_bottom < h0 {
                        mask |= DOT_BITS[0][dy];
                    }
                    if global_from_bottom < h1 {
                        mask |= DOT_BITS[1][dy];
                    }
                }
                if mask != 0 {
                    let ch = char::from_u32(0x2800 + mask as u32).unwrap_or('⠀');
                    let ratio = 1.0 - (cy as f32 / area.height.max(1) as f32);
                    if let Some(cell) = buf.cell_mut((area.x + cx as u16, area.y + cy as u16)) {
                        let mut s = [0u8; 4];
                        cell.set_symbol(ch.encode_utf8(&mut s));
                        cell.set_fg(palette.spectrum_color(ratio));
                    }
                }
            }
        }
    }

    fn render_led(&self, area: Rect, buf: &mut Buffer, palette: &ThemePalette) {
        let cols = (area.width as usize) / 3;
        if cols == 0 {
            return;
        }
        let x_pad = (area.width.saturating_sub((cols * 3) as u16)) / 2;

        for c in 0..cols {
            let (val, peak) = self.sample_bar(c, cols);
            let lit_rows = (val * area.height as f32).round() as u16;
            let peak_row = ((peak * (area.height as f32 - 0.01)).floor() as u16)
                .min(area.height.saturating_sub(1));
            let x = area.x + x_pad + (c as u16) * 3;

            for r in 0..area.height {
                let y = area.y + area.height - 1 - r;
                let active_cell = r < lit_rows || (peak > 0.08 && r == peak_row);
                if active_cell {
                    let color = palette.spectrum_color((r as f32 + 0.5) / area.height as f32);
                    for dx in 0..2 {
                        if let Some(cell) = buf.cell_mut((x + dx, y)) {
                            cell.set_symbol("▬");
                            cell.set_fg(color);
                        }
                    }
                }
            }
        }
    }
}
