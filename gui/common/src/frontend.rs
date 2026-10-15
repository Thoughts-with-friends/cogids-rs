//! Frontend state shared by the GUIs that run the emulator on its own
//! thread (Tauri, freya): `config.json` ([`GuiConfig`]), starting and
//! stopping games, and the menu actions that need no window.
//!
//! ```text
//!  GUI menu click --Action--> Frontend::apply()   (start / stop / pause /
//!                                                  view / speed / cheats)
//!                  the GUI itself: Open ROM, Import savefile, Quit,
//!                  Screen size, dialogs
//!  Frontend::start(rom) --> Emulator + EmuThread (emu_thread.rs)
//! ```
use crate::cheats;
use crate::emu_thread::{EmuThread, Shared};
use crate::menu::{Action, remember};
use crate::view::View;
use crate::{AudioOutput, Instance, Settings, speed};
use ds_emu::Emulator;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// GUI state saved as `config.json` in the instance folder.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct GuiConfig {
    // Aliases: the egui GUI writes the same file with snake_case keys.
    #[serde(alias = "rom_dir")]
    pub rom_dir: PathBuf,
    #[serde(alias = "selected_rom")]
    pub selected_rom: Option<PathBuf>,
    /// View menu settings; flattened, so the old `scale` / `layout` keys are
    /// its fields.
    #[serde(flatten)]
    pub view: View,
    /// 0-100.
    pub volume: u8,
    #[serde(alias = "limit_framerate")]
    pub limit_framerate: bool,
    /// File > Open recent, newest first.
    #[serde(alias = "recent_roms")]
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

/// What a frontend running the emulator on its own thread keeps: the
/// instance folder, settings, GUI config, audio output and the emulator
/// thread with the running ROM (used by the Tauri and freya GUIs).
pub struct Frontend {
    pub instance: Instance,
    pub settings: Mutex<Settings>,
    pub config: Mutex<GuiConfig>,
    pub audio: AudioOutput,
    pub shared: Arc<Shared>,
    pub emu: Mutex<Option<EmuThread>>,
    /// The running game's ROM.
    pub rom: Mutex<Option<PathBuf>>,
}

/// Locks `m`, recovering from a poisoned lock.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The file name of `path` (the whole path if it has none).
pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl Frontend {
    pub fn new(instance: Instance, settings: Settings) -> Self {
        let config: GuiConfig = std::fs::read_to_string(instance.config_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let audio = AudioOutput::new(f32::from(config.volume.min(100)) / 100.0);
        let shared = Arc::new(Shared::default());
        shared.set_limit_framerate(config.limit_framerate);
        Self::apply_view(&shared, &config.view);
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

    /// Speed and internal resolution of `view` to the emulator thread.
    pub fn apply_view(shared: &Shared, view: &View) {
        shared.set_speed(view.speed);
        shared.set_render(view.render_scale(), view.xbrz);
    }

    /// The running game's enabled cheats to the emulator thread.
    pub fn load_cheats(&self) {
        let Some(rom) = lock(&self.rom).clone() else {
            return;
        };
        let list = cheats::load(&cheats::cheats_path(&self.instance, &rom));
        let on = lock(&self.config).view.cheats_enabled;
        self.shared.set_cheats(cheats::enabled_codes(&list, on));
    }

    pub fn save_config(&self, config: &GuiConfig) {
        if let Ok(text) = serde_json::to_string_pretty(config)
            && let Err(e) = std::fs::write(self.instance.config_path(), text)
        {
            log::error!("Failed to write config: {e}");
        }
    }

    /// Replaces the GUI config (volume, frame limiter, speed and
    /// resolution take effect at once) and saves it.
    pub fn set_config(&self, config: GuiConfig) {
        self.audio
            .set_volume(f32::from(config.volume.min(100)) / 100.0);
        self.shared.set_limit_framerate(config.limit_framerate);
        Self::apply_view(&self.shared, &config.view);
        self.save_config(&config);
        *lock(&self.config) = config;
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
    ///
    /// # Errors
    /// If the ROM cannot be loaded.
    pub fn start(&self, rom: &Path) -> Result<(), String> {
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
        self.load_cheats();

        let mut config = lock(&self.config);
        remember(&mut config.recent_roms, rom);
        config.selected_rom = Some(rom.to_owned());
        self.save_config(&config);
        drop(config);
        Ok(())
    }

    /// Applies a menu action the backend can carry out.
    ///
    /// # Errors
    /// If a game cannot be started or a folder / process cannot be opened.
    pub fn apply(&self, action: Action) -> Result<(), String> {
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
                crate::reveal(self.instance.root()).map_err(|e| e.to_string())?;
            }
            Action::LaunchInstance => crate::launch_instance().map_err(|e| e.to_string())?,
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
            | Action::SetSpeed(_)
            | Action::CycleSpeed
            | Action::SetRenderScale(_)
            | Action::ToggleXbrz
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
                    Action::SetSpeed(p) => view.speed = speed::valid(p),
                    Action::CycleSpeed => view.speed = speed::next(view.speed),
                    Action::SetRenderScale(n) => view.render_scale = n,
                    Action::ToggleXbrz => view.xbrz ^= true,
                    _ => {}
                }
                Self::apply_view(&self.shared, &config.view);
                self.save_config(&config);
                drop(config);
            }
            Action::ToggleCheats => {
                let mut config = lock(&self.config);
                config.view.cheats_enabled ^= true;
                self.save_config(&config);
                drop(config);
                self.load_cheats();
            }
            // Dialogs and the window are the frontend's.
            Action::OpenRom | Action::ImportSavefile | Action::Quit | Action::Open(_) => {}
        }
        Ok(())
    }
}

/// `.nds` files in `dir`, sorted by name.
pub fn rom_paths(dir: &Path) -> Vec<PathBuf> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The egui GUI writes `config.json` with snake_case keys.
    #[test]
    fn reads_the_egui_config() {
        let config: GuiConfig = serde_json::from_str(
            r#"{"rom_dir":"D:/roms","selected_rom":"D:/roms/a.nds","recent_roms":["D:/roms/a.nds"],"limit_framerate":false,"volume":40,"lan_name":"P1"}"#,
        )
        .unwrap_or_default();
        assert_eq!(config.rom_dir, PathBuf::from("D:/roms"));
        assert_eq!(config.recent_roms, [PathBuf::from("D:/roms/a.nds")]);
        assert!(!config.limit_framerate);
        assert_eq!(config.volume, 40);
    }
}
