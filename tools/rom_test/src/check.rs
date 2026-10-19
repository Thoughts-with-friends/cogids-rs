//! Turns a case's [`Expect`] and the baseline into pass / fail results.
use crate::config::Case;
use crate::run::{Metrics, TERMINATE_WINDOW};
use std::collections::BTreeMap;

/// Audio frames per video frame (32728.5 Hz / 59.826 fps).
const AUDIO_PER_FRAME: f64 = 547.06;

/// One check of one case.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    /// Informational checks never fail a case.
    pub info: bool,
    pub detail: String,
}

fn check(name: &str, ok: bool, detail: String) -> Check {
    Check { name: name.to_owned(), ok, info: false, detail }
}

/// `"0xADDR[:WIDTH]=0xVALUE"` -> (address, value, width 8 / 16 / 32);
/// decimal also accepted, width defaults to 32.
pub fn parse_ram_width(entry: &str) -> Option<(u32, u32, u32)> {
    let (left, value) = entry.split_once('=')?;
    let (addr, width) = match left.split_once(':') {
        Some((a, w)) => (a, w.trim().parse().ok().filter(|w| [8, 16, 32].contains(w))?),
        None => (left, 32),
    };
    let (a, v) = parse_ram(&format!("{addr}={value}"))?;
    Some((a, v, width))
}

/// `"0xADDR=0xVALUE"` (decimal also accepted).
pub fn parse_ram(entry: &str) -> Option<(u32, u32)> {
    let num = |s: &str| {
        let s = s.trim();
        s.strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .map_or_else(|| s.parse().ok(), |h| u32::from_str_radix(h, 16).ok())
    };
    let (a, v) = entry.split_once('=')?;
    Some((num(a)?, num(v)?))
}

/// `"SCREEN:X:Y=0xRRGGBB"` -> (lower screen?, x, y, color).
pub fn parse_pixel(entry: &str) -> Option<(bool, usize, usize, u32)> {
    let (pos, color) = entry.split_once('=')?;
    let mut it = pos.split(':');
    let lower = match it.next()? {
        "upper" => false,
        "lower" => true,
        _ => return None,
    };
    let x: usize = it.next()?.trim().parse().ok()?;
    let y: usize = it.next()?.trim().parse().ok()?;
    let c = color.trim();
    let c = u32::from_str_radix(c.strip_prefix("0x").or_else(|| c.strip_prefix('#')).unwrap_or(c), 16).ok()?;
    (x < 256 && y < 192).then_some((lower, x, y, c))
}

fn pixels(m: &Metrics, out: &mut Vec<Check>) {
    for (entry, got) in &m.pixels {
        let want = parse_pixel(entry).map(|p| p.3);
        let ok = want.is_some() && want == *got;
        let detail = match got {
            Some(g) => format!("{entry}: got {g:#08x}"),
            None => format!("{entry}: bad entry (SCREEN:X:Y=0xRRGGBB)"),
        };
        out.push(check("pixel", ok, detail));
    }
}

fn screens(case: &Case, m: &Metrics, out: &mut Vec<Check>) {
    for name in &case.expect.not_blank {
        let i = usize::from(name == "lower");
        let ok = (name == "upper" || name == "lower") && !m.blank[i];
        out.push(check("not_blank", ok, format!("{name}: {}", if m.blank[i] { "flat" } else { "has content" })));
    }
    if case.expect.screen_changes {
        out.push(check("screen_changes", m.distinct_screens >= 2, format!("{} distinct frames", m.distinct_screens)));
    }
    if let Some(min) = case.expect.polygons_min {
        out.push(check("polygons_min", m.max_polygons >= min, format!("max {} polygons (need {min})", m.max_polygons)));
    }
}

fn audio(case: &Case, m: &Metrics, out: &mut Vec<Check>) {
    if case.expect.audio {
        out.push(check("audio", m.audio_nonzero, if m.audio_nonzero { "sound".into() } else { "silent".into() }));
    }
    if case.expect.audio_rate {
        let frames = f64::from(m.frames.saturating_sub(10).max(1));
        let rate = m.audio_frames as f64 / frames;
        let ok = (rate - AUDIO_PER_FRAME).abs() <= AUDIO_PER_FRAME * 0.03;
        out.push(check("audio_rate", ok, format!("{rate:.1} per frame (expect ~{AUDIO_PER_FRAME})")));
    }
}

fn system(case: &Case, m: &Metrics, out: &mut Vec<Check>) {
    if case.expect.no_terminate {
        let stuck = m.irq_off.len() >= TERMINATE_WINDOW && m.irq_off.iter().all(|&off| off);
        out.push(check("no_terminate", !stuck, if stuck { "IRQs off at every frame end: OS_Terminate?".into() } else { "running".into() }));
    }
    if case.expect.save_written {
        let detail = m.save.as_ref().map_or_else(|| "no .sav".to_owned(), |(_, len)| format!("{len} bytes"));
        out.push(check("save_written", m.save.as_ref().is_some_and(|s| s.1 > 0), detail));
    }
    for entry in &case.expect.ram {
        let want = parse_ram_width(entry).map(|(a, v, _)| (a, v));
        let got = want.and_then(|(a, _)| m.ram.iter().find(|r| r.0 == a));
        let ok = matches!((want, got), (Some((_, v)), Some(&(_, g))) if v == g);
        let detail = got.map_or_else(|| format!("{entry}: unreadable"), |&(a, g)| format!("[{a:#010x}] = {g:#010x}"));
        out.push(check("ram", ok, detail));
    }
    if let Some(max) = case.expect.max_ms_per_frame {
        let mut c = check("max_ms_per_frame", m.ms_per_frame <= max, format!("{:.2} ms (limit {max})", m.ms_per_frame));
        c.info = true;
        out.push(c);
    }
}

/// Compares the snapshot hashes with the baseline of this case.
fn baseline(base: Option<&BTreeMap<String, String>>, m: &Metrics, out: &mut Vec<Check>) {
    let Some(base) = base else { return };
    let diff: Vec<&String> = m.hashes.iter().filter(|(k, v)| base.get(*k).is_some_and(|b| b != *v)).map(|(k, _)| k).collect();
    let missing = m.hashes.keys().filter(|k| !base.contains_key(*k)).count();
    let detail = if diff.is_empty() {
        format!("{} snapshots match ({missing} new)", m.hashes.len() - missing)
    } else {
        format!("differ: {}", diff.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
    };
    out.push(check("baseline", diff.is_empty(), detail));
}

/// Every check of `case`.
pub fn evaluate(case: &Case, m: &Metrics, base: Option<&BTreeMap<String, String>>) -> Vec<Check> {
    let mut out = Vec::new();
    screens(case, m, &mut out);
    pixels(m, &mut out);
    audio(case, m, &mut out);
    system(case, m, &mut out);
    baseline(base, m, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ram_entries() {
        assert_eq!(parse_ram("0x02000000=0x1"), Some((0x0200_0000, 1)));
        assert_eq!(parse_ram("16 = 32"), Some((16, 32)));
        assert_eq!(parse_ram("nope"), None);
        assert_eq!(parse_ram_width("0x04000130:16=0x3FE"), Some((0x0400_0130, 0x3FE, 16)));
        assert_eq!(parse_ram_width("0x10:12=1"), None);
        assert_eq!(parse_pixel("lower:255:191=0xF80000"), Some((true, 255, 191, 0x00F8_0000)));
        assert_eq!(parse_pixel("upper:256:0=0"), None);
    }
}