mod write_arm7;
mod write_arm9;

use crate::emulator::Emulator;
use ds_cpu::arm_cpu::CpuType;

/// Folds main RAM mirrors together so a watchpoint matches any of them.
const fn watch_key(address: u32) -> u32 {
    if address >> 24 == 0x02 {
        address & 0x023F_FFFC
    } else {
        address & !3
    }
}

impl Emulator {
    /// Prints a CPU write that hits `Config::watch_addr`.
    #[cold]
    fn log_watched_write(&self, address: u32, value: u32, size: u32, cpu_type: CpuType) {
        let cpu = self.get_cpu(cpu_type);
        let pc = cpu
            .get_pc()
            .wrapping_sub(if cpu.cpsr.thumb_on { 4 } else { 8 });
        eprintln!(
            "WATCH arm{} pc~{pc:08X} write{size} [{address:08X}] = {value:08X}",
            if cpu_type == CpuType::Arm9 { 9 } else { 7 },
        );
    }

    fn check_watch(&self, address: u32, value: u32, size: u32, cpu_type: CpuType) {
        if let Some(watch) = self.config.watch_addr
            && watch_key(address) == watch_key(watch)
        {
            self.log_watched_write(address, value, size, cpu_type);
        }
    }

    pub fn write_word(&mut self, address: u32, word: u32, cpu_type: CpuType) {
        self.check_watch(address, word, 32, cpu_type);
        if cpu_type == CpuType::Arm9 {
            if !self.arm9_cp15.tcm_write(address, &word.to_le_bytes()) {
                self.arm9_write_word(address, word);
            }
        } else {
            self.arm7_write_word(address, word);
        }
    }

    pub fn write_halfword(&mut self, address: u32, halfword: u16, cpu_type: CpuType) {
        self.check_watch(address, halfword.into(), 16, cpu_type);
        if cpu_type == CpuType::Arm9 {
            if !self.arm9_cp15.tcm_write(address, &halfword.to_le_bytes()) {
                self.arm9_write_halfword(address, halfword);
            }
        } else {
            self.arm7_write_halfword(address, halfword);
        }
    }

    pub fn write_byte(&mut self, address: u32, byte: u8, cpu_type: CpuType) {
        self.check_watch(address, byte.into(), 8, cpu_type);
        if cpu_type == CpuType::Arm9 {
            if !self.arm9_cp15.tcm_write(address, &[byte]) {
                self.arm9_write_byte(address, byte);
            }
        } else {
            self.arm7_write_byte(address, byte);
        }
    }
}
