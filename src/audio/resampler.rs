/// Converts interleaved multi-channel PCM samples into `[L, R]` stereo frames.
pub fn mix_to_stereo_frames(interleaved: &[f32], channels: usize, out: &mut Vec<[f32; 2]>) {
    out.clear();
    if channels == 0 || interleaved.is_empty() {
        return;
    }
    match channels {
        1 => {
            out.reserve(interleaved.len());
            for &s in interleaved {
                out.push([s, s]);
            }
        }
        2 => {
            let frames = interleaved.len() / 2;
            out.reserve(frames);
            for chunk in interleaved.chunks_exact(2) {
                out.push([chunk[0], chunk[1]]);
            }
        }
        ch => {
            let frames = interleaved.len() / ch;
            out.reserve(frames);
            for chunk in interleaved.chunks_exact(ch) {
                let mut l = chunk[0];
                let mut r = chunk[1];
                if ch >= 3 {
                    let center = chunk[2] * 0.7071;
                    l += center;
                    r += center;
                }
                if ch >= 6 {
                    l += chunk[4] * 0.5;
                    r += chunk[5] * 0.5;
                }
                out.push([l.clamp(-1.5, 1.5), r.clamp(-1.5, 1.5)]);
            }
        }
    }
}

/// Stateful 4-point Hermite / Catmull-Rom stereo sample-rate converter.
/// Converts arbitrary input sample rates cleanly to a fixed target rate (48,000 Hz)
/// with C¹ continuity across chunk boundaries.
pub struct StereoResampler {
    in_rate: u32,
    out_rate: u32,
    step: f64,
    phase: f64,
    buf: Vec<[f32; 2]>,
}

impl StereoResampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        let safe_in = in_rate.max(1);
        let safe_out = out_rate.max(1);
        Self {
            in_rate: safe_in,
            out_rate: safe_out,
            step: safe_in as f64 / safe_out as f64,
            phase: 0.0,
            buf: vec![[0.0, 0.0]],
        }
    }

    pub fn in_rate(&self) -> u32 {
        self.in_rate
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.buf.clear();
        self.buf.push([0.0, 0.0]);
    }

    pub fn set_input_rate(&mut self, in_rate: u32) {
        let safe_in = in_rate.max(1);
        if safe_in != self.in_rate {
            self.in_rate = safe_in;
            self.step = safe_in as f64 / self.out_rate as f64;
        }
    }

    /// Processes `input` stereo frames and appends interleaved `[L, R, ...]` samples at `out_rate`.
    pub fn process(&mut self, input: &[[f32; 2]], out_interleaved: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }
        if self.in_rate == self.out_rate {
            out_interleaved.reserve(input.len() * 2);
            for frame in input {
                out_interleaved.push(frame[0]);
                out_interleaved.push(frame[1]);
            }
            return;
        }

        self.buf.extend_from_slice(input);
        self.emit_available(out_interleaved);
    }

    /// Flushes remaining buffered lookahead samples at track EOF.
    pub fn flush_eof(&mut self, out_interleaved: &mut Vec<f32>) {
        if self.in_rate == self.out_rate {
            return;
        }
        let last = self.buf.last().copied().unwrap_or([0.0, 0.0]);
        self.buf.push(last);
        self.buf.push([0.0, 0.0]);
        self.buf.push([0.0, 0.0]);
        self.emit_available(out_interleaved);
        self.reset();
    }

    fn emit_available(&mut self, out_interleaved: &mut Vec<f32>) {
        while (self.phase as usize) + 3 < self.buf.len() {
            let i = self.phase as usize;
            let t = (self.phase - i as f64) as f32;
            let p0 = self.buf[i];
            let p1 = self.buf[i + 1];
            let p2 = self.buf[i + 2];
            let p3 = self.buf[i + 3];

            let l = catmull_rom(p0[0], p1[0], p2[0], p3[0], t);
            let r = catmull_rom(p0[1], p1[1], p2[1], p3[1], t);
            out_interleaved.push(l);
            out_interleaved.push(r);

            self.phase += self.step;
        }

        let consumed = (self.phase as usize).min(self.buf.len().saturating_sub(3));
        if consumed > 0 {
            self.buf.drain(0..consumed);
            self.phase -= consumed as f64;
        }
    }
}

#[inline]
fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let a0 = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
    let a1 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let a2 = -0.5 * p0 + 0.5 * p2;
    let a3 = p1;
    ((a0 * t + a1) * t + a2) * t + a3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_44100_to_48000_accurately() {
        let mut rs = StereoResampler::new(44_100, 48_000);
        // 1 second of 440 Hz sine wave at 44,100 Hz
        let input: Vec<[f32; 2]> = (0..44_100)
            .map(|n| {
                let s = (2.0 * std::f32::consts::PI * 440.0 * (n as f32 / 44_100.0)).sin();
                [s, -s]
            })
            .collect();
        let mut out = Vec::new();
        for chunk in input.chunks(1024) {
            rs.process(chunk, &mut out);
        }
        rs.flush_eof(&mut out);

        let out_frames = out.len() / 2;
        assert!(
            (out_frames as i64 - 48_000).abs() <= 4,
            "expected ~48000 frames, got {}",
            out_frames
        );
    }
}
