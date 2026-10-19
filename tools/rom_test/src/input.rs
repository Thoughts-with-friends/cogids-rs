//! Scripted input: `key:start:every[:end]` presses and `x:y:start:every[:end]`
//! touches, each held for [`HOLD`] frames (the same syntax as the headless
//! harness in chapter 19).
use ds_emu::Emulator;

/// Frames a press or touch is held.
pub const HOLD: u32 = 6;

/// When an event is active.
#[derive(Debug, Clone, Copy)]
struct Timing {
    start: u32,
    every: u32,
    end: u32,
}

impl Timing {
    /// `start:every[:end]`; `every = 0` means once.
    fn parse(parts: &[&str]) -> Option<Self> {
        let num = |i: usize| parts.get(i).and_then(|p| p.parse::<u32>().ok());
        Some(Self {
            start: num(0)?,
            every: num(1)?,
            end: if parts.len() > 2 { num(2)? } else { u32::MAX },
        })
    }

    /// Whether the event is held during `frame`.
    const fn held(self, frame: u32) -> bool {
        if frame < self.start || frame > self.end {
            return false;
        }
        let since = frame - self.start;
        if self.every == 0 {
            since < HOLD
        } else {
            since % self.every < HOLD
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    A,
    B,
    X,
    Y,
    L,
    R,
    Start,
    Select,
    Up,
    Down,
    Left,
    Right,
}

impl Key {
    fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "a" => Self::A,
            "b" => Self::B,
            "x" => Self::X,
            "y" => Self::Y,
            "l" => Self::L,
            "r" => Self::R,
            "start" => Self::Start,
            "select" => Self::Select,
            "up" => Self::Up,
            "down" => Self::Down,
            "left" => Self::Left,
            "right" => Self::Right,
            _ => return None,
        })
    }

    fn press(self, emu: &mut Emulator) {
        match self {
            Self::A => emu.button_a_pressed(),
            Self::B => emu.button_b_pressed(),
            Self::X => emu.button_x_pressed(),
            Self::Y => emu.button_y_pressed(),
            Self::L => emu.button_l_pressed(),
            Self::R => emu.button_r_pressed(),
            Self::Start => emu.button_start_pressed(),
            Self::Select => emu.button_select_pressed(),
            Self::Up => emu.button_up_pressed(),
            Self::Down => emu.button_down_pressed(),
            Self::Left => emu.button_left_pressed(),
            Self::Right => emu.button_right_pressed(),
        }
    }

    fn release(self, emu: &mut Emulator) {
        match self {
            Self::A => emu.button_a_released(),
            Self::B => emu.button_b_released(),
            Self::X => emu.button_x_released(),
            Self::Y => emu.button_y_released(),
            Self::L => emu.button_l_released(),
            Self::R => emu.button_r_released(),
            Self::Start => emu.button_start_released(),
            Self::Select => emu.button_select_released(),
            Self::Up => emu.button_up_released(),
            Self::Down => emu.button_down_released(),
            Self::Left => emu.button_left_released(),
            Self::Right => emu.button_right_released(),
        }
    }
}

/// The parsed input script of one case.
#[derive(Debug, Default)]
pub struct Script {
    presses: Vec<(Key, Timing)>,
    touches: Vec<((i32, i32), Timing)>,
}

impl Script {
    /// Parses the `press` and `touch` lists of a case.
    ///
    /// # Errors
    /// The first entry that does not parse.
    pub fn parse(press: &[String], touch: &[String]) -> Result<Self, String> {
        let mut script = Self::default();
        for p in press {
            let parts: Vec<&str> = p.split(':').collect();
            let key = parts.first().and_then(|k| Key::parse(k));
            let timing = parts.get(1..).and_then(Timing::parse);
            match (key, timing) {
                (Some(k), Some(t)) => script.presses.push((k, t)),
                _ => return Err(format!("bad press \"{p}\" (key:start:every[:end])")),
            }
        }
        for t in touch {
            let parts: Vec<&str> = t.split(':').collect();
            let num = |i: usize| parts.get(i).and_then(|v| v.parse::<i32>().ok());
            let timing = parts.get(2..).and_then(Timing::parse);
            match (num(0), num(1), timing) {
                (Some(x), Some(y), Some(tm)) if (0..256).contains(&x) && (0..192).contains(&y) => {
                    script.touches.push(((x, y), tm));
                }
                _ => {
                    return Err(format!(
                        "bad touch \"{t}\" (x:y:start:every[:end], x < 256, y < 192)"
                    ));
                }
            }
        }
        Ok(script)
    }

    /// Sets the buttons and the stylus for `frame` (before `run()`).
    pub fn apply(&self, emu: &mut Emulator, frame: u32) {
        for &(key, timing) in &self.presses {
            if timing.held(frame) {
                key.press(emu);
            } else {
                key.release(emu);
            }
        }
        let touch = self
            .touches
            .iter()
            .find(|(_, t)| t.held(frame))
            .map(|(p, _)| *p);
        match touch {
            Some((x, y)) => emu.touchscreen_press(x, y),
            None => emu.touchscreen_press(0, 0xFFF),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_holds_for_six_frames() {
        let t = Timing::parse(&["10", "30"]).unwrap_or(Timing {
            start: 0,
            every: 0,
            end: 0,
        });
        assert!(!t.held(9));
        assert!(t.held(10) && t.held(15));
        assert!(!t.held(16));
        assert!(t.held(40));
    }

    #[test]
    fn rejects_bad_entries() {
        assert!(Script::parse(&["jump:1:2".to_owned()], &[]).is_err());
        assert!(Script::parse(&[], &["300:5:1:0".to_owned()]).is_err());
        assert!(Script::parse(&["a:1:2:9".to_owned()], &["10:20:1:0".to_owned()]).is_ok());
    }
}
