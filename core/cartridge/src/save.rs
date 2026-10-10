//! Cartridge backup memory (save chip) on the AUXSPI bus.
//!
//! Emulates the three serial save chips found in retail DS cartridges, following
//! GBATEK "DS Cartridge Backup" and melonDS `NDSCart/CartRetail.cpp`:
//!
//! - **EEPROM tiny** (512 bytes): 1 address byte, bit 3 of the command selects
//!   the upper 256 bytes.
//! - **EEPROM / FRAM** (8 KiB, 64 KiB: 2 address bytes; 128 KiB: 3 bytes).
//! - **FLASH** (256 KiB .. 1 MiB): 3 address bytes, page/sector erase.
//!
//! The chip contents are kept in memory and written to the `.sav` file by
//! [`BackupMemory::flush`] (see [`BackupMemory::end_frame`] for the debounce).
use std::path::{Path, PathBuf};

/// Save memory types as used by the melonDS game database
/// ([`crate::rom_list::ROM_LIST`]).
///
/// | code | chip         | size    |
/// |------|--------------|---------|
/// | 0    | none         | -       |
/// | 1    | EEPROM tiny  | 512 B   |
/// | 2..4 | EEPROM       | 8K/64K/128K |
/// | 5..7 | FLASH        | 256K/512K/1M |
/// | 8..10| NAND         | 8M/16M/64M (unsupported, treated as none) |
pub type SaveMemType = u8;

/// Chip size in bytes for each [`SaveMemType`] code.
const SAVE_SIZES: [usize; 11] = [
    0,
    512,
    8 * 1024,
    64 * 1024,
    128 * 1024,
    256 * 1024,
    512 * 1024,
    1024 * 1024,
    8 * 1024 * 1024,
    16 * 1024 * 1024,
    64 * 1024 * 1024,
];

/// Serial protocol spoken by the save chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveChip {
    /// No backup memory: the bus floats.
    None,
    /// 512-byte EEPROM (address bit 8 is carried in the command byte).
    EepromTiny,
    /// EEPROM or FRAM with 2 (<= 64 KiB) or 3 (128 KiB) address bytes.
    Eeprom,
    /// SPI FLASH with 3 address bytes.
    Flash,
}

/// Status register bit 1: write enable latch.
const STATUS_WEL: u8 = 1 << 1;

/// Number of frames without save writes after which the file is flushed.
const FLUSH_DELAY_FRAMES: u32 = 30;

/// Backup memory chip plus the `.sav` file it is persisted to.
#[derive(Debug)]
pub struct BackupMemory {
    chip: SaveChip,
    /// Chip contents (`len()` is the chip size).
    data: Vec<u8>,
    /// File the contents are loaded from and flushed to.
    path: Option<PathBuf>,

    /// Byte index within the current chip-select cycle (0 = command byte).
    pos: u32,
    cmd: u8,
    addr: u32,
    status: u8,

    /// Infrared transceiver in front of the chip (Pokémon HG/SS, B/W, B2/W2
    /// and pedometer carts). Every chip select starts with an IR command
    /// byte; command 0x00 passes the remaining bytes to the save chip
    /// (melonDS `CartRetailIR`).
    ir: bool,
    ir_pos: u32,
    ir_cmd: u8,

    /// Contents changed since the last flush.
    dirty: bool,
    /// Frames elapsed since the last write while `dirty`.
    idle_frames: u32,
}

impl Default for BackupMemory {
    fn default() -> Self {
        Self::new(0)
    }
}

impl BackupMemory {
    /// Creates blank (all `0xFF`) backup memory of the given database type.
    pub fn new(mem_type: SaveMemType) -> Self {
        let mem_type = if mem_type as usize >= SAVE_SIZES.len() {
            0
        } else {
            mem_type
        };
        let chip = match mem_type {
            1 => SaveChip::EepromTiny,
            2..=4 => SaveChip::Eeprom,
            5..=7 => SaveChip::Flash,
            // NAND carts (WarioWare DIY, Jam with the Band) use the ROM bus,
            // not AUXSPI; they are not emulated.
            _ => SaveChip::None,
        };
        let size = if chip == SaveChip::None {
            0
        } else {
            SAVE_SIZES[mem_type as usize]
        };

        Self {
            chip,
            data: vec![0xFF; size],
            path: None,
            pos: 0,
            cmd: 0,
            addr: 0,
            status: 0,
            ir: false,
            ir_pos: 0,
            ir_cmd: 0,
            dirty: false,
            idle_frames: 0,
        }
    }

