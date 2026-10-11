//! Drives the SPU from the ARM7 clock and hands its output to the frontend.
use crate::emulator::Emulator;
use lunaris_ds_audio::{CYCLES_PER_SAMPLE, SoundBus};
use lunaris_ds_mem_const::{ARM7_WRAM_MASK, ARM7_WRAM_START, MAIN_RAM_MASK};

/// The memory the SPU reaches (sound DMA / capture): main RAM, shared WRAM
/// and ARM7 WRAM, where sound data lives. Other regions read as 0 and
/// ignore writes.
struct SpuBus<'a> {
    main_ram: &'a mut [u8],
    shared_wram: &'a mut [u8],
    arm7_wram: &'a mut [u8],
    wram_cnt: u8,
}

impl SpuBus<'_> {
    fn locate(&mut self, addr: u32) -> Option<&mut [u8]> {
        let (mem, off): (&mut [u8], usize) = match addr >> 24 {
            0x02 => (&mut *self.main_ram, (addr & MAIN_RAM_MASK) as usize),
            0x03 if addr >= ARM7_WRAM_START => {
                (&mut *self.arm7_wram, (addr & ARM7_WRAM_MASK) as usize)
            }
            0x03 => match self.wram_cnt & 3 {
                0 => (&mut *self.arm7_wram, (addr & ARM7_WRAM_MASK) as usize),
                1 => (&mut *self.shared_wram, (addr & 0x3FFF) as usize),
                2 => (&mut *self.shared_wram, ((addr & 0x3FFF) + 0x4000) as usize),
                _ => (&mut *self.shared_wram, (addr & 0x7FFF) as usize),
            },
            _ => return None,
        };
        mem.get_mut(off..off + 4)
    }
}

impl SoundBus for SpuBus<'_> {
    fn read32(&mut self, addr: u32) -> u32 {
        self.locate(addr & !3)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn write32(&mut self, addr: u32, value: u32) {
        if let Some(b) = self.locate(addr & !3) {
            b.copy_from_slice(&value.to_le_bytes());
        }
    }
}

impl Emulator {
    /// Advances the SPU by `cycles` ARM7 bus cycles, producing one stereo
    /// sample every [`CYCLES_PER_SAMPLE`] cycles (32768 Hz).
    pub fn run_spu(&mut self, cycles: u32) {
        self.spu_cycles += cycles;
        while self.spu_cycles >= CYCLES_PER_SAMPLE {
            self.spu_cycles -= CYCLES_PER_SAMPLE;
            let Self {
                spu,
                main_ram,
                shared_wram,
                arm7_wram,
                wram_cnt,
                ..
            } = self;
            let mut bus = SpuBus {
                main_ram,
                shared_wram,
                arm7_wram,
                wram_cnt: *wram_cnt,
            };
            spu.mix(&mut bus);
        }
    }

    /// Moves the audio produced so far (interleaved stereo `i16`,
    /// [`lunaris_ds_audio::SAMPLE_RATE`] Hz) into `out`.
    pub fn take_audio_samples(&mut self, out: &mut Vec<i16>) {
        self.spu.take_samples(out);
    }
}
