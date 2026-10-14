//! Tauri commands used by the frontend (`gui/tauri/src/services/api/emulator.ts`).
//!
//! ```text
//!  React MenuBar --get_menu--> ds_common::menu::menu_bar (same tree as egui)
//!                --menu_action(Action)--> apply() here, or the frontend for
//!                  what needs a dialog / the window (Open ROM, Import
//!                  savefile, Quit, Screen size, dialogs)
//!  React screens --layout_screens(w, h)--> ds_common::view::place
//! ```
use crate::emu_thread::{EmuThread, Shared};
use ds_common::menu::{Action, Menu, MenuState, menu_bar, remember};
use ds_common::rom_info::RomInfo;
use ds_common::view::{Placement, View, place};
use ds_common::{AudioOutput, Instance, Settings};
use ds_emu::Emulator;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::State;

/// Why the Tauri GUI's multiplayer entries are disabled.
const NO_MULTIPLAYER: &str =
    "LAN play is only available in the egui GUI for now (Launch new instance works)";

/// GUI state saved as `config.json` in the instance folder.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct GuiConfig {
    pub rom_dir: PathBuf,
    pub selected_rom: Option<PathBuf>,
    /// View menu settings; flattened, so the old `scale` / `layout` keys are
    /// its fields.
    #[serde(flatten)]
    pub view: View,
    /// 0-100.
    pub volume: u8,
    pub limit_framerate: bool,
    /// File > Open recent, newest first.
    pub recent_roms: Vec<PathBuf>,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            rom_dir: PathBuf::new(),
            selected_rom: None,
            view: View::default(),
            volume: 80,
            limit_framerate: true,
            recent_roms: Vec::new(),
        }
    }
}