    /// Puts an infrared transceiver in front of the save chip.
    pub const fn set_infrared(&mut self, ir: bool) {
        self.ir = ir;
    }

    /// Maps a save file length to a database type, if it is a valid chip size.
    pub fn type_from_size(len: usize) -> Option<SaveMemType> {
        (1..=7)
            .find(|&t| SAVE_SIZES[t] == len)
            .map(|t| t as SaveMemType)
    }

    /// Chip protocol.
    pub const fn chip(&self) -> SaveChip {
        self.chip
    }

    /// Chip size in bytes.
    pub const fn size(&self) -> usize {
        self.data.len()
    }

    /// Chip contents.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// `.sav` path the contents are persisted to.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Sets the `.sav` path used by [`Self::flush`].
    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }

    /// Copies existing save data in. A shorter image leaves the rest blank
    /// (`0xFF`), a longer one is truncated, like melonDS does.
    pub fn load_contents(&mut self, contents: &[u8]) {
        let len = contents.len().min(self.data.len());
        self.data[..len].copy_from_slice(&contents[..len]);
    }

    /// Whether there are unwritten changes.
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Writes the contents to the `.sav` file if they changed.
    ///
    /// The file is written to a temporary sibling first and then renamed, so
    /// an interrupted write never leaves a truncated save behind.
    ///
    /// # Errors
    /// Returns the I/O error if the file could not be written.
    pub fn flush(&mut self) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let Some(path) = &self.path else {
            return Ok(());
        };

        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp = path.clone().into_os_string();
        tmp.push(".tmp");
        std::fs::write(&tmp, &self.data)?;
        std::fs::rename(&tmp, path)?;

        self.dirty = false;
        self.idle_frames = 0;
        Ok(())
    }

    /// Called once per emulated frame: flushes the file once the game has
    /// stopped writing for [`FLUSH_DELAY_FRAMES`] frames, so a multi-page save
    /// is written once instead of once per page.
    ///
    /// # Errors
    /// See [`Self::flush`].
    pub fn end_frame(&mut self) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        self.idle_frames += 1;
        if self.idle_frames >= FLUSH_DELAY_FRAMES {
            self.flush()?;
        }
        Ok(())
    }

    /// Resets the serial state machine (power on).
    pub const fn reset(&mut self) {
        self.ir_pos = 0;
        self.ir_cmd = 0;
        self.pos = 0;
        self.cmd = 0;
        self.addr = 0;
        self.status = 0;
    }

    /// Chip select asserted: the next byte is a command.
    pub const fn select(&mut self) {
        self.pos = 0;
        self.ir_pos = 0;
    }

    /// Chip select released: a finished write clears the write enable latch.
    pub const fn release(&mut self) {
        if self.pos > 0 && is_write_command(self.chip, self.cmd) {
            self.status &= !STATUS_WEL;
        }
        self.pos = 0;
    }

    /// Exchanges one byte with the chip (through the IR transceiver, if
    /// present) and returns its reply.
    pub fn transfer(&mut self, value: u8) -> u8 {
        if !self.ir {
            return self.transfer_chip(value);
        }

        let pos = self.ir_pos;
        self.ir_pos = self.ir_pos.saturating_add(1);
        if pos == 0 {
            self.ir_cmd = value;
            return 0;
        }
        match self.ir_cmd {
            // Pass-through to the save chip.
            0x00 => self.transfer_chip(value),
            // IR chip ID.
            0x08 => 0xAA,
            // Actual infrared communication is not emulated.
            _ => 0,
        }
    }

    fn transfer_chip(&mut self, value: u8) -> u8 {
        if self.chip == SaveChip::None {
            return 0xFF;
        }

        let reply = if self.pos == 0 {
            match value {
                // Write disable / write enable.
                0x04 => self.status &= !STATUS_WEL,
                0x06 => self.status |= STATUS_WEL,
                _ => {}
            }
            self.cmd = value;
            self.addr = 0;
            0xFF
        } else {
            match self.chip {
                SaveChip::EepromTiny => self.eeprom_tiny(value),
                SaveChip::Eeprom => self.eeprom(value),
                SaveChip::Flash => self.flash(value),
                SaveChip::None => 0xFF,
            }
        };

        self.pos = self.pos.saturating_add(1);
        reply
    }

    const fn write_enabled(&self) -> bool {
        self.status & STATUS_WEL != 0
    }

    const fn mark_written(&mut self) {
        self.dirty = true;
        self.idle_frames = 0;
    }

    /// 512-byte EEPROM: `cmd | 8` addresses the upper half.
    fn eeprom_tiny(&mut self, value: u8) -> u8 {
        let high = if self.cmd & 0x08 != 0 { 0x100 } else { 0 };
        match self.cmd {
            // Write status register (only the block protect bits).
            0x01 => {
                if self.pos == 1 {
                    self.status = (self.status & 0x01) | (value & 0x0C);
                }
                0xFF
            }
            // Read status register (unused bits read as 1).
            0x05 => self.status | 0xF0,
            // Write low / high.
            0x02 | 0x0A => {
                if self.pos < 2 {
                    self.addr = value as u32;
                } else {
                    if self.write_enabled() {
                        self.data[((self.addr + high) & 0x1FF) as usize] = value;
                        self.mark_written();
                    }
                    self.addr += 1;
                }
                0xFF
            }
            // Read low / high.
            0x03 | 0x0B => {
                if self.pos < 2 {
                    self.addr = value as u32;
                    0xFF
                } else {
                    let ret = self.data[((self.addr + high) & 0x1FF) as usize];
                    self.addr += 1;
                    ret
                }
            }
            _ => 0xFF,
        }
    }

    /// 8 KiB .. 128 KiB EEPROM / FRAM.
    fn eeprom(&mut self, value: u8) -> u8 {
        let addr_bytes = if self.data.len() > 0x1_0000 { 3 } else { 2 };
        let mask = self.data.len() as u32 - 1;
        match self.cmd {
            0x01 => {
                if self.pos == 1 {
                    self.status = (self.status & 0x01) | (value & 0x0C);
                }
                0xFF
            }
            0x05 => self.status,
            0x02 => {
                if self.pos <= addr_bytes {
                    self.addr = (self.addr << 8) | value as u32;
                } else {
                    if self.write_enabled() {
                        self.data[(self.addr & mask) as usize] = value;
                        self.mark_written();
                    }
                    self.addr += 1;
                }
                0xFF
            }
            0x03 => {
                if self.pos <= addr_bytes {
                    self.addr = (self.addr << 8) | value as u32;
                    0xFF
                } else {
                    let ret = self.data[(self.addr & mask) as usize];
                    self.addr += 1;
                    ret
                }
            }
            _ => 0xFF,
        }
    }

    /// SPI FLASH (ST M25PE / Sanyo LE25FW compatible command set).
    fn flash(&mut self, value: u8) -> u8 {
        let mask = self.data.len() as u32 - 1;
        match self.cmd {
            0x05 => self.status,
            // Read JEDEC ID: ST Micro, then capacity (0x12 = 256K .. 0x14 = 1M).
            0x9F => match self.pos {
                1 => 0x20,
                2 => 0x40,
                3 => 0x11 + (self.data.len() >> 18).trailing_zeros() as u8 + 1,
                _ => 0xFF,
            },
            // Read / fast read (one dummy byte after the address).
            0x03 | 0x0B => {
                let dummy = u32::from(self.cmd == 0x0B);
                if self.pos <= 3 {
                    self.addr = (self.addr << 8) | value as u32;
                    0xFF
                } else if self.pos <= 3 + dummy {
                    0xFF
                } else {
                    let ret = self.data[(self.addr & mask) as usize];
                    self.addr = self.addr.wrapping_add(1);
                    ret
                }
            }
            // Page write (erase + program) / page program (bits 1 -> 0 only).
            0x0A | 0x02 => {
                if self.pos <= 3 {
                    self.addr = (self.addr << 8) | value as u32;
                } else if self.write_enabled() {
                    let index = (self.addr & mask) as usize;
                    if self.cmd == 0x0A {
                        self.data[index] = value;
                    } else {
                        self.data[index] &= value;
                    }
                    self.mark_written();
                    // The address wraps within the 256-byte page.
                    self.addr = (self.addr & !0xFF) | (self.addr.wrapping_add(1) & 0xFF);
                }
                0xFF
            }
            // Page erase (256 B) / sector erase (64 KiB).
            0xDB | 0xD8 => {
                if self.pos <= 3 {
                    self.addr = (self.addr << 8) | value as u32;
                }
                if self.pos == 3 && self.write_enabled() {
                    let len: u32 = if self.cmd == 0xDB { 0x100 } else { 0x1_0000 };
                    let start = (self.addr & mask & !(len - 1)) as usize;
                    let end = (start + len as usize).min(self.data.len());
                    self.data[start..end].fill(0xFF);
                    self.mark_written();
                }
                0xFF
            }
            // Chip erase.
            0xC7 => {
                if self.pos == 1 && self.write_enabled() {
                    self.data.fill(0xFF);
                    self.mark_written();
                }
                0xFF
            }
            _ => 0xFF,
        }
    }
}

