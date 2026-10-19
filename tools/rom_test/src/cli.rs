//! Command line: options, and the single case given with `--rom` (no
//! config file needed).
use crate::config::{Case, Expect};
use std::path::PathBuf;

/// Command line options.
#[derive(Debug, Default)]
pub struct Args {
    pub config: Option<PathBuf>,
    pub features: Vec<String>,
    pub case: Option<String>,
    pub jobs: Option<usize>,
    pub results: Option<PathBuf>,
    pub update_baseline: bool,
    pub no_png: bool,
    pub list: bool,
    pub help: bool,
    /// `--rom`: run one case built from the options below.
    pub rom: Option<String>,
    pub name: Option<String>,
    pub frames: Option<u32>,
    pub press: Vec<String>,
    pub touch: Vec<String>,
    pub cheats: Vec<String>,
    pub save_from: Option<String>,
    pub hd: Option<usize>,
    pub xbrz: bool,
    pub snapshots: Vec<u32>,
    pub expect: Vec<String>,
}

fn number<T: std::str::FromStr>(opt: &str, v: &str) -> Result<T, String> {
    v.parse().map_err(|_| format!("{opt}: \"{v}\" is not a number"))
}

impl Args {
    /// Parses `std::env::args()`.
    ///
    /// # Errors
    /// Unknown options, missing values or bad numbers.
    pub fn parse() -> Result<Self, String> {
        Self::parse_from(std::env::args().skip(1))
    }

    fn parse_from(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut a = Self::default();
        let mut it = args.into_iter();
        while let Some(opt) = it.next() {
            if a.flag(&opt) {
                continue;
            }
            let v = it.next().ok_or_else(|| format!("{opt} needs a value"))?;
            a.value(&opt, v)?;
        }
        Ok(a)
    }

    /// Options without a value; `false` if `opt` is not one.
    fn flag(&mut self, opt: &str) -> bool {
        match opt {
            "--update-baseline" => self.update_baseline = true,
            "--no-png" => self.no_png = true,
            "--list" => self.list = true,
            "--xbrz" => self.xbrz = true,
            "-h" | "--help" => self.help = true,
            _ => return false,
        }
        true
    }

    fn value(&mut self, opt: &str, v: String) -> Result<(), String> {
        match opt {
            "--config" => self.config = Some(v.into()),
            "--feature" => self.features.extend(v.split(',').map(str::to_owned)),
            "--case" => self.case = Some(v),
            "--jobs" => self.jobs = Some(number(opt, &v)?),
            "--results" => self.results = Some(v.into()),
            "--rom" => self.rom = Some(v),
            "--name" => self.name = Some(v),
            "--frames" => self.frames = Some(number(opt, &v)?),
            "--press" => self.press.push(v),
            "--touch" => self.touch.push(v),
            "--cheat" => self.cheats.push(v),
            "--save-from" => self.save_from = Some(v),
            "--hd" => self.hd = Some(number(opt, &v)?),
            "--snapshot" => self.snapshots.push(number(opt, &v)?),
            "--expect" => self.expect.push(v),
            _ => return Err(format!("unknown option {opt} (see --help)")),
        }
        Ok(())
    }

    /// The case given with `--rom`, if any.
    ///
    /// # Errors
    /// A bad `--expect`.
    pub fn rom_case(&self) -> Result<Option<Case>, String> {
        let Some(rom) = &self.rom else { return Ok(None) };
        let mut expect = Expect::default();
        for e in &self.expect {
            apply_expect(&mut expect, e)?;
        }
        Ok(Some(Case {
            feature: self.features.first().cloned().unwrap_or_else(|| "boot".to_owned()),
            name: self.name.clone().unwrap_or_else(|| "cli".to_owned()),
            rom: rom.clone(),
            frames: self.frames.unwrap_or(300),
            press: self.press.clone(),
            touch: self.touch.clone(),
            cheats: self.cheats.clone(),
            save_from: self.save_from.clone(),
            hd: self.hd,
            xbrz: self.xbrz,
            snapshots: self.snapshots.clone(),
            memo: None,
            expect,
        }))
    }
}

/// `--expect KEY[=VALUE]`: the same keys as `[case.expect]`.
///
/// # Errors
/// Unknown keys or bad values.
pub fn apply_expect(x: &mut Expect, entry: &str) -> Result<(), String> {
    let (key, value) = entry.split_once('=').unwrap_or((entry, ""));
    let num = |v: &str| v.parse::<f64>().map_err(|_| format!("--expect {entry}: not a number"));
    match key {
        "not_blank" => x.not_blank.extend(value.split(',').filter(|s| !s.is_empty()).map(str::to_owned)),
        "screen_changes" => x.screen_changes = true,
        "polygons_min" => x.polygons_min = Some(num(value)? as i32),
        "audio" => x.audio = true,
        "audio_rate" => x.audio_rate = true,
        "save_written" => x.save_written = true,
        "no_terminate" => x.no_terminate = true,
        "ram" => x.ram.push(value.to_owned()),
        "pixel" => x.pixel.push(value.to_owned()),
        "max_ms_per_frame" => x.max_ms_per_frame = Some(num(value)?),
        _ => return Err(format!("--expect: unknown check \"{key}\"")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        Args::parse_from(list.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn builds_a_case_from_options() {
        let a = args(&["--rom", "x.nds", "--feature", "video2d", "--frames", "60", "--expect", "pixel=lower:1:2=0xF80000", "--expect", "no_terminate"]);
        let case = a.and_then(|a| a.rom_case()).ok().flatten();
        assert!(case.as_ref().is_some_and(|c| c.feature == "video2d" && c.frames == 60));
        assert!(case.is_some_and(|c| c.expect.no_terminate && c.expect.pixel.len() == 1));
    }

    #[test]
    fn rejects_unknown_options() {
        assert!(args(&["--bogus"]).is_err());
        assert!(args(&["--frames", "ten"]).is_err());
        assert!(args(&["--rom", "x", "--expect", "nope"]).and_then(|a| a.rom_case()).is_err());
    }
}