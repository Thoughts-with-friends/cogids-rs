//! Tauri commands used by the frontend (`gui/tauri/src/services/api/emulator.ts`).
//!
//! ```text
//!  React MenuBar --get_menu--> ds_common::menu::menu_bar (same tree as egui)
//!                --menu_action(Action)--> apply() here, or the frontend for
//!                  what needs a dialog / the window (Open ROM, Import
//!                  savefile, Quit, Screen size, dialogs)
//!  React screens --layout_screens(w, h)--> ds_common::view::place
//! ```
use ds_common::Settings;
use ds_common::cheats::{self, Cheat};
pub use ds_common::frontend::GuiConfig;
use ds_common::frontend::{Frontend, file_name, lock, rom_paths};
use ds_common::menu::{Action, Menu, MenuState, menu_bar};
use ds_common::rom_info::RomInfo;
use ds_common::view::{Placement, place};
use serde::Serialize;
use std::path::Path;
use tauri::State;

/// The Tauri-managed state ([`Frontend`], shared with the freya GUI).
pub type AppState = Frontend;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Why the Tauri GUI's multiplayer entries are disabled.
const NO_MULTIPLAYER: &str =
    "LAN play is only available in the egui GUI for now (Launch new instance works)";

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> GuiConfig {
    lock(&state.config).clone()
}

#[tauri::command]
pub fn set_config(state: State<'_, AppState>, config: GuiConfig) {
    state.set_config(config);
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

/// The newest frame after `last_seq` as binary: 8-byte frame number, then
/// 4-byte width and height of one screen (all little-endian), then the
/// upper and lower screen in RGBA (256x192 each at 1x, larger at a higher
/// internal resolution). Empty if there is no new frame.
#[tauri::command]
pub fn get_frame(state: State<'_, AppState>, last_seq: u64) -> tauri::ipc::Response {
    let body = state
        .shared
        .frame_since(last_seq)
        .map(|(seq, width, rgba)| {
            let height = (rgba.len() / 8).checked_div(width as usize).unwrap_or(0) as u32;
            let mut out = Vec::with_capacity(16 + rgba.len());
            out.extend_from_slice(&seq.to_le_bytes());
            out.extend_from_slice(&width.to_le_bytes());
            out.extend_from_slice(&height.to_le_bytes());
            out.extend_from_slice(&rgba);
            out
        })
        .unwrap_or_default();
    tauri::ipc::Response::new(body)
}

/// System > Setup cheat codes: the running game's cheat list.
#[tauri::command]
pub fn get_cheats(state: State<'_, AppState>) -> Result<Vec<Cheat>, String> {
    let rom = lock(&state.rom).clone().ok_or("No game is running")?;
    Ok(cheats::load(&cheats::cheats_path(&state.instance, &rom)))
}

/// Saves the running game's cheat list and runs its enabled codes. Fails
/// (saving nothing) if an enabled cheat's code is invalid.
#[tauri::command]
pub fn set_cheats(state: State<'_, AppState>, cheats: Vec<Cheat>) -> Result<(), String> {
    let rom = lock(&state.rom).clone().ok_or("No game is running")?;
    for cheat in cheats.iter().filter(|c| c.enabled) {
        cheat.words().map_err(|e| format!("{}: {e}", cheat.name))?;
    }
    cheats::save(&cheats::cheats_path(&state.instance, &rom), &cheats)
        .map_err(|e| format!("Cannot save the cheats: {e}"))?;
    state.load_cheats();
    Ok(())
}

/// Parses a cheat code (to validate it while it is typed).
#[tauri::command]
pub fn check_cheat_code(code: String) -> Result<(), String> {
    ds_emu::parse_ar_code(&code).map(|_| ())
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
    use std::path::PathBuf;

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
