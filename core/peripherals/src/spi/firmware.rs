//! Firmware flash memory (SPI device 1).
//!
//! ```text
//!  ARM7 CPU
//!    |  SPICNT  (0x040001C0)  bits 8-9: device select, bit 11: chip-select hold
//!    |  SPIDATA (0x040001C2)  one byte out / one byte in per transfer
//!    v
//!  +--------------------------- SPI bus (spi/mod.rs) ---------------------------+
//!  |        device 0                device 1                   device 2          |
//!  +-----------|-----------------------|--------------------------|--------------+
//!              v                       v                          v
//!        power.rs                firmware.rs                touchscreen.rs
//!                                ^^^ this file
//!        power management IC     firmware flash memory      touch screen controller
//!        (power off, backlight,  (user settings, Wi-Fi      (TSC: pen position,
//!         amplifier, mic gain)    settings, boot code)       mic, temperature ADC)
//! ```
//!
//! None of these chips is mapped into the ARM7 address space: the CPU can
//! only talk to them by selecting one with SPICNT and then exchanging bytes
//! through SPIDATA. That is why the three device models live under `spi/`,
//! next to the bus that multiplexes them.
//!
//! The 256 KiB flash holds the boot code, the Wi-Fi settings and two copies
//! of the user settings (name, language, touch screen calibration...).
//! Games read it with the flash READ command; this model also selects the
//! valid user settings block and patches it on load.
//! See GBATEK "DS Firmware Serial Flash Memory".

/// SPI flash holding the DS firmware / user settings.
#[derive(Debug)]
pub struct Firmware {
    /// Firmware data (262 KB)
    pub(crate) raw_firmware: Vec<u8>,
    /// Status register
    status_reg: u8,
    /// User data section
    pub user_data: i32,

    /// Current SPI flash command byte (valid while `selected`).
    command: u8,
    /// Chip select is held: the next byte continues `command`.
    selected: bool,
    /// Bytes received for the current command, including the command byte.
    data_pos: u32,
    /// Current address
    address: u32,
}

impl Firmware {
    /// Firmware size in bytes (256 KB)
    pub const SIZE: usize = 1024 * 256;

    /// Create new Firmware controller
    pub fn new() -> Self {
        Self {
            raw_firmware: vec![0_u8; Self::SIZE],
            status_reg: 0,
            user_data: 0,
            command: 0,
            selected: false,
            data_pos: 0,
            address: 0,
        }
    }

    /// Loads a firmware image (the system crate picks a dump from disk or
    /// the built-in replacement image).
    ///
    /// - Selects the newer valid user settings block (USER0 / USER1)
    /// - Patches several firmware configuration fields
    /// - Recalculates CRCs for user data and header
    pub fn load(&mut self, image: &[u8]) {
        // Ensure firmware buffer has the correct size
        if self.raw_firmware.len() != Self::SIZE {
            self.raw_firmware.resize(Self::SIZE, 0);
        }
        let copy_size = image.len().min(Self::SIZE);
        self.raw_firmware[..copy_size].copy_from_slice(&image[..copy_size]);

        // Initial user data base address
        self.user_data = 0x3FE00;

        // Read USER1 sequence number and compare against USER0
        let user0_seq = self.read_u16((self.user_data + 0x70) as usize);
        let user1_seq = self.read_u16((self.user_data + 0x170) as usize);

        if user1_seq == ((user0_seq + 1) & 0x7F) {
            // Verify CRC of USER1 data
            #[rustfmt::skip]
            let verify = self.verify_crc(0xFFFF, (self.user_data + 0x100) as usize, 0x70, (self.user_data + 0x172) as usize);
            if verify {
                // Switch to USER1 block
                self.user_data += 0x100;
            }
        }

        self.patch_user_settings();

        // Recalculate firmware header CRC
        let header_len = self.read_u16(0x2C) as usize;
        let header_crc = Self::create_crc(&self.raw_firmware[0x2C..], header_len, 0x0000);
        self.write_u16(0x2A, header_crc);

        // Debug output of CRC verification results
        #[cfg(feature = "tracing")]
        {
            tracing::error!(
                "\nFW: USER0 CRC16 = {}",
                if self.verify_crc(0xFFFF, 0x3FE00, 0x70, 0x3FE72) {
                    "GOOD"
                } else {
                    "BAD"
                },
            );
            tracing::error!(
                "FW: USER1 CRC16 = {}",
                if self.verify_crc(0xFFFF, 0x3FF00, 0x70, 0x3FF72) {
                    "GOOD"
                } else {
                    "BAD"
                },
            );
        }

        // Reset command and status registers
        self.release();
        self.status_reg = 0;
    }

