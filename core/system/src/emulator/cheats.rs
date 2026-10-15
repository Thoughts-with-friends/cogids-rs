// SPDX-License-Identifier: GPL-3.0-or-later
//! Action Replay DS cheat engine, a port of melonDS `AREngine.cpp`.
//!
//! ```text
//!  GUI cheat list --parse_ar_code(text)--> Vec<u32> (address/value pairs)
//!       |  set_cheats(enabled codes)
//!       v
//!  run() start of every frame --> run_cheats() --> one ArState per code:
//!       00-2x  writes            3x-Ax  conditionals (IF ... ENDIF)
//!       Bx/C-  offset / loops    Dx     data register, ENDIF / NEXT
//!       Ex/Fx  block copies
//! ```
//!
//! Like melonDS, codes read and write memory through the ARM7 bus (main RAM
//! is shared, so ARM9 data is reachable as well). See GBATEK "DS Cart
//! Cheat Action Replay DS".
use super::Emulator;

/// Parses Action Replay code text: 8-digit hex words, two per line (any
/// whitespace between them works too).
///
/// # Errors
/// Returns a message when a word is not 8 hex digits or the words do not
/// pair up.
pub fn parse_ar_code(text: &str) -> Result<Vec<u32>, String> {
    let mut words = Vec::new();
    for word in text.split_whitespace() {
        if word.len() != 8 || !word.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("\"{word}\" is not an 8-digit hex number"));
        }
        words.push(u32::from_str_radix(word, 16).map_err(|e| e.to_string())?);
    }
    if words.len() % 2 != 0 {
        return Err("The code must have an even number of words (address value pairs)".to_owned());
    }
    Ok(words)
}

/// Interpreter state of one code (melonDS `RunCheat` locals).
struct ArState {
    /// Index of the next word.
    pc: usize,
    offset: u32,
    datareg: u32,
    cond: bool,
    condstack: u32,
    loopstart: usize,
    loopcount: u32,
    loopcond: bool,
    loopcondstack: u32,
    c5count: u32,
}

impl ArState {
    const fn new() -> Self {
        Self {
            pc: 0,
            offset: 0,
            datareg: 0,
            cond: true,
            condstack: 0,
            loopstart: 0,
            loopcount: 0,
            loopcond: true,
            loopcondstack: 0,
            c5count: 0,
        }
    }

    /// Opens an IF block.
    const fn push_cond(&mut self, cond: bool) {
        self.condstack = (self.condstack << 1) | self.cond as u32;
        self.cond = cond;
    }

    /// Word `i` of the code (0 past the end, like reading padding).
    fn word(code: &[u32], i: usize) -> u32 {
        code.get(i).copied().unwrap_or(0)
    }
}

/// What an opcode asks the loop to do next.
enum Flow {
    Continue,
    Stop,
}

impl Emulator {
    /// Replaces the codes run every frame (each one a list of words; the
    /// GUI passes only the enabled ones).
    pub fn set_cheats(&mut self, codes: Vec<Vec<u32>>) {
        self.cheats = codes;
    }

    /// Runs every code once (start of each frame, as melonDS does).
    pub(crate) fn run_cheats(&mut self) {
        if self.cheats.is_empty() {
            return;
        }
        let cheats = std::mem::take(&mut self.cheats);
        for code in &cheats {
            self.run_cheat(code);
        }
        self.cheats = cheats;
    }

    /// One code (melonDS `AREngine::RunCheat`).
    fn run_cheat(&mut self, code: &[u32]) {
        let mut s = ArState::new();
        // Guards against codes that loop forever.
        let mut budget = 1_000_000_u32;
        while s.pc < code.len() {
            budget = budget.saturating_sub(1);
            if budget == 0 {
                break;
            }
            let a = ArState::word(code, s.pc);
            let b = ArState::word(code, s.pc + 1);
            s.pc += 2;
            let op = (a >> 24) as u8;

            // Everything but ENDIF / NEXT / C5 is skipped inside a false IF;
            // a skipped block copy also skips its data words.
            if ((op < 0xD0 && op != 0xC5) || op > 0xD2) && !s.cond {
                if op & 0xF0 == 0xE0 {
                    s.pc += (b.div_ceil(8) as usize) * 2;
                }
                continue;
            }

            let flow = match op >> 4 {
                0x0..=0x2 => {
                    self.ar_write(op, a, b, &s);
                    Flow::Continue
                }
                0x3..=0xA => {
                    self.ar_condition(op, a, b, &mut s);
                    Flow::Continue
                }
                0xB => {
                    s.offset = self.arm7_read_word((a & 0x0FFF_FFFF).wrapping_add(s.offset));
                    Flow::Continue
                }
                0xC => self.ar_control(op, b, &mut s),
                0xD => {
                    self.ar_data(op, a, b, &mut s);
                    Flow::Continue
                }
                0xE => {
                    self.ar_copy_in(a, b, code, &mut s);
                    Flow::Continue
                }
                _ => {
                    self.ar_copy(a, b, &s);
                    Flow::Continue
                }
            };
            if matches!(flow, Flow::Stop) {
                break;
            }
        }
    }

