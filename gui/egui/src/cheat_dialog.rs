//! System > Setup cheat codes: the game's info on the left, its cheat list
//! (checkbox, name, type) in the middle and the selected cheat's name, memo
//! and code on the right (layout of `docs/design/cheat_gui.png`).
//!
//! ```text
//!  open  -> CheatEditor::new: cheats/<rom>.json + ROM info
//!  click a row        -> its name / memo / code go to the editor
//!  Add / Save / Delete / checkboxes -> the working list
//!  OK     -> write the file, `applied` = true (main hands the codes to
//!            the emulator)        Cancel -> forget the changes
//! ```
use std::path::{Path, PathBuf};

use ds_common::cheats::{self, Cheat};
use ds_common::rom_info::{ICON_SIZE, RomInfo};
use eframe::egui;

const LIST_WIDTH: f32 = 380.0;
const EDITOR_WIDTH: f32 = 380.0;
const INFO_WIDTH: f32 = 230.0;
const SELECTED: egui::Color32 = egui::Color32::from_rgb(0x2B, 0x6C, 0x96);

/// What the window asks of the app after a frame.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Open,
    /// OK: the list was saved; reload the emulator's codes.
    Applied,
    Closed,
}

pub struct CheatEditor {
    rom: PathBuf,
    path: PathBuf,
    /// The working list (written on OK).
    list: Vec<Cheat>,
    selected: Option<usize>,
    /// The selected cheat as edited (written to the list by Save).
    name: String,
    memo: String,
    code: String,
    info: Option<RomInfo>,
    icon: Option<egui::TextureHandle>,
    error: Option<String>,
}

impl CheatEditor {
    pub fn new(ctx: &egui::Context, instance: &ds_common::Instance, rom: &Path) -> Self {
        let path = cheats::cheats_path(instance, rom);
        let info = RomInfo::read(rom).ok();
        let icon = info.as_ref().filter(|i| !i.icon_rgba.is_empty()).map(|i| {
            ctx.load_texture(
                "cheat-rom-icon",
                egui::ColorImage::from_rgba_unmultiplied([ICON_SIZE, ICON_SIZE], &i.icon_rgba),
                egui::TextureOptions::NEAREST,
            )
        });
        let mut editor = Self {
            rom: rom.to_owned(),
            list: cheats::load(&path),
            path,
            selected: None,
            name: String::new(),
            memo: String::new(),
            code: String::new(),
            info,
            icon,
            error: None,
        };
        editor.select(if editor.list.is_empty() {
            None
        } else {
            Some(0)
        });
        editor
    }

    /// The game this editor is for.
    pub fn rom(&self) -> &Path {
        &self.rom
    }

    fn select(&mut self, index: Option<usize>) {
        self.selected = index.filter(|&i| i < self.list.len());
        let cheat = self
            .selected
            .map(|i| self.list[i].clone())
            .unwrap_or_default();
        self.name = cheat.name;
        self.memo = cheat.memo;
        self.code = cheat.code;
    }

    /// The editor differs from the selected cheat.
    fn edited(&self) -> bool {
        self.selected.is_some_and(|i| {
            let c = &self.list[i];
            c.name != self.name || c.memo != self.memo || c.code != self.code
        })
    }

    fn save_selected(&mut self) {
        if let Some(i) = self.selected {
            let c = &mut self.list[i];
            c.name.clone_from(&self.name);
            c.memo.clone_from(&self.memo);
            c.code.clone_from(&self.code);
        }
    }

