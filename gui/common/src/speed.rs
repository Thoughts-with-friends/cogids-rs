//! Emulation speed (0.5x - 4x): the choices, cycling through them with the
//! gamepad's left stick button, and fitting the sound to the speed.
//!
//! ```text
//!  View::speed (percent) --> egui: frame debt grows by DS_FPS * speed
//!                        --> emu thread: frame time = 1 / (DS_FPS * speed)
//!  frames run per real second --> SpeedMeter::tick() --> measured speed
//!  each frame's samples  --> TimeStretch::process(ratio) --> AudioOutput::push
//! ```
//!
//! Faster or slower than 1x the sound keeps its pitch and only its tempo
//! changes (like melonDS): the stretcher cuts it into ~37 ms grains and
//! overlaps them closer together (faster) or further apart (slower). The
//! ratio is the speed the emulator really reaches, not the chosen one, so
//! when a heavy frame (e.g. a high internal resolution) keeps it below the
//! target the sound still matches the game instead of running dry.
//!
//! Speeds are kept as whole percents so they compare exactly.
use std::time::Instant;

/// Speeds offered, in percent of the DS's own.
pub const SPEEDS: [u16; 6] = [50, 100, 150, 200, 300, 400];

/// Normal speed.
pub const NORMAL: u16 = 100;

/// `percent` if it is one of [`SPEEDS`], otherwise normal speed.
pub fn valid(percent: u16) -> u16 {
    if SPEEDS.contains(&percent) {
        percent
    } else {
        NORMAL
    }
}

/// The speed after `percent` (wrapping from 4x back to 0.5x).
pub fn next(percent: u16) -> u16 {
    let i = SPEEDS
        .iter()
        .position(|&s| s == valid(percent))
        .unwrap_or(1);
    SPEEDS[(i + 1) % SPEEDS.len()]
}

/// "0.5x", "1x", "1.5x", ...
pub fn label(percent: u16) -> String {
    let whole = percent / 100;
    match percent % 100 {
        0 => format!("{whole}x"),
        frac if frac % 10 == 0 => format!("{whole}.{}x", frac / 10),
        frac => format!("{whole}.{frac:02}x"),
    }
}

/// Frames per second at `percent` speed.
pub fn fps(percent: u16) -> f64 {
    crate::DS_FPS * f64::from(valid(percent)) / 100.0
}

/// Measures how fast emulation really runs (emulated seconds per real
/// second), smoothed over about half a second.
#[derive(Debug, Clone)]
pub struct SpeedMeter {
    last: Option<Instant>,
    speed: f64,
}

/// Smoothing time of [`SpeedMeter`] in seconds.
const METER_SECS: f64 = 0.5;

impl SpeedMeter {
    /// A meter assuming `percent` until measured.
    pub fn new(percent: u16) -> Self {
        Self {
            last: None,
            speed: f64::from(valid(percent)) / 100.0,
        }
    }

    /// Starts over (after a pause or a speed change).
    pub fn reset(&mut self, percent: u16) {
        *self = Self::new(percent);
    }

    /// `frames` were emulated since the last tick; returns the speed.
    pub fn tick(&mut self, frames: u32) -> f64 {
        let now = Instant::now();
        if let Some(last) = self.last {
            let dt = now.duration_since(last).as_secs_f64();
            if dt > 0.0 {
                let now_speed = f64::from(frames) / crate::DS_FPS / dt;
                self.speed =
                    (now_speed - self.speed).mul_add((dt / METER_SECS).min(1.0), self.speed);
            }
        }
        self.last = Some(now);
        self.speed
    }

    /// The last measured speed.
    pub const fn speed(&self) -> f64 {
        self.speed
    }
}

/// Tempo ratio for the sound at `percent` speed when the emulator
/// measures `measured`: what it really reaches, never above the target.
pub fn stretch_ratio(percent: u16, measured: f64) -> f64 {
    measured.clamp(0.25, f64::from(valid(percent)) / 100.0)
}

/// Frames of one grain (~37 ms at the SPU rate).
const GRAIN: usize = 1200;
/// Frames cross-faded between grains (~7 ms).
const OVERLAP: usize = 240;
/// Each grain may start up to this many frames (~9 ms) away from its
/// nominal position, where it best continues the previous one.
const SEEK: usize = 300;
/// Output frames per grain.
const HOP: usize = GRAIN - OVERLAP;

