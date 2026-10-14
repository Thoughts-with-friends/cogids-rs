//! Per-instance data folder (`./instances/instanceN`):
//!
//! ```text
//! instances/instance1/
//! ├── cheats/         (reserved for cheat files)
//! ├── logs/           lunaris.log (rotated, 4 files kept)
//! ├── saves/          <rom name>.sav
//! ├── states/         (reserved for save states)
//! ├── config.json     GUI state: ROM folder, selected ROM, scale, layout
//! ├── settings.json   emulator settings: log level, BIOS/firmware, MAC, ...
//! └── instance.lock   held while a process uses the folder
//! ```
//!
//! Each running process takes the first folder whose `instance.lock` is not
//! held by another process (instance1, instance2, ...), so two emulators on
//! one machine — e.g. both ends of a multiplayer test — never share saves or
//! settings (and get different Wi-Fi MACs, see [`Settings::ensure_mac`]).
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Folder holding the instance folders, relative to the working directory.
const INSTANCES_DIR: &str = "instances";

/// Instance folders tried before giving up on locking (`instance1..=N`).
const MAX_INSTANCES: u32 = 8;

/// Sub folders created on start-up.
const SUB_DIRS: [&str; 4] = ["cheats", "logs", "saves", "states"];

#[derive(Debug, Clone)]
pub struct Instance {
    root: PathBuf,
    number: u32,
    /// Keeps `instance.lock` locked for the life of the process.
    _lock: Option<Arc<File>>,
}

impl Instance {
    /// Opens (and creates, if needed) the first free `instances/instanceN`
    /// below the working directory.
    pub fn open() -> Self {
        Self::open_in(Path::new("."))
    }

    /// Opens (and creates, if needed) the first `instances/instanceN` below
    /// `base` that no other process is using. Falls back to `instance1`
    /// (unlocked) if none can be locked.
    pub fn open_in(base: &Path) -> Self {
        let dir = |n: u32| base.join(INSTANCES_DIR).join(format!("instance{n}"));
        let locked = (1..=MAX_INSTANCES).find_map(|n| {
            let root = dir(n);
            std::fs::create_dir_all(&root).ok()?;
            let lock = File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(root.join("instance.lock"))
                .ok()?;
            lock.try_lock().ok()?;
            Some((n, lock))
        });
        let (number, lock) = if let Some((n, lock)) = locked {
            (n, Some(Arc::new(lock)))
        } else {
            eprintln!("No free instance folder; sharing instance1");
            (1, None)
        };
        let root = dir(number);
        for sub in SUB_DIRS {
            if let Err(e) = std::fs::create_dir_all(root.join(sub)) {
                eprintln!("Failed to create {}: {e}", root.join(sub).display());
            }
        }
        Self {
            root,
            number,
            _lock: lock,
        }
    }

