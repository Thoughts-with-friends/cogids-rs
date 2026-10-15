//! Emulation speed (0.5x - 4x): the choices, cycling through them with the
//! gamepad's left stick button, and fitting the sound to the speed.
//!
//! ```text
//!  View::speed (percent) --> egui: frame debt grows by DS_FPS * speed
//!                        --> Tauri: frame time = 1 / (DS_FPS * speed)
//!  one frame's samples   --> stretch_audio() --> AudioOutput::push
//! ```
//!
//! Speeds are kept as whole percents so they compare exactly.

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

/// Resamples one frame of interleaved stereo samples to its real duration
/// at `percent` speed.
///
/// The pitch rises when faster and drops when slower; at normal speed the
/// samples are passed through unchanged.
pub fn stretch_audio(samples: &[i16], percent: u16, out: &mut Vec<i16>) {
    out.clear();
    let percent = valid(percent);
    if percent == NORMAL {
        out.extend_from_slice(samples);
        return;
    }
    let frames = samples.len() / 2;
    let wanted = frames * 100 / usize::from(percent);
    for i in 0..wanted {
        let src = (i * usize::from(percent) / 100).min(frames.saturating_sub(1));
        out.extend_from_slice(&samples[src * 2..src * 2 + 2]);
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
    fn audio_follows_speed() {
        let frame: Vec<i16> = (0..1600).collect(); // 800 stereo frames
        let mut out = Vec::new();
        stretch_audio(&frame, NORMAL, &mut out);
        assert_eq!(out, frame);
        stretch_audio(&frame, 200, &mut out);
        assert_eq!(out.len(), 800);
        assert_eq!(&out[..4], &[0, 1, 4, 5]);
        stretch_audio(&frame, 50, &mut out);
        assert_eq!(out.len(), 3200);
        stretch_audio(&[], 400, &mut out);
        assert!(out.is_empty());
    }
}
