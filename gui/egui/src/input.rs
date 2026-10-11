//! Host input: keyboard and the touch screen (gamepads: `ds_common`).
use ds_emu::Emulator;
use ds_mem_addr::{PIXELS_PER_LINE, SCANLINES};
use eframe::egui;

/// Frames a touch is kept down after the pointer is released, so that a
/// quick tap (press and release within one host frame) still reaches the
/// game: the SDK samples the touch screen only once per frame or less.
const TOUCH_MIN_FRAMES: u8 = 4;

/// Keyboard -> DS buttons. Arrows = D-pad, X = A, Z = B, S = X, A = Y,
/// Q = L, W = R, Enter = START, Backspace = SELECT.
pub fn apply_keyboard(ctx: &egui::Context, emu: &mut Emulator) {
    use egui::Key;
    ctx.input(|i| {
        let k = &mut emu.key_input;
        k.up = i.key_down(Key::ArrowUp);
        k.down = i.key_down(Key::ArrowDown);
        k.left = i.key_down(Key::ArrowLeft);
        k.right = i.key_down(Key::ArrowRight);
        k.button_a = i.key_down(Key::X);
        k.button_b = i.key_down(Key::Z);
        k.button_l = i.key_down(Key::Q);
        k.button_r = i.key_down(Key::W);
        k.start = i.key_down(Key::Enter);
        k.select = i.key_down(Key::Backspace);
        emu.ext_key_in.button_x = i.key_down(Key::S);
        emu.ext_key_in.button_y = i.key_down(Key::A);
    });
}

/// Stylus state for the lower screen.
#[derive(Default)]
pub struct Touch {
    /// Last touched DS pixel.
    pos: Option<(i32, i32)>,
    /// Frames the pen stays down after release (see [`TOUCH_MIN_FRAMES`]).
    hold: u8,
}

impl Touch {
    /// Feeds the current stylus state to the emulator (before `run`).
    pub fn apply(&mut self, emu: &mut Emulator) {
        match self.pos {
            Some((x, y)) if self.hold > 0 => {
                emu.touchscreen_press(x, y);
                self.hold -= 1;
            }
            _ => {
                self.pos = None;
                emu.touchscreen_press(0, 0xFFF);
            }
        }
    }

    /// Reads this frame's pointer input against the lower screen widget.
    ///
    /// A press inside the screen starts a touch. While the button stays down
    /// the stylus follows the pointer (clamped to the screen edges when
    /// dragged outside). Press events are scanned too, so taps shorter than
    /// a host frame are not lost.
    pub fn update(&mut self, ctx: &egui::Context, resp: &egui::Response) {
        let rect = resp.rect;
        let to_ds = |pos: egui::Pos2| {
            let rel = (pos - rect.min) / rect.size();
            (
                (rel.x * PIXELS_PER_LINE as f32).clamp(0.0, (PIXELS_PER_LINE - 1) as f32) as i32,
                (rel.y * SCANLINES as f32).clamp(0.0, (SCANLINES - 1) as f32) as i32,
            )
        };

        let tapped = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } if rect.contains(*pos) => Some(*pos),
                _ => None,
            })
        });

        let held = resp
            .is_pointer_button_down_on()
            .then(|| resp.interact_pointer_pos())
            .flatten();

        if let Some(pos) = held.or(tapped) {
            self.pos = Some(to_ds(pos));
            self.hold = TOUCH_MIN_FRAMES;
        }
    }
}
