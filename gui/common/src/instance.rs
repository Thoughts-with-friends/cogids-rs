//! Per-instance data folder (`./instances/instance1`):
//!
//! ```text
//! instances/instance1/
//! ├── cheats/         (reserved for cheat files)
//! ├── logs/           lunaris.log (rotated, 4 files kept)
//! ├── saves/          <rom name>.sav
//! ├── states/         (reserved for save states)
//! ├── config.json     GUI state: ROM folder, selected ROM, scale, layout
//! └── settings.json   emulator settings: log level, BIOS/firmware, ...
//! ```
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Name of the instance folder, relative to the working directory.
const INSTANCE_DIR: &str = "instances/instance1";

/// Sub folders created on start-up.
const SUB_DIRS: [&str; 4] = ["cheats", "logs", "saves", "states"];

#[derive(Debug, Clone)]
pub struct Instance {
    root: PathBuf,
}

impl Instance {
    /// Opens (and creates, if needed) `instances/instance1` below the
    /// working directory.
    pub fn open() -> Self {
        Self::open_in(Path::new("."))
    }

    /// Opens (and creates, if needed) `instances/instance1` below `base`.
    pub fn open_in(base: &Path) -> Self {
        let root = base.join(INSTANCE_DIR);
        for dir in SUB_DIRS {
            if let Err(e) = std::fs::create_dir_all(root.join(dir)) {
                eprintln!("Failed to create {}: {e}", root.join(dir).display());
            }
        }
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.json")
    }

    pub fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn saves_dir(&self) -> PathBuf {
        self.root.join("saves")
    }

    /// Copies a save that an older version kept next to the ROM
    /// (`<rom>.sav`) into `saves/` the first time the ROM is started, so
    /// existing progress is not lost. The original file is left untouched.
    pub fn import_legacy_save(&self, rom: &Path) {
        let Some(name) = rom.with_extension("sav").file_name().map(ToOwned::to_owned) else {
            return;
        };
        let target = self.saves_dir().join(name);
        let legacy = rom.with_extension("sav");
        if target.exists() || !legacy.is_file() {
            return;
        }
        match std::fs::copy(&legacy, &target) {
            Ok(_) => log::info!("Imported save {} -> {}", legacy.display(), target.display()),
            Err(e) => log::error!("Failed to import save {}: {e}", legacy.display()),
        }
    }
}

/// Emulator settings (`settings.json`).
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Settings {
    /// Log level written to `logs/` (`error`, `warn`, `info`, `debug`, `trace`).
    pub log_level: String,
    /// High-level emulation of BIOS calls (no BIOS dump needed).
    pub hle_bios: bool,
    /// Optional real BIOS / firmware dumps.
    pub arm9_bios_path: Option<PathBuf>,
    pub arm7_bios_path: Option<PathBuf>,
    pub firmware_path: Option<PathBuf>,
    /// Frames skipped between rendered frames.
    pub frameskip: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            log_level: "info".to_owned(),
            hle_bios: true,
            arm9_bios_path: None,
            arm7_bios_path: None,
            firmware_path: None,
            frameskip: 0,
        }
    }
}

impl Settings {
    /// Loads `settings.json`, writing the defaults if it does not exist.
    pub fn load(path: &Path) -> Self {
        if let Ok(text) = std::fs::read_to_string(path) {
            serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!("Invalid {}: {e}; using defaults", path.display());
                Self::default()
            })
        } else {
            let settings = Self::default();
            settings.save(path);
            settings
        }
    }

    pub fn save(&self, path: &Path) {
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    /// Applies the settings to a freshly created emulator (before `load_rom`).
    pub fn apply(&self, emu: &mut ds_emu::Emulator) {
        emu.config.hle_bios = self.hle_bios;
        emu.config.frameskip = self.frameskip;
        if let Some(path) = &self.firmware_path {
            emu.config.firmware_path = path.to_string_lossy().into_owned();
        }
        let read = |path: &Option<PathBuf>| {
            let path = path.as_ref()?;
            std::fs::read(path)
                .inspect_err(|e| log::error!("Failed to read {}: {e}", path.display()))
                .ok()
        };
        if let Some(bin) = read(&self.arm9_bios_path) {
            emu.load_bios9(&bin);
        }
        if let Some(bin) = read(&self.arm7_bios_path) {
            emu.load_bios7(&bin);
        }
    }
}
