// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! High-level BIOS: the BIOS functions games call with `SWI`, in Rust.
//!
//! Used instead of running BIOS code on the emulated CPU, unless the
//! replacement BIOS images in [`crate::image`] are selected.
//!
//! ```text
//!  game code: SWI #n
//!       |  (CPU exception, intercepted by the system)
//!       v
//!  swi::arm9(ctx) / swi::arm7(ctx)  -- decode n, read r0-r3, do the work
//!       |
//!       v
//!  SwiContext  -- registers, memory and halt of the calling CPU,
//!                 implemented by the system crate
//! ```
//!
//! See GBATEK "BIOS Functions".
mod tables;

/// What a BIOS function needs from the CPU that executed `SWI`.
pub trait SwiContext {
    /// Register `index` (r0-r15) of the calling CPU.
    fn reg(&self, index: usize) -> u32;
    fn set_reg(&mut self, index: usize, value: u32);
    /// PC of the calling CPU while the SWI is handled (the `SWI`
    /// instruction is found relative to it).
    fn pc(&self) -> u32;
    /// Whether `SWI` was executed in THUMB state.
    fn is_thumb(&self) -> bool;

    fn read8(&mut self, addr: u32) -> u8;
    fn read16(&mut self, addr: u32) -> u16;
    fn read32(&mut self, addr: u32) -> u32;
    fn write16(&mut self, addr: u32, value: u16);
    fn write32(&mut self, addr: u32, value: u32);

    /// Stalls the CPU for `cycles` cycles.
    fn add_cycles(&mut self, cycles: i32);
    /// Halts the CPU until an enabled interrupt arrives.
    fn halt(&mut self);
    /// Enables the interrupts in `mask` (IE, IME) and halts until one
    /// arrives.
    fn wait_for_interrupt(&mut self, mask: u32);
}

/// `SWI` number from the instruction before the return address.
fn opcode(ctx: &mut impl SwiContext) -> u8 {
    let swi = ctx.pc() - if ctx.is_thumb() { 2 } else { 4 };
    (ctx.read16(swi - 2) & 0xFF) as u8
}

/// ARM9 BIOS functions. Returns the cycles taken.
pub fn arm9(ctx: &mut impl SwiContext) -> i32 {
    match opcode(ctx) {
        0x03 => {
            let value = ctx.reg(0) * 2;
            ctx.add_cycles(value as i32);
        }
        0x04 => {
            let mask = ctx.reg(1);
            ctx.wait_for_interrupt(mask);
        }
        0x05 => ctx.wait_for_interrupt(1), // VBlank (IE/IF bit 0)
        0x06 => ctx.halt(),
        0x0B => cpu_set(ctx, true),
        0x0E => get_crc16(ctx),
        _opcode => {
            #[cfg(feature = "tracing")]
            tracing::error!("Unrecognized HLE SWI9 ${_opcode:02X}");
        }
    }
    1
}

/// ARM7 BIOS functions. Returns the cycles taken.
pub fn arm7(ctx: &mut impl SwiContext) -> i32 {
    let opcode = opcode(ctx);
    match opcode {
        0x03 => {
            let value = ctx.reg(0) * 4;
            ctx.add_cycles(value as i32);
        }
        0x04 => {
            let mask = ctx.reg(1);
            ctx.wait_for_interrupt(mask);
        }
        0x05 => ctx.wait_for_interrupt(1), // VBlank (IE/IF bit 0)
        0x06 => ctx.halt(),
        0x0B => cpu_set(ctx, false),
        0x0E => get_crc16(ctx),
        // Sound helpers used by the SDK sound driver to turn notes and
        // decibels into timer / volume values. Out-of-range indexes return
        // garbage on hardware; 0 here.
        0x1A..=0x1C => {
            let index = ctx.reg(0) as usize;
            let value = match opcode {
                0x1A => tables::SINE_TABLE.get(index).map(|&v| u32::from(v)),
                0x1B => tables::PITCH_TABLE.get(index).map(|&v| u32::from(v)),
                _ => tables::VOLUME_TABLE.get(index).map(|&v| u32::from(v)),
            };
            ctx.set_reg(0, value.unwrap_or(0));
        }
        _ => {
            #[cfg(feature = "tracing")]
            tracing::error!("Unrecognized HLE SWI7 ${opcode:02X}");
        }
    }
    1
}

/// `SWI 0x0B` CpuSet: copy or fill (r0 = source, r1 = destination,
/// r2 = length and flags). The ARM7 version refuses to touch the BIOS area.
fn cpu_set(ctx: &mut impl SwiContext, arm9: bool) {
    let mut source = ctx.reg(0);
    let mut dest = ctx.reg(1);
    let flags = ctx.reg(2);

    if !arm9 && source < 0x4000 && dest < 0x4000 {
        return;
    }

    let mut len = flags & 0x1F_FFFF;
    let memfill = flags & (1 << 24) != 0;
    let size_word = flags & (1 << 26) != 0;

    while len != 0 {
        if size_word {
            let v = ctx.read32(source);
            ctx.write32(dest, v);
        } else {
            let v = ctx.read16(source);
            ctx.write16(dest, v);
        }
        if !memfill {
            source += 2 << u32::from(size_word);
        }
        dest += 2 << u32::from(size_word);
        len -= 1;
    }
}

/// `SWI 0x0E` GetCRC16: r0 = initial value, r1 = address, r2 = length in
/// bytes; returns the CRC in r0.
///
/// This is the reflected CRC-16 with polynomial 0xA001 (GBATEK "BIOS Misc
/// Functions"), the same checksum the firmware stores for its user
/// settings.
fn get_crc16(ctx: &mut impl SwiContext) {
    let mut crc = (ctx.reg(0) & 0xFFFF) as u16;
    let addr = ctx.reg(1);
    let len = ctx.reg(2);

    for i in 0..len {
        crc ^= u16::from(ctx.read8(addr.wrapping_add(i)));
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    ctx.set_reg(0, u32::from(crc));
}
