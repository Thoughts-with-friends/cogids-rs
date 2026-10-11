//! Gamepad support: controllers are detected automatically (hot-plug) via
//! `gilrs` and mapped onto the DS buttons.
use ds_emu::Emulator;
use gilrs::{Axis, Button, Gilrs};

/// Analog stick deflection treated as a D-pad press.
const STICK_THRESHOLD: f32 = 0.5;

/// Connected gamepads. Controllers are picked up automatically when they are
/// plugged in (and dropped when unplugged) through `gilrs`.
pub struct Gamepads {
    gilrs: Option<Gilrs>,
}

impl Gamepads {
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(gilrs) => Some(gilrs),
            Err(e) => {
                tracing::error!("Gamepad support unavailable: {e}");
                None
            }
        };
        Self { gilrs }
    }

    /// Drains pending events; this also refreshes the cached button states.
    pub fn poll(&mut self) {
        let Some(gilrs) = &mut self.gilrs else {
            return;
        };
        while let Some(event) = gilrs.next_event() {
            match event.event {
                gilrs::EventType::Connected => {
                    tracing::info!("Gamepad connected: {}", gilrs.gamepad(event.id).name());
                }
                gilrs::EventType::Disconnected => {
                    tracing::info!("Gamepad disconnected: {}", gilrs.gamepad(event.id).name());
                }
                _ => {}
            }
        }
    }

    /// Names of the connected gamepads.
    pub fn names(&self) -> Vec<String> {
        self.gilrs.as_ref().map_or_else(Vec::new, |gilrs| {
            gilrs
                .gamepads()
                .map(|(_, pad)| pad.name().to_owned())
                .collect()
        })
    }

    /// ORs every connected gamepad into the DS buttons (call after the
    /// keyboard so either device works).
    ///
    /// Face buttons are mapped by position like on the DS: right = A,
    /// bottom = B, top = X, left = Y. L/R accept both shoulder buttons and
    /// triggers; the left stick doubles as the D-pad.
    pub fn apply(&self, emu: &mut Emulator) {
        let Some(gilrs) = &self.gilrs else {
            return;
        };
        for (_, pad) in gilrs.gamepads() {
            let down = |b: Button| pad.is_pressed(b);
            let x = pad.value(Axis::LeftStickX);
            let y = pad.value(Axis::LeftStickY);

            let k = &mut emu.key_input;
            k.button_a |= down(Button::East);
            k.button_b |= down(Button::South);
            k.button_l |= down(Button::LeftTrigger) || down(Button::LeftTrigger2);
            k.button_r |= down(Button::RightTrigger) || down(Button::RightTrigger2);
            k.start |= down(Button::Start);
            k.select |= down(Button::Select);
            k.up |= down(Button::DPadUp) || y > STICK_THRESHOLD;
            k.down |= down(Button::DPadDown) || y < -STICK_THRESHOLD;
            k.left |= down(Button::DPadLeft) || x < -STICK_THRESHOLD;
            k.right |= down(Button::DPadRight) || x > STICK_THRESHOLD;
            emu.ext_key_in.button_x |= down(Button::North);
            emu.ext_key_in.button_y |= down(Button::West);
        }
    }
}

impl Default for Gamepads {
    fn default() -> Self {
        Self::new()
    }
}