    /// 0x/1x/2x: 32/16/8-bit write to `a + offset`.
    fn ar_write(&mut self, op: u8, a: u32, b: u32, s: &ArState) {
        let addr = (a & 0x0FFF_FFFF).wrapping_add(s.offset);
        match op >> 4 {
            0 => self.arm7_write_word(addr, b),
            1 => self.arm7_write_halfword(addr, b as u16),
            _ => self.arm7_write_byte(addr, b as u8),
        }
    }

    /// 3x-6x compare a word, 7x-Ax compare a masked halfword.
    fn ar_condition(&mut self, op: u8, a: u32, b: u32, s: &mut ArState) {
        let mut addr = a & 0x0FFF_FFFF;
        if addr == 0 {
            addr = s.offset;
        }
        let cond = if op >> 4 <= 0x6 {
            let chk = self.arm7_read_word(addr);
            match op >> 4 {
                0x3 => b > chk,
                0x4 => b < chk,
                0x5 => b == chk,
                _ => b != chk,
            }
        } else {
            let chk = !(b >> 16) as u16 & self.arm7_read_halfword(addr);
            let val = b as u16;
            match op >> 4 {
                0x7 => val > chk,
                0x8 => val < chk,
                0x9 => val == chk,
                _ => val != chk,
            }
        };
        s.push_cond(cond);
    }

    /// C0 loop, C4 (unsupported), C5 counter IF, C6 store offset.
    fn ar_control(&mut self, op: u8, b: u32, s: &mut ArState) -> Flow {
        match op {
            0xC0 => {
                s.loopstart = s.pc;
                s.loopcount = b;
                s.loopcond = s.cond;
                s.loopcondstack = s.condstack;
            }
            0xC4 => {
                #[cfg(feature = "log")]
                log::warn!("AR: opcode C4000000 is not supported");
                return Flow::Stop;
            }
            0xC5 => {
                s.c5count = s.c5count.wrapping_add(1);
                if s.cond {
                    let mask = b & 0xFFFF;
                    let chk = b >> 16;
                    s.push_cond(s.c5count & mask == chk);
                }
            }
            0xC6 => self.arm7_write_word(b, s.offset),
            _ => {
                #[cfg(feature = "log")]
                log::warn!("AR: bad opcode {op:02X}");
                return Flow::Stop;
            }
        }
        Flow::Continue
    }

    /// D0-D2 block ends, D3-DC offset and data register operations.
    fn ar_data(&mut self, op: u8, a: u32, b: u32, s: &mut ArState) {
        let at = b.wrapping_add(s.offset);
        match op {
            0xD0 => {
                s.cond = s.condstack & 1 != 0;
                s.condstack >>= 1;
            }
            0xD1 | 0xD2 => {
                if s.loopcount > 0 {
                    s.loopcount -= 1;
                    s.pc = s.loopstart;
                } else if op == 0xD1 {
                    s.cond = s.loopcond;
                    s.condstack = s.loopcondstack;
                } else {
                    s.offset = 0;
                    s.datareg = 0;
                    s.condstack = 0;
                    s.cond = true;
                }
            }
            0xD3 => s.offset = b,
            0xD4 => s.datareg = data_op(s.datareg, a, b),
            0xD5 => s.datareg = b,
            0xD6 => {
                self.arm7_write_word(at, s.datareg);
                s.offset = s.offset.wrapping_add(4);
            }
            0xD7 => {
                self.arm7_write_halfword(at, s.datareg as u16);
                s.offset = s.offset.wrapping_add(2);
            }
            0xD8 => {
                self.arm7_write_byte(at, s.datareg as u8);
                s.offset = s.offset.wrapping_add(1);
            }
            0xD9 => s.datareg = self.arm7_read_word(at),
            0xDA => s.datareg = u32::from(self.arm7_read_halfword(at)),
            0xDB => s.datareg = u32::from(self.arm7_read_byte(at)),
            0xDC => s.offset = s.offset.wrapping_add(b),
            _ => {
                #[cfg(feature = "log")]
                log::warn!("AR: bad opcode {a:08X} {b:08X}");
            }
        }
    }