    /// Draws the window; returns what happened.
    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let mut outcome = Outcome::Open;
        let mut open = true;
        egui::Window::new("Cheats")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    self.ui_info(ui);
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_width(LIST_WIDTH + EDITOR_WIDTH + 16.0);
                        self.ui_cheats(ui);
                    });
                });
                ui.separator();
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                let row = egui::vec2(ui.min_rect().width(), 24.0);
                let layout = egui::Layout::right_to_left(egui::Align::Center);
                ui.allocate_ui_with_layout(row, layout, |ui| {
                    if ui.button("Cancel").clicked() {
                        outcome = Outcome::Closed;
                    }
                    if ui.button("OK").clicked() {
                        outcome = self.apply();
                    }
                });
            });
        if !open {
            outcome = Outcome::Closed;
        }
        outcome
    }

    /// OK: keeps an unsaved edit, writes the file.
    fn apply(&mut self) -> Outcome {
        self.save_selected_if_valid();
        match cheats::save(&self.path, &self.list) {
            Ok(()) => Outcome::Applied,
            Err(e) => {
                self.error = Some(format!("Cannot save {}: {e}", self.path.display()));
                Outcome::Open
            }
        }
    }

    /// Left: the game.
    fn ui_info(&self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.set_width(INFO_WIDTH);
            ui.vertical_centered(|ui| {
                ui.strong("Info");
                if let Some(icon) = &self.icon {
                    ui.add(egui::Image::new(icon).fit_to_exact_size(egui::vec2(96.0, 96.0)));
                }
            });
            ui.add_space(6.0);
            let file = self.rom.display().to_string();
            let rows: Vec<(&str, String)> = match &self.info {
                Some(i) => vec![
                    (
                        "Name",
                        i.title_ja
                            .clone()
                            .or_else(|| i.title_en.clone())
                            .unwrap_or_else(|| i.title.clone()),
                    ),
                    ("Game code", i.game_code.clone()),
                    ("Format", "NDS".to_owned()),
                    ("Size", human_size(i.file_size)),
                    ("Filepath", file),
                ],
                None => vec![("Filepath", file)],
            };
            egui::Grid::new("cheat-info").num_columns(2).show(ui, |ui| {
                for (k, v) in rows {
                    ui.label(k);
                    let first_line = v.lines().next().unwrap_or_default().to_owned();
                    ui.add(
                        egui::TextEdit::singleline(&mut first_line.as_str())
                            .desired_width(INFO_WIDTH - 80.0),
                    )
                    .on_hover_text(v);
                    ui.end_row();
                }
            });
        });
    }

    /// Middle and right: the list and the editor.
    fn ui_cheats(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if ui.button("Add cheat").clicked() {
                self.save_selected_if_valid();
                self.list.push(Cheat {
                    name: "New cheat".to_owned(),
                    ..Cheat::default()
                });
                self.select(Some(self.list.len() - 1));
            }
        });
        let parsed = Cheat {
            code: self.code.clone(),
            ..Cheat::default()
        }
        .words();
        ui.horizontal(|ui| {
            ui.label("Available cheats:");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(EDITOR_WIDTH - 150.0);
                if ui
                    .add_enabled(self.selected.is_some(), egui::Button::new("Delete"))
                    .clicked()
                    && let Some(i) = self.selected
                {
                    self.list.remove(i);
                    self.select(if self.list.is_empty() {
                        None
                    } else {
                        Some(i.min(self.list.len() - 1))
                    });
                }
                let can_save = self.edited() && parsed.is_ok();
                if ui
                    .add_enabled(can_save, egui::Button::new("Save"))
                    .clicked()
                {
                    self.save_selected();
                }
            });
        });
        ui.horizontal_top(|ui| {
            self.ui_list(ui);
            ui.vertical(|ui| {
                ui.set_width(EDITOR_WIDTH);
                self.ui_editor(ui, parsed.err());
            });
        });
    }

    fn save_selected_if_valid(&mut self) {
        let valid = Cheat {
            code: self.code.clone(),
            ..Cheat::default()
        }
        .words()
        .is_ok();
        if self.edited() && valid {
            self.save_selected();
        }
    }

    /// The table: enabled checkbox, name, type.
    fn ui_list(&mut self, ui: &mut egui::Ui) {
        let mut clicked = None;
        // The frame would inherit the horizontal layout of the row it is in.
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(LIST_WIDTH);
                ui.set_min_height(380.0);
                egui::Grid::new("cheat-head")
                    .num_columns(3)
                    .min_col_width(20.0)
                    .show(ui, |ui| {
                        ui.label("");
                        ui.add_sized(
                            [250.0, 18.0],
                            egui::Label::new(egui::RichText::new("Name").strong()),
                        );
                        ui.strong("Type");
                        ui.end_row();
                    });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(350.0)
                    .show(ui, |ui| {
                        for (i, cheat) in self.list.iter_mut().enumerate() {
                            let selected = self.selected == Some(i);
                            let row = ui.horizontal(|ui| {
                                ui.checkbox(&mut cheat.enabled, "");
                                let name = egui::Label::new(&cheat.name)
                                    .truncate()
                                    .sense(egui::Sense::click());
                                let r1 = ui.add_sized([250.0, 18.0], name);
                                let r2 = ui
                                    .add(egui::Label::new(&cheat.kind).sense(egui::Sense::click()));
                                r1.clicked() || r2.clicked()
                            });
                            if selected {
                                ui.painter().rect_filled(
                                    row.response.rect.expand(1.0),
                                    2.0,
                                    SELECTED.gamma_multiply(0.35),
                                );
                            }
                            if row.inner {
                                clicked = Some(i);
                            }
                        }
                    });
            })
        });
        if let Some(i) = clicked
            && self.selected != Some(i)
        {
            self.save_selected_if_valid();
            self.select(Some(i));
        }
    }

    /// Name, memo and code of the selected cheat.
    fn ui_editor(&mut self, ui: &mut egui::Ui, code_error: Option<String>) {
        let enabled = self.selected.is_some();
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY));
            });
            ui.label("Memo:");
            egui::ScrollArea::vertical()
                .id_salt("cheat-memo")
                .max_height(170.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.memo)
                            .desired_rows(9)
                            .desired_width(f32::INFINITY),
                    );
                });
            ui.label("Code:");
            egui::ScrollArea::vertical()
                .id_salt("cheat-code")
                .max_height(150.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.code)
                            .code_editor()
                            .desired_rows(7)
                            .desired_width(f32::INFINITY)
                            .hint_text("02000000 00000063"),
                    );
                });
            if let Some(e) = code_error {
                ui.colored_label(egui::Color32::from_rgb(0xE5, 0x39, 0x35), e);
            }
        });
    }
}

