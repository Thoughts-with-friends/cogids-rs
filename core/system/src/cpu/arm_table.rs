// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! armtable.cpp
//!
//! The ARM dispatch table is indexed by a 12-bit opcode built from
//! instruction bits 27-20 (upper 8 bits) and bits 7-4 (lower 4 bits):
//!
//! ```text
//! op = ((instruction >> 16) & 0xFF0) | ((instruction >> 4) & 0xF)
//! ```
//!
//! Instead of listing all 4096 entries by hand, the table is generated at
//! compile time by [`decode`].
use ds_cpu::arm_cpu::CpuType;

use super::interpreter::arm_instruction::*;
use crate::emulator::Emulator;

/// Function pointer type for ARM/Thumb instruction interpreters
pub type InterpreterFunc = fn(emu: &mut Emulator, cpu_type: CpuType, instruction: u32);

/// arm interpreter function table
pub static ARM_TABLE: [InterpreterFunc; 4096] = {
    let mut table: [InterpreterFunc; 4096] = [undefined; 4096];
    let mut op = 0;
    while op < table.len() {
        table[op] = decode(op);
        op += 1;
    }
    table
};

/// Selects the interpreter for a 12-bit table index.
const fn decode(op: usize) -> InterpreterFunc {
    let hi = op >> 4; // instruction bits 27-20
    let lo = op & 0xF; // instruction bits 7-4
    let load = hi & 0x01 != 0; // bit 20 (L)

    match hi >> 5 {
        // bits 27-25
        0b000 => decode_000(hi, lo),
        0b001 => data_processing, // immediate operand
        0b010 | 0b011 => match (hi & 0x04 != 0, load) {
            // bit 22 (B), bit 20 (L)
            (false, false) => store_word,
            (false, true) => load_word,
            (true, false) => store_byte,
            (true, true) => load_byte,
        },
        0b100 => match load {
            false => store_block,
            true => load_block,
        },
        0b101 => match hi & 0x10 != 0 {
            // bit 24 (link)
            false => branch,
            true => branch_link,
        },
        0b110 => undefined, // coprocessor data transfer
        _ => match (hi & 0x10 != 0, lo & 0x1 != 0) {
            // bit 24 (swi), bit 4 (register transfer)
            (true, _) => swi,
            (false, true) => coprocessor_reg_transfer,
            (false, false) => undefined, // coprocessor data operation
        },
    }
}

/// Decodes the `000` group: data processing (register operand), multiplies,
/// halfword/doubleword transfers and miscellaneous instructions.
const fn decode_000(hi: usize, lo: usize) -> InterpreterFunc {
    let load = hi & 0x01 != 0; // bit 20 (L)
    // bits 24-23 = 10, bit 20 = 0: MRS/MSR/BX/CLZ/QADD/SMLAxy etc.
    let misc = hi & 0x19 == 0x10;

    match lo {
        // Multiply and swap
        0b1001 => match hi {
            0x00..=0x03 => multiply,
            0x08..=0x0F => multiply_long,
            0x10 | 0x14 => swap,
            _ => undefined,
        },
        // Halfword / signed / doubleword transfers
        0b1011 if load => load_halfword,
        0b1011 => store_halfword,
        0b1101 if load => load_signed_byte,
        0b1101 => undefined,
        0b1111 if load => load_signed_halfword,
        0b1111 => store_doubleword,
        // Miscellaneous instructions
        0b0001 if misc && hi == 0x12 => branch_exchange,
        0b0001 if misc && hi == 0x16 => count_leading_zeros,
        0b0011 if misc && hi == 0x12 => blx_reg,
        0b0101 if misc => saturated_op,
        0b1000 | 0b1010 | 0b1100 | 0b1110 if misc => signed_halfword_multiply,
        _ => data_processing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(instruction: u32) -> InterpreterFunc {
        let op = ((instruction >> 4) & 0xF) | ((instruction >> 16) & 0xFF0);
        ARM_TABLE[op as usize]
    }

    #[test]
    fn decodes_representative_instructions() {
        let cases: [(u32, InterpreterFunc, &str); 18] = [
            (0xE081_0002, data_processing, "add r0, r1, r2"),
            (0xE3A0_0001, data_processing, "mov r0, #1"),
            (0xE000_0291, multiply, "mul r0, r1, r2"),
            (0xE081_0392, multiply_long, "umull r0, r1, r2, r3"),
            (0xE101_0092, swap, "swp r0, r2, [r1]"),
            (0xE12F_FF11, branch_exchange, "bx r1"),
            (0xE12F_FF31, blx_reg, "blx r1"),
            (0xE16F_0F11, count_leading_zeros, "clz r0, r1"),
            (0xE102_0051, saturated_op, "qadd r0, r1, r2"),
            (0xE1D1_00B0, load_halfword, "ldrh r0, [r1]"),
            (0xE1C1_00F0, store_doubleword, "strd r0, [r1]"),
            (0xE591_0000, load_word, "ldr r0, [r1]"),
            (0xE5C1_0000, store_byte, "strb r0, [r1]"),
            (0xE8BD_8000, load_block, "pop {pc}"),
            (0xEB00_0000, branch_link, "bl"),
            (
                0xEE11_0F10,
                coprocessor_reg_transfer,
                "mrc p15, 0, r0, c1, c0, 0",
            ),
            (0xEF00_0000, swi, "swi 0"),
            (0xE1C1_00D0, undefined, "ldrd slot (unimplemented)"),
        ];
        for (instruction, expected, asm) in cases {
            #[cfg(feature = "log")]
            assert!(std::ptr::fn_addr_eq(lookup(instruction), expected), "{asm}");
        }
    }
}