/// Changes the tempo of interleaved stereo sound without changing its
/// pitch (WSOLA: waveform-similarity overlap-add).
///
/// ```text
///  input  |--grain k--|            nominal starts HOP * ratio apart,
///               |--grain k+1--|    each moved by up to SEEK frames to
///                                  where its start resembles `tail`
///  output |--k--|xx|--k+1--|xx|    xx = OVERLAP frames cross-faded
/// ```
///
/// Kept across frames: one emulated frame (~547 frames) is shorter than a
/// grain, so stretching each frame alone would buzz at 60 Hz.
#[derive(Debug, Clone)]
pub struct TimeStretch {
    input: Vec<[f32; 2]>,
    /// Nominal start of the next grain in `input` (always >= SEEK).
    pos: f64,
    /// Input that follows the last grain written, faded into the next.
    tail: Vec<[f32; 2]>,
    /// Scratch: mono input around the next grain's start.
    mono: Vec<f32>,
}

impl Default for TimeStretch {
    fn default() -> Self {
        Self {
            input: Vec::new(),
            pos: SEEK as f64,
            tail: Vec::new(),
            mono: Vec::new(),
        }
    }
}

impl TimeStretch {
    /// Drops all buffered sound (after a pause, at 1x, on stop).
    pub fn reset(&mut self) {
        self.input.clear();
        self.tail.clear();
        self.pos = SEEK as f64;
    }

    /// Appends `samples` and writes to `out` (cleared first) the sound
    /// stretched so far, `ratio` times faster (0.25 - 8).
    pub fn process(&mut self, samples: &[i16], ratio: f64, out: &mut Vec<i16>) {
        out.clear();
        let ratio = ratio.clamp(0.25, 8.0);
        self.input.extend(
            samples
                .as_chunks::<2>()
                .0
                .iter()
                .map(|s| [f32::from(s[0]), f32::from(s[1])]),
        );
        let push = |out: &mut Vec<i16>, s: [f32; 2]| {
            out.extend(s.map(|v| v.round().clamp(-32768.0, 32767.0) as i16));
        };
        while self.pos as usize + SEEK + GRAIN <= self.input.len() {
            let nominal = self.pos as usize;
            let start = if self.tail.is_empty() {
                nominal
            } else {
                self.best_start(nominal)
            };
            let grain = &self.input[start..start + GRAIN];
            for (i, &s) in grain[..OVERLAP].iter().enumerate() {
                let s = match self.tail.get(i) {
                    Some(t) => {
                        let w = (i as f32 + 0.5) / OVERLAP as f32;
                        [
                            f32::mul_add(s[0] - t[0], w, t[0]),
                            f32::mul_add(s[1] - t[1], w, t[1]),
                        ]
                    }
                    None => s,
                };
                push(out, s);
            }
            for &s in &grain[OVERLAP..HOP] {
                push(out, s);
            }
            self.tail.clear();
            self.tail.extend_from_slice(&grain[HOP..]);
            self.pos = (HOP as f64).mul_add(ratio, self.pos);
        }
        // Keep only what the next grain's search can still reach.
        let done = (self.pos as usize)
            .saturating_sub(SEEK)
            .min(self.input.len());
        if done > 0 {
            self.input.drain(..done);
            self.pos -= done as f64;
        }
    }

