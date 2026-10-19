//! Runs one case on its own emulator and records what the checks need.
//!
//! ```text
//!  prepare(): case folder, isolated save folder (+ save_from copy), emulator
//!             with fixed RTC, load_rom, cheats
//!  per frame: script.apply -> run() -> audio, polygons, screen hash, IRQ state
//!  snapshot frames: native (+ hi-res) screens hashed and saved as PNG
//!  end:       flush_save, RAM words
//! ```
use crate::config::{Case, Config, Settings};
use crate::input::Script;
use ds_emu::Emulator;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const W: usize = 256;
const H: usize = 192;
/// Frames ignored by the audio checks (start-up).
const AUDIO_SKIP: u32 = 10;
/// Frame ends sampled by `no_terminate`.
pub const TERMINATE_WINDOW: usize = 60;

/// What a run produced.
#[derive(Debug, Default)]
pub struct Metrics {
    pub frames: u32,
    pub ms_per_frame: f64,
    pub max_polygons: i32,
    pub audio_frames: u64,
    pub audio_nonzero: bool,
    pub distinct_screens: usize,
    pub blank: [bool; 2],
    /// CPSR.I of the ARM9 at the last frame ends (newest last).
    pub irq_off: Vec<bool>,
    pub save: Option<(PathBuf, u64)>,
    pub ram: Vec<(u32, u32)>,
    /// Requested pixels of the last frame (entry text, 0xRRGGBB).
    pub pixels: Vec<(String, Option<u32>)>,
    /// `"<frame>/<screen>"` -> hash (native and `hd` variants).
    pub hashes: BTreeMap<String, String>,
    pub pngs: Vec<String>,
}

/// FNV-1a over the pixels.
fn hash(pixels: &[u32]) -> u64 {
    pixels.iter().fold(0xCBF2_9CE4_8422_2325_u64, |h, &p| {
        (h ^ u64::from(p & 0x00FF_FFFF)).wrapping_mul(0x0100_0000_01B3)
    })
}

