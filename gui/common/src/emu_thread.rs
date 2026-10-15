//! Runs the emulator on its own thread at the native DS frame rate.
//!
//! The UI talks to it through [`Shared`]: it writes the pressed keys, the
//! stylus position, the speed, the internal resolution and new cheat codes,
//! and reads the latest frame (RGBA, both screens, possibly upscaled).
use crate::{AudioOutput, speed};
use ds_emu::Emulator;
use ds_mem_addr::{PIXELS_PER_LINE, SCANLINES};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Bytes of one native screen in RGBA.
#[cfg(test)]
pub const SCREEN_BYTES: usize = PIXELS_PER_LINE * SCANLINES * 4;

/// Frames a touch is kept down after release, so quick taps register.
const TOUCH_MIN_FRAMES: u32 = 4;

/// DS button bits used by [`Shared::keys`] (sent by the frontend).
pub mod key {
    pub const A: u32 = 1 << 0;
    pub const B: u32 = 1 << 1;
    pub const SELECT: u32 = 1 << 2;
    pub const START: u32 = 1 << 3;
    pub const RIGHT: u32 = 1 << 4;
    pub const LEFT: u32 = 1 << 5;
    pub const UP: u32 = 1 << 6;
    pub const DOWN: u32 = 1 << 7;
    pub const R: u32 = 1 << 8;
    pub const L: u32 = 1 << 9;
    pub const X: u32 = 1 << 10;
    pub const Y: u32 = 1 << 11;
}

#[derive(Default)]
struct TouchState {
    pos: Option<(i32, i32)>,
    /// The frontend released the stylus.
    released: bool,
    /// Frames the current touch has been held.
    held: u32,
}

/// State shared between the UI (commands) and the emulator thread.
#[derive(Default)]
pub struct Shared {
    running: AtomicBool,
    keys: AtomicU32,
    touch: Mutex<TouchState>,
    /// Upper then lower screen, RGBA, `frame_width` pixels wide.
    frame: Mutex<Vec<u8>>,
    frame_width: AtomicU32,
    frame_seq: AtomicU64,
    /// The frontend fetched the last frame (the next one is worth making).
    frame_taken: AtomicBool,
    fps: AtomicU32,
    /// System > Pause.
    paused: AtomicBool,
    /// System > Frame step: run one frame while paused.
    step: AtomicBool,
    /// Config > Limit framerate turned off.
    unlimited: AtomicBool,
    /// Emulation speed in percent (0 = not set yet, 1x).
    speed: AtomicU32,
    /// Internal resolution factor (bits 0-7, 0 = 1x) and xBRZ (bit 8).
    render: AtomicU32,
    /// Cheat codes to hand to the emulator at the next frame.
    cheats: Mutex<Option<Vec<Vec<u32>>>>,
}

impl Shared {
    pub fn set_keys(&self, keys: u32) {
        self.keys.store(keys, Ordering::Relaxed);
    }

    /// `Some((x, y))` presses (or moves) the stylus, `None` lifts it.
    pub fn set_touch(&self, pos: Option<(i32, i32)>) {
        let Ok(mut t) = self.touch.lock() else {
            return;
        };
        match pos {
            Some((x, y)) => {
                if t.pos.is_none() {
                    t.held = 0;
                }
                t.pos = Some((x.clamp(0, 255), y.clamp(0, 191)));
                t.released = false;
            }
            None => t.released = true,
        }
    }

    /// The latest frame if it is newer than `last_seq`: `(seq, width,
    /// rgba)` (both screens, upper first, `width` pixels wide each).
    pub fn frame_since(&self, last_seq: u64) -> Option<(u64, u32, Vec<u8>)> {
        let seq = self.frame_seq.load(Ordering::Acquire);
        if seq == last_seq || seq == 0 {
            return None;
        }
        let frame = self.frame.lock().ok()?;
        let width = self.frame_width.load(Ordering::Acquire);
        let rgba = frame.clone();
        drop(frame);
        self.frame_taken.store(true, Ordering::Release);
        Some((seq, width, rgba))
    }

