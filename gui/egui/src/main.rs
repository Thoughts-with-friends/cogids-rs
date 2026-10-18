// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! egui frontend with melonDS's menu bar (File / System / View / Config /
//! Help, shared with the Tauri GUI through `ds_common::menu`).
//!
//! ```text
//!  update()
//!    1. Ctrl+key shortcuts            -> Action
//!    2. multiplayer.tick              (LAN session, ready light)
//!    3. menu bar (menu.rs)            -> Action
//!    4. apply(Action)                 open / reset / pause / view / dialogs
//!    5. screens: run the owed frames  (none while paused, one per step),
//!       draw both screens where ds_common::view::place puts them,
//!       stylus through the bottom screen's ScreenRect
//!    6. dialogs.rs windows
//! ```
mod cheat_dialog;
mod config;
mod dialogs;
mod font;
mod hd;
mod input;
mod menu;
mod multiplayer;

use self::config::{Config, load_config, save_config, set_config_path};
use self::dialogs::{Dialogs, Parts};
use self::input::Touch;
use self::multiplayer::Multiplayer;
use ds_common::menu::{Action, MenuState, menu_bar, remember};
use ds_common::view::{ScreenRect, place};
use ds_common::{AudioOutput, DS_FPS, Gamepads, Instance, Settings, cheats, speed};
use ds_emu::Emulator;
use ds_mem_addr::{PIXELS_PER_LINE, SCANLINES};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Height reserved for the menu bar when sizing the window (View > Screen
/// size).
const MENU_BAR_HEIGHT: f32 = 24.0;

/// Longest emulation may take per GUI frame when running faster than 1x or
/// unlimited (keeps the window responsive when the PC cannot keep up).
const UNLIMITED_BUDGET: Duration = Duration::from_millis(15);

// ===== util =====

/// Swaps the R and B channels of 0xAARRGGBB pixels in place, converting them
/// to the 0xAABBGGRR byte layout that `ColorImage::from_rgba_unmultiplied`
/// reads as little-endian RGBA. The DS screen is always opaque, so alpha is
/// forced to 0xFF (a powered-off engine leaves zeroed, fully transparent
/// pixels in its framebuffer).
fn swap_red_blue(buffer: &mut [u32]) {
    for px in buffer {
        *px = 0xFF00_0000
            | (*px & 0x0000_FF00)
            | ((*px & 0x00FF_0000) >> 16)
            | ((*px & 0x0000_00FF) << 16);
    }
}

/// `.nds` files in `dir`, sorted by name.
fn list_nds_files(dir: &Path) -> Vec<PathBuf> {
    let mut roms: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("nds"))
                })
                .collect()
        })
        .unwrap_or_default();
    roms.sort();
    roms
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

// ===== app =====
struct App {
    config: Config,
    emu: Option<Emulator>,
    /// The running game's ROM.
    rom: Option<PathBuf>,
    paused: bool,
    /// Frame step requested: run exactly one frame, then stay paused.
    step: bool,

    /// Upper and lower screen.
    textures: [Option<egui::TextureHandle>; 2],
    pixels: Vec<u32>,
    /// Frames ran since the textures were last filled, and the (scale,
    /// xBRZ) they were made with.
    new_frame: bool,
    /// Upscales frames at an internal resolution of 2x+ (started on first use).
    hd: Option<hd::HdWorker>,
    texture_mode: (usize, bool, bool),

    /// Emulated frames per second, measured.
    fps: f32,
    fps_frames: u32,
    fps_start: Instant,

    /// `.nds` files of `folder_dir` (File > ROM folder).
    folder_dir: PathBuf,
    folder_roms: Vec<PathBuf>,

    gamepads: Gamepads,
    touch: Touch,

    instance: Instance,
    settings: Settings,
    multiplayer: Multiplayer,
    dialogs: Dialogs,