fn save_png(path: &Path, pixels: &[u32], width: usize) -> Result<(), String> {
    let rgb: Vec<u8> = pixels
        .iter()
        .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8])
        .collect();
    let height = pixels.len() / width.max(1);
    image::save_buffer(path, &rgb, width as u32, height as u32, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// A folder-safe version of a case name.
pub fn slug(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// The emulator for `case`, with its save kept in `save_dir`.
fn prepare(config: &Config, case: &Case, save_dir: &Path, settings: &Settings) -> Result<Emulator, String> {
    let rom = config.rom_path(case);
    if !rom.is_file() {
        return Err(format!("ROM not found: {}", rom.display()));
    }
    std::fs::create_dir_all(save_dir).map_err(|e| format!("{}: {e}", save_dir.display()))?;
    // Never let a test write next to the real ROM.
    if rom.parent().is_some_and(|p| p.canonicalize().ok() == save_dir.canonicalize().ok()) {
        return Err("the save folder must not be the ROM folder".to_owned());
    }
    if let Some(src) = config.save_from(case) {
        let name = rom.with_extension("sav");
        let dst = save_dir.join(name.file_name().unwrap_or_default());
        std::fs::copy(&src, &dst).map_err(|e| format!("save_from {}: {e}", src.display()))?;
    }
    let mut emu = Emulator::new();
    emu.config.save_dir = Some(save_dir.to_owned());
    let [y, mo, d, h, mi, s] = settings.fixed_time.unwrap_or([2026, 1, 1, 12, 0, 0]);
    emu.rtc.set_fixed_time(Some((y as i32, mo, d, h, mi, s)));
    emu.load_rom(&rom).map_err(|e| format!("load_rom: {e}"))?;
    let mut codes = Vec::new();
    for code in &case.cheats {
        codes.push(ds_emu::parse_ar_code(code).map_err(|e| format!("cheat: {e}"))?);
    }
    emu.set_cheats(codes);
    if let Some(scale) = case.hd.filter(|&s| s > 1) {
        emu.gpu.set_render_scale(scale);
    }
    Ok(emu)
}

/// Hashes (and saves) both screens at `frame`, natively and hi-res.
fn snapshot(emu: &mut Emulator, case: &Case, dir: &Path, frame: u32, m: &mut Metrics, png: bool) -> Result<(), String> {
    let mut native = vec![0_u32; W * H];
    for (i, name) in ["upper", "lower"].into_iter().enumerate() {
        if i == 0 {
            emu.get_upper_frame(&mut native);
        } else {
            emu.get_lower_frame(&mut native);
        }
        m.hashes.insert(format!("{frame:06}/{name}"), format!("{:016x}", hash(&native)));
        if png {
            let file = format!("{frame:06}_{name}.png");
            save_png(&dir.join(&file), &native, W)?;
            m.pngs.push(file);
        }
        if let Some(scale) = case.hd.filter(|&s| s > 1) {
            let mut hi = Vec::new();
            let width = emu.gpu.screen_hd(i == 0, scale, case.xbrz, &mut hi);
            m.hashes.insert(format!("{frame:06}/{name}_hd{scale}"), format!("{:016x}", hash(&hi)));
            if png {
                let file = format!("{frame:06}_{name}_hd{scale}.png");
                save_png(&dir.join(&file), &hi, width)?;
                m.pngs.push(file);
            }
        }
    }
    Ok(())
}

/// Per-frame bookkeeping after `run()`.
fn observe(emu: &mut Emulator, frame: u32, audio: &mut Vec<i16>, seen: &mut std::collections::HashSet<u64>, m: &mut Metrics) {
    m.max_polygons = m.max_polygons.max(emu.gpu.engine_3d.rend_poly_count);
    audio.clear();
    emu.take_audio_samples(audio);
    if frame >= AUDIO_SKIP {
        m.audio_frames += (audio.len() / 2) as u64;
        m.audio_nonzero |= audio.iter().any(|&s| s != 0);
    }
    let mut up = vec![0_u32; W * H];
    let mut lo = vec![0_u32; W * H];
    emu.get_upper_frame(&mut up);
    emu.get_lower_frame(&mut lo);
    seen.insert(hash(&up) ^ hash(&lo).rotate_left(1));
    m.blank = [up.iter().all(|&p| p == up[0]), lo.iter().all(|&p| p == lo[0])];
    m.irq_off.push(emu.arm9.get_cpsr().irq_disabled);
    if m.irq_off.len() > TERMINATE_WINDOW {
        m.irq_off.remove(0);
    }
}

/// Reads the `pixel` entries the case checks from the last frame.
fn read_pixels(emu: &Emulator, case: &Case, m: &mut Metrics) {
    let mut up = vec![0_u32; W * H];
    let mut lo = vec![0_u32; W * H];
    emu.get_upper_frame(&mut up);
    emu.get_lower_frame(&mut lo);
    for entry in &case.expect.pixel {
        let value = crate::check::parse_pixel(entry).and_then(|(lower, x, y, _)| {
            let screen = if lower { &lo } else { &up };
            screen.get(y * W + x).map(|p| p & 0x00FF_FFFF)
        });
        m.pixels.push((entry.clone(), value));
    }
}

/// Reads the `ram` words the case checks.
fn read_ram(emu: &mut Emulator, case: &Case, m: &mut Metrics) {
    for entry in &case.expect.ram {
        if let Some((addr, _, width)) = crate::check::parse_ram_width(entry) {
            let value = match width {
                8 => u32::from(emu.arm9_read_byte(addr)),
                16 => u32::from(emu.arm9_read_halfword(addr)),
                _ => emu.arm9_read_word(addr),
            };
            m.ram.push((addr, value));
        }
    }
}

/// Runs `case`, writing its PNGs and save into `dir`.
///
/// # Errors
/// If the case cannot be started (missing ROM, bad script, ...).
pub fn run(config: &Config, case: &Case, dir: &Path, png: bool) -> Result<Metrics, String> {
    let script = Script::parse(&case.press, &case.touch)?;
    let save_dir = dir.join("save");
    let mut emu = prepare(config, case, &save_dir, &config.settings)?;
    let mut m = Metrics::default();
    let mut audio = Vec::with_capacity(4096);
    let mut seen = std::collections::HashSet::new();
    let last = case.frames - 1;
    let start = Instant::now();
    for frame in 0..case.frames {
        script.apply(&mut emu, frame);
        emu.run();
        observe(&mut emu, frame, &mut audio, &mut seen, &mut m);
        if frame == last || case.snapshots.contains(&frame) {
            snapshot(&mut emu, case, dir, frame, &mut m, png)?;
        }
    }
    m.frames = case.frames;
    m.ms_per_frame = start.elapsed().as_secs_f64() * 1000.0 / f64::from(case.frames);
    m.distinct_screens = seen.len();
    read_ram(&mut emu, case, &mut m);
    read_pixels(&emu, case, &mut m);
    emu.flush_save().map_err(|e| format!("flush_save: {e}"))?;
    m.save = std::fs::read_dir(&save_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| e.path().extension().is_some_and(|x| x == "sav"))
        .and_then(|e| Some((e.path(), e.metadata().ok()?.len())));
    Ok(m)
}