fn human_size(bytes: u64) -> String {
    let mib = bytes as f64 / (1024.0 * 1024.0);
    if mib >= 1024.0 {
        format!("{:.1} GiB", mib / 1024.0)
    } else {
        format!("{mib:.1} MiB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> CheatEditor {
        let cheat = |name: &str, code: &str| Cheat {
            name: name.to_owned(),
            code: code.to_owned(),
            ..Cheat::default()
        };
        let mut e = CheatEditor {
            rom: PathBuf::from("game.nds"),
            path: std::env::temp_dir().join("cogids-cheat-dialog-test.json"),
            list: vec![
                cheat("v1.4.0 60 FPS patch", "02000000 00000001"),
                cheat("プレイヤー無敵", "02000004 00000002"),
            ],
            selected: None,
            name: String::new(),
            memo: String::new(),
            code: String::new(),
            info: None,
            icon: None,
            error: None,
        };
        e.select(Some(0));
        e
    }

    /// Lays the window out headlessly on a wide screen a few times.
    fn run(e: &mut CheatEditor) -> egui::Rect {
        let ctx = egui::Context::default();
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1920.0, 1080.0),
            )),
            ..Default::default()
        };
        for _ in 0..5 {
            let _ = ctx.run(input(), |ctx| {
                let _ = e.show(ctx);
            });
        }
        ctx.memory(|m| m.area_rect(egui::Id::new("Cheats")))
            .unwrap_or(egui::Rect::NOTHING)
    }

    #[test]
    fn window_keeps_its_size() {
        let rect = run(&mut editor());
        assert!(
            rect.width() > 600.0 && rect.width() < 1150.0,
            "width {}",
            rect.width()
        );
        assert!(rect.height() < 900.0, "height {}", rect.height());
    }

    #[test]
    fn add_select_save_and_delete() {
        let mut e = editor();
        e.name = "60 FPS".to_owned();
        assert!(e.edited());
        e.save_selected_if_valid();
        assert_eq!(e.list[0].name, "60 FPS");

        // An invalid code is not saved.
        e.code = "XYZ".to_owned();
        e.save_selected_if_valid();
        assert_eq!(e.list[0].code, "02000000 00000001");

        e.select(Some(1));
        assert_eq!(e.name, "プレイヤー無敵");
        e.list.remove(1);
        e.select(Some(0));
        assert_eq!(e.list.len(), 1);
        let _ = run(&mut e);
    }
}
