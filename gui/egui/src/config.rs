use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// How the two DS screens are arranged in the window.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScreenLayout {
    /// Upper screen above the lower screen (like the real console).
    #[default]
    Vertical,
    /// Upper screen left of the lower screen.
    Horizontal,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    pub rom_dir: PathBuf,
    pub selected_rom: Option<PathBuf>,
    pub scale: f32,
    pub show_fps: bool,
    pub layout: ScreenLayout,
    /// Output volume in percent (0-100).
    pub volume: u8,
    /// Multiplayer: player name shown to the others, the host address last
    /// joined, and whether the panel is open.
    pub lan_name: String,
    pub lan_address: String,
    pub multiplayer_open: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rom_dir: PathBuf::from(""),
            selected_rom: None,
            scale: 2.0,
            show_fps: true,
            layout: ScreenLayout::default(),
            volume: 80,
            lan_name: "Player".to_owned(),
            lan_address: String::new(),
            multiplayer_open: false,
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
