//! Draws the shared melonDS-style menu bar ([`ds_common::menu`]) with egui.
//!
//! ```text
//!  App::ui_menu_bar
//!    menu_bar(state) -> Vec<Menu>      (ds_common, same tree as Tauri)
//!    bar(ui, &menus) -> Option<Action> (here: menu_button / checkbox /
//!                                       radio, disabled entries show why)
//!    App::apply(action)                (main.rs)
//! ```
use ds_common::menu::{Action, Item, Menu};
use eframe::egui;

/// Draws the bar; returns the entry clicked this frame, if any.
pub fn bar(ui: &mut egui::Ui, menus: &[Menu]) -> Option<Action> {
    let mut picked = None;
    egui::MenuBar::new().ui(ui, |ui| {
        for menu in menus {
            ui.menu_button(&menu.label, |ui| items(ui, &menu.items, &mut picked));
        }
    });
    picked
}

fn items(ui: &mut egui::Ui, list: &[Item], picked: &mut Option<Action>) {
    for item in list {
        match item {
            Item::Separator => {
                ui.separator();
            }
            Item::Label { text } => {
                ui.weak(text);
            }
            Item::Submenu {
                label,
                enabled,
                items: sub,
            } => {
                ui.add_enabled_ui(*enabled, |ui| {
                    ui.menu_button(label, |ui| items(ui, sub, picked));
                });
            }
            Item::Entry {
                label,
                action,
                enabled,
                checked,
                radio,
                reason,
                shortcut,
                hint,
            } => {
                let response = match checked {
                    Some(on) if *radio => {
                        ui.add_enabled(*enabled, egui::RadioButton::new(*on, label))
                    }
                    Some(on) => {
                        let mut on = *on;
                        ui.add_enabled(*enabled, egui::Checkbox::new(&mut on, label))
                    }
                    None => {
                        let mut button = egui::Button::new(label);
                        if let Some(keys) = shortcut {
                            button = button.shortcut_text(keys);
                        }
                        ui.add_enabled(*enabled, button)
                    }
                };
                let response = match (reason, hint) {
                    (Some(why), _) => response.on_disabled_hover_text(why),
                    (None, Some(hint)) => response.on_hover_text(hint),
                    (None, None) => response,
                };
                if response.clicked()
                    && picked.is_none()
                    && let Some(action) = action
                {
                    *picked = Some(action.clone());
                    ui.close();
                }
            }
        }
    }
}
