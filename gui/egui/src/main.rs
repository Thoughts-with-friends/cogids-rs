// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod audio;
mod config;
mod font;
mod input;
mod instance;

use self::audio::AudioOutput;
use self::config::{Config, ScreenLayout, load_config, save_config, set_config_path};
use self::input::{Gamepads, Touch};

/// Native DS refresh rate (33.513982 MHz / (355 * 263 * 6)).
const DS_FPS: f64 = 59.826_098;
use self::instance::{Instance, Settings};
use eframe::egui;
use lunaris_ds_emu::Emulator;
use lunaris_ds_mem_const::{PIXELS_PER_LINE, SCANLINES};
use std::{path::PathBuf, time::Instant};

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

fn list_nds_files(dir: &std::path::Path) -> Vec<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("nds"))
            })
            .collect()
    } else {
        vec![]
    }
}

// ===== app =====
struct App {
    config: Config,
    emu: Option<Emulator>,
    is_running: bool,

    rom_list: Vec<PathBuf>,

    upper_tex: Option<egui::TextureHandle>,
    lower_tex: Option<egui::TextureHandle>,

    last_time: Instant,
    frame_count: u64,
    fps: f32,
    rom_list_open: bool,

    gamepads: Gamepads,
    touch: Touch,

    instance: Instance,
    settings: Settings,

    audio: AudioOutput,
    audio_buf: Vec<i16>,
    /// Emulated frames owed to real time (frame pacing).
    frame_debt: f64,
    last_tick: Instant,
}

impl App {
    fn new(instance: Instance, settings: Settings) -> Self {
        let config = load_config();
        let config_volume = config.volume.min(100);
        let rom_list = list_nds_files(&config.rom_dir);

        Self {
            config,
            emu: None,
            is_running: false,
            rom_list,
            upper_tex: None,
            lower_tex: None,
            last_time: Instant::now(),
            frame_count: 0,
            fps: 0.0,
            rom_list_open: false,
            gamepads: Gamepads::new(),
            touch: Touch::default(),
            audio: AudioOutput::new(f32::from(config_volume) / 100.0),
            audio_buf: Vec::with_capacity(4096),
            frame_debt: 0.0,
            last_tick: Instant::now(),
            instance,
            settings,
        }
    }

