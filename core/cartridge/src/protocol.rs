//! Card protocol on the ROM bus: the 8-byte command buffer (ROMCMD),
//! ROMCTRL and the data port. Commands are KEY1-encrypted during the
//! secure area phase ([`crate::key1`]). See GBATEK "DS Cartridge Protocol".
use crate::NDSCart;

/// Cartridge command types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CartCommand {
    /// No command
    Empty = 0,
    /// Dummy/NOP command
    Dummy = 1,
    /// Get cartridge header
    GetHeader = 2,
    /// Get chip ID
    GetChipId = 3,
    /// Enable KEY1 encryption
    EnableKey1 = 4,
    /// Enable KEY2 encryption
    EnableKey2 = 5,
    /// Get secure area block
    GetSecureAreaBlock = 6,
    /// Read ROM data
    ReadRom = 7,
}

impl CartCommand {
    /// Convert numeric value to CartCommand
    pub const fn from_value(val: u32) -> Self {
        match val {
            1 => Self::Dummy,
            2 => Self::GetHeader,
            3 => Self::GetChipId,
            4 => Self::EnableKey1,
            5 => Self::EnableKey2,
            6 => Self::GetSecureAreaBlock,
            7 => Self::ReadRom,
            _ => Self::Empty,
        }
    }
}

/// ROM control register
#[derive(Debug, Clone, Copy)]
pub struct RegRomCtrl {
    /// KEY1 gap clocks (timing parameter)
    pub key1_gap: u32,
    /// KEY2 data enabled
    pub key2_data_enabled: bool,
    /// Apply KEY2 seed
    pub key2_apply_seed: bool,
    /// KEY2 gap setting
    pub key2_gap: u32,
    /// KEY2 command enabled
    pub key2_cmd_enabled: bool,
    /// Word ready signal
    pub word_ready: bool,
    /// Block size (transfer size)
    pub block_size: u32,
    /// Use slow transfer timing
    pub slow_transfer: bool,
    /// KEY1 gap uses clock cycles
    pub key1_gap_clocks: bool,
    /// Block transfer in progress
    pub block_busy: bool,
}

impl RegRomCtrl {
    /// Create new ROM control register
    pub const fn new() -> Self {
        Self {
            key1_gap: 0,
            key2_data_enabled: false,
            key2_apply_seed: false,
            key2_gap: 0,
            key2_cmd_enabled: false,
            word_ready: false,
            block_size: 0,
            slow_transfer: false,
            key1_gap_clocks: false,
            block_busy: false,
        }
    }

    /// Get register value as 32-bit word
    pub const fn get(&self) -> u32 {
        let mut value = 0;
        value |= self.key1_gap;
        if self.key2_data_enabled {
            value |= 1 << 13;
        }
        if self.key2_apply_seed {
            value |= 1 << 15;
        }

        value |= self.key2_gap << 16;
        if self.key2_cmd_enabled {
            value |= 1 << 22;
        }
        if self.word_ready {
            value |= 1 << 23;
        }
        value |= self.block_size << 24;
        if self.slow_transfer {
            value |= 1 << 27;
        }
        if self.key1_gap_clocks {
            value |= 1 << 28;
        }
        value |= 1 << 29; //Always one? Who knows

        if self.block_busy {
            value |= 1 << 31;
        }

        value
    }

    /// Set register value from 32-bit word
    pub const fn set(&mut self, value: u32) {
        self.key1_gap = value & 0x3F;
        self.key2_data_enabled = (value & (1 << 6)) != 0;
        self.key2_apply_seed = (value & (1 << 7)) != 0;
        self.key2_gap = (value >> 8) & 0x3F;
        self.key2_cmd_enabled = (value & (1 << 14)) != 0;
        self.word_ready = (value & (1 << 15)) != 0;
        self.block_size = (value >> 16) & 0x7;
        self.slow_transfer = (value & (1 << 19)) != 0;
        self.key1_gap_clocks = (value & (1 << 20)) != 0;
        self.block_busy = (value & (1 << 31)) != 0;
    }
}

impl Default for RegRomCtrl {
    fn default() -> Self {
        Self::new()
    }
}

/// Game Cartridge Controller
/// Manages ROM data, encryption/decryption, and cartridge interface
impl NDSCart {
    /// Reads a command byte from command buffer.
    pub fn read_command(&self, index: usize) -> u8 {
        self.command_buffer.get(index).copied().unwrap_or(0)
    }

    /// Writes a command byte into command buffer.
    pub fn receive_command(&mut self, command: u8, index: usize) {
        if let Some(slot) = self.command_buffer.get_mut(index) {
            *slot = command;
        }
    }

    // Moved functions to struct Emulator.
    // pub fn run(&mut self, cycles: i32);

    /// Runs debug encryption test routine.
    pub fn debug_encrypt(&mut self) {
        todo!()
    }

    /// Reads raw byte from ROM without timing effects.
    pub fn direct_read(&self, address: u32) -> u8 {
        self.rom.get(address as usize).copied().unwrap_or_else(|| {
            #[cfg(feature = "log")]
            log::warn!("Direct read out of bounds: {address:08X}");
            0
        })
    }

