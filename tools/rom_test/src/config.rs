//! The local test config (`docs/cogids-rs/local/roms.toml`, git-ignored).
//!
//! ```text
//!  [settings]  results folder, jobs, fixed RTC time
//!  [roms]      alias = "path to .nds"         (private: never committed)
//!  [[case]]    feature, name, rom, frames, inputs, snapshots, [case.expect]
//! ```
//!
//! See `docs/cogids-rs/roms.example.toml` and chapter 20 of the design docs.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Feature ids and the design chapter each one belongs to.
pub const FEATURES: [(&str, u8, &str); 18] = [
    ("boot", 1, "起動・ROM 読み込み"),
    ("scheduler", 2, "フレーム実行とスケジューラ"),
    ("cpu", 3, "CPU"),
    ("bios", 4, "BIOS とファームウェア"),
    ("memory", 5, "メモリバス"),
    ("interrupt", 6, "割り込み・タイマ・IPC"),
    ("dma", 7, "DMA"),
    ("video2d", 8, "2D 映像"),
    ("geometry3d", 9, "3D ジオメトリ"),
    ("raster3d", 10, "3D ラスタライザ"),
    ("hires", 11, "高解像度・xBRZ"),
    ("sound", 12, "音声"),
    ("cartridge", 13, "カード・セーブ"),
    ("input", 14, "SPI・RTC・入力"),
    ("wifi", 15, "Wi-Fi"),
    ("cheats", 16, "チート"),
    ("frontend", 17, "フロントエンド共通"),
    ("gui", 18, "GUI"),
];

/// Chapter number of a feature id.
pub fn chapter(feature: &str) -> Option<u8> {
    FEATURES.iter().find(|f| f.0 == feature).map(|f| f.1)
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Results folder, relative to the config file (default `results`).
    pub results: Option<PathBuf>,
    /// Cases run at the same time (default 2).
    pub jobs: Option<usize>,
    /// RTC date and time `[year, month, day, hour, minute, second]`, so
    /// runs are repeatable (default 2026-01-01 12:00:00).
    pub fixed_time: Option<[u32; 6]>,
}

/// What a case must show. Every key is optional.
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// Screens (`"upper"` / `"lower"`) that must not be one flat color at
    /// the end.
    #[serde(default)]
    pub not_blank: Vec<String>,
    /// The picture changes during the run (at least two different frames).
    #[serde(default)]
    pub screen_changes: bool,
    /// At least this many 3D polygons in some frame.
    pub polygons_min: Option<i32>,
    /// Some non-silent audio after the first frames.
    #[serde(default)]
    pub audio: bool,
    /// About 547 stereo frames of audio per video frame (32728.5 Hz).
    #[serde(default)]
    pub audio_rate: bool,
    /// A `.sav` exists in the case's own save folder after the run.
    #[serde(default)]
    pub save_written: bool,
    /// The ARM9 did not end in `OS_Terminate` (IRQs off at every one of
    /// the last 60 frame ends).
    #[serde(default)]
    pub no_terminate: bool,
    /// `"0xADDR[:WIDTH]=0xVALUE"`: ARM9 memory checked at the end (WIDTH
    /// 8 / 16 / 32, default 32).
    #[serde(default)]
    pub ram: Vec<String>,
    /// `"SCREEN:X:Y=0xRRGGBB"`: a pixel of the last frame (`upper` /
    /// `lower`, native 256x192, colors as the frontends get them).
    #[serde(default)]
    pub pixel: Vec<String>,
    /// Average milliseconds per frame (informational with `jobs > 1`).
    pub max_ms_per_frame: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Case {
    /// Feature id (see [`FEATURES`]).
    pub feature: String,
    pub name: String,
    /// A `[roms]` alias or a path.
    pub rom: String,
    pub frames: u32,
    /// `key:start:every[:end]` (a b x y l r start select up down left right);
    /// each press is held for 6 frames.
    #[serde(default)]
    pub press: Vec<String>,
    /// `x:y:start:every[:end]`, held for 6 frames.
    #[serde(default)]
    pub touch: Vec<String>,
    /// Action Replay codes (text, two words per line) run every frame.
    #[serde(default)]
    pub cheats: Vec<String>,
    /// A `.sav` copied into the case's save folder first (never written).
    pub save_from: Option<String>,
    /// Internal resolution for the hi-res snapshots (1 = none).
    pub hd: Option<usize>,
    #[serde(default)]
    pub xbrz: bool,
    /// Frames (0-based) whose screens are hashed and saved as PNG; the
    /// last frame is always included.
    #[serde(default)]
    pub snapshots: Vec<u32>,
    /// Free notes (what the case shows).
    pub memo: Option<String>,
    #[serde(default)]
    pub expect: Expect,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub roms: BTreeMap<String, String>,
    #[serde(default, rename = "case")]
    pub cases: Vec<Case>,
    /// Folder of the config file (relative ROM paths start here).
    #[serde(skip)]
    pub dir: Option<PathBuf>,
}

impl Config {
    /// Reads and checks `path`.
    ///
    /// # Errors
    /// If the file cannot be read or parsed, or a case is invalid.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut config: Self =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        config.dir = path.parent().map(Path::to_path_buf);
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        let mut names = std::collections::BTreeSet::new();
        for case in &self.cases {
            if chapter(&case.feature).is_none() {
                return Err(format!(
                    "case \"{}\": unknown feature \"{}\" (see --list)",
                    case.name, case.feature
                ));
            }
            if !names.insert(case.name.as_str()) {
                return Err(format!("case \"{}\" is defined twice", case.name));
            }
            if case.frames == 0 {
                return Err(format!("case \"{}\": frames must be > 0", case.name));
            }
        }
        Ok(())
    }

    /// A config holding only `case` (`rom_test --rom`).
    ///
    /// # Errors
    /// If the case is invalid.
    pub fn single(case: Case) -> Result<Self, String> {
        let config = Self {
            cases: vec![case],
            ..Self::default()
        };
        config.validate()?;
        Ok(config)
    }

    /// The ROM path of `case` (alias resolved; a relative path is taken
    /// from the config file's folder).
    pub fn rom_path(&self, case: &Case) -> PathBuf {
        let path = PathBuf::from(self.roms.get(&case.rom).unwrap_or(&case.rom));
        match &self.dir {
            Some(dir) if path.is_relative() => dir.join(path),
            _ => path,
        }
    }

    /// `save_from` of `case`, relative to the config file's folder.
    pub fn save_from(&self, case: &Case) -> Option<PathBuf> {
        let path = PathBuf::from(case.save_from.as_ref()?);
        Some(match &self.dir {
            Some(dir) if path.is_relative() => dir.join(path),
            _ => path,
        })
    }
}