    /// Fixes up the user settings (GBATEK "DS Firmware User Settings") and
    /// mirrors them into both slots.
    ///
    /// The bundled image ships empty settings. Software (libnds, the Nitro
    /// SDK) picks one of the two slots by their update counters and CRCs, so
    /// patching only one slot let it read the other, empty one: the touch
    /// calibration was all zeros and every touch converted to pixel (0, 0).
    /// Like melonDS, the calibration maps ADC `x << 4, y << 4` (what
    /// [`ds_peripherals::spi::touchscreen::TouchScreen`] reports) to pixel `x, y`.
    fn patch_user_settings(&mut self) {
        let base = self.user_data as usize;

        // Empty settings: fill in sane defaults (melonDS `UserData`).
        if self.read_u16(base) == 0 || self.read_u16(base) == 0xFFFF {
            self.raw_firmware[base..base + 0x70].fill(0);
            self.write_u16(base, 5); // version
            self.raw_firmware[base + 0x03] = 1; // birthday month
            self.raw_firmware[base + 0x04] = 1; // birthday day
            for (i, ch) in "Lunaris".encode_utf16().enumerate() {
                self.write_u16(base + 0x06 + i * 2, ch);
            }
            self.write_u16(base + 0x1A, 7); // nickname length
            // Language 0 (Japanese), backlight max.
            self.write_u16(base + 0x64, 0x0030);
        }

        // Touch screen calibration: ADC (0,0) = pixel (0,0),
        // ADC (255<<4, 191<<4) = pixel (255,191).
        self.write_u16(base + 0x58, 0);
        self.write_u16(base + 0x5A, 0);
        self.raw_firmware[base + 0x5C] = 0;
        self.raw_firmware[base + 0x5D] = 0;
        self.write_u16(base + 0x5E, 255 << 4);
        self.write_u16(base + 0x60, 191 << 4);
        self.raw_firmware[base + 0x62] = 255;
        self.raw_firmware[base + 0x63] = 191;

        let crc = Self::create_crc(&self.raw_firmware[base..], 0x70, 0xFFFF);
        self.write_u16(base + 0x72, crc);

        // Same settings (and update counter) in the other slot.
        let other = 0x3FE00 + ((base - 0x3FE00) ^ 0x100);
        self.raw_firmware.copy_within(base..base + 0x100, other);
    }

    /// Read a little-endian u16 from firmware
    pub fn read_u16(&self, offset: usize) -> u16 {
        u16::from_le_bytes([self.raw_firmware[offset], self.raw_firmware[offset + 1]])
    }

    /// Read a little-endian u32 from firmware
    pub fn read_u32(&self, offset: usize) -> u32 {
        u32::from_le_bytes([
            self.raw_firmware[offset],
            self.raw_firmware[offset + 1],
            self.raw_firmware[offset + 2],
            self.raw_firmware[offset + 3],
        ])
    }

    /// Write a little-endian u16 into firmware
    fn write_u16(&mut self, offset: usize, value: u16) {
        let bytes = value.to_le_bytes();
        self.raw_firmware[offset] = bytes[0];
        self.raw_firmware[offset + 1] = bytes[1];
    }

    /// Create a CRC16 value from the given data buffer.
    ///
    /// This is a faithful Rust translation of the original
    /// C++ `Firmware::create_CRC` implementation.
    /// All bit operations, constants, and control flow
    /// are preserved exactly.
    pub fn create_crc(data: &[u8], length: usize, mut start: u32) -> u16 {
        // CRC polynomial lookup table (identical to C++ code)
        let stuff: [u16; 8] = [
            0xC0C1, 0xC181, 0xC301, 0xC601, 0xCC01, 0xD801, 0xF001, 0xA001,
        ];

        // Process each byte
        #[expect(clippy::needless_range_loop)]
        for i in 0..length {
            start ^= data[i] as u32;

            // Process each bit
            for (j, &v) in stuff.iter().enumerate() {
                if (start & 0x1) != 0 {
                    start >>= 1;
                    start ^= (v as u32) << (7 - j);
                } else {
                    start >>= 1;
                }
            }
        }

        // Return lower 16 bits (equivalent to uint16_t cast)
        (start & 0xFFFF) as u16
    }