    /// Ex: copies the `b` bytes that follow in the code to `a + offset`.
    fn ar_copy_in(&mut self, a: u32, b: u32, code: &[u32], s: &mut ArState) {
        let mut dst = (a & 0x0FFF_FFFF).wrapping_add(s.offset);
        let mut left = b;
        while left >= 8 {
            for _ in 0..2 {
                self.arm7_write_word(dst, ArState::word(code, s.pc));
                s.pc += 1;
                dst = dst.wrapping_add(4);
            }
            left -= 8;
        }
        if left > 0 {
            let bytes: Vec<u8> = (0..2)
                .flat_map(|i| ArState::word(code, s.pc + i).to_le_bytes())
                .collect();
            s.pc += 2;
            let mut i = 0;
            if left >= 4 {
                self.arm7_write_word(dst, ArState::word(code, s.pc - 2));
                dst = dst.wrapping_add(4);
                i = 4;
                left -= 4;
            }
            while left > 0 {
                self.arm7_write_byte(dst, bytes[i]);
                dst = dst.wrapping_add(1);
                i += 1;
                left -= 1;
            }
        }
    }

    /// Fx: copies `b` bytes from `offset` to `a`.
    fn ar_copy(&mut self, a: u32, b: u32, s: &ArState) {
        let mut src = s.offset;
        let mut dst = a & 0x0FFF_FFFF;
        let mut left = b;
        while left >= 4 {
            let v = self.arm7_read_word(src);
            self.arm7_write_word(dst, v);
            src = src.wrapping_add(4);
            dst = dst.wrapping_add(4);
            left -= 4;
        }
        while left > 0 {
            let v = self.arm7_read_byte(src);
            self.arm7_write_byte(dst, v);
            src = src.wrapping_add(1);
            dst = dst.wrapping_add(1);
            left -= 1;
        }
    }
}

/// D4: data register arithmetic (`a & 0xFF` selects the operation).
fn data_op(reg: u32, a: u32, b: u32) -> u32 {
    let sh = b & 0xFF;
    match a & 0xFF {
        0x00 => reg.wrapping_add(b),
        0x01 => reg | b,
        0x02 => reg & b,
        0x03 => reg ^ b,
        0x04 => reg.checked_shl(sh).unwrap_or(0),
        0x05 => reg.checked_shr(sh).unwrap_or(0),
        0x06 => reg.rotate_right(b & 0x1F),
        0x07 => ((reg as i32) >> sh.min(31)) as u32,
        0x08 => reg.wrapping_mul(b),
        _ => {
            #[cfg(feature = "log")]
            log::warn!("AR: bad D4 opcode {a:08X} {b:08X}");
            reg
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pairs() {
        assert_eq!(
            parse_ar_code("60D4BF48 00000000\n  b0d4bf48 00000000 "),
            Ok(vec![0x60D4_BF48, 0, 0xB0D4_BF48, 0])
        );
        assert!(parse_ar_code("123 00000000").is_err());
        assert!(parse_ar_code("XXXXXXXX 00000000").is_err());
        assert!(parse_ar_code("00000000").is_err());
        assert_eq!(parse_ar_code(""), Ok(vec![]));
    }

    #[test]
    fn data_ops() {
        assert_eq!(data_op(1, 0xD400_0004, 40), 0);
        assert_eq!(data_op(0x8000_0000, 0xD400_0007, 4), 0xF800_0000);
        assert_eq!(data_op(0x8000_0000, 0xD400_0007, 99), 0xFFFF_FFFF);
        assert_eq!(data_op(1, 0xD400_0006, 1), 0x8000_0000);
    }

    /// Main RAM through the ARM7 bus: writes, an IF that fails, a loop and
    /// a block copy.
    #[test]
    fn runs_codes_on_main_ram() {
        let mut emu = Emulator::new();
        let ram = 0x0200_0000;
        let code = parse_ar_code(
            "
            02000010 11223344
            12000014 0000BEEF
            52000010 00000000
            02000018 DEADDEAD
            D0000000 00000000
            D3000000 02000020
            C0000000 00000002
            D5000000 00000007
            D6000000 00000000
            D2000000 00000000
            E2000040 00000006
            04030201 00000605
            ",
        )
        .unwrap_or_default();
        emu.set_cheats(vec![code]);
        emu.run_cheats();
        assert_eq!(emu.arm7_read_word(ram + 0x10), 0x1122_3344);
        assert_eq!(emu.arm7_read_halfword(ram + 0x14), 0xBEEF);
        assert_eq!(emu.arm7_read_word(ram + 0x18), 0, "skipped by the IF");
        for i in 0..3 {
            assert_eq!(emu.arm7_read_word(ram + 0x20 + i * 4), 7, "loop {i}");
        }
        assert_eq!(emu.arm7_read_word(ram + 0x40), 0x0403_0201);
        assert_eq!(emu.arm7_read_halfword(ram + 0x44), 0x0605);
    }
}
