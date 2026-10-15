//! The windows the menus open (melonDS's dialogs): ROM info, the Config
//! settings, multiplayer and About.
//!
//! ```text
//!  Action::Open(Window) -> Dialogs::open(window)
//!  every frame          -> Dialogs::show(ctx, Parts { config, settings, ... })
//!                          one egui::Window per open dialog, closable
//! ```
//!
//! Emulator settings (`settings.json`) are saved as soon as they change and
//! take effect when a game is next started or reset.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ds_common::menu::Window;
use ds_common::rom_info::{ICON_SIZE, RomInfo};
use ds_common::{AudioOutput, Instance, Settings, format_mac, parse_mac};
use eframe::egui;

use crate::cheat_dialog::{CheatEditor, Outcome};
use crate::config::Config;
use crate::multiplayer::Multiplayer;

/// What the dialogs read and change.
pub struct Parts<'a> {
    pub config: &'a mut Config,
    pub settings: &'a mut Settings,
    pub instance: &'a Instance,
    pub multiplayer: &'a mut Multiplayer,
    pub audio: &'a AudioOutput,
    /// The running game's ROM, if any.
    pub rom: Option<&'a Path>,
    pub gamepads: Vec<String>,
}

#[derive(Default)]
pub struct Dialogs {
    open: HashSet<Window>,
    /// ROM info of this path, with its icon.
    rom_info: Option<(
        PathBuf,
        Result<RomInfo, String>,
        Option<egui::TextureHandle>,
    )>,
    /// The Wifi settings' MAC text box.
    mac_text: Option<String>,
    /// System > Setup cheat codes, for the running game.
    cheats: Option<CheatEditor>,
    /// The cheat list was saved (OK); the app reloads the codes.
    cheats_applied: bool,
}

/// A path text box with a Browse button; `true` if it changed.
fn path_row(ui: &mut egui::Ui, label: &str, path: &mut Option<PathBuf>, folder: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let mut text = path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        if ui
            .add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(320.0)
                    .hint_text("(built-in)"),
            )
            .changed()
        {
            *path = (!text.trim().is_empty()).then(|| PathBuf::from(text.trim()));
            changed = true;
        }
        if ui.button("Browse...").clicked() {
            let dialog = rfd::FileDialog::new();
            let picked = if folder {
                dialog.pick_folder()
            } else {
                dialog.pick_file()
            };
            if let Some(picked) = picked {
                *path = Some(picked);
                changed = true;
            }
        }
        if path.is_some() && ui.button("Clear").clicked() {
            *path = None;
            changed = true;
        }
    });
    changed
}

impl Dialogs {
    pub fn open(&mut self, window: Window) {
        self.open.insert(window);
    }

    /// The cheat dialog's OK was pressed since the last call.
    pub const fn take_cheats_applied(&mut self) -> bool {
        std::mem::replace(&mut self.cheats_applied, false)
    }

