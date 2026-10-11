// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! cpuinsters.hpp
//!
pub mod arm_instruction;
pub mod thumb_instruction;

use ds_cpu::arm_cpu::CpuType;

use self::arm_instruction::blx;
use crate::cpu::arm_table;
use crate::emulator::Emulator;

/// Interprets an ARM instruction
pub fn arm_interpret(emu: &mut Emulator, cpu_type: CpuType) {
    let instruction: u32 = emu.get_cpu_mut(cpu_type).get_current_instr();
    let condition: u32 = (instruction & 0xF000_0000) >> 28;

    // In ARM, PC reads as current + 8
    let cpu_id = emu.get_cpu(cpu_type).get_id();
    let test_mode = emu.config.test;

    // Debug
    if cpu_id <= 0 && test_mode {
        if cpu_id <= 0 {
            #[cfg(feature = "log")]
            log::trace!("(9A)");
        } else {
            #[cfg(feature = "log")]
            log::trace!("(7A)");
        }
        let pc: u32 = emu.get_cpu(cpu_type).get_pc().wrapping_sub(8);

        #[cfg(feature = "log")]
        log::trace!("[{pc:08X}] {instruction:08X} - ");

        let cpu = emu.get_cpu_mut(cpu_type);
        let disasm = ds_cpu::disassemble::disasm_arm(cpu, instruction, pc);
        #[cfg(feature = "log")]
        log::trace!("disasm: {disasm}");
    }

    // Build opcode
    let op: u32 = ((instruction >> 4) & 0xF) | ((instruction >> 16) & 0xFF0);

    // Special BLX handling
    match condition == 15 && (instruction & 0xFE00_0000) == 0xFA00_0000 && cpu_id <= 0 {
        true => blx(emu, cpu_type, instruction),
        false => {
            if emu.get_cpu_mut(cpu_type).check_condition(condition as i32) {
                // Assuming arm_table is indexed by u32 and stores fn(&mut Cpu, u32)
                arm_table::ARM_TABLE[op as usize](emu, cpu_type, instruction);
            }
        }
    }
}