    /// Reads little-endian halfword from ROM.
    pub fn direct_read_halfword(&self, address: u32) -> u16 {
        let lo = self.direct_read(address) as u16;
        let hi = self.direct_read(address + 1) as u16;
        lo | (hi << 8)
    }

    /// Reads little-endian word from ROM.
    pub fn direct_read_word(&self, address: u32) -> u32 {
        let lo = self.direct_read_halfword(address) as u32;
        let hi = self.direct_read_halfword(address + 2) as u32;
        lo | (hi << 16)
    }

    /// Returns ROM CTRL register value.
    pub const fn get_romctrl(&self) -> u32 {
        self.romctrl.get()
    }

    /// Returns cartridge output register value.
    pub const fn get_output(&mut self) -> u32 {
        if self.romctrl.word_ready {
            self.romctrl.word_ready = false;
            self.cycles_left = 8;
        }

        self.data_output
    }

    /// Updates ROMCTRL register and may trigger a new cartridge transfer.
    ///
    /// # Panics
    pub fn set_romctrl(&mut self, value: u32) {
        // #[cfg(feature = "log")]
        // log::debug!("Setting ROMCTRL: {value:08X}");

        let old_transfer_busy = self.romctrl.block_busy;

        self.romctrl.key1_gap = value & 0x1FFF;
        self.romctrl.key2_data_enabled = ((value >> 13) & 1) != 0;
        self.romctrl.key2_apply_seed = ((value >> 15) & 1) != 0;
        self.romctrl.key2_gap = (value >> 16) & 0x3F;
        self.romctrl.key2_cmd_enabled = ((value >> 22) & 1) != 0;
        self.romctrl.block_size = (value >> 24) & 0x7;
        self.romctrl.slow_transfer = ((value >> 27) & 1) != 0;
        self.romctrl.key1_gap_clocks = ((value >> 28) & 1) != 0;
        self.romctrl.block_busy = ((value >> 31) & 1) != 0;

        if !old_transfer_busy && self.romctrl.block_busy && self.auxspicnt & (1 << 15) != 0 {
            self.romctrl.word_ready = false;

            self.bytes_left = match self.romctrl.block_size {
                0 => 0,
                7 => 4,
                n => 0x100 << n,
            };

            if self.cmd_encrypt_mode != 0 {
                self.cycles_left += self.romctrl.key1_gap as i32;
            }

            if self.cmd_encrypt_mode == 1 {
                self.cycles_left += self.romctrl.key1_gap as i32;

                let mut data = [0_u8; 8];
                for (data, buffer) in data.iter_mut().zip(self.command_buffer) {
                    *data = buffer;
                }

                // To [u32; 2]
                #[expect(clippy::unwrap_used)]
                let mut data = [
                    u32::from_le_bytes(data[0..4].try_into().unwrap()),
                    u32::from_le_bytes(data[4..8].try_into().unwrap()),
                ];
                self.key1_decrypt(&mut data);

                // #[cfg(feature = "log")]
                // log::trace!(
                //     "Data sent {:02X?} decrypted {data:02X?}",
                //     self.command_buffer,
                // );

                // to bytes
                let mut out = [0_u8; 8];
                out[0..4].copy_from_slice(&data[0].to_le_bytes());
                out[4..8].copy_from_slice(&data[1].to_le_bytes());

                for (buffer, data) in self.command_buffer.iter_mut().rev().zip(out) {
                    *buffer = data;
                }
            }

            self.command_id = CartCommand::Empty;

            match self.command_buffer[0] {
                0x9F => self.command_id = CartCommand::Dummy,
                0x00 => {
                    self.command_id = CartCommand::GetHeader;
                    self.rom_data_index = 0;
                }
                0x90 | 0xB8 => self.command_id = CartCommand::GetChipId,
                0x3C => self.command_id = CartCommand::EnableKey1,
                0xB7 => {
                    self.command_id = CartCommand::ReadRom;

                    self.rom_data_index = (((self.command_buffer[1] as u32) << 24)
                        | ((self.command_buffer[2] as u32) << 16)
                        | ((self.command_buffer[3] as u32) << 8)
                        | (self.command_buffer[4] as u32))
                        as usize;

                    if self.bytes_left > 0x1000 {
                        #[cfg(feature = "log")]
                        log::error!("ROM read bytes_left > 0x1000");
                    }
                }

                _ => match self.command_buffer[0] & 0xF0 {
                    0x40 => self.command_id = CartCommand::Dummy,
                    0x10 => self.command_id = CartCommand::GetChipId,
                    0x20 => {
                        self.command_id = CartCommand::GetSecureAreaBlock;
                        self.secure_area_index = ((self.command_buffer[2] & 0xF0) as u32) << 8;
                    }
                    0xA0 => {
                        self.command_id = CartCommand::Dummy;
                        self.cmd_encrypt_mode = 2;
                    }
                    _ => {}
                },
            }
        }
    }
}
