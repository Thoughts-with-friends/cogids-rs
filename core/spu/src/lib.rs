//! Sound Processing Unit (SPU) of the Nintendo DS.
//!
//! ```text
//!  channel 0..15 (channel.rs)      PCM8 / PCM16 / ADPCM (adpcm.rs) on all,
//!        |                         PSG on 8-13, noise on 14-15
//!        v
//!  mixer (mixer.rs) -- volume, panning, SOUNDCNT output select
//!        |        \
//!        |         `--> capture 0 / 1 (capture.rs) --> memory
//!        v
//!  stereo samples (~32728 Hz) --> frontend
//! ```
//!
//! Follows GBATEK "DS Sound" and melonDS `SPU.cpp`. The SPU accesses memory
//! (sample data, capture) through a caller-provided [`SoundBus`], so it does
//! not own memory: see [`Spu::mix`].
mod adpcm;
mod capture;
mod channel;
mod mixer;

use capture::Capture;
use channel::Channel;

/// Nominal output sample rate in Hz.
pub const SAMPLE_RATE: u32 = 32768;

/// ARM7 bus cycles (33.51 MHz) per output sample.
pub const CYCLES_PER_SAMPLE: u32 = 1024;

/// Exact output rate: the ARM7 clock (33.513982 MHz) / 1024, about
/// 32728 Hz (slightly below the nominal [`SAMPLE_RATE`]).
pub const OUTPUT_RATE_HZ: f64 = 33_513_982.0 / CYCLES_PER_SAMPLE as f64;

/// Memory accesses of the SPU (sound DMA) on the ARM7 bus.
pub trait SoundBus {
    fn read32(&mut self, addr: u32) -> u32;
    /// Word write to a sound register (ds_emu arm7_write_word,
    /// 0x04000400..0x0400051F) -> write().
    fn write32(&mut self, addr: u32, value: u32);
}

type Bus<'a> = dyn SoundBus + 'a;

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
    /// Creates a silent SPU: 16 channels, 2 capture units, empty buffer.
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

    /// Halfword read of a sound register (from ds_emu arm7_read_halfword).
    pub fn read16(&self, address: u32) -> u16 {
        (self.read32(address) >> ((address & 2) * 8)) as u16
    }

    /// Byte read of a sound register.
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

    /// Halfword write to a sound register -> write().
    pub fn write16(&mut self, address: u32, value: u16) {
        let shift = (address & 2) * 8;
        self.write(address, u32::from(value) << shift, 0xFFFF << shift);
    }

    /// Byte write to a sound register -> write().
    pub fn write8(&mut self, address: u32, value: u8) {
        let shift = (address & 3) * 8;
        self.write(address, u32::from(value) << shift, 0xFF << shift);
    }

    // Older accessors kept for the register dispatch code.
    /// SOUNDCNT (master volume / output select / enable).
    pub fn get_soundcnt(&self) -> u16 {
        self.soundcnt
    }
    /// SOUNDBIAS.
    pub fn get_soundbias(&self) -> u16 {
        self.bias
    }
    /// SNDCAP0CNT.
    pub fn get_sndcap0(&self) -> u8 {
        self.captures[0].cnt
    }
    /// SNDCAP1CNT.
    pub fn get_sndcap1(&self) -> u8 {
        self.captures[1].cnt
    }
    /// Writes SOUNDCNT.
    pub fn set_soundcnt(&mut self, value: u16) {
        self.write16(0x0400_0500, value);
    }
    /// Writes the low byte of SOUNDCNT (master volume).
    pub fn set_soundcnt_lo(&mut self, value: u8) {
        self.write8(0x0400_0500, value);
    }
    /// Writes the high byte of SOUNDCNT (output select, enable).
    pub fn set_soundcnt_hi(&mut self, value: u8) {
        self.write8(0x0400_0501, value);
    }
    /// Writes SOUNDBIAS.
    pub fn set_soundbias(&mut self, value: u16) {
        self.write16(0x0400_0504, value);
    }
    /// Writes SNDCAP0CNT.
    pub fn set_sndcap0(&mut self, value: u8) {
        self.write8(0x0400_0508, value);
    }
    /// Writes SNDCAP1CNT.
    pub fn set_sndcap1(&mut self, value: u8) {
        self.write8(0x0400_0509, value);
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
