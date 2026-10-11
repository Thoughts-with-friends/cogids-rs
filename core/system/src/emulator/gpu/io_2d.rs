//! ARM9 access to the 2D engine registers (0x04000000 / 0x04001000 + 0x00..0x6F).
//!
//! Games write these registers with byte, halfword and word stores, and
//! several are wider than the access (DISPCNT, BGxX/BGxY, DISPCAPCNT are 32
//! bits). Every access is merged into a raw shadow copy first and the whole
//! register is then handed to the GPU, so partial writes never clobber the
//! other half (e.g. `STRH` to DISPCNT must keep the display mode in bits
//! 16-17; clearing it selects "display off", a white screen).
//!
//! Register layout follows GBATEK "DS Video" / melonDS `GPU2D::Write*`.
use crate::emulator::Emulator;

/// Size of one engine's register block in bytes.
pub const IO_2D_SIZE: usize = 0x70;

/// Whether `address` is a 2D engine register handled here.
///
/// DISPSTAT/VCOUNT (0x04000004..7) are shared registers and are excluded.
pub const fn is_2d_reg(address: u32) -> bool {
    let offset = address & 0xFFF;
    (address & !0xFFF == 0x0400_0000 || address & !0xFFF == 0x0400_1000)
        && offset < IO_2D_SIZE as u32
        && !(offset >= 4 && offset < 8)
}

/// Engine index (0 = A, 1 = B) and register offset of a 2D register address.
const fn split(address: u32) -> (usize, usize) {
    (((address >> 12) & 1) as usize, (address & 0xFFF) as usize)
}

impl Emulator {
    fn io_2d_u16(&self, engine: usize, offset: usize) -> u16 {
        let regs = &self.io_2d[engine];
        u16::from_le_bytes([regs[offset & !1], regs[(offset & !1) + 1]])
    }

    fn io_2d_u32(&self, engine: usize, offset: usize) -> u32 {
        let lo = u32::from(self.io_2d_u16(engine, offset & !3));
        let hi = u32::from(self.io_2d_u16(engine, (offset & !3) + 2));
        lo | (hi << 16)
    }

    /// Byte write to a 2D engine register.
    pub fn write_io_2d_u8(&mut self, address: u32, value: u8) {
        let (engine, offset) = split(address);
        self.sync_dispcapcnt(engine, offset);
        self.io_2d[engine][offset] = value;
        self.apply_io_2d(engine, offset & !1);
    }

