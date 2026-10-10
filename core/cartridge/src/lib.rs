//! Game Cartridge controller for Nintendo DS
//! Handles ROM loading, encryption/decryption, and cartridge access

use std::path::{Path, PathBuf};

pub mod rom_list;
pub mod save;

pub use save::{BackupMemory, SaveChip, SaveMemType};

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
#[derive(Debug)]
pub struct NDSCart {
    /// KEY1 encryption buffer (0x1048 bytes)
    pub key1_buffer: Vec<u8>,
    /// Encryption mode (0=normal, 1=encrypted)
    pub cmd_encrypt_mode: u32,

    /// ROM data
    pub rom: Vec<u8>,
    /// Save database
    pub save_database: Vec<u8>,
    /// ROM filename
    pub rom_name: String,
    /// Backup memory (save chip) behind AUXSPI
    pub backup: BackupMemory,
    /// Database size
    pub database_size: u64,
    /// ROM size
    pub rom_size: u64,

    /// Command buffer
    pub command_buffer: [u8; 8],
    /// Data output from cartridge
    pub data_output: u32,
    /// Current index in ROM data
    pub rom_data_index: usize,
    /// Current command being executed
    pub command_id: CartCommand,

    /// Secure area block index
    pub secure_area_index: u32,

    /// Cycles remaining for current operation
    pub cycles_left: i32,
    /// Bytes remaining to transfer
    pub bytes_left: i32,

    /// ROM control register
    pub romctrl: RegRomCtrl,
    /// AUXSPICNT (0x040001A0): bits 0-1 baud rate, 6 chip select hold,
    /// 7 busy, 13 SPI mode, 14 transfer IRQ, 15 slot enable
    pub auxspicnt: u16,
    /// Last byte received on AUXSPIDATA (0x040001A2)
    spi_data: u8,
    /// Save chip select line is held low between bytes
    spi_selected: bool,

    /// Encryption seed 0
    encrypt_seed0: u64,
    /// Encryption seed 1
    encrypt_seed1: u64,

    /// Keycode for encryption (3 words)
    pub keycode: [u32; 3],
}

impl Default for NDSCart {
    fn default() -> Self {
        Self::new()
    }
}

impl NDSCart {
    /// Create new cartridge controller
    pub fn new() -> Self {
        Self {
            key1_buffer: vec![0_u8; 0x1048],
            cmd_encrypt_mode: 0,
            rom: Vec::new(),
            save_database: Vec::new(),
            rom_name: String::new(),
            backup: BackupMemory::default(),
            database_size: 0,
            rom_size: 0,
            command_buffer: [0_u8; 8],
            data_output: 0,
            rom_data_index: 0,
            command_id: CartCommand::Empty,
            secure_area_index: 0,
            cycles_left: 0,
            bytes_left: 0,
            romctrl: RegRomCtrl::new(),
            auxspicnt: 0,
            spi_data: 0,
            spi_selected: false,
            encrypt_seed0: 0,
            encrypt_seed1: 0,
            keycode: [0_u32; 3],
        }
    }

    /// Resets cartridge state to power-on defaults.
    ///
    /// The loaded ROM and the backup memory contents are kept.
    pub const fn power_on(&mut self) {
        self.cycles_left = 8;
        self.bytes_left = 0;
        self.romctrl.word_ready = true;
        self.romctrl.block_busy = false;
        self.cmd_encrypt_mode = 0;

        self.auxspicnt = 0;
        self.spi_data = 0;
        self.spi_selected = false;
        self.backup.reset();
    }

