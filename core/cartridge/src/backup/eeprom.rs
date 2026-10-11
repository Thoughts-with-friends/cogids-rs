//! EEPROM / FRAM backup chips: tiny EEPROM (512 bytes, 1 address byte)
//! and EEPROM / FRAM (2 or 3 address bytes). FRAM speaks the same protocol.
use super::BackupMemory;

impl BackupMemory {
    /// 512-byte EEPROM: `cmd | 8` addresses the upper half.
    pub(super) fn eeprom_tiny(&mut self, value: u8) -> u8 {
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
    pub(super) fn eeprom(&mut self, value: u8) -> u8 {
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
}