    /// Halfword write to a 2D engine register.
    pub fn write_io_2d_u16(&mut self, address: u32, value: u16) {
        let (engine, offset) = split(address & !1);
        self.sync_dispcapcnt(engine, offset);
        self.io_2d[engine][offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        self.apply_io_2d(engine, offset);
    }

    /// Word write to a 2D engine register.
    pub fn write_io_2d_u32(&mut self, address: u32, value: u32) {
        let (engine, offset) = split(address & !3);
        self.sync_dispcapcnt(engine, offset);
        self.io_2d[engine][offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        // 32-bit registers are applied once with the whole value.
        self.apply_io_2d(engine, offset);
        if !matches!(offset, 0x00 | 0x28 | 0x2C | 0x38 | 0x3C | 0x64) {
            self.apply_io_2d(engine, offset + 2);
        }
    }

    /// DISPCAPCNT bit 31 is cleared by the hardware when a capture ends, so
    /// the shadow is refreshed from the GPU before a partial write merges
    /// into it.
    fn sync_dispcapcnt(&mut self, engine: usize, offset: usize) {
        if engine == 0 && (0x64..0x68).contains(&offset) {
            let current = self.gpu.get_dispcapcnt_a();
            self.io_2d[0][0x64..0x68].copy_from_slice(&current.to_le_bytes());
        }
    }

    /// Halfword read. Write-only registers read as 0 (GBATEK).
    pub fn read_io_2d_u16(&self, address: u32) -> u16 {
        let (engine, offset) = split(address & !1);
        match offset {
            // DISPCNT, BGxCNT, WININ/WINOUT, BLDCNT/BLDALPHA, DISPCAPCNT,
            // MASTER_BRIGHT are readable.
            0x00..=0x03 | 0x08..=0x0F | 0x48..=0x4B | 0x50..=0x53 | 0x6C..=0x6D => {
                self.io_2d_u16(engine, offset)
            }
            0x64..=0x67 if engine == 0 => {
                let word = self.gpu.get_dispcapcnt_a();
                if offset & 2 == 0 {
                    word as u16
                } else {
                    (word >> 16) as u16
                }
            }
            // DISP3DCNT carries hardware status bits.
            0x60 if engine == 0 => self.gpu.get_disp3dcnt(),
            _ => 0,
        }
    }

    /// Byte read.
    pub fn read_io_2d_u8(&self, address: u32) -> u8 {
        let half = self.read_io_2d_u16(address & !1);
        if address & 1 == 0 {
            half as u8
        } else {
            (half >> 8) as u8
        }
    }

    /// Word read.
    pub fn read_io_2d_u32(&self, address: u32) -> u32 {
        let address = address & !3;
        u32::from(self.read_io_2d_u16(address))
            | (u32::from(self.read_io_2d_u16(address + 2)) << 16)
    }

    /// Forwards the (complete) register containing halfword `offset` to the GPU.
    fn apply_io_2d(&mut self, engine: usize, offset: usize) {
        let half = self.io_2d_u16(engine, offset);
        // The full 32-bit register for DISPCNT, BGxX/BGxY and DISPCAPCNT.
        let word = self.io_2d_u32(engine, offset);
        let gpu = &mut self.gpu;
        let a = engine == 0;
        match offset {
            0x00 | 0x02 => {
                if a {
                    gpu.set_dispcnt_a(word);
                } else {
                    gpu.set_dispcnt_b(word);
                }
            }
            0x08..=0x0E => {
                let index = (offset - 0x08) / 2;
                if a {
                    gpu.set_bgcnt_a(half, index);
                } else {
                    gpu.set_bgcnt_b(half, index);
                }
            }
            0x10..=0x1E => {
                let index = (offset - 0x10) / 4;
                match (a, offset & 2 == 0) {
                    (true, true) => gpu.set_bghofs_a(half, index),
                    (true, false) => gpu.set_bgvofs_a(half, index),
                    (false, true) => gpu.set_bghofs_b(half, index),
                    (false, false) => gpu.set_bgvofs_b(half, index),
                }
            }
            0x20..=0x26 => {
                let index = (offset - 0x20) / 2;
                if a {
                    gpu.set_bg2p_a(half, index);
                } else {
                    gpu.set_bg2p_b(half, index);
                }
            }
            0x30..=0x36 => {
                let index = (offset - 0x30) / 2;
                if a {
                    gpu.set_bg3p_a(half, index);
                } else {
                    gpu.set_bg3p_b(half, index);
                }
            }
            // BG2X/BG2Y/BG3X/BG3Y: 28-bit reference points; any write reloads
            // the internal counter.
            0x28..=0x2E | 0x38..=0x3E => match (a, offset & !3) {
                (true, 0x28) => gpu.set_bg2x_a(word),
                (true, 0x2C) => gpu.set_bg2y_a(word),
                (true, 0x38) => gpu.set_bg3x_a(word),
                (true, _) => gpu.set_bg3y_a(word),
                (false, 0x28) => gpu.set_bg2x_b(word),
                (false, 0x2C) => gpu.set_bg2y_b(word),
                (false, 0x38) => gpu.set_bg3x_b(word),
                (false, _) => gpu.set_bg3y_b(word),
            },
            0x40 => {
                if a {
                    gpu.set_win0h_a(half);
                } else {
                    gpu.set_win0h_b(half);
                }
            }
            0x42 => {
                if a {
                    gpu.set_win1h_a(half);
                } else {
                    gpu.set_win1h_b(half);
                }
            }
            0x44 => {
                if a {
                    gpu.set_win0v_a(half);
                } else {
                    gpu.set_win0v_b(half);
                }
            }
            0x46 => {
                if a {
                    gpu.set_win1v_a(half);
                } else {
                    gpu.set_win1v_b(half);
                }
            }
            0x48 => {
                if a {
                    gpu.set_winin_a(half);
                } else {
                    gpu.set_winin_b(half);
                }
            }
            0x4A => {
                if a {
                    gpu.set_winout_a(half);
                } else {
                    gpu.set_winout_b(half);
                }
            }
            0x4C => {
                if a {
                    gpu.set_mosaic_a(half);
                } else {
                    gpu.set_mosaic_b(half);
                }
            }
            0x50 => {
                if a {
                    gpu.set_bldcnt_a(half);
                } else {
                    gpu.set_bldcnt_b(half);
                }
            }
            0x52 => {
                if a {
                    gpu.set_bldalpha_a(half);
                } else {
                    gpu.set_bldalpha_b(half);
                }
            }
            0x54 => {
                if a {
                    gpu.set_bldy_a(half as u8);
                } else {
                    gpu.set_bldy_b(half as u8);
                }
            }
            0x60 if a => gpu.set_disp3dcnt(half),
            0x64 | 0x66 if a => {
                gpu.set_dispcapcnt(word);
            }
            0x6C => {
                if a {
                    gpu.set_master_bright_a(half);
                } else {
                    gpu.set_master_bright_b(half);
                }
            }
            // Unused / not emulated (0x4E, 0x56..0x5E, DISP_MMEM_FIFO 0x68).
            _ => {}
        }
    }
}