/// Commands after which the chip clears its write enable latch.
const fn is_write_command(chip: SaveChip, cmd: u8) -> bool {
    match chip {
        SaveChip::EepromTiny => matches!(cmd, 0x01 | 0x02 | 0x0A),
        SaveChip::Eeprom => matches!(cmd, 0x01 | 0x02),
        SaveChip::Flash => matches!(cmd, 0x02 | 0x0A | 0xDB | 0xD8 | 0xC7),
        SaveChip::None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cycle(mem: &mut BackupMemory, bytes: &[u8]) -> Vec<u8> {
        mem.select();
        let out = bytes.iter().map(|&b| mem.transfer(b)).collect();
        mem.release();
        out
    }

    #[test]
    fn eeprom_write_then_read() {
        let mut mem = BackupMemory::new(3); // 64 KiB
        cycle(&mut mem, &[0x06]);
        cycle(&mut mem, &[0x02, 0x12, 0x34, 0xAA, 0xBB]);
        assert!(mem.is_dirty());
        let out = cycle(&mut mem, &[0x03, 0x12, 0x34, 0, 0]);
        assert_eq!(&out[3..], &[0xAA, 0xBB]);
        // The write enable latch is cleared after the write.
        assert_eq!(cycle(&mut mem, &[0x05, 0])[1] & STATUS_WEL, 0);
    }

    #[test]
    fn eeprom_tiny_high_half() {
        let mut mem = BackupMemory::new(1);
        cycle(&mut mem, &[0x06]);
        cycle(&mut mem, &[0x0A, 0x10, 0x5A]);
        assert_eq!(mem.data()[0x110], 0x5A);
        assert_eq!(cycle(&mut mem, &[0x0B, 0x10, 0])[2], 0x5A);
    }

    #[test]
    fn flash_erase_program_read() {
        let mut mem = BackupMemory::new(6); // 512 KiB
        assert_eq!(cycle(&mut mem, &[0x9F, 0, 0, 0])[1..], [0x20, 0x40, 0x13]);
        cycle(&mut mem, &[0x06]);
        cycle(&mut mem, &[0x0A, 0x01, 0x00, 0xFF, 0x11, 0x22]);
        // Wrapped within the page.
        assert_eq!(mem.data()[0x100FF], 0x11);
        assert_eq!(mem.data()[0x10000], 0x22);
        cycle(&mut mem, &[0x06]);
        cycle(&mut mem, &[0xDB, 0x01, 0x00, 0x00]);
        assert!(mem.data()[0x10000..0x10100].iter().all(|&b| b == 0xFF));
        cycle(&mut mem, &[0x06]);
        cycle(&mut mem, &[0x02, 0x01, 0x00, 0x00, 0x0F]);
        let out = cycle(&mut mem, &[0x0B, 0x01, 0x00, 0x00, 0, 0]);
        assert_eq!(out[5], 0x0F);
    }

    #[test]
    fn infrared_pass_through() {
        let mut mem = BackupMemory::new(6);
        mem.set_infrared(true);
        cycle(&mut mem, &[0x00, 0x06]);
        let out = cycle(&mut mem, &[0x00, 0x05, 0]);
        assert_eq!(out[2] & STATUS_WEL, STATUS_WEL);
        assert_eq!(cycle(&mut mem, &[0x08, 0])[1], 0xAA);
    }

    #[test]
    fn writes_need_write_enable() {
        let mut mem = BackupMemory::new(2);
        cycle(&mut mem, &[0x02, 0x00, 0x00, 0x55]);
        assert_eq!(mem.data()[0], 0xFF);
        assert!(!mem.is_dirty());
    }
}
