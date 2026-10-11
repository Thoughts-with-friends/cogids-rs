//! Sound Processing Unit (SPU) of the Nintendo DS.
//!
//! 16 channels (PCM8 / PCM16 / IMA-ADPCM on all, PSG square waves on 8-13,
//! noise on 14-15) mixed to a stereo 32768 Hz output, following GBATEK
//! "DS Sound" and melonDS `SPU.cpp`. Sound capture is not emulated (the
//! two capture units record the mixer output back to memory (used by games
//! such as New Super Mario Bros. for effects, played back on channels 1/3).
//!
//! The SPU accesses memory through a caller-provided [`SoundBus`], so it
//! does not own memory: see [`Spu::mix`].

/// Nominal output sample rate in Hz.
pub const SAMPLE_RATE: u32 = 32768;

/// ARM7 bus cycles (33.51 MHz) per output sample.
pub const CYCLES_PER_SAMPLE: u32 = 1024;

/// Exact output rate: the ARM7 clock (33.513982 MHz) / 1024, about
/// 32728 Hz (slightly below the nominal [`SAMPLE_RATE`]).
pub const OUTPUT_RATE_HZ: f64 = 33_513_982.0 / CYCLES_PER_SAMPLE as f64;

const ADPCM_INDEX_TABLE: [i8; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

const ADPCM_TABLE: [u16; 89] = [
    0x0007, 0x0008, 0x0009, 0x000A, 0x000B, 0x000C, 0x000D, 0x000E, 0x0010, 0x0011, 0x0013, 0x0015,
    0x0017, 0x0019, 0x001C, 0x001F, 0x0022, 0x0025, 0x0029, 0x002D, 0x0032, 0x0037, 0x003C, 0x0042,
    0x0049, 0x0050, 0x0058, 0x0061, 0x006B, 0x0076, 0x0082, 0x008F, 0x009D, 0x00AD, 0x00BE, 0x00D1,
    0x00E6, 0x00FD, 0x0117, 0x0133, 0x0151, 0x0173, 0x0198, 0x01C1, 0x01EE, 0x0220, 0x0256, 0x0292,
    0x02D4, 0x031C, 0x036C, 0x03C3, 0x0424, 0x048E, 0x0502, 0x0583, 0x0610, 0x06AB, 0x0756, 0x0812,
    0x08E0, 0x09C3, 0x0ABD, 0x0BD0, 0x0CFF, 0x0E4C, 0x0FBA, 0x114C, 0x1307, 0x14EE, 0x1706, 0x1954,
    0x1BDC, 0x1EA5, 0x21B6, 0x2515, 0x28CA, 0x2CDF, 0x315B, 0x364B, 0x3BB9, 0x41B2, 0x4844, 0x4F7E,
    0x5771, 0x602F, 0x69CE, 0x7462, 0x7FFF,
];

/// Duty cycles 0-7 of the PSG channels (eighths high).
const PSG_TABLE: [[i16; 8]; 8] = {
    let mut table = [[-0x7FFF; 8]; 8];
    let mut duty = 0;
    while duty < 7 {
        let mut step = 7 - duty;
        while step < 8 {
            table[duty][step] = 0x7FFF;
            step += 1;
        }
        duty += 1;
    }
    table
};

/// Memory accesses of the SPU (sound DMA) on the ARM7 bus.
pub trait SoundBus {
    fn read32(&mut self, addr: u32) -> u32;
    fn write32(&mut self, addr: u32, value: u32);
}

type Bus<'a> = dyn SoundBus + 'a;

/// One sound channel (SOUNDxCNT/SAD/TMR/PNT/LEN at 0x04000400 + x * 0x10).
#[derive(Debug, Clone, Default)]
struct Channel {
    num: usize,
    cnt: u32,
    src: u32,
    timer_reload: u16,
    /// Loop start in bytes.
    loop_pos: u32,
    /// Length after the loop start, in bytes.
    length: u32,

    volume: i32,
    volume_shift: u32,
    pan: i32,
    key_on: bool,

    timer: u32,
    pos: i32,
    cur_sample: i16,
    noise: u16,

    adpcm_val: i32,
    adpcm_index: i32,
    adpcm_val_loop: i32,
    adpcm_index_loop: i32,
    adpcm_byte: u8,

    /// 32-byte sound FIFO (melonDS models the hardware buffering).
    fifo: [u8; 32],
    fifo_read: usize,
    fifo_write: usize,
    fifo_offset: u32,
    fifo_level: i32,
}

impl Channel {
    fn set_cnt(&mut self, value: u32) {
        let old = self.cnt;
        self.cnt = value & 0xFF7F_837F;
        self.volume = (self.cnt & 0x7F) as i32;
        if self.volume == 127 {
            self.volume = 128;
        }
        self.volume_shift = [4, 3, 2, 0][((self.cnt >> 8) & 3) as usize];
        self.pan = ((self.cnt >> 16) & 0x7F) as i32;
        if self.pan == 127 {
            self.pan = 128;
        }
        if value & (1 << 31) != 0 && old & (1 << 31) == 0 {
            self.key_on = true;
        }
    }

    const fn format(&self) -> u32 {
        (self.cnt >> 29) & 3
    }

    const fn repeat(&self) -> u32 {
        (self.cnt >> 27) & 3
    }

    fn start(&mut self, bus: &mut Bus) {
        self.timer = u32::from(self.timer_reload);
        self.pos = if self.format() == 3 { -1 } else { -3 };
        self.noise = 0x7FFF;
        self.cur_sample = 0;
        self.fifo_read = 0;
        self.fifo_write = 0;
        self.fifo_offset = 0;
        self.fifo_level = 0;
        if self.format() != 3 {
            self.fifo_fill(bus);
            self.fifo_fill(bus);
        }
    }

    fn fifo_fill(&mut self, bus: &mut Bus) {
        let total = self.loop_pos + self.length;
        if self.fifo_offset >= total {
            match self.repeat() {
                r if r & 1 != 0 => self.fifo_offset = self.loop_pos,
                r if r & 2 != 0 => return,
                _ => {}
            }
        }
        let burst = 16.min(total.saturating_sub(self.fifo_offset));
        for _ in (0..burst).step_by(4) {
            let addr = self.src.wrapping_add(self.fifo_offset);
            // Sound DMA cannot read the ARM7 BIOS.
            let word = if addr >= 0x4000 { bus.read32(addr) } else { 0 };
            for b in word.to_le_bytes() {
                self.fifo[self.fifo_write] = b;
                self.fifo_write = (self.fifo_write + 1) & 31;
            }
            self.fifo_offset += 4;
        }
        self.fifo_level += burst as i32;
    }

    fn fifo_read(&mut self, bytes: usize, bus: &mut Bus) -> u32 {
        let mut value = 0;
        for i in 0..bytes {
            value |= u32::from(self.fifo[self.fifo_read]) << (i * 8);
            self.fifo_read = (self.fifo_read + 1) & 31;
        }
        self.fifo_level -= bytes as i32;
        if self.fifo_level <= 16 {
            self.fifo_fill(bus);
        }
        value
    }

    /// Handles the end of the sample data. Returns `false` if the channel
    /// stopped (one-shot).
    fn wrap_or_stop(&mut self, restart_pos: i32) -> bool {
        let repeat = self.repeat();
        if repeat & 1 != 0 {
            self.pos = restart_pos;
            true
        } else if repeat & 2 != 0 {
            self.cur_sample = 0;
            self.cnt &= !(1 << 31);
            false
        } else {
            true
        }
    }

    fn next_pcm8(&mut self, bus: &mut Bus) {
        self.pos += 1;
        if self.pos < 0 {
            return;
        }
        if self.pos as u32 >= self.loop_pos + self.length
            && !self.wrap_or_stop(self.loop_pos as i32)
        {
            return;
        }
        self.cur_sample = i16::from(self.fifo_read(1, bus) as u8 as i8) << 8;
    }

    fn next_pcm16(&mut self, bus: &mut Bus) {
        self.pos += 1;
        if self.pos < 0 {
            return;
        }
        if (self.pos as u32) << 1 >= self.loop_pos + self.length
            && !self.wrap_or_stop((self.loop_pos >> 1) as i32)
        {
            return;
        }
        self.cur_sample = self.fifo_read(2, bus) as u16 as i16;
    }

    fn next_adpcm(&mut self, bus: &mut Bus) {
        self.pos += 1;
        if self.pos < 8 {
            if self.pos == 0 {
                let header = self.fifo_read(4, bus);
                self.adpcm_val = i32::from(header as u16 as i16);
                self.adpcm_index = ((header >> 16) & 0x7F).min(88) as i32;
                self.adpcm_val_loop = self.adpcm_val;
                self.adpcm_index_loop = self.adpcm_index;
            }
            return;
        }

        if (self.pos as u32) >> 1 >= self.loop_pos + self.length {
            if self.repeat() & 1 != 0 {
                self.pos = (self.loop_pos << 1) as i32;
                self.adpcm_val = self.adpcm_val_loop;
                self.adpcm_index = self.adpcm_index_loop;
                self.adpcm_byte = self.fifo_read(1, bus) as u8;
            } else if self.repeat() & 2 != 0 {
                self.cur_sample = 0;
                self.cnt &= !(1 << 31);
                return;
            }
        } else {
            if self.pos & 1 == 0 {
                self.adpcm_byte = self.fifo_read(1, bus) as u8;
            } else {
                self.adpcm_byte >>= 4;
            }

            let val = ADPCM_TABLE[self.adpcm_index as usize];
            let mut diff = i32::from(val >> 3);
            if self.adpcm_byte & 1 != 0 {
                diff += i32::from(val >> 2);
            }
            if self.adpcm_byte & 2 != 0 {
                diff += i32::from(val >> 1);
            }
            if self.adpcm_byte & 4 != 0 {
                diff += i32::from(val);
            }
            if self.adpcm_byte & 8 != 0 {
                self.adpcm_val = (self.adpcm_val - diff).max(-0x7FFF);
            } else {
                self.adpcm_val = (self.adpcm_val + diff).min(0x7FFF);
            }
            self.adpcm_index = (self.adpcm_index
                + i32::from(ADPCM_INDEX_TABLE[(self.adpcm_byte & 7) as usize]))
            .clamp(0, 88);

            if self.pos as u32 == self.loop_pos << 1 {
                self.adpcm_val_loop = self.adpcm_val;
                self.adpcm_index_loop = self.adpcm_index;
            }
        }
        self.cur_sample = self.adpcm_val as i16;
    }

    fn next_psg(&mut self) {
        self.pos += 1;
        self.cur_sample = PSG_TABLE[((self.cnt >> 24) & 7) as usize][(self.pos & 7) as usize];
    }

    fn next_noise(&mut self) {
        if self.noise & 1 != 0 {
            self.noise = (self.noise >> 1) ^ 0x6000;
            self.cur_sample = -0x7FFF;
        } else {
            self.noise >>= 1;
            self.cur_sample = 0x7FFF;
        }
    }

    /// Advances the channel by `cycles` SPU cycles (16.76 MHz) and returns
    /// its volume-scaled output.
    fn run(&mut self, cycles: u32, bus: &mut Bus) -> i32 {
        if self.cnt & (1 << 31) == 0 {
            return 0;
        }
        // 0 = PCM8, 1 = PCM16, 2 = ADPCM, 3 = PSG (ch 8-13) / noise (ch 14-15).
        let kind = match self.format() {
            3 if (8..14).contains(&self.num) => 3,
            3 if self.num >= 14 => 4,
            3 => return 0,
            f => f,
        };
        if kind < 3 && self.loop_pos + self.length < 16 {
            return 0;
        }
        if self.key_on {
            self.start(bus);
            self.key_on = false;
        }

        self.timer += cycles;
        while self.timer >> 16 != 0 {
            self.timer = u32::from(self.timer_reload) + (self.timer - 0x1_0000);
            match kind {
                0 => self.next_pcm8(bus),
                1 => self.next_pcm16(bus),
                2 => self.next_adpcm(bus),
                3 => self.next_psg(),
                _ => self.next_noise(),
            }
            if self.cnt & (1 << 31) == 0 {
                break;
            }
        }

        (i32::from(self.cur_sample) << self.volume_shift) * self.volume
    }

    fn pan_output(&self, sample: i32, left: &mut i32, right: &mut i32) {
        *left += ((i64::from(sample) * i64::from(128 - self.pan)) >> 10) as i32;
        *right += ((i64::from(sample) * i64::from(self.pan)) >> 10) as i32;
    }
}

/// Sound capture unit (SNDCAPxCNT/DAD/LEN, melonDS `SPUCaptureUnit`).
/// Capture 0 records the left mixer, capture 1 the right one, clocked by the
/// timer of channel 1 / channel 3.
#[derive(Debug, Clone, Default)]
struct Capture {
    cnt: u8,
    dst: u32,
    /// Length in bytes.
    length: u32,
    timer_reload: u16,
    timer: u32,
    pos: u32,
    fifo: [u8; 16],
    fifo_read: usize,
    fifo_write: usize,
    write_offset: u32,
    level: u32,
}

impl Capture {
    fn set_cnt(&mut self, value: u8) {
        if value & 0x80 != 0 && self.cnt & 0x80 == 0 {
            self.timer = u32::from(self.timer_reload);
            self.pos = 0;
            self.fifo_read = 0;
            self.fifo_write = 0;
            self.write_offset = 0;
            self.level = 0;
        }
        let mut value = value & 0x8F;
        if value & 0x80 == 0 {
            value &= !1;
        }
        self.cnt = value;
    }

    fn flush(&mut self, bus: &mut Bus) {
        for _ in 0..4 {
            let i = self.fifo_read * 4;
            let word = u32::from_le_bytes([
                self.fifo[i],
                self.fifo[i + 1],
                self.fifo[i + 2],
                self.fifo[i + 3],
            ]);
            bus.write32(self.dst.wrapping_add(self.write_offset), word);
            self.fifo_read = (self.fifo_read + 1) & 3;
            self.level = self.level.saturating_sub(4);
            self.write_offset += 4;
            if self.write_offset >= self.length {
                self.write_offset = 0;
                break;
            }
        }
    }

    fn push(&mut self, bytes: &[u8], bus: &mut Bus) {
        for &b in bytes {
            self.fifo[self.fifo_write] = b;
            self.fifo_write = (self.fifo_write + 1) & 0xF;
        }
        self.level += bytes.len() as u32;
        if self.level >= 16 {
            self.flush(bus);
        }
    }

    fn run(&mut self, cycles: u32, sample: i32, bus: &mut Bus) {
        self.timer += cycles;
        let pcm8 = self.cnt & 0x08 != 0;
        while self.timer >> 16 != 0 {
            self.timer = u32::from(self.timer_reload) + (self.timer - 0x1_0000);
            if pcm8 {
                self.push(&[(sample >> 8) as u8], bus);
                self.pos += 1;
            } else {
                self.push(&(sample as i16).to_le_bytes(), bus);
                self.pos += 2;
            }
            if self.pos >= self.length {
                if self.level >= 4 {
                    self.flush(bus);
                }
                if self.cnt & 0x04 != 0 {
                    self.cnt &= 0x7F; // one-shot: done
                    return;
                }
                self.pos = 0;
            }
        }
    }
}

/// The DS sound unit.
#[derive(Debug)]
pub struct Spu {
    channels: [Channel; 16],
    soundcnt: u16,
    bias: u16,
    captures: [Capture; 2],
    /// Interleaved stereo output not yet taken by the frontend.
    samples: Vec<i16>,
}

/// Kept for the old name used across the code base.
pub type SPU = Spu;

impl Default for Spu {
    fn default() -> Self {
        Self::new()
    }
}

impl Spu {
    pub fn new() -> Self {
        let mut channels: [Channel; 16] = Default::default();
        for (i, ch) in channels.iter_mut().enumerate() {
            ch.num = i;
        }
        Self {
            channels,
            soundcnt: 0,
            bias: 0x200,
            captures: Default::default(),
            samples: Vec::with_capacity(4096),
        }
    }

    /// Resets the unit (keeps the sample buffer allocation).
    pub fn power_on(&mut self) {
        let samples = std::mem::take(&mut self.samples);
        *self = Self::new();
        self.samples = samples;
        self.samples.clear();
    }

    // ===== registers (0x04000400 - 0x0400051F) =====

    /// 32-bit read of a sound register.
    pub fn read32(&self, address: u32) -> u32 {
        let address = address & !3;
        match address {
            0x0400_0400..=0x0400_04FF => {
                let ch = &self.channels[((address >> 4) & 0xF) as usize];
                // Only SOUNDxCNT is readable.
                if address & 0xF == 0 { ch.cnt } else { 0 }
            }
            0x0400_0500 => u32::from(self.soundcnt),
            0x0400_0504 => u32::from(self.bias),
            0x0400_0508 => u32::from(self.captures[0].cnt) | (u32::from(self.captures[1].cnt) << 8),
            0x0400_0510 => self.captures[0].dst,
            0x0400_0518 => self.captures[1].dst,
            _ => 0,
        }
    }

    pub fn read16(&self, address: u32) -> u16 {
        (self.read32(address) >> ((address & 2) * 8)) as u16
    }

    pub fn read8(&self, address: u32) -> u8 {
        (self.read32(address) >> ((address & 3) * 8)) as u8
    }

    /// Writes the bytes of `value` selected by `mask` (aligned to the word
    /// containing `address`).
    pub fn write(&mut self, address: u32, value: u32, mask: u32) {
        let address = address & !3;
        let merge = |old: u32| (old & !mask) | (value & mask);
        match address {
            0x0400_0400..=0x0400_04FF => {
                let ch = &mut self.channels[((address >> 4) & 0xF) as usize];
                match address & 0xF {
                    0x0 => ch.set_cnt(merge(ch.cnt)),
                    0x4 => ch.src = merge(ch.src) & 0x07FF_FFFC,
                    0x8 => {
                        let word = merge(u32::from(ch.timer_reload) | ((ch.loop_pos >> 2) << 16));
                        ch.timer_reload = word as u16;
                        ch.loop_pos = (word >> 16) << 2;
                        // Captures run on the timers of channels 1 and 3.
                        match ch.num {
                            1 => self.captures[0].timer_reload = word as u16,
                            3 => self.captures[1].timer_reload = word as u16,
                            _ => {}
                        }
                    }
                    _ => ch.length = (merge(ch.length >> 2) & 0x001F_FFFF) << 2,
                }
            }
            0x0400_0500 => {
                let word = merge(u32::from(self.soundcnt));
                self.soundcnt = (word as u16) & 0xBF7F;
            }
            0x0400_0504 => self.bias = (merge(u32::from(self.bias)) & 0x3FF) as u16,
            0x0400_0508 => {
                let old = u32::from(self.captures[0].cnt) | (u32::from(self.captures[1].cnt) << 8);
                let word = merge(old);
                if mask & 0xFF != 0 {
                    self.captures[0].set_cnt(word as u8);
                }
                if mask & 0xFF00 != 0 {
                    self.captures[1].set_cnt((word >> 8) as u8);
                }
            }
            0x0400_0510 | 0x0400_0518 => {
                let cap = &mut self.captures[((address >> 3) & 1) as usize];
                cap.dst = merge(cap.dst) & 0x07FF_FFFC;
            }
            0x0400_0514 | 0x0400_051C => {
                let cap = &mut self.captures[((address >> 3) & 1) as usize];
                let words = merge(cap.length >> 2) & 0xFFFF;
                cap.length = (words << 2).max(4);
            }
            _ => {}
        }
    }

    pub fn write32(&mut self, address: u32, value: u32) {
        self.write(address, value, 0xFFFF_FFFF);
    }

    pub fn write16(&mut self, address: u32, value: u16) {
        let shift = (address & 2) * 8;
        self.write(address, u32::from(value) << shift, 0xFFFF << shift);
    }

    pub fn write8(&mut self, address: u32, value: u8) {
        let shift = (address & 3) * 8;
        self.write(address, u32::from(value) << shift, 0xFF << shift);
    }

    // Older accessors kept for the register dispatch code.
    pub fn get_soundcnt(&self) -> u16 {
        self.soundcnt
    }
    pub fn get_soundbias(&self) -> u16 {
        self.bias
    }
    pub fn get_sndcap0(&self) -> u8 {
        self.captures[0].cnt
    }
    pub fn get_sndcap1(&self) -> u8 {
        self.captures[1].cnt
    }
    pub fn set_soundcnt(&mut self, value: u16) {
        self.write16(0x0400_0500, value);
    }
    pub fn set_soundcnt_lo(&mut self, value: u8) {
        self.write8(0x0400_0500, value);
    }
    pub fn set_soundcnt_hi(&mut self, value: u8) {
        self.write8(0x0400_0501, value);
    }
    pub fn set_soundbias(&mut self, value: u16) {
        self.write16(0x0400_0504, value);
    }
    pub fn set_sndcap0(&mut self, value: u8) {
        self.write8(0x0400_0508, value);
    }
    pub fn set_sndcap1(&mut self, value: u8) {
        self.write8(0x0400_0509, value);
    }

    // ===== mixing =====

    /// Produces one output sample covering `CYCLES_PER_SAMPLE` bus cycles
    /// and appends it to the sample buffer. `bus` reads sample data.
    pub fn mix(&mut self, bus: &mut Bus) {
        // Channel timers run at half the bus clock.
        let cycles = CYCLES_PER_SAMPLE / 2;
        let (mut left, mut right) = (0_i32, 0_i32);
        let (mut left_out, mut right_out) = (0_i32, 0_i32);

        if self.soundcnt & (1 << 15) != 0 {
            let mut out = [0_i32; 16];
            for (o, ch) in out.iter_mut().zip(self.channels.iter_mut()) {
                *o = ch.run(cycles, bus);
            }
            for (i, &o) in out.iter().enumerate() {
                // SOUNDCNT bits 12/13: channel 1/3 not sent to the mixer.
                if (i == 1 && self.soundcnt & (1 << 12) != 0)
                    || (i == 3 && self.soundcnt & (1 << 13) != 0)
                {
                    continue;
                }
                self.channels[i].pan_output(o, &mut left, &mut right);
            }

            // Sound capture of the left / right mixer.
            for (i, mixed) in [left, right].into_iter().enumerate() {
                if self.captures[i].cnt & 0x80 != 0 {
                    let sample = (mixed >> 8).clamp(-0x8000, 0x7FFF);
                    self.captures[i].run(cycles, sample, bus);
                }
            }

            let side = |ch: &Channel, sample: i32, right_side: bool| {
                let pan = if right_side { ch.pan } else { 128 - ch.pan };
                ((i64::from(sample) * i64::from(pan)) >> 10) as i32
            };
            let (ch1, ch3) = (&self.channels[1], &self.channels[3]);
            left_out = match (self.soundcnt >> 8) & 3 {
                0 => left,
                1 => side(ch1, out[1], false),
                2 => side(ch3, out[3], false),
                _ => side(ch1, out[1], false) + side(ch3, out[3], false),
            };
            right_out = match (self.soundcnt >> 10) & 3 {
                0 => right,
                1 => side(ch1, out[1], true),
                2 => side(ch3, out[3], true),
                _ => side(ch1, out[1], true) + side(ch3, out[3], true),
            };
        }

        let mut master = i64::from(self.soundcnt & 0x7F);
        if master == 127 {
            master = 128;
        }
        let finish = |v: i32| -> i16 {
            // The SOUNDBIAS used by commercial games (0x200) is the DC
            // offset of an unsigned DAC; it is removed here.
            let v = ((i64::from(v) * master) >> 7) >> 8;
            v.clamp(-0x8000, 0x7FFF) as i16
        };
        // Without a consumer (headless use) keep at most ~2 s of audio.
        if self.samples.len() >= SAMPLE_RATE as usize * 4 {
            self.samples.drain(..SAMPLE_RATE as usize * 2);
        }
        self.samples.push(finish(left_out));
        self.samples.push(finish(right_out));
    }

    /// Takes the buffered interleaved stereo samples (32768 Hz).
    pub fn take_samples(&mut self, out: &mut Vec<i16>) {
        out.append(&mut self.samples);
    }

    /// Drops buffered samples (e.g. when no audio output is attached).
    pub fn clear_samples(&mut self) {
        self.samples.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fill(u32);
    impl SoundBus for Fill {
        fn read32(&mut self, _addr: u32) -> u32 {
            self.0
        }
        fn write32(&mut self, _addr: u32, _value: u32) {}
    }

    #[test]
    fn psg_square_wave_is_audible() {
        let mut spu = Spu::new();
        spu.write16(0x0400_0500, 0x807F); // enable, master volume max
        let base = 0x0400_0400 + 8 * 0x10;
        spu.write16(base + 8, 0xF000); // timer reload
        // Volume 127, pan centre, duty 4, PSG format, start.
        spu.write32(base, 0x7F | (64 << 16) | (4 << 24) | (3 << 29) | (1 << 31));
        let mut bus = Fill(0);
        for _ in 0..256 {
            spu.mix(&mut bus);
        }
        let mut out = Vec::new();
        spu.take_samples(&mut out);
        assert_eq!(out.len(), 512);
        assert!(out.iter().any(|&s| s > 1000) && out.iter().any(|&s| s < -1000));
    }

    #[test]
    fn pcm16_one_shot_stops() {
        let mut spu = Spu::new();
        spu.write16(0x0400_0500, 0x807F);
        spu.write32(0x0400_0404, 0x0200_0000);
        spu.write16(0x0400_0408, 0xFC00);
        spu.write32(0x0400_040C, 8); // 32 bytes
        spu.write32(
            0x0400_0400,
            0x7F | (64 << 16) | (1 << 29) | (2 << 27) | (1 << 31),
        );
        let mut bus = Fill(0x4000_4000);
        for _ in 0..4096 {
            spu.mix(&mut bus);
        }
        assert_eq!(
            spu.read32(0x0400_0400) & (1 << 31),
            0,
            "one-shot sound ends"
        );
    }
}
