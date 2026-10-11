//! Frontend pieces shared by the egui and Tauri GUIs: the instance data
//! folder and settings, audio output and gamepad input.
pub mod audio;
pub mod gamepad;
pub mod instance;
pub mod log;

pub use audio::AudioOutput;
pub use gamepad::Gamepads;
pub use instance::{Instance, Settings};

/// Native DS refresh rate (33.513982 MHz / (355 * 263 * 6)).
pub const DS_FPS: f64 = 59.826_098;