    /// The cheat window (opened per game; Cancel drops its changes).
    fn show_cheats(&mut self, ctx: &egui::Context, parts: &Parts<'_>) {
        let Some(rom) = parts.rom.filter(|_| self.open.contains(&Window::Cheats)) else {
            self.open.remove(&Window::Cheats);
            self.cheats = None;
            return;
        };
        if self.cheats.as_ref().is_none_or(|c| c.rom() != rom) {
            self.cheats = Some(CheatEditor::new(ctx, parts.instance, rom));
        }
        let Some(editor) = self.cheats.as_mut() else {
            return;
        };
        match editor.show(ctx) {
            Outcome::Open => {}
            outcome => {
                self.cheats_applied |= outcome == Outcome::Applied;
                self.open.remove(&Window::Cheats);
                self.cheats = None;
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, parts: &mut Parts<'_>) {
        self.show_cheats(ctx, parts);
        for window in [
            Window::RomInfo,
            Window::Multiplayer,
            Window::EmuSettings,
            Window::Input,
            Window::Audio,
            Window::Wifi,
            Window::Paths,
            Window::About,
        ] {
            if !self.open.contains(&window) {
                continue;
            }
            let mut open = true;
            let title = match window {
                Window::RomInfo => "ROM info",
                Window::Multiplayer => "Multiplayer",
                Window::EmuSettings => "Emu settings",
                Window::Input => "Input and hotkeys",
                Window::Audio => "Audio settings",
                Window::Wifi => "Wifi settings",
                Window::Paths => "Path settings",
                Window::About => "About",
                Window::Cheats => continue,
            };
            egui::Window::new(title)
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| match window {
                    Window::RomInfo => self.rom_info(ui, parts.rom),
                    Window::Multiplayer => {
                        let mac = parts.settings.mac_address.clone().unwrap_or_default();
                        parts
                            .multiplayer
                            .ui(ui, parts.config, &mac, parts.instance.number());
                    }
                    Window::EmuSettings => emu_settings(ui, parts),
                    Window::Input => input(ui, &parts.gamepads),
                    Window::Audio => audio(ui, parts),
                    Window::Wifi => self.wifi(ui, parts),
                    Window::Paths => paths(ui, parts),
                    Window::About => about(ui),
                    Window::Cheats => {}
                });
            if !open {
                self.open.remove(&window);
            }
        }
    }

    fn rom_info(&mut self, ui: &mut egui::Ui, rom: Option<&Path>) {
        let Some(rom) = rom else {
            ui.label("No game is running.");
            return;
        };
        if self.rom_info.as_ref().is_none_or(|(p, ..)| p != rom) {
            let info = RomInfo::read(rom).map_err(|e| e.to_string());
            let icon = info
                .as_ref()
                .ok()
                .filter(|i| !i.icon_rgba.is_empty())
                .map(|i| {
                    ui.ctx().load_texture(
                        "rom-icon",
                        egui::ColorImage::from_rgba_unmultiplied(
                            [ICON_SIZE, ICON_SIZE],
                            &i.icon_rgba,
                        ),
                        egui::TextureOptions::NEAREST,
                    )
                });
            self.rom_info = Some((rom.to_owned(), info, icon));
        }
        let Some((_, info, icon)) = &self.rom_info else {
            return;
        };
        let info = match info {
            Ok(info) => info,
            Err(e) => {
                ui.colored_label(egui::Color32::RED, e);
                return;
            }
        };
        ui.horizontal(|ui| {
            if let Some(icon) = icon {
                ui.add(egui::Image::new(icon).fit_to_exact_size(egui::vec2(64.0, 64.0)));
            }
            ui.vertical(|ui| {
                for title in [&info.title_ja, &info.title_en].into_iter().flatten() {
                    ui.label(title);
                }
            });
        });
        ui.separator();
        egui::Grid::new("rom-info")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                let mut row = |k: &str, v: String| {
                    ui.label(k);
                    ui.monospace(v);
                    ui.end_row();
                };
                row("Title", info.title.clone());
                row("Game code", info.game_code.clone());
                row("Maker code", info.maker_code.clone());
                row("Version", info.version.to_string());
                row("File size", format!("{} bytes", info.file_size));
                row("Capacity", format!("{} MiB", info.capacity / (1024 * 1024)));
                for (cpu, b) in [("ARM9", info.arm9), ("ARM7", info.arm7)] {
                    row(
                        cpu,
                        format!(
                            "ROM {:08X}  entry {:08X}  RAM {:08X}  size {:08X}",
                            b.rom_offset, b.entry, b.ram_address, b.size
                        ),
                    );
                }
                row("Path", info.path.clone());
            });
    }

    fn wifi(&mut self, ui: &mut egui::Ui, parts: &mut Parts<'_>) {
        let text = self
            .mac_text
            .get_or_insert_with(|| parts.settings.mac_address.clone().unwrap_or_default());
        ui.label(format!(
            "Instance: instance{}  (each instance keeps its own MAC)",
            parts.instance.number()
        ));
        ui.horizontal(|ui| {
            ui.label("MAC address:");
            ui.add(egui::TextEdit::singleline(text).desired_width(160.0));
            let valid = parse_mac(text);
            if ui
                .add_enabled(valid.is_some(), egui::Button::new("Save"))
                .clicked()
                && let Some(mac) = valid
            {
                parts.settings.mac_address = Some(format_mac(mac));
                parts.settings.save(&parts.instance.settings_path());
            }
            if ui.button("Randomize").clicked() {
                parts.settings.mac_address = None;
                parts.settings.ensure_mac();
                parts.settings.save(&parts.instance.settings_path());
                *text = parts.settings.mac_address.clone().unwrap_or_default();
            }
            if valid.is_none() {
                ui.colored_label(egui::Color32::RED, "format: 00:09:BF:12:34:56");
            }
        });
        ui.small(
            "Consoles playing together need different MACs. Takes effect on the next start/reset.",
        );
    }
}

