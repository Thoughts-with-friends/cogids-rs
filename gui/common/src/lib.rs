//! Frontend pieces shared by the egui and Tauri GUIs: the instance data
//! folder and settings, audio output and gamepad input.
pub mod audio;
pub mod gamepad;
pub mod instance;
pub mod log;
pub mod menu;
pub mod rom_info;
pub mod view;

pub use audio::AudioOutput;
pub use gamepad::Gamepads;
pub use instance::{Instance, Settings, format_mac, launch_instance, parse_mac, reveal};

/// Native DS refresh rate (33.513982 MHz / (355 * 263 * 6)).
pub const DS_FPS: f64 = 59.826_098;