    /// Loads save database file into memory.
    ///
    /// # Errors
    pub fn load_database(&mut self, file_name: &Path) -> Result<(), CartridgeError> {
        use std::fs::File;
        use std::io::Read;

        self.save_database.clear();
        self.database_size = 0;

        // #[cfg(feature = "tracing")]
        // tracing::info!("Loading save database: {}", file_name.display());

        let mut file = File::open(file_name).map_err(|source| {
            #[cfg(feature = "tracing")]
            tracing::error!("Failed to open save database: {}", file_name.display());

            CartridgeError::OpenSaveDatabase {
                path: file_name.to_path_buf(),
                source,
            }
        })?;

        let metadata = std::fs::metadata(file_name).map_err(|source| {
            #[cfg(feature = "tracing")]
            tracing::error!("Failed to read metadata: {}", file_name.display());

            CartridgeError::MetadataSaveDatabase {
                path: file_name.to_path_buf(),
                source,
            }
        })?;

        let size = metadata.len();
        self.database_size = size;

        if size % 19 != 0 {
            #[cfg(feature = "tracing")]
            tracing::error!(
                "Invalid save database format: {} size={size}",
                file_name.display(),
            );

            return Err(CartridgeError::InvalidSaveDatabaseFormat {
                path: file_name.to_path_buf(),
                size,
            });
        }

        self.save_database.resize(size as usize, 0);

        file.read_exact(&mut self.save_database).map_err(|source| {
            #[cfg(feature = "tracing")]
            tracing::error!(
                "Failed to read save database contents: {}",
                file_name.display()
            );

            CartridgeError::ReadSaveDatabase {
                path: file_name.to_path_buf(),
                source,
            }
        })?;

        // #[cfg(feature = "tracing")]
        // tracing::info!("Save database successfully loaded: {}", file_name.display());

        Ok(())
    }

    // Move to struct Emulator
    // Loads ROM image and optional save data.
    // pub fn load_rom( &mut self, file_name: &Path, is_direct_boot_enabled: bool,) -> Result<(), CartridgeError>;

    /// Writes save data to disk if modified.
    ///
    /// # Errors
    ///
    /// Called once per frame; the file is only written after the game has
    /// stopped writing for a short while (see [`BackupMemory::end_frame`]).
    pub fn save_check(&mut self) -> Result<(), CartridgeError> {
        self.backup
            .end_frame()
            .map_err(|source| self.save_error(source))
    }

    /// Writes pending save data to disk immediately (e.g. on exit).
    ///
    /// # Errors
    pub fn flush_save(&mut self) -> Result<(), CartridgeError> {
        self.backup
            .flush()
            .map_err(|source| self.save_error(source))
    }

    fn save_error(&self, source: std::io::Error) -> CartridgeError {
        CartridgeError::SaveWrite {
            path: self
                .backup
                .path()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
            source,
        }
    }

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
            #[cfg(feature = "tracing")]
            tracing::warn!("Direct read out of bounds: {address:08X}");
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

    /// Returns AUXSPICNT register value.
    ///
    /// Transfers complete instantly, so the busy bit (7) never reads as set.
    pub const fn get_auxspicnt(&self) -> u16 {
        self.auxspicnt & !(1 << 7)
    }

    /// Reads AUXSPIDATA: the byte the save chip sent back during the last
    /// transfer (0 while the slot or SPI mode is disabled).
    pub const fn read_auxspidata(&self) -> u8 {
        if self.auxspicnt & 0xA000 != 0xA000 {
            return 0;
        }
        self.spi_data
    }

    /// Writes AUXSPICNT bits selected by `mask`.
    ///
    /// Clearing bit 13 (SPI mode) while the chip is selected releases the
    /// chip select line, setting it again re-asserts it (melonDS
    /// `NDSCartSlot::Interface::WriteSPICnt`).
    pub fn write_auxspicnt(&mut self, value: u16, mask: u16) {
        #[cfg(feature = "tracing")]
        tracing::trace!(target: "auxspi", "cnt {:04X} -> {value:04X}/{mask:04X} sel={}", self.auxspicnt, self.spi_selected);
        let value = value & mask;
        if self.spi_selected {
            if self.auxspicnt & !value & (1 << 13) != 0 {
                self.backup.release();
                self.spi_selected = false;
            } else if !self.auxspicnt & value & (1 << 13) != 0 {
                self.backup.select();
            }
        }
        self.auxspicnt = (self.auxspicnt & !mask) | (value & 0xE043);
    }

    /// Sets AUXSPICNT register value.
    pub fn set_auxspicnt(&mut self, value: u16) {
        self.write_auxspicnt(value, 0xFFFF);
    }

    /// Sets low byte of AUXSPICNT register.
    pub fn set_lo_auxspicnt(&mut self, value: u8) {
        self.write_auxspicnt(value as u16, 0x00FF);
    }

    /// Sets high byte of AUXSPICNT register.
    pub fn set_hi_auxspicnt(&mut self, value: u8) {
        self.write_auxspicnt((value as u16) << 8, 0xFF00);
    }