fn emu_settings(ui: &mut egui::Ui, parts: &mut Parts<'_>) {
    let s = &mut *parts.settings;
    let mut changed = false;
    changed |= ui
        .checkbox(
            &mut s.hle_bios,
            "High-level BIOS emulation (no BIOS dump needed)",
        )
        .changed();
    changed |= path_row(ui, "ARM9 BIOS:", &mut s.arm9_bios_path, false);
    changed |= path_row(ui, "ARM7 BIOS:", &mut s.arm7_bios_path, false);
    changed |= path_row(ui, "Firmware:", &mut s.firmware_path, false);
    ui.horizontal(|ui| {
        ui.label("Frame skip:");
        changed |= ui
            .add(egui::DragValue::new(&mut s.frameskip).range(0..=9))
            .changed();
    });
    ui.horizontal(|ui| {
        ui.label("Log level:");
        egui::ComboBox::from_id_salt("log-level")
            .selected_text(s.log_level.clone())
            .show_ui(ui, |ui| {
                for level in ["error", "warn", "info", "debug", "trace"] {
                    changed |= ui
                        .selectable_value(&mut s.log_level, level.to_owned(), level)
                        .changed();
                }
            });
    });
    if changed {
        s.save(&parts.instance.settings_path());
    }
    ui.small("Saved to settings.json; applies when a game is next started or reset (log level: on restart).");
}

fn input(ui: &mut egui::Ui, gamepads: &[String]) {
    egui::Grid::new("keys")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            ui.strong("DS");
            ui.strong("Keyboard");
            ui.strong("Gamepad");
            ui.end_row();
            for (ds, key, pad) in [
                ("A", "X", "right face button"),
                ("B", "Z", "bottom face button"),
                ("X", "S", "top face button"),
                ("Y", "A", "left face button"),
                ("L / R", "Q / W", "shoulders / triggers"),
                ("START / SELECT", "Enter / Backspace", "Start / Back"),
                ("D-pad", "Arrow keys", "D-pad / left stick"),
                ("Touch screen", "Mouse (left button)", "-"),
            ] {
                ui.label(ds);
                ui.monospace(key);
                ui.label(pad);
                ui.end_row();
            }
        });
    ui.separator();
    ui.strong("Hotkeys");
    for (what, key) in [
        ("Open ROM", "Ctrl+O"),
        ("Pause / resume", "Ctrl+P"),
        ("Reset", "Ctrl+R"),
        ("Frame step", "Ctrl+N"),
        ("Next emulation speed", "Left stick press (gamepad)"),
        ("Quit", "Ctrl+Q"),
    ] {
        ui.horizontal(|ui| {
            ui.monospace(format!("{key:<8}"));
            ui.label(what);
        });
    }
    ui.separator();
    ui.label(if gamepads.is_empty() {
        "Gamepads: none connected".to_owned()
    } else {
        format!("Gamepads: {}", gamepads.join(", "))
    });
    ui.small("The mapping is fixed; remapping is not implemented yet.");
}

fn audio(ui: &mut egui::Ui, parts: &mut Parts<'_>) {
    ui.horizontal(|ui| {
        ui.label("Volume:");
        if ui
            .add(egui::Slider::new(&mut parts.config.volume, 0..=100).suffix("%"))
            .changed()
        {
            parts
                .audio
                .set_volume(f32::from(parts.config.volume) / 100.0);
        }
    });
}

fn paths(ui: &mut egui::Ui, parts: &mut Parts<'_>) {
    let mut dir = Some(parts.config.rom_dir.clone()).filter(|d| !d.as_os_str().is_empty());
    if path_row(ui, "ROM folder:", &mut dir, true) {
        parts.config.rom_dir = dir.unwrap_or_default();
    }
    ui.separator();
    let root = std::path::absolute(parts.instance.root())
        .unwrap_or_else(|_| parts.instance.root().to_owned());
    ui.label("Saves, settings and logs (this instance):");
    ui.horizontal(|ui| {
        ui.monospace(root.display().to_string());
        if ui.button("Open").clicked()
            && let Err(e) = ds_common::reveal(&root)
        {
            log::error!("Failed to open {}: {e}", root.display());
        }
    });
}

fn about(ui: &mut egui::Ui) {
    ui.heading("cogids-rs");
    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
    ui.label("A Nintendo DS emulator written in Rust.");
    ui.separator();
    ui.label("Core based on CorgiDS; Wi-Fi and multiplayer after melonDS (melonds_net).");
    ui.label("Menus laid out after melonDS.");
    ui.label("License: GPL-3.0");
    ui.hyperlink_to(
        "github.com/Thoughts-with-friends/cogids-rs",
        "https://github.com/Thoughts-with-friends/cogids-rs",
    );
}