    /// `N` of `instanceN` (1 for the first process).
    pub const fn number(&self) -> u32 {
        self.number
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

    /// Where the save of `rom` lives (`saves/<rom name>.sav`).
    pub fn save_path(&self, rom: &Path) -> PathBuf {
        self.saves_dir()
            .join(rom.with_extension("sav").file_name().unwrap_or_default())
    }

    /// File > Import savefile: replaces the save of `rom` with `source`,
    /// keeping the old one as `<name>.sav.bak`. Call it with the game
    /// stopped (a running game would write its save back over the import).
    ///
    /// # Errors
    /// If the backup or the copy fails.
    pub fn import_save(&self, rom: &Path, source: &Path) -> std::io::Result<PathBuf> {
        let target = self.save_path(rom);
        if target.is_file() {
            let backup = target.with_extension("sav.bak");
            std::fs::copy(&target, &backup)?;
            log::info!("Backed up {} -> {}", target.display(), backup.display());
        }
        std::fs::copy(source, &target)?;
        log::info!("Imported save {} -> {}", source.display(), target.display());
        Ok(target)
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
    /// Wi-Fi MAC address (`00:09:BF:xx:xx:xx`) patched into the firmware.
    /// Generated once per instance folder: consoles playing together must
    /// not share one.
    pub mac_address: Option<String>,
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
            mac_address: None,
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

    /// Makes sure [`Self::mac_address`] holds a valid MAC, generating a
    /// random one (Nintendo's `00:09:BF` prefix) if not. Returns `true` if
    /// it changed (so the caller can save the settings).
    pub fn ensure_mac(&mut self) -> bool {
        if self.mac().is_some() {
            return false;
        }
        let [a, b, c, ..] = random_u64().to_le_bytes();
        let mac = [0x00, 0x09, 0xBF, a, b, c];
        self.mac_address = Some(format_mac(mac));
        true
    }

    /// The configured MAC address, if valid.
    pub fn mac(&self) -> Option<[u8; 6]> {
        parse_mac(self.mac_address.as_deref()?)
    }

    /// Applies the settings to a freshly created emulator (before `load_rom`).
    pub fn apply(&self, emu: &mut ds_emu::Emulator) {
        emu.config.hle_bios = self.hle_bios;
        emu.config.mac_address = self.mac();
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

/// `00:09:BF:12:34:56`.
pub fn format_mac(mac: [u8; 6]) -> String {
    mac.map(|b| format!("{b:02X}")).join(":")
}

/// Parses `00:09:BF:12:34:56` (or `-` separated); a unicast address only.
pub fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let mut mac = [0; 6];
    let mut parts = text.split([':', '-']);
    for byte in &mut mac {
        *byte = u8::from_str_radix(parts.next()?.trim(), 16).ok()?;
    }
    (parts.next().is_none() && mac[0] & 1 == 0).then_some(mac)
}

/// Seeded by the OS (`RandomState`), the time and the process id.
fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    h.finish()
}

/// Shows `dir` in the system file manager.
///
/// # Errors
/// If the file manager cannot be started.
pub fn reveal(dir: &Path) -> std::io::Result<()> {
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_owned());
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(dir)
        .spawn()
        .map(drop)
}

/// System > Multiplayer > Launch new instance: starts this program again;
/// the new process takes the next free instance folder (own saves, settings
/// and Wi-Fi MAC).
///
/// # Errors
/// If the executable cannot be found or started.
pub fn launch_instance() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe).spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_round_trip() {
        let mac = [0x00, 0x09, 0xBF, 0x12, 0xAB, 0x05];
        assert_eq!(parse_mac(&format_mac(mac)), Some(mac));
        assert_eq!(parse_mac("01:00:00:00:00:00"), None, "multicast");
        assert_eq!(parse_mac("00:09:BF:12:34"), None);
        let mut settings = Settings::default();
        assert!(settings.ensure_mac());
        assert!(!settings.ensure_mac());
        assert_eq!(
            settings.mac().map(|m| m[..3].to_vec()),
            Some(vec![0, 9, 0xBF])
        );
    }

    #[test]
    fn import_save_keeps_a_backup() {
        let base = std::env::temp_dir().join(format!("ds_common_import_{}", std::process::id()));
        let instance = Instance::open_in(&base);
        let rom = Path::new("D:/roms/game.nds");
        let save = instance.save_path(rom);
        assert!(save.ends_with("saves/game.sav"));
        let _ = std::fs::write(&save, b"old");
        let source = base.join("new.sav");
        let _ = std::fs::write(&source, b"new");
        assert!(instance.import_save(rom, &source).is_ok());
        assert_eq!(std::fs::read(&save).ok().as_deref(), Some(&b"new"[..]));
        assert_eq!(
            std::fs::read(save.with_extension("sav.bak"))
                .ok()
                .as_deref(),
            Some(&b"old"[..])
        );
        drop(instance);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn second_process_gets_another_folder() {
        let base = std::env::temp_dir().join(format!("ds_common_instance_{}", std::process::id()));
        let first = Instance::open_in(&base);
        let second = Instance::open_in(&base);
        assert_eq!((first.number(), second.number()), (1, 2));
        drop(first);
        let third = Instance::open_in(&base);
        assert_eq!(third.number(), 1, "released on drop");
        drop((second, third));
        let _ = std::fs::remove_dir_all(&base);
    }
}