    /// Where the stylus is held, if it is (DS pixels).
    pub fn touch_pos(&self) -> Option<(i32, i32)> {
        self.touch.lock().ok().and_then(|t| t.pos)
    }

    /// Emulation speed in percent (`speed::SPEEDS`).
    pub fn set_speed(&self, percent: u16) {
        self.speed
            .store(u32::from(speed::valid(percent)), Ordering::Relaxed);
    }

    /// Internal resolution and the xBRZ filter of the frames made from now.
    pub fn set_render(&self, scale: u8, xbrz: bool) {
        let bits = u32::from(scale) | (u32::from(xbrz) << 8);
        self.render.store(bits, Ordering::Relaxed);
    }

    /// Replaces the running game's cheat codes (from the next frame).
    pub fn set_cheats(&self, codes: Vec<Vec<u32>>) {
        if let Ok(mut cheats) = self.cheats.lock() {
            *cheats = Some(codes);
        }
    }

    /// Emulated frames per second over the last second.
    pub fn fps(&self) -> f32 {
        f32::from_bits(self.fps.load(Ordering::Relaxed))
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// Pauses and runs exactly one frame.
    pub fn frame_step(&self) {
        self.paused.store(true, Ordering::Relaxed);
        self.step.store(true, Ordering::Relaxed);
    }

    pub fn set_limit_framerate(&self, limit: bool) {
        self.unlimited.store(!limit, Ordering::Relaxed);
    }

    /// Frames produced so far (the frame sequence number).
    #[cfg(test)]
    pub fn frames(&self) -> u64 {
        self.frame_seq.load(Ordering::Acquire)
    }
}

/// A running emulator thread.
pub struct EmuThread {
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
}

impl EmuThread {
    /// Starts `emu` (with its ROM already loaded).
    pub fn start(emu: Emulator, shared: Arc<Shared>, audio: AudioOutput) -> Self {
        shared.running.store(true, Ordering::Relaxed);
        let thread_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("emulator".to_owned())
            .spawn(move || run(emu, &thread_shared, &audio))
            .inspect_err(|e| log::error!("Failed to start emulator thread: {e}"))
            .ok();
        Self { shared, handle }
    }

    /// Stops the thread; the emulator writes its save on drop.
    pub fn stop(mut self) {
        self.shared.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            log::error!("Emulator thread panicked");
        }
    }
}

fn apply_keys(emu: &mut Emulator, keys: u32) {
    let on = |bit| keys & bit != 0;
    let k = &mut emu.key_input;
    k.button_a = on(key::A);
    k.button_b = on(key::B);
    k.select = on(key::SELECT);
    k.start = on(key::START);
    k.right = on(key::RIGHT);
    k.left = on(key::LEFT);
    k.up = on(key::UP);
    k.down = on(key::DOWN);
    k.button_r = on(key::R);
    k.button_l = on(key::L);
    emu.ext_key_in.button_x = on(key::X);
    emu.ext_key_in.button_y = on(key::Y);
}

fn apply_touch(emu: &mut Emulator, shared: &Shared) {
    let Ok(mut t) = shared.touch.lock() else {
        return;
    };
    match t.pos {
        Some((x, y)) => {
            emu.touchscreen_press(x, y);
            t.held += 1;
            if t.released && t.held >= TOUCH_MIN_FRAMES {
                t.pos = None;
                t.released = false;
            }
        }
        None => emu.touchscreen_press(0, 0xFFF),
    }
}

/// 0xAARRGGBB -> RGBA bytes appended to `dst`.
fn push_rgba(src: &[u32], dst: &mut Vec<u8>) {
    dst.extend(
        src.iter()
            .flat_map(|px| [(px >> 16) as u8, (px >> 8) as u8, *px as u8, 0xFF]),
    );
}

