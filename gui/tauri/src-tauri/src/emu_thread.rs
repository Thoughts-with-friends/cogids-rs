//! Runs the emulator on its own thread at the native DS frame rate.
//!
//! The UI talks to it through [`Shared`]: it writes the pressed keys and the
//! stylus position, and reads the latest frame (RGBA, both screens).
use ds_common::{AudioOutput, DS_FPS, Gamepads};
use ds_emu::Emulator;
use ds_mem_addr::{PIXELS_PER_LINE, SCANLINES};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Bytes of one screen in RGBA.
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
    /// Upper then lower screen, RGBA.
    frame: Mutex<Vec<u8>>,
    frame_seq: AtomicU64,
    gamepads: Mutex<Vec<String>>,
    fps: AtomicU32,
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

    /// The latest frame if it is newer than `last_seq`: `(seq, rgba)`.
    pub fn frame_since(&self, last_seq: u64) -> Option<(u64, Vec<u8>)> {
        let seq = self.frame_seq.load(Ordering::Acquire);
        if seq == last_seq || seq == 0 {
            return None;
        }
        let frame = self.frame.lock().ok()?.clone();
        Some((seq, frame))
    }

    pub fn gamepads(&self) -> Vec<String> {
        self.gamepads.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Emulated frames per second over the last second.
    pub fn fps(&self) -> f32 {
        f32::from_bits(self.fps.load(Ordering::Relaxed))
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
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

/// 0xAARRGGBB -> RGBA bytes.
fn to_rgba(src: &[u32], dst: &mut [u8]) {
    for (px, out) in src.iter().zip(dst.as_chunks_mut::<4>().0) {
        out[0] = (px >> 16) as u8;
        out[1] = (px >> 8) as u8;
        out[2] = *px as u8;
        out[3] = 0xFF;
    }
}

fn run(mut emu: Emulator, shared: &Shared, audio: &AudioOutput) {
    // Gamepads are polled on this thread (gilrs is not shared).
    let mut gamepads = Gamepads::new();
    let mut pixels = vec![0_u32; PIXELS_PER_LINE * SCANLINES];
    let mut rgba = vec![0_u8; SCREEN_BYTES * 2];
    let mut samples = Vec::with_capacity(4096);

    let frame_time = Duration::from_secs_f64(1.0 / DS_FPS);
    let mut next = Instant::now();
    let (mut fps_start, mut fps_frames) = (Instant::now(), 0_u32);

    while shared.running.load(Ordering::Relaxed) {
        gamepads.poll();
        if let Ok(mut names) = shared.gamepads.lock() {
            *names = gamepads.names();
        }
        apply_keys(&mut emu, shared.keys.load(Ordering::Relaxed));
        gamepads.apply(&mut emu);
        apply_touch(&mut emu, shared);

        emu.run();

        emu.take_audio_samples(&mut samples);
        audio.push(&samples);
        samples.clear();

        emu.get_upper_frame(&mut pixels);
        to_rgba(&pixels, &mut rgba[..SCREEN_BYTES]);
        emu.get_lower_frame(&mut pixels);
        to_rgba(&pixels, &mut rgba[SCREEN_BYTES..]);
        if let Ok(mut frame) = shared.frame.lock() {
            frame.clone_from(&rgba);
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
        if next > now {
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
    /// `LUNARIS_TEST_ROM=<path> cargo test -p lunaris -- --ignored`
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

        let (seq, rgba) = shared.frame_since(0).expect("a frame was produced");
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
}