    /// The start within `nominal` +- SEEK whose first OVERLAP frames are
    /// most like `tail` (normalised cross-correlation of the mono sums).
    fn best_start(&mut self, nominal: usize) -> usize {
        let from = nominal - SEEK;
        self.mono.clear();
        self.mono.extend(
            self.input[from..nominal + SEEK + OVERLAP]
                .iter()
                .map(|s| s[0] + s[1]),
        );
        let tail: Vec<f32> = self.tail[..OVERLAP].iter().map(|s| s[0] + s[1]).collect();
        let mut energy: f32 = self.mono[..OVERLAP].iter().map(|v| v * v).sum();
        let (mut best, mut best_score) = (SEEK, f32::MIN);
        for c in 0..=2 * SEEK {
            if c > 0 {
                let (old, new) = (self.mono[c - 1], self.mono[c + OVERLAP - 1]);
                energy = f32::mul_add(new, new, f32::mul_add(old, -old, energy)).max(0.0);
            }
            let dot: f32 = tail
                .iter()
                .zip(&self.mono[c..c + OVERLAP])
                .map(|(a, b)| a * b)
                .sum();
            let score = dot / (energy + 1.0).sqrt();
            if score > best_score {
                (best, best_score) = (c, score);
            }
        }
        from + best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_through_all_speeds() {
        let mut s = NORMAL;
        let mut seen = Vec::new();
        for _ in 0..SPEEDS.len() {
            s = next(s);
            seen.push(s);
        }
        assert_eq!(seen, [150, 200, 300, 400, 50, 100]);
        assert_eq!(next(123), 150, "unknown speeds count as 1x");
    }

    #[test]
    fn labels() {
        let labels: Vec<String> = SPEEDS.iter().map(|&s| label(s)).collect();
        assert_eq!(labels, ["0.5x", "1x", "1.5x", "2x", "3x", "4x"]);
    }

    #[test]
    fn ratio_follows_the_real_speed() {
        assert_eq!(stretch_ratio(200, 2.0), 2.0);
        // Too slow for 2x: the sound follows what is reached.
        assert_eq!(stretch_ratio(200, 1.3), 1.3);
        // Never faster than chosen (catching up after a stall).
        assert_eq!(stretch_ratio(200, 3.0), 2.0);
        assert_eq!(stretch_ratio(50, 0.5), 0.5);
    }

    /// A 440 Hz stereo sine at the SPU rate, fed one emulated frame at a
    /// time, then stretched at `ratio`: returns (input, output) frames and
    /// the output's frequency from its zero crossings.
    fn stretch_sine(ratio: f64) -> (usize, usize, f64) {
        let rate = ds_spu::OUTPUT_RATE_HZ;
        let per_frame = 547;
        let mut ts = TimeStretch::default();
        let (mut t, mut out_all, mut chunk) = (0_usize, Vec::new(), Vec::new());
        let mut out = Vec::new();
        for _ in 0..300 {
            chunk.clear();
            for _ in 0..per_frame {
                let v =
                    (f64::sin(t as f64 * 440.0 * std::f64::consts::TAU / rate) * 12000.0) as i16;
                chunk.extend([v, v]);
                t += 1;
            }
            ts.process(&chunk, ratio, &mut out);
            out_all.extend_from_slice(&out);
        }
        let left: Vec<i16> = out_all.iter().step_by(2).copied().collect();
        let skip = GRAIN; // past the first grain
        let rising = left[skip..]
            .windows(2)
            .filter(|w| w[0] < 0 && w[1] >= 0)
            .count();
        let secs = (left.len() - skip) as f64 / rate;
        (t, left.len(), rising as f64 / secs)
    }

    #[test]
    fn faster_keeps_the_pitch() {
        let (input, output, freq) = stretch_sine(2.0);
        let expected = input as f64 / 2.0;
        assert!(
            (output as f64 - expected).abs() < (GRAIN + SEEK) as f64,
            "{output} frames for {input} at 2x"
        );
        assert!((freq - 440.0).abs() < 440.0 * 0.03, "{freq} Hz");
    }

    #[test]
    fn slower_keeps_the_pitch() {
        let (input, output, freq) = stretch_sine(0.5);
        let expected = input as f64 * 2.0;
        assert!(
            (output as f64 - expected).abs() < (GRAIN + SEEK) as f64 * 2.0,
            "{output} frames for {input} at 0.5x"
        );
        assert!((freq - 440.0).abs() < 440.0 * 0.03, "{freq} Hz");
    }

    #[test]
    fn reset_drops_buffered_sound() {
        let mut ts = TimeStretch::default();
        let mut out = Vec::new();
        ts.process(&[1000; 1094], 2.0, &mut out);
        ts.reset();
        ts.process(&[], 2.0, &mut out);
        assert!(out.is_empty());
        assert!(ts.input.is_empty());
    }
}