pub struct AppState {
    pub instance: Instance,
    pub settings: Mutex<Settings>,
    pub config: Mutex<GuiConfig>,
    pub audio: AudioOutput,
    pub shared: Arc<Shared>,
    pub emu: Mutex<Option<EmuThread>>,
    /// The running game's ROM.
    pub rom: Mutex<Option<PathBuf>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl AppState {
    pub fn new(instance: Instance, settings: Settings) -> Self {
        let config: GuiConfig = std::fs::read_to_string(instance.config_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let audio = AudioOutput::new(f32::from(config.volume.min(100)) / 100.0);
        let shared = Arc::new(Shared::default());
        shared.set_limit_framerate(config.limit_framerate);
        Self {
            instance,
            settings: Mutex::new(settings),
            config: Mutex::new(config),
            audio,
            shared,
            emu: Mutex::new(None),
            rom: Mutex::new(None),
        }
    }

    fn save_config(&self, config: &GuiConfig) {
        if let Ok(text) = serde_json::to_string_pretty(config)
            && let Err(e) = std::fs::write(self.instance.config_path(), text)
        {
            log::error!("Failed to write config: {e}");
        }
    }

    /// Stops the running emulator (its save is written on stop).
    pub fn stop(&self) {
        let thread = lock(&self.emu).take();
        if let Some(thread) = thread {
            thread.stop();
        }
        *lock(&self.rom) = None;
        self.shared.set_paused(false);
    }

    /// Loads `rom` and runs it (stopping a previous game first so its save
    /// is flushed before the new one is read).
    fn start(&self, rom: &Path) -> Result<(), String> {
        self.stop();

        let mut emu = Emulator::new();
        lock(&self.settings).apply(&mut emu);
        emu.config.save_dir = Some(self.instance.saves_dir());
        self.instance.import_legacy_save(rom);
        emu.load_rom(rom).map_err(|e| {
            log::error!("Failed to load ROM {}: {e:?}", rom.display());
            format!("Cannot load {}: {e}", file_name(rom))
        })?;
        log::info!("Started {}", rom.display());

        let thread = EmuThread::start(emu, Arc::clone(&self.shared), self.audio.clone());
        *lock(&self.emu) = Some(thread);
        *lock(&self.rom) = Some(rom.to_owned());

        let mut config = lock(&self.config);
        remember(&mut config.recent_roms, rom);
        config.selected_rom = Some(rom.to_owned());
        self.save_config(&config);
        drop(config);
        Ok(())
    }

    /// Applies a menu action the backend can carry out.
    fn apply(&self, action: Action) -> Result<(), String> {
        let rom = lock(&self.rom).clone();
        match action {
            Action::OpenRomPath(path) => return self.start(&path),
            Action::Eject | Action::Stop => self.stop(),
            Action::Reset => {
                if let Some(rom) = rom {
                    return self.start(&rom);
                }
            }
            Action::TogglePause => self.shared.set_paused(!self.shared.is_paused()),
            Action::FrameStep => self.shared.frame_step(),
            Action::OpenDirectory => {
                ds_common::reveal(self.instance.root()).map_err(|e| e.to_string())?;
            }
            Action::LaunchInstance => ds_common::launch_instance().map_err(|e| e.to_string())?,
            Action::ClearRecent
            | Action::SetRotation(_)
            | Action::SetGap(_)
            | Action::SetLayout(_)
            | Action::ToggleSwap
            | Action::SetSizing(_)
            | Action::ToggleIntegerScaling
            | Action::ToggleFiltering
            | Action::ToggleShowFps
            | Action::ToggleLimitFramerate
            | Action::ScreenSize(_) => {
                let mut config = lock(&self.config);
                let view = &mut config.view;
                match action {
                    Action::SetRotation(r) => view.rotation = r,
                    Action::SetGap(g) => view.gap = g,
                    Action::SetLayout(l) => view.layout = l,
                    Action::ToggleSwap => view.swap ^= true,
                    Action::SetSizing(s) => view.sizing = s,
                    Action::ToggleIntegerScaling => view.integer_scaling ^= true,
                    Action::ToggleFiltering => view.filtering ^= true,
                    Action::ToggleShowFps => view.show_fps ^= true,
                    Action::ScreenSize(n) => view.scale = f32::from(n),
                    Action::ClearRecent => config.recent_roms.clear(),
                    Action::ToggleLimitFramerate => {
                        config.limit_framerate ^= true;
                        self.shared.set_limit_framerate(config.limit_framerate);
                    }
                    _ => {}
                }
                self.save_config(&config);
                drop(config);
            }
            // Dialogs and the window are the frontend's.
            Action::OpenRom | Action::ImportSavefile | Action::Quit | Action::Open(_) => {}
        }
        Ok(())
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// `.nds` files in `dir`, sorted by name.
fn rom_paths(dir: &Path) -> Vec<PathBuf> {
    let mut roms: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("nds")))
                .collect()
        })
        .unwrap_or_default();
    roms.sort();
    roms
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> GuiConfig {
    lock(&state.config).clone()
}

#[tauri::command]
pub fn set_config(state: State<'_, AppState>, config: GuiConfig) {
    state
        .audio
        .set_volume(f32::from(config.volume.min(100)) / 100.0);
    state.shared.set_limit_framerate(config.limit_framerate);
    state.save_config(&config);
    *lock(&state.config) = config;
}

/// `.nds` files in `dir`, sorted by name.
#[tauri::command]
pub fn list_roms(dir: String) -> Vec<String> {
    rom_paths(Path::new(&dir))
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Loads `path` and starts emulating it.
#[tauri::command]
pub fn start_emulator(state: State<'_, AppState>, path: String) -> Result<(), String> {
    state.start(Path::new(&path))
}

#[tauri::command]
pub fn stop_emulator(state: State<'_, AppState>) {
    state.stop();
}

#[tauri::command]
pub fn is_running(state: State<'_, AppState>) -> bool {
    state.shared.is_running() && lock(&state.emu).is_some()
}

/// The menu bar (shared with the egui GUI).
#[tauri::command]
pub fn get_menu(state: State<'_, AppState>) -> Vec<Menu> {
    let config = lock(&state.config).clone();
    let loaded = lock(&state.rom).as_deref().map(file_name);
    let folder_roms = rom_paths(&config.rom_dir);
    menu_bar(&MenuState {
        loaded: loaded.as_deref(),
        paused: state.shared.is_paused(),
        view: &config.view,
        limit_framerate: config.limit_framerate,
        recent: &config.recent_roms,
        folder_roms: &folder_roms,
        multiplayer: Err(NO_MULTIPLAYER),
    })
}

/// Carries out a menu entry (those needing a dialog or the window are done
/// by the frontend).
#[tauri::command]
pub fn menu_action(state: State<'_, AppState>, action: Action) -> Result<(), String> {
    state.apply(action)
}

/// File > Import savefile: stop, back the old save up, copy, restart.
#[tauri::command]
pub fn import_savefile(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let rom = lock(&state.rom).clone().ok_or("No game is running")?;
    state.stop();
    state
        .instance
        .import_save(&rom, Path::new(&path))
        .map_err(|e| format!("Import failed: {e}"))?;
    state.start(&rom)
}

/// System > ROM info of the running game.
#[tauri::command]
pub fn rom_info(state: State<'_, AppState>) -> Result<RomInfo, String> {
    let rom = lock(&state.rom).clone().ok_or("No game is running")?;
    RomInfo::read(&rom).map_err(err)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    lock(&state.settings).clone()
}

/// Saves `settings.json`; applies when a game is next started or reset.
#[tauri::command]
pub fn set_settings(state: State<'_, AppState>, settings: Settings) -> Result<(), String> {
    if let Some(mac) = &settings.mac_address
        && ds_common::parse_mac(mac).is_none()
    {
        return Err(format!("Invalid MAC address {mac}"));
    }
    settings.save(&state.instance.settings_path());
    *lock(&state.settings) = settings;
    Ok(())
}

/// Where the screens go in a `width` x `height` area (CSS pixels).
#[tauri::command]
pub fn layout_screens(state: State<'_, AppState>, width: f32, height: f32) -> Placement {
    place(&lock(&state.config).view, width, height)
}

/// DS buttons pressed on the keyboard (bits: see `emu_thread::key`).
#[tauri::command]
pub fn set_keys(state: State<'_, AppState>, keys: u32) {
    state.shared.set_keys(keys);
}

/// Stylus on the lower screen in DS pixels; `null` lifts it.
#[tauri::command]
pub fn set_touch(state: State<'_, AppState>, x: Option<i32>, y: Option<i32>) {
    state.shared.set_touch(x.zip(y));
}

/// The newest frame after `last_seq` as binary: 8-byte little-endian frame
/// number, then the upper and lower screen in RGBA (256x192 each). Empty if
/// there is no new frame.
#[tauri::command]
pub fn get_frame(state: State<'_, AppState>, last_seq: u64) -> tauri::ipc::Response {
    let body = state
        .shared
        .frame_since(last_seq)
        .map(|(seq, rgba)| {
            let mut out = Vec::with_capacity(8 + rgba.len());
            out.extend_from_slice(&seq.to_le_bytes());
            out.extend_from_slice(&rgba);
            out
        })
        .unwrap_or_default();
    tauri::ipc::Response::new(body)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    running: bool,
    paused: bool,
    fps: f32,
    /// File name of the running game.
    rom: Option<String>,
    instance_dir: String,
    instance: u32,
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Status {
    Status {
        running: is_running(state.clone()),
        paused: state.shared.is_paused(),
        fps: state.shared.fps(),
        rom: lock(&state.rom).as_deref().map(file_name),
        instance_dir: std::path::absolute(state.instance.root())
            .unwrap_or_else(|_| state.instance.root().to_path_buf())
            .to_string_lossy()
            .into_owned(),
        instance: state.instance.number(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_keeps_loading() {
        let config: GuiConfig = serde_json::from_str(
            r#"{"romDir":"D:/roms","selectedRom":null,"scale":3.0,"layout":"horizontal","volume":50}"#,
        )
        .unwrap_or_default();
        assert_eq!(config.rom_dir, PathBuf::from("D:/roms"));
        assert_eq!(config.view.scale, 3.0);
        assert_eq!(
            config.view.layout,
            ds_common::view::ScreenLayout::Horizontal
        );
        assert!(config.limit_framerate);
        let json = serde_json::to_value(&config).unwrap_or_default();
        assert_eq!(json["layout"], "horizontal");
        assert_eq!(json["integerScaling"], false);
    }
}
