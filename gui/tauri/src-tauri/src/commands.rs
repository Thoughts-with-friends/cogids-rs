//! Tauri commands used by the frontend (`gui/tauri/src/services/api/emulator.ts`).
use crate::emu_thread::{EmuThread, Shared};
use ds_common::{AudioOutput, Instance, Settings};
use ds_emu::Emulator;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::State;

/// GUI state saved as `config.json` in the instance folder.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct GuiConfig {
    pub rom_dir: PathBuf,
    pub selected_rom: Option<PathBuf>,
    pub scale: f32,
    /// `"vertical"` or `"horizontal"`.
    pub layout: String,
    /// 0-100.
    pub volume: u8,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            rom_dir: PathBuf::new(),
            selected_rom: None,
            scale: 2.0,
            layout: "vertical".to_owned(),
            volume: 80,
        }
    }
}

pub struct AppState {
    pub instance: Instance,
    pub settings: Settings,
    pub config: Mutex<GuiConfig>,
    pub audio: AudioOutput,
    pub shared: Arc<Shared>,
    pub emu: Mutex<Option<EmuThread>>,
}

impl AppState {
    pub fn new(instance: Instance, settings: Settings) -> Self {
        let config = std::fs::read_to_string(instance.config_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let config: GuiConfig = config;
        let audio = AudioOutput::new(f32::from(config.volume.min(100)) / 100.0);
        Self {
            instance,
            settings,
            config: Mutex::new(config),
            audio,
            shared: Arc::new(Shared::default()),
            emu: Mutex::new(None),
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
        let thread = self.emu.lock().ok().and_then(|mut e| e.take());
        if let Some(thread) = thread {
            thread.stop();
        }
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Result<GuiConfig, String> {
    state.config.lock().map(|c| c.clone()).map_err(err)
}

#[tauri::command]
pub fn set_config(state: State<'_, AppState>, config: GuiConfig) -> Result<(), String> {
    state
        .audio
        .set_volume(f32::from(config.volume.min(100)) / 100.0);
    state.save_config(&config);
    *state.config.lock().map_err(err)? = config;
    Ok(())
}

/// `.nds` files in `dir`, sorted by name.
#[tauri::command]
pub fn list_roms(dir: String) -> Vec<String> {
    let mut roms: Vec<String> = std::fs::read_dir(Path::new(&dir))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("nds")))
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    roms.sort();
    roms
}

/// Loads `path` and starts emulating it (stopping a previous game first so
/// its save is flushed before the new one is read).
#[tauri::command]
pub fn start_emulator(state: State<'_, AppState>, path: String) -> Result<(), String> {
    state.stop();

    let rom = PathBuf::from(&path);
    let mut emu = Emulator::new();
    state.settings.apply(&mut emu);
    emu.config.save_dir = Some(state.instance.saves_dir());
    state.instance.import_legacy_save(&rom);
    emu.load_rom(&rom).map_err(|e| {
        log::error!("Failed to load ROM {path}: {e:?}");
        format!("{e}")
    })?;
    log::info!("Started {path}");

    let thread = EmuThread::start(emu, Arc::clone(&state.shared), state.audio.clone());
    *state.emu.lock().map_err(err)? = Some(thread);
    Ok(())
}

#[tauri::command]
pub fn stop_emulator(state: State<'_, AppState>) {
    state.stop();
}

#[tauri::command]
pub fn is_running(state: State<'_, AppState>) -> bool {
    state.shared.is_running() && state.emu.lock().is_ok_and(|e| e.is_some())
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
    fps: f32,
    instance_dir: String,
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Status {
    Status {
        running: is_running(state.clone()),
        fps: state.shared.fps(),
        instance_dir: std::path::absolute(state.instance.root())
            .unwrap_or_else(|_| state.instance.root().to_path_buf())
            .to_string_lossy()
            .into_owned(),
    }
}