    /// Writes AUXSPIDATA: exchanges one byte with the save chip.
    ///
    /// The chip is selected by the first byte and released after any byte
    /// written while AUXSPICNT bit 6 (chip select hold) is clear.
    pub fn set_auxspidata(&mut self, value: u8) {
        // Slot enabled and in SPI (backup) mode.
        if self.auxspicnt & 0xA000 != 0xA000 {
            return;
        }

        if !self.spi_selected {
            self.backup.select();
        }
        self.spi_data = self.backup.transfer(value);
        #[cfg(feature = "tracing")]
        tracing::trace!(target: "auxspi", "sel={} in={value:02X} out={:02X} hold={}", self.spi_selected, self.spi_data, self.auxspicnt & (1 << 6) != 0);

        let hold = self.auxspicnt & (1 << 6) != 0;
        if !hold {
            self.backup.release();
        }
        self.spi_selected = hold;
    }

    /// Whether a ROM transfer should raise the cartridge IRQ (AUXSPICNT bit 14).
    pub const fn transfer_irq_enabled(&self) -> bool {
        self.auxspicnt & (1 << 14) != 0
    }

    /// Updates ROMCTRL register and may trigger a new cartridge transfer.
    ///
    /// # Panics
    pub fn set_romctrl(&mut self, value: u32) {
        // #[cfg(feature = "tracing")]
        // tracing::debug!("Setting ROMCTRL: {value:08X}");

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

                // #[cfg(feature = "tracing")]
                // tracing::trace!(
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
                        #[cfg(feature = "tracing")]
                        tracing::error!("ROM read bytes_left > 0x1000");
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

    /// Sets low 32 bits of KEY2 seed 0.
    pub const fn set_lo_key2_seed0(&mut self, word: u32) {
        self.encrypt_seed0 >>= 32;
        self.encrypt_seed0 <<= 32;
        self.encrypt_seed0 |= word as u64;
    }

    /// Sets low 32 bits of KEY2 seed 1.
    pub const fn set_lo_key2_seed1(&mut self, word: u32) {
        self.encrypt_seed1 >>= 32;
        self.encrypt_seed1 <<= 32;
        self.encrypt_seed1 |= word as u64;
    }

    /// Sets high 32 bits of KEY2 seed 0.
    pub const fn set_hi_key2_seed0(&mut self, word: u32) {
        let word = word & 0x7F;
        self.encrypt_seed0 <<= 32;
        self.encrypt_seed0 >>= 32;
        self.encrypt_seed0 |= (word as u64) << 32;
    }

    /// Sets high 32 bits of KEY2 seed 1.
    pub const fn set_hi_key2_seed1(&mut self, word: u32) {
        let word = word & 0x7F;
        self.encrypt_seed1 <<= 32;
        self.encrypt_seed1 >>= 32;
        self.encrypt_seed1 |= (word as u64) << 32;
    }

    #[inline]
    fn read_keybuf_u32(&self, index: usize) -> u32 {
        let i = index * 4;
        u32::from_le_bytes([
            self.key1_buffer[i],
            self.key1_buffer[i + 1],
            self.key1_buffer[i + 2],
            self.key1_buffer[i + 3],
        ])
    }

    #[inline]
    fn write_keybuf_u32(&mut self, index: usize, value: u32) {
        let i = index * 4;
        let b = value.to_le_bytes();
        self.key1_buffer[i] = b[0];
        self.key1_buffer[i + 1] = b[1];
        self.key1_buffer[i + 2] = b[2];
        self.key1_buffer[i + 3] = b[3];
    }

    /// Performs KEY1 encryption on data buffer.
    #[must_use]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip(self), level = "trace"))]
    pub fn key1_encrypt(&mut self, mut y: u32, mut x: u32) -> [u32; 2] {
        // #[cfg(feature = "tracing")]
        // tracing::info!(?y, ?x);

        for i in 0..=0xF {
            let z = (self.read_keybuf_u32(i) ^ x) as usize;
            x = self.read_keybuf_u32(0x012 + ((z >> 24) & 0xFF));
            x = x.wrapping_add(self.read_keybuf_u32(0x112 + ((z >> 16) & 0xFF)));
            x ^= self.read_keybuf_u32(0x212 + ((z >> 8) & 0xFF));
            x = x.wrapping_add(self.read_keybuf_u32(0x312 + (z & 0xFF)));
            x ^= y;
            y = z as u32;
        }

        [
            x ^ self.read_keybuf_u32(0x10),
            y ^ self.read_keybuf_u32(0x11),
        ]
    }

    /// Performs KEY1 decryption on data buffer.
    fn key1_decrypt(&self, data: &mut [u32]) {
        let mut y = data[0];
        let mut x = data[1];

        for i in (0x2..=0x11).rev() {
            let z = self.read_keybuf_u32(i) ^ x;

            x = self.read_keybuf_u32(0x012 + ((z >> 24) as usize & 0xFF));
            x = x.wrapping_add(self.read_keybuf_u32(0x112 + ((z >> 16) as usize & 0xFF)));
            x ^= self.read_keybuf_u32(0x212 + ((z >> 8) as usize & 0xFF));
            x = x.wrapping_add(self.read_keybuf_u32(0x312 + (z as usize & 0xFF)));
            x ^= y;

            y = z;
        }

        data[0] = x ^ self.read_keybuf_u32(1);
        data[1] = y ^ self.read_keybuf_u32(0);
    }

    /// Applies keycode transformation.
    pub fn apply_keycode(&mut self, modulo: u32) {
        {
            let y = self.keycode[1];
            let x = self.keycode[2];
            let data = self.key1_encrypt(y, x);
            self.keycode[1] = data[0];
            self.keycode[2] = data[1];
        }
        {
            let y = self.keycode[0];
            let x = self.keycode[1];
            let data = self.key1_encrypt(y, x);
            self.keycode[0] = data[0];
            self.keycode[1] = data[1];
        }

        for i in 0..0x11 {
            let mut value = self.read_keybuf_u32(i);
            value ^= byteswap_word(self.keycode[i % modulo as usize]);
            self.write_keybuf_u32(i, value);
        }

        let mut scratch = [0; 2];
        for i in (0..=0x410).skip(1) {
            {
                let y = scratch[0];
                let x = scratch[1];
                let data = self.key1_encrypt(y, x);
                scratch[0] = data[0];
                scratch[1] = data[1];
            }

            self.write_keybuf_u32(i, scratch[1]);
            self.write_keybuf_u32(i + 1, scratch[0]);
        }
    }

    // Movd to struct Emulator method
    // fn init_keycode(&mut self, _idcode: u32, _level: i32, _modulo: u32) ;
}

const fn byteswap_word(word: u32) -> u32 {
    let mut result = 0;
    result |= word >> 24;
    result |= (word & 0xFF0000) >> 8;
    result |= (word & 0xFF00) << 8;
    result |= word << 24;
    result
}

/// Cartridge related errors.
#[derive(Debug, snafu::Snafu)]
pub enum CartridgeError {
    /// Save database file could not be opened.
    #[snafu(display("Failed to load save database: {}", path.display()))]
    OpenSaveDatabase {
        path: PathBuf,
        source: std::io::Error,
    },