    audio: AudioOutput,
    audio_buf: Vec<i16>,
    /// `audio_buf` fitted to the emulation speed.
    audio_out: Vec<i16>,
    stretch: speed::TimeStretch,
    meter: speed::SpeedMeter,
    /// Speed the meter was started for (0: start again).
    metered: u16,
    /// Emulated frames owed to real time (frame pacing).
    frame_debt: f64,
    last_tick: Instant,

    /// Last error, shown in the status bar until dismissed.
    error: Option<String>,
}

impl App {
    fn new(ctx: &egui::Context, instance: Instance, settings: Settings) -> Self {
        let mut config = load_config();
        if config.lan_name.trim().is_empty() || config.lan_name == "Player" {
            // Distinct default names, so players can tell each other apart.
            config.lan_name = format!("Player{}", instance.number());
        }
        let volume = config.volume.min(100);
        let folder_roms = list_nds_files(&config.rom_dir);

        Self {
            folder_dir: config.rom_dir.clone(),
            folder_roms,
            config,
            emu: None,
            rom: None,
            paused: false,
            step: false,
            textures: [None, None],
            pixels: vec![0; PIXELS_PER_LINE * SCANLINES],
            new_frame: false,
            texture_mode: (1, false, false),
            hd: None,
            fps: 0.0,
            fps_frames: 0,
            fps_start: Instant::now(),
            gamepads: Gamepads::new(),
            touch: Touch::default(),
            audio: AudioOutput::new(f32::from(volume) / 100.0),
            audio_buf: Vec::with_capacity(4096),
            audio_out: Vec::with_capacity(4096),
            stretch: speed::TimeStretch::default(),
            meter: speed::SpeedMeter::new(speed::NORMAL),
            metered: 0,
            frame_debt: 0.0,
            last_tick: Instant::now(),
            instance,
            settings,
            multiplayer: Multiplayer::new(ctx),
            dialogs: Dialogs::default(),
            error: None,
        }
    }

    // ===== actions =====