    // ===== UI =====
    fn ui_top(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("ROM DIR:");

                let mut dir_str = self.config.rom_dir.to_string_lossy().to_string();
                if ui.text_edit_singleline(&mut dir_str).changed() {
                    self.config.rom_dir = dir_str.into();
                    self.rom_list = list_nds_files(&self.config.rom_dir);
                }
            });

            ui.separator();

            self.ui_rom_list(ui);

            ui.separator();

            self.ui_run_button(ui);
        });
    }

    fn ui_run_button(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Run").clicked() {
                self.run_emulator();
            }

            if ui.button("Stop").clicked() {
                self.stop_emulator();
            }

            ui.separator();

            ui.label("Scale: ");
            ui.add(egui::Slider::new(&mut self.config.scale, 1.0..=5.0));

            ui.separator();
            ui.label("Layout:");
            ui.radio_value(&mut self.config.layout, ScreenLayout::Vertical, "Vertical");
            ui.radio_value(
                &mut self.config.layout,
                ScreenLayout::Horizontal,
                "Horizontal",
            );

            ui.separator();
            ui.label("Volume:");
            if ui
                .add(egui::Slider::new(&mut self.config.volume, 0..=100).suffix("%"))
                .changed()
            {
                self.audio.set_volume(f32::from(self.config.volume) / 100.0);
            }

            ui.separator();
            ui.checkbox(&mut self.config.show_fps, "FPS");
            self.update_fps(ui);
        });

        ui.horizontal(|ui| {
            let pads = self.gamepads.names();
            if pads.is_empty() {
                ui.small("Controller: none (keyboard: arrows, X=A, Z=B, S=X, A=Y, Q=L, W=R, Enter=START, Backspace=SELECT)");
            } else {
                ui.small(format!("Controller: {}", pads.join(", ")));
            }
        });
    }

    fn ui_rom_list(&mut self, ui: &mut egui::Ui) {
        ui.separator();

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                let label = if self.rom_list_open {
                    "ROM List ▼"
                } else {
                    "ROM List ▶"
                };

                if ui.button(label).clicked() {
                    self.rom_list_open = !self.rom_list_open;
                }

                if self.rom_list_open {
                    egui::ScrollArea::vertical()
                        .max_height(150.0)
                        .show(ui, |ui| {
                            for path in &self.rom_list {
                                let name = path.file_name().unwrap_or_default().to_string_lossy();

                                let selected =
                                    self.config.selected_rom.as_ref().is_some_and(|p| p == path);

                                let resp = ui.selectable_label(selected, name);

                                if resp.clicked() {
                                    self.config.selected_rom = Some(path.clone());
                                }
                            }
                        });
                }
            });

            ui.separator();

            ui.vertical(|ui| {
                ui.label("Selected:");

                if let Some(path) = &self.config.selected_rom {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();

                    ui.monospace(name);

                    ui.small(path.to_string_lossy());
                } else {
                    ui.label("None");
                }
            });
        });
    }

    fn ui_emulator(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.is_running && self.emu.is_some() {
                self.render_frame(ctx, ui);
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a ROM and press Run");
                });
            }
        });
    }

    // ===== emulator control =====
    fn run_emulator(&mut self) {
        let Some(path) = &self.config.selected_rom else {
            tracing::error!("No ROM selected");
            return;
        };

        // Flush the running game first so a restart reloads its latest save.
        if let Some(mut old) = self.emu.take() {
            Self::flush_save(&mut old);
        }

        let mut emu = Emulator::new();
        // emu.config.test = true; // enable disassemble
        self.settings.apply(&mut emu);
        // Saves live in instances/instance1/saves; bring over one kept next to the ROM.
        emu.config.save_dir = Some(self.instance.saves_dir());
        self.instance.import_legacy_save(path);

        match emu.load_rom(path) {
            Ok(_) => {
                self.emu = Some(emu);
                self.is_running = true;
                save_config(&self.config);
            }
            Err(e) => {
                tracing::error!("Failed to load ROM: {e:?}");
                self.is_running = false;
            }
        }
    }

    fn stop_emulator(&mut self) {
        self.is_running = false;
        self.audio.clear();
        if let Some(mut emu) = self.emu.take() {
            Self::flush_save(&mut emu);
        }
    }

    /// Writes pending save data to `<rom>.sav` (also done on drop, but this
    /// reports failures).
    fn flush_save(emu: &mut Emulator) {
        if let Err(e) = emu.flush_save() {
            tracing::error!("Failed to write save file: {e}");
        }
    }

    // ===== rendering =====
    /// Runs one emulated frame with the current input and draws both screens.
    fn render_frame(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        const PIXEL: usize = PIXELS_PER_LINE * SCANLINES;
        let Some(emu) = self.emu.as_mut() else {
            return;
        };

        let mut upper_buffer = vec![0_u32; PIXEL];
        let mut lower_buffer = vec![0_u32; PIXEL];

        // Run as many frames as real time asks for (~59.83 per second), so
        // speed and audio do not depend on the monitor refresh rate.
        let now = Instant::now();
        self.frame_debt = now
            .duration_since(self.last_tick)
            .as_secs_f64()
            .mul_add(DS_FPS, self.frame_debt);
        self.last_tick = now;
        if self.frame_debt > 4.0 {
            self.frame_debt = 1.0; // stalled (e.g. window dragged): don't race
        }
        let frames = self.frame_debt.floor() as u32;
        self.frame_debt -= f64::from(frames);

        self.gamepads.poll();
        for _ in 0..frames {
            input::apply_keyboard(ctx, emu);
            self.gamepads.apply(emu);
            self.touch.apply(emu);
            emu.run();

            emu.take_audio_samples(&mut self.audio_buf);
            self.audio.push(&self.audio_buf);
            self.audio_buf.clear();
        }
        emu.get_upper_frame(&mut upper_buffer);
        emu.get_lower_frame(&mut lower_buffer);

        // The core emits pixels as 0xAARRGGBB (CorgiDS/Qt ARGB32 convention),
        // but `from_rgba_unmultiplied` expects little-endian RGBA byte order.
        // Swap the R and B channels here so colors aren't flipped on screen.
        swap_red_blue(&mut upper_buffer);
        swap_red_blue(&mut lower_buffer);

        let upper_img = egui::ColorImage::from_rgba_unmultiplied(
            [PIXELS_PER_LINE, SCANLINES],
            bytemuck::cast_slice(&upper_buffer),
        );
        let lower_img = egui::ColorImage::from_rgba_unmultiplied(
            [PIXELS_PER_LINE, SCANLINES],
            bytemuck::cast_slice(&lower_buffer),
        );

        Self::update_texture(ctx, &mut self.upper_tex, "upper", upper_img);
        Self::update_texture(ctx, &mut self.lower_tex, "lower", lower_img);

        let (Some(upper), Some(lower)) = (self.upper_tex.as_ref(), self.lower_tex.as_ref()) else {
            return;
        };
        let scale = self.config.scale;
        let size = upper.size_vec2() * scale;

        let draw = |ui: &mut egui::Ui| {
            ui.add(egui::Image::new(upper).fit_to_exact_size(size));
            ui.add_space(8.0);
            ui.add(
                egui::Image::new(lower)
                    .fit_to_exact_size(size)
                    .sense(egui::Sense::click_and_drag()),
            )
        };
        let lower_resp = match self.config.layout {
            ScreenLayout::Vertical => ui.vertical_centered(draw).inner,
            ScreenLayout::Horizontal => {
                ui.horizontal_centered(|ui| {
                    // Center the pair horizontally as well.
                    let total = size.x.mul_add(2.0, 8.0);
                    ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                    draw(ui)
                })
                .inner
            }
        };

        // Stylus input for the next frame.
        self.touch.update(ctx, &lower_resp);
    }

    fn update_texture(
        ctx: &egui::Context,
        tex_slot: &mut Option<egui::TextureHandle>,
        name: &str,
        img: egui::ColorImage,
    ) {
        if let Some(tex) = tex_slot {
            tex.set(img, egui::TextureOptions::NEAREST);
        } else {
            *tex_slot = Some(ctx.load_texture(name, img, egui::TextureOptions::NEAREST));
        }
    }

    // ===== fps =====
    fn update_fps(&mut self, ui: &mut egui::Ui) {
        self.frame_count += 1;

        let now = Instant::now();
        let elapsed = now.duration_since(self.last_time).as_secs_f32();

        if elapsed >= 1.0 {
            self.fps = self.frame_count as f32 / elapsed;
            self.frame_count = 0;
            self.last_time = now;
        }

        if self.config.show_fps {
            ui.label(format!(": {:.1}", self.fps));
        }
    }
}

// ===== eframe =====
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui_top(ctx);
        self.ui_emulator(ctx);
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
    let settings = Settings::load(&instance.settings_path());

    match tracing_rotation::init(instance.logs_dir(), "lunaris.log") {
        Ok(()) => {
            if let Err(e) = tracing_rotation::change_level(&settings.log_level) {
                eprintln!("Invalid log level {:?}: {e}", settings.log_level);
            }
        }
        Err(e) => eprintln!("Failed to start logging: {e}"),
    }
    tracing::info!("Started (logs, saves and settings in ./instances/instance1)");

    let options = eframe::NativeOptions::default();

    eframe::run_native(
        "NDS Emulator",
        options,
        Box::new(|cc| {
            self::font::setup_custom_fonts::<&std::path::Path>(&cc.egui_ctx, None);
            Ok(Box::new(App::new(instance, settings)))
        }),
    )
}