    /// Verify a CRC16 value stored in firmware against a calculated CRC.
    ///
    /// This function mirrors the original C++ `Firmware::verify_CRC`
    /// implementation exactly, including:
    /// - Little-endian uint16_t access
    /// - Debug output formatting
    /// - CRC comparison logic
    pub fn verify_crc(&self, start: u32, offset: usize, length: usize, crc_offset: usize) -> bool {
        // Read stored CRC (little-endian)
        let stored_crc = u16::from_le_bytes([
            self.raw_firmware[crc_offset],
            self.raw_firmware[crc_offset + 1],
        ]);

        // Calculate CRC from firmware data
        let calculated_crc = Self::create_crc(&self.raw_firmware[offset..], length, start);

        // Debug output (matches C++ printf behavior)
        // println!("\nStored CRC: ${:04X}", stored_crc);
        // println!("Calc CRC: ${:04X}", calculated_crc);

        stored_crc == calculated_crc
    }

    /// Transfer data byte via SPI
    /// Input: byte to send to firmware
    /// Returns: byte received from firmware
    /// Clocks one byte through the SPI flash (melonDS `FirmwareMem::Write`).
    ///
    /// The first byte after chip select is the command; for READ (0x03) /
    /// FAST READ (0x0B) the next three bytes are a big-endian address and
    /// every following byte streams data out. Returns the byte shifted out.
    pub fn transfer_data(&mut self, input: u8) -> u8 {
        if !self.selected {
            self.selected = true;
            self.command = input;
            self.data_pos = 1;
            self.address = 0;
            // WREN / WRDI are single-byte commands: they take effect on the
            // command byte itself (melonDS `FirmwareMem::Write`). Handling
            // them only on a following byte left the write enable latch
            // clear, so the SDK's NVRAM writes retried forever.
            match input {
                0x06 => self.status_reg |= 0x02,
                0x04 => self.status_reg &= !0x02,
                _ => {}
            }
            return 0;
        }

        let pos = self.data_pos;
        self.data_pos = self.data_pos.saturating_add(1);
        match self.command {
            // READ / FAST READ (FAST READ has one extra dummy byte)
            0x03 | 0x0B => {
                let data_start = if self.command == 0x0B { 5 } else { 4 };
                if pos < 4 {
                    self.address = (self.address << 8) | u32::from(input);
                    0
                } else if pos < data_start {
                    0
                } else {
                    let len = self.raw_firmware.len() as u32;
                    let byte = self.raw_firmware[(self.address % len) as usize];
                    self.address = self.address.wrapping_add(1);
                    byte
                }
            }
            0x04 => {
                self.status_reg &= !0x02; // WRDI
                0
            }
            0x05 => self.status_reg, // RDSR
            0x06 => {
                self.status_reg |= 0x02; // WREN
                0
            }
            // PAGE WRITE / PAGE PROGRAM
            0x0A | 0x02 => {
                if pos < 4 {
                    self.address = (self.address << 8) | u32::from(input);
                } else if self.status_reg & 0x02 != 0 {
                    let len = self.raw_firmware.len() as u32;
                    self.raw_firmware[(self.address % len) as usize] = input;
                    self.address = self.address.wrapping_add(1);
                }
                0
            }
            // RDID: manufacturer / device ID
            0x9F => match pos {
                1 => 0x20,
                2 => 0x40,
                3 => 0x12,
                _ => 0,
            },
            _ => 0xFF,
        }
    }

    /// Chip select released (SPICNT bit 11 clear after a transfer): the next
    /// byte starts a new command. Write commands also drop the write latch.
    pub const fn release(&mut self) {
        if self.selected && matches!(self.command, 0x0A | 0x02) {
            self.status_reg &= !0x02;
        }
        self.selected = false;
        self.command = 0;
        self.data_pos = 0;
    }

    /// Whether a firmware image has been loaded (user settings located).
    pub const fn is_loaded(&self) -> bool {
        self.user_data != 0
    }
}

impl Default for Firmware {
    fn default() -> Self {
        Self::new()
    }
}