    /// Ctrl+letter shortcuts (plain letters are DS buttons).
    fn shortcut(ctx: &egui::Context) -> Option<Action> {
        ctx.input(|i| {
            if !i.modifiers.command {
                return None;
            }
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    ..
                } => key.name().chars().next().and_then(Action::from_shortcut),
                _ => None,
            })
        })
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::OpenRom => {
                let mut dialog = rfd::FileDialog::new().add_filter("Nintendo DS ROM", &["nds"]);
                if self.config.rom_dir.is_dir() {
                    dialog = dialog.set_directory(&self.config.rom_dir);
                }
                if let Some(path) = dialog.pick_file() {
                    self.start(ctx, path);
                }
            }
            Action::OpenRomPath(path) => self.start(ctx, path),
            Action::ClearRecent => self.config.recent_roms.clear(),
            Action::Eject | Action::Stop => self.stop(ctx),
            Action::ImportSavefile => self.import_savefile(ctx),
            Action::OpenDirectory => {
                if let Err(e) = ds_common::reveal(self.instance.root()) {
                    self.error = Some(format!("Cannot open the data directory: {e}"));
                }
            }
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::TogglePause => {
                self.paused = !self.paused;
                self.audio.clear();
            }
            Action::Reset => {
                if let Some(rom) = self.rom.clone() {
                    self.start(ctx, rom);
                }
            }
            Action::FrameStep => {
                self.paused = true;
                self.step = true;
                self.audio.clear();
            }
            Action::LaunchInstance => {
                if let Err(e) = ds_common::launch_instance() {
                    self.error = Some(format!("Cannot start another instance: {e}"));
                }
            }
            Action::ScreenSize(n) => {
                let view = &mut self.config.view;
                view.scale = f32::from(n);
                let p = place(view, 1.0, 1.0);
                let size = egui::vec2(
                    p.natural_w.mul_add(view.scale, 16.0),
                    p.natural_h.mul_add(view.scale, MENU_BAR_HEIGHT + 16.0),
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
            Action::SetRotation(r) => self.config.view.rotation = r,
            Action::SetGap(g) => self.config.view.gap = g,
            Action::SetLayout(l) => self.config.view.layout = l,
            Action::ToggleSwap => self.config.view.swap ^= true,
            Action::SetSizing(s) => self.config.view.sizing = s,
            Action::ToggleIntegerScaling => self.config.view.integer_scaling ^= true,
            Action::ToggleFiltering => self.config.view.filtering ^= true,
            Action::ToggleShowFps => self.config.view.show_fps ^= true,
            Action::ToggleLimitFramerate => {
                self.config.limit_framerate = !self.config.limit_framerate;
                self.audio.clear();
            }
            Action::SetSpeed(percent) => {
                self.config.view.speed = speed::valid(percent);
                self.audio.clear();
            }
            Action::CycleSpeed => {
                self.config.view.speed = speed::next(self.config.view.speed);
                self.audio.clear();
            }
            Action::SetRenderScale(n) => self.config.view.render_scale = n,
            Action::ToggleXbrz => self.config.view.xbrz ^= true,
            Action::ToggleCheats => {
                self.config.view.cheats_enabled ^= true;
                self.load_cheats();
            }
            Action::Open(window) => self.dialogs.open(window),
        }
        save_config(&self.config);
    }

    /// Hands the running game's enabled cheats to the emulator (on start,
    /// on the cheat dialog's OK and on System > Enable cheats).
    fn load_cheats(&mut self) {
        let (Some(emu), Some(rom)) = (self.emu.as_mut(), self.rom.as_deref()) else {
            return;
        };
        let list = cheats::load(&cheats::cheats_path(&self.instance, rom));
        let codes = cheats::enabled_codes(&list, self.config.view.cheats_enabled);
        log::info!("Cheats: {} of {} enabled", codes.len(), list.len());
        emu.set_cheats(codes);
    }

    /// Starts `path` (stopping the current game first so its save is
    /// flushed before anything is read).
    fn start(&mut self, ctx: &egui::Context, path: PathBuf) {
        if let Some(mut old) = self.emu.take() {
            Self::flush_save(&mut old);
        }
        self.audio.clear();

        let mut emu = Emulator::new();
        self.settings.apply(&mut emu);
        // Saves live in instances/instanceN/saves; bring over one kept next to the ROM.
        emu.config.save_dir = Some(self.instance.saves_dir());
        self.instance.import_legacy_save(&path);
        // Local wireless / LAN play (the Wi-Fi joins a session once the game
        // powers it on).
        emu.set_mp_link(Some(self.multiplayer.link()));

        match emu.load_rom(&path) {
            Ok(()) => {
                log::info!("Started {}", path.display());
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "{} - cogids (instance{})",
                    file_name(&path),
                    self.instance.number()
                )));
                remember(&mut self.config.recent_roms, &path);
                self.config.selected_rom = Some(path.clone());
                self.emu = Some(emu);
                self.rom = Some(path);
                self.load_cheats();
                self.paused = false;
                self.step = false;
                self.frame_debt = 0.0;
                self.last_tick = Instant::now();
                self.error = None;
            }
            Err(e) => {
                log::error!("Failed to load ROM {}: {e:?}", path.display());
                self.error = Some(format!("Cannot load {}: {e}", file_name(&path)));
                self.rom = None;
            }
        }
        save_config(&self.config);
    }

    fn stop(&mut self, ctx: &egui::Context) {
        self.audio.clear();
        if let Some(mut emu) = self.emu.take() {
            Self::flush_save(&mut emu);
        }
        self.rom = None;
        self.paused = false;
        self.textures = [None, None];
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "cogids (instance{})",
            self.instance.number()
        )));
    }

    /// File > Import savefile: stop (the running game would write its own
    /// save back over the import), back the old save up, copy, restart.
    fn import_savefile(&mut self, ctx: &egui::Context) {
        let Some(rom) = self.rom.clone() else {
            return;
        };
        let Some(source) = rfd::FileDialog::new()
            .add_filter("DS save", &["sav", "dsv", "bin"])
            .pick_file()
        else {
            return;
        };
        self.stop(ctx);
        match self.instance.import_save(&rom, &source) {
            Ok(_) => self.start(ctx, rom),
            Err(e) => self.error = Some(format!("Import failed: {e}")),
        }
    }

    /// Writes pending save data to `<rom>.sav` (also done on drop, but this
    /// reports failures).
    fn flush_save(emu: &mut Emulator) {
        if let Err(e) = emu.flush_save() {
            log::error!("Failed to write save file: {e}");
        }
    }

    // ===== UI =====

    fn ui_menu_bar(&mut self, ctx: &egui::Context) -> Option<Action> {
        if self.folder_dir != self.config.rom_dir {
            self.folder_dir.clone_from(&self.config.rom_dir);
            self.folder_roms = list_nds_files(&self.config.rom_dir);
        }
        let loaded = self.rom.as_deref().map(file_name);
        let state = MenuState {
            loaded: loaded.as_deref(),
            paused: self.paused,
            view: &self.config.view,
            limit_framerate: self.config.limit_framerate,
            recent: &self.config.recent_roms,
            folder_roms: &self.folder_roms,
            multiplayer: Ok(()),
        };
        let menus = menu_bar(&state);
        egui::TopBottomPanel::top("menu")
            .show(ctx, |ui| menu::bar(ui, &menus))
            .inner
    }

    fn ui_status(&mut self, ctx: &egui::Context) {
        let Some(error) = self.error.clone() else {
            return;
        };
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(0xE5, 0x39, 0x35), error);
                if ui.small_button("x").clicked() {
                    self.error = None;
                }
            });
        });
    }

    fn ui_central(&mut self, ctx: &egui::Context) -> Option<Action> {
        let mut action = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                if self.emu.is_some() {
                    self.run_frames(ctx);
                    self.draw_screens(ctx, ui);
                } else {
                    action = self.ui_welcome(ui);
                }
            });
        action
    }

    /// No game: how to start one, and the ROM folder's games.
    fn ui_welcome(&self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.heading("No game running");
            ui.label("File > Open ROM... (Ctrl+O), Open recent, or pick one below.");
            ui.add_space(12.0);
            egui::ScrollArea::vertical().show(ui, |ui| {
                for path in self.config.recent_roms.iter().chain(&self.folder_roms) {
                    if ui
                        .button(file_name(path))
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        action = Some(Action::OpenRomPath(path.clone()));
                    }
                }
            });
        });
        action
    }

    /// The emulation speed in percent (1x while in a LAN session).
    fn speed(&self) -> u16 {
        if self.multiplayer.in_session() {
            speed::NORMAL
        } else {
            speed::valid(self.config.view.speed)
        }
    }

    /// Runs the frames real time asks for (~59.83 per second at 1x): none while
    /// paused, exactly one for a frame step, as many as fit in
    /// [`UNLIMITED_BUDGET`] without the frame limiter (or above 1x).
    fn run_frames(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_tick).as_secs_f64();
        self.last_tick = now;

        let percent = self.speed();
        let frames = if self.paused {
            self.frame_debt = 0.0;
            self.metered = 0;
            u32::from(std::mem::take(&mut self.step))
        } else if self.multiplayer.busy() {
            0 // joining a LAN session: the backend is the worker's
        } else if self.config.limit_framerate {
            let fps = speed::fps(percent);
            self.frame_debt = elapsed.mul_add(fps, self.frame_debt);
            // Stalled (e.g. window dragged): don't race. ~4 frames' worth at 1x.
            let cap = 4.0 * fps / DS_FPS;
            if self.frame_debt > cap {
                self.frame_debt = cap / 4.0;
            }
            let frames = self.frame_debt.floor() as u32;
            self.frame_debt -= f64::from(frames);
            frames
        } else {
            u32::MAX
        };

        let Some(emu) = self.emu.as_mut() else {
            return;
        };
        // The 3D of every rendered frame is kept for the internal resolution.
        emu.gpu
            .set_render_scale(usize::from(self.config.view.render_scale()));
        let start = Instant::now();
        let mut ran = 0;
        if percent != self.metered {
            self.meter.reset(percent);
            self.stretch.reset();
            self.metered = percent;
        }
        let ratio = speed::stretch_ratio(percent, self.meter.speed());
        while ran < frames {
            input::apply_keyboard(ctx, emu);
            self.gamepads.apply(emu);
            self.touch.apply(emu);
            emu.run();
            ran += 1;

            emu.take_audio_samples(&mut self.audio_buf);
            if self.config.limit_framerate && !self.paused {
                // Other speeds keep the pitch (see `speed`).
                if percent == speed::NORMAL {
                    self.audio.push(&self.audio_buf);
                } else {
                    self.stretch
                        .process(&self.audio_buf, ratio, &mut self.audio_out);
                    self.audio.push(&self.audio_out);
                }
            }
            self.audio_buf.clear();
            if (frames == u32::MAX || percent > speed::NORMAL)
                && start.elapsed() >= UNLIMITED_BUDGET
            {
                // Too slow for this speed: run as fast as possible instead
                // of piling up debt.
                self.frame_debt = 0.0;
                break;
            }
        }

        self.meter.tick(ran);
        self.new_frame |= ran > 0;
        self.fps_frames += ran;
        let window = self.fps_start.elapsed();
        if window >= Duration::from_secs(1) {
            self.fps = self.fps_frames as f32 / window.as_secs_f32();
            self.fps_frames = 0;
            self.fps_start = Instant::now();
        }
    }

    /// Both screens where the View settings put them, plus the stylus.
    fn draw_screens(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(area, egui::Sense::click_and_drag());
        let placement = place(&self.config.view, area.width(), area.height());
        // Enlarging follows Screen filtering; shrinking (a high internal
        // resolution in a small window) is always smooth.
        let options = egui::TextureOptions {
            minification: egui::TextureFilter::Linear,
            ..if self.config.view.filtering {
                egui::TextureOptions::LINEAR
            } else {
                egui::TextureOptions::NEAREST
            }
        };

        // Internal resolution: 3D rendered larger, 2D upscaled (xBRZ or
        // nearest). Refilled only when a frame ran or the mode changed.
        let scale = usize::from(self.config.view.render_scale());
        let mode = (
            scale,
            self.config.view.xbrz && scale >= 2,
            self.config.view.filtering,
        );
        let stale = self.new_frame || mode != self.texture_mode || self.textures[0].is_none();
        let upload = |textures: &mut [Option<egui::TextureHandle>; 2],
                      index: usize,
                      width: usize,
                      pixels: &mut Vec<u32>| {
            let height = pixels.len() / width;
            swap_red_blue(pixels);
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [width, height],
                bytemuck::cast_slice(pixels),
            );
            match &mut textures[index] {
                Some(tex) => tex.set(image, options),
                slot @ None => {
                    let name = ["upper", "lower"][index];
                    *slot = Some(ctx.load_texture(name, image, options));
                }
            }
        };
        if scale >= 2 {
            // Upscaled on the HD worker (see `hd`); shown when it is done.
            let hd = self.hd.get_or_insert_with(|| hd::HdWorker::new(ctx));
            if let Some(emu) = self.emu.as_mut().filter(|_| stale)
                && hd.submit(emu.gpu.hd_frame(), mode.0, mode.1, self.paused)
            {
                self.new_frame = false;
                self.texture_mode = mode;
            }
            if let Some(mut image) = hd.take() {
                for (index, pixels) in image.screens.iter_mut().enumerate() {
                    upload(&mut self.textures, index, image.width, pixels);
                }
            }
        } else if let Some(emu) = self.emu.as_mut().filter(|_| stale) {
            self.new_frame = false;
            self.texture_mode = mode;
            for index in 0..2 {
                let width = emu.gpu.screen_hd(index == 0, 1, false, &mut self.pixels);
                upload(&mut self.textures, index, width, &mut self.pixels);
            }
        }

        for (screen, tex) in placement.screens.iter().zip(&self.textures) {
            let (Some(screen), Some(tex)) = (screen, tex) else {
                continue;
            };
            paint_screen(ui, tex, screen, area.min);
        }

        if self.config.view.show_fps {
            let percent = self.speed();
            let text = if self.paused {
                format!("{:.1} fps  (paused)", self.fps)
            } else if percent == speed::NORMAL {
                format!("{:.1} fps", self.fps)
            } else {
                format!("{:.1} fps  [{}]", self.fps, speed::label(percent))
            };
            ui.painter().text(
                area.min + egui::vec2(6.0, 4.0),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::monospace(13.0),
                egui::Color32::from_rgb(0x8F, 0xE0, 0x8F),
            );
        }

        // Stylus input for the next frame.
        self.touch
            .update(ctx, &response, placement.screens[1].as_ref(), area.min);
    }
}