/// Both screens at the current internal resolution into `shared.frame`.
fn publish_frame(emu: &mut Emulator, shared: &Shared, pixels: &mut Vec<u32>, rgba: &mut Vec<u8>) {
    let render = shared.render.load(Ordering::Relaxed);
    let scale = (render & 0xFF).clamp(1, 4) as usize;
    let xbrz = render & 0x100 != 0 && scale >= 2;
    rgba.clear();
    let mut width = PIXELS_PER_LINE;
    for upper in [true, false] {
        width = emu.gpu.screen_hd(upper, scale, xbrz, pixels);
        push_rgba(pixels, rgba);
    }
    if let Ok(mut frame) = shared.frame.lock() {
        std::mem::swap(&mut *frame, rgba);
        shared.frame_width.store(width as u32, Ordering::Release);
    }
}

fn run(mut emu: Emulator, shared: &Shared, audio: &AudioOutput) {
    let mut pixels = vec![0_u32; PIXELS_PER_LINE * SCANLINES];
    let mut rgba = Vec::new();
    let mut samples = Vec::with_capacity(4096);
    let mut stretched = Vec::with_capacity(4096);

    let mut next = Instant::now();
    shared.frame_taken.store(true, Ordering::Release);
    let (mut fps_start, mut fps_frames) = (Instant::now(), 0_u32);

    while shared.running.load(Ordering::Relaxed) {
        if shared.paused.load(Ordering::Relaxed) && !shared.step.swap(false, Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            next = Instant::now();
            continue;
        }
        let unlimited = shared.unlimited.load(Ordering::Relaxed);
        let percent = speed::valid(shared.speed.load(Ordering::Relaxed) as u16);
        let frame_time = Duration::from_secs_f64(1.0 / speed::fps(percent));
        let render = shared.render.load(Ordering::Relaxed);
        emu.gpu
            .set_render_scale((render & 0xFF).clamp(1, 4) as usize);
        if let Some(codes) = shared.cheats.lock().ok().and_then(|mut c| c.take()) {
            emu.set_cheats(codes);
        }

        // Keyboard and gamepads are both read by the frontend (Web Gamepad
        // API, see `useGamepads`), which sends the combined button bits.
        apply_keys(&mut emu, shared.keys.load(Ordering::Relaxed));
        apply_touch(&mut emu, shared);

        emu.run();

        emu.take_audio_samples(&mut samples);
        // Fast-forwarding or stepping would only garble the sound.
        if !unlimited && !shared.paused.load(Ordering::Relaxed) {
            speed::stretch_audio(&samples, percent, &mut stretched);
            audio.push(&stretched);
        }
        samples.clear();

        // Faster than 1x, a frame the frontend has not fetched yet is not
        // replaced (upscaling frames nobody sees is wasted work); at 1x and
        // for single steps every frame is published.
        let taken = shared.frame_taken.swap(false, Ordering::AcqRel);
        if taken
            || (percent <= speed::NORMAL && !unlimited)
            || shared.paused.load(Ordering::Relaxed)
        {
            publish_frame(&mut emu, shared, &mut pixels, &mut rgba);
        }
        shared.frame_seq.fetch_add(1, Ordering::Release);

        fps_frames += 1;
        let elapsed = fps_start.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let fps = fps_frames as f32 / elapsed.as_secs_f32();
            shared.fps.store(fps.to_bits(), Ordering::Relaxed);
            (fps_start, fps_frames) = (Instant::now(), 0);
        }

        // Pace to ~59.83 fps; resync instead of racing after a stall.
        next += frame_time;
        let now = Instant::now();
        if unlimited {
            next = now;
        } else if next > now {
            std::thread::sleep(next - now);
        } else if now - next > frame_time * 4 {
            next = now;
        }
    }

    audio.clear();
    if let Err(e) = emu.flush_save() {
        log::error!("Failed to write save file: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a real ROM on the emulator thread for a moment and checks that
    /// frames come out at roughly the DS rate.
    /// `LUNARIS_TEST_ROM=<path> cargo test -p ds_common -- --ignored`
    #[test]
    #[ignore = "requires a ROM on disk"]
    fn thread_produces_frames() {
        let rom = std::env::var("LUNARIS_TEST_ROM")
            .unwrap_or_else(|_| "D:/GAME/Emulator/DS/Roms/hello_world.nds".to_owned());
        let mut emu = Emulator::new();
        emu.config.save_dir = Some(std::env::temp_dir());
        emu.load_rom(&rom).expect("ROM loads");

        let shared = Arc::new(Shared::default());
        let thread = EmuThread::start(emu, Arc::clone(&shared), AudioOutput::new(0.0));
        shared.set_keys(key::A);
        shared.set_touch(Some((10, 20)));
        shared.set_touch(None);
        std::thread::sleep(Duration::from_millis(2500));

        let (seq, width, rgba) = shared.frame_since(0).expect("a frame was produced");
        assert_eq!(width, 256);
        assert_eq!(rgba.len(), SCREEN_BYTES * 2);
        assert!(
            rgba.as_chunks::<4>().0.iter().any(|p| p[..3] != [0, 0, 0]),
            "frame has content"
        );
        assert!(
            (100..=200).contains(&seq),
            "~60 fps pacing, got {seq} frames"
        );
        assert!(
            shared.fps() > 50.0 && shared.fps() < 70.0,
            "fps {}",
            shared.fps()
        );

        thread.stop();
        assert!(!shared.is_running());
    }

    /// Pause stops the frames, a frame step makes exactly one.
    /// `cargo test --release -p ds_common -- --ignored pause_and_step`
    #[test]
    #[ignore = "requires a ROM on disk"]
    fn pause_and_step() {
        let rom = std::env::var("LUNARIS_TEST_ROM")
            .unwrap_or_else(|_| "D:/GAME/Emulator/DS/Roms/hello_world.nds".to_owned());
        let mut emu = Emulator::new();
        emu.config.save_dir = Some(std::env::temp_dir());
        emu.load_rom(&rom).expect("ROM loads");

        let shared = Arc::new(Shared::default());
        let thread = EmuThread::start(emu, Arc::clone(&shared), AudioOutput::new(0.0));
        std::thread::sleep(Duration::from_millis(300));
        shared.set_paused(true);
        std::thread::sleep(Duration::from_millis(100));
        let paused_at = shared.frames();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(shared.frames(), paused_at, "no frames while paused");

        shared.frame_step();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(shared.frames(), paused_at + 1, "one frame per step");

        shared.set_paused(false);
        std::thread::sleep(Duration::from_millis(300));
        assert!(shared.frames() > paused_at + 5, "running again");
        thread.stop();
    }

    /// 2x speed doubles the frame rate; 2x resolution doubles the frame
    /// size. `cargo test --release -p ds_common -- --ignored speed_and_resolution`
    #[test]
    #[ignore = "requires a ROM on disk"]
    fn speed_and_resolution() {
        let rom = std::env::var("LUNARIS_TEST_ROM")
            .unwrap_or_else(|_| "D:/GAME/Emulator/DS/Roms/hello_world.nds".to_owned());
        let mut emu = Emulator::new();
        emu.config.save_dir = Some(std::env::temp_dir());
        emu.load_rom(&rom).expect("ROM loads");

        let shared = Arc::new(Shared::default());
        shared.set_speed(200);
        shared.set_render(2, true);
        let thread = EmuThread::start(emu, Arc::clone(&shared), AudioOutput::new(0.0));
        std::thread::sleep(Duration::from_millis(2500));
        assert!(
            shared.fps() > 105.0 && shared.fps() < 135.0,
            "2x speed: fps {}",
            shared.fps()
        );
        let (_, width, rgba) = shared.frame_since(0).expect("a frame");
        assert_eq!(width, 512);
        assert_eq!(rgba.len(), SCREEN_BYTES * 2 * 4);
        thread.stop();
    }
}