    /// Save database metadata could not be read.
    #[snafu(display("Failed to read save database metadata: {}", path.display()))]
    MetadataSaveDatabase {
        path: PathBuf,
        source: std::io::Error,
    },

    /// Save database has invalid format.
    #[snafu(display("Save database corrupted or wrong format: {} (size={size})", path.display()))]
    InvalidSaveDatabaseFormat { path: PathBuf, size: u64 },

    /// Save database read failed.
    #[snafu(display("Failed to read save database contents: {}", path.display()))]
    ReadSaveDatabase {
        path: PathBuf,
        source: std::io::Error,
    },

    /// ROM file open failed.
    #[snafu(display("Failed to load ROM: {}", path.display()))]
    OpenRom {
        path: PathBuf,
        source: std::io::Error,
    },

    /// ROM metadata read failed.
    #[snafu(display("Failed to read ROM metadata: {}", path.display()))]
    MetadataRom {
        path: PathBuf,
        source: std::io::Error,
    },

    /// ROM read failed.
    #[snafu(display("Failed to read ROM contents: {}", path.display()))]
    ReadRom {
        path: PathBuf,
        source: std::io::Error,
    },

    /// Save read failed.
    #[snafu(display("Failed to read save file: {}", path.display()))]
    ReadSave {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("Failed to write save file: {}", path.display()))]
    SaveWrite {
        path: PathBuf,
        source: std::io::Error,
    },
}