/// Paints one screen texture into `screen.rect` (relative to `origin`),
/// rotated clockwise by `screen.rotation`.
fn paint_screen(ui: &egui::Ui, tex: &egui::TextureHandle, screen: &ScreenRect, origin: egui::Pos2) {
    let r = screen.rect;
    let rect = egui::Rect::from_min_size(origin + egui::vec2(r.x, r.y), egui::vec2(r.w, r.h));
    // The image is laid out unrotated, centred on the box, then turned.
    let unrotated = if screen.rotation.is_multiple_of(180) {
        rect
    } else {
        egui::Rect::from_center_size(rect.center(), egui::vec2(rect.height(), rect.width()))
    };
    egui::Image::new(tex)
        .rotate(
            f32::from(screen.rotation).to_radians(),
            egui::Vec2::splat(0.5),
        )
        .paint_at(ui, unrotated);
}

// ===== eframe =====
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut actions = Vec::new();
        actions.extend(Self::shortcut(ctx));
        // Left stick presses step the emulation speed.
        self.gamepads.poll();
        let presses = self.gamepads.take_speed_presses();
        if self.emu.is_some() {
            actions.extend((0..presses).map(|_| Action::CycleSpeed));
        }

        let running = self.emu.is_some() && !self.paused;
        self.multiplayer
            .tick(self.emu.as_mut(), running, &mut self.config);

        actions.extend(self.ui_menu_bar(ctx));
        self.ui_status(ctx);
        actions.extend(self.ui_central(ctx));

        let gamepads = self.gamepads.names();
        let mut parts = Parts {
            config: &mut self.config,
            settings: &mut self.settings,
            instance: &self.instance,
            multiplayer: &mut self.multiplayer,
            audio: &self.audio,
            rom: self.rom.as_deref(),
            gamepads,
        };
        self.dialogs.show(ctx, &mut parts);
        if self.dialogs.take_cheats_applied() {
            self.load_cheats();
        }

        for action in actions {
            self.apply(ctx, action);
        }
        ctx.request_repaint();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(emu) = self.emu.as_mut() {
            Self::flush_save(emu);
        }
        save_config(&self.config);
    }
}

// ===== entry =====
fn main() -> eframe::Result<()> {
    let instance = Instance::open();
    set_config_path(instance.config_path());
    let mut settings = Settings::load(&instance.settings_path());
    if settings.ensure_mac() {
        settings.save(&instance.settings_path());
    }

    if let Err(err) = ds_common::log::setup_logging_in(&instance.logs_dir()) {
        log::error!("Failed to start logging: {err}");
    }
    log::info!(
        "Started (logs, saves and settings in {}, Wi-Fi MAC {})",
        instance.root().display(),
        settings.mac_address.as_deref().unwrap_or("-")
    );

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        &format!("cogids (instance{})", instance.number()),
        options,
        Box::new(|cc| {
            self::font::setup_custom_fonts::<&std::path::Path>(&cc.egui_ctx, None);
            Ok(Box::new(App::new(&cc.egui_ctx, instance, settings)))
        }),
    )
}
