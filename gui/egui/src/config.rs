use ds_common::view::View;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// GUI state (`config.json`).
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    // Aliases: the Tauri / freya GUIs write the same file in camelCase.
    #[serde(alias = "romDir")]
    pub rom_dir: PathBuf,
    #[serde(alias = "selectedRom")]
    pub selected_rom: Option<PathBuf>,
    /// Screen arrangement (View menu). Flattened: the old `scale`, `layout`
    /// and `show_fps` keys are its fields.
    #[serde(flatten)]
    pub view: View,
    /// Output volume in percent (0-100).
    pub volume: u8,
    /// Run at the DS's 59.83 fps (off: as fast as possible).
    #[serde(alias = "limitFramerate")]
    pub limit_framerate: bool,
    /// File > Open recent, newest first.
    #[serde(alias = "recentRoms")]
    pub recent_roms: Vec<PathBuf>,
    /// Multiplayer: player name shown to the others and the host address
    /// last joined.
    pub lan_name: String,
    pub lan_address: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rom_dir: PathBuf::from(""),
            selected_rom: None,
            view: View::default(),
            volume: 80,
            limit_framerate: true,
            recent_roms: Vec::new(),
            lan_name: "Player".to_owned(),
            lan_address: String::new(),
        }
    }
}

/// `config.json` inside the instance folder (set once at start-up).
static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Legacy location (working directory) migrated on first start.
const LEGACY_CONFIG_PATH: &str = "config.json";

/// Selects the config file; a legacy `./config.json` is copied over the
/// first time so the ROM folder setting is kept.
pub fn set_config_path(path: PathBuf) {
    if !path.exists() && Path::new(LEGACY_CONFIG_PATH).is_file() {
        let _ = fs::copy(LEGACY_CONFIG_PATH, &path);
    }
    let _ = CONFIG_PATH.set(path);
}

fn config_path() -> &'static Path {
    CONFIG_PATH
        .get()
        .map_or_else(|| Path::new(LEGACY_CONFIG_PATH), PathBuf::as_path)
}

pub fn load_config() -> Config {
    if let Ok(text) = fs::read_to_string(config_path()) {
        serde_json::from_str(&text).unwrap_or_default()
    } else {
        let cfg = Config::default();
        save_config(&cfg);
        cfg
    }
}

pub fn save_config(cfg: &Config) {
    if let Ok(text) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(config_path(), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Tauri and freya GUIs write `config.json` in camelCase.
    #[test]
    fn reads_the_tauri_and_freya_config() {
        let config: Config = serde_json::from_str(
            r#"{"romDir":"D:/roms","selectedRom":null,"recentRoms":["D:/roms/a.nds"],"limitFramerate":false,"volume":40,"speed":200}"#,
        )
        .unwrap_or_default();
        assert_eq!(config.rom_dir, PathBuf::from("D:/roms"));
        assert_eq!(config.recent_roms, [PathBuf::from("D:/roms/a.nds")]);
        assert!(!config.limit_framerate);
        assert_eq!(config.view.speed, 200);
    }
}
