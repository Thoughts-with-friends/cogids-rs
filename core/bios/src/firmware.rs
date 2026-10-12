// SPDX-FileCopyrightText: (C) 2007 kelpsyberry
// SPDX-License-Identifier: GPL-3.0-or-later
// https://github.com/kelpsyberry/dust/blob/main/core/src/spi/firmware.rs#L8

/// DSType enum
/// DS, Lite, DSi, iQue DS, iQue DS Lite
#[derive(Clone, Copy)]
pub enum DSType {
    Ds,
    Lite,
    Dsi,
    /// iQue: Chinese Nintendo DS, uses region-locked cartridges and localized OS
    Ique,
    IqueLite,
}

impl DSType {
    /// Firmware header byte 0x1D (console type) of the model.
    const fn model_spec(&self) -> u8 {
        match self {
            Self::Ds => 0xFF,
            Self::Lite => 0x20,
            Self::Ique => 0x57,
            Self::IqueLite => 0x43,
            Self::Dsi => 0x63,
        }
    }
}

/// DS firmware for standard DS/Lite (256KB)
///
/// # Example
/// ```no_run
/// use ds_bios::firmware::FIRMWARE_DS;
/// std::fs::write("firmware.bin", FIRMWARE_DS).unwrap();
/// ```
pub static FIRMWARE_DS: [u8; 0x40000] = default_firmware::<0x40000>(DSType::Ds);
/// DS firmware for DSi (128KB)
pub static FIRMWARE_DSI: [u8; 0x20000] = default_firmware::<0x20000>(DSType::Dsi);
/// DS firmware for iQue DS (512KB)
pub static FIRMWARE_DS_IQUE: [u8; 0x80000] = default_firmware::<0x80000>(DSType::Ique);

/// Wi-Fi MAC address of the built-in firmware (melonDS' default; 00:09:BF
/// is Nintendo's OUI).
///
/// It must not be all zeros: the SDK copies it to 0x027FFCF4 at boot
/// (`OS_GetMacAddress`), and the anti-piracy code of later games (e.g.
/// Pokemon + Nobunaga's Ambition) treats a zero MAC as an emulator and calls
/// `OS_Terminate`.
const MAC_ADDRESS: [u8; 6] = [0x00, 0x09, 0xBF, 0x11, 0x22, 0x33];

/// User settings 0x58..0x63: ADC (x1, y1) = (0x200, 0x200) at pixel
/// (32, 32) and ADC (x2, y2) = (0xE00, 0xA00) at pixel (224, 160).
const TOUCH_CALIBRATION: [u8; 12] = [
    0x00, 0x02, 0x00, 0x02, 32, 32, 0x00, 0x0E, 0x00, 0x0A, 224, 160,
];

/// Wi-Fi calibration 0x44..0x63: initial values of the Wi-Fi I/O ports
/// (melonDS `FirmwareHeader`).
const WIFI_INIT_PORTS: [u16; 16] = [
    0x0002, 0x0017, 0x0026, 0x1818, 0x0048, 0x4840, 0x0058, 0x0042, 0x0146, 0x8064, 0xE6E6, 0x2443,
    0x000E, 0x0001, 0x0001, 0x0402,
];

/// Wi-Fi calibration 0x64..0xCC: initial baseband registers 0x00..0x68.
const WIFI_BB_INIT: [u8; 0x69] = [
    0x03, 0x17, 0x40, 0x00, 0x1B, 0x6C, 0x48, 0x80, 0x38, 0x00, 0x35, 0x07, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xB0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC7, 0xBB, 0x01, 0x24, 0x7F,
    0x5A, 0x01, 0x3F, 0x01, 0x3F, 0x36, 0x1D, 0x00, 0x78, 0x35, 0x55, 0x12, 0x34, 0x1C, 0x00, 0x01,
    0x0E, 0x38, 0x03, 0x70, 0xC5, 0x2A, 0x0A, 0x08, 0x04, 0x01, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFE,
    0xFE, 0xFE, 0xFE, 0xFC, 0xFC, 0xFA, 0xFA, 0xFA, 0xFA, 0xFA, 0xF8, 0xF8, 0xF6, 0x00, 0x12, 0x14,
    0x12, 0x41, 0x23, 0x03, 0x04, 0x70, 0x35, 0x0E, 0x2C, 0x2C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x0E, 0x00, 0x00, 0x12, 0x28, 0x1C,
];

/// Wi-Fi calibration 0xCE..0xF6: initial RF registers of an RF chip type 3.
const WIFI_RF_INIT: [u8; 0x29] = [
    0x31, 0x4C, 0x4F, 0x21, 0x00, 0x10, 0xB0, 0x08, 0xFA, 0x15, 0x26, 0xE6, 0xC1, 0x01, 0x0E, 0x50,
    0x05, 0x00, 0x6D, 0x12, 0x00, 0x00, 0x01, 0xFF, 0x0E, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x06,
    0x06, 0x00, 0x00, 0x00, 0x18, 0x00, 0x02, 0x00, 0x00,
];

/// Wi-Fi calibration 0xF8..0x133: per-channel BB/RF values (type 3).
const WIFI_CHANNEL_DATA: [u8; 0x3C] = [
    0x1E, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x0E, 0x16, 0x26,
    0x1C, 0x1C, 0x1C, 0x1D, 0x1D, 0x1D, 0x1E, 0x1E, 0x1E, 0x1E, 0x1F, 0x1E, 0x1F, 0x18, 0x01, 0x4B,
    0x4B, 0x4B, 0x4B, 0x4C, 0x4C, 0x4C, 0x4C, 0x4C, 0x4C, 0x4C, 0x4D, 0x4D, 0x4D, 0x02, 0x6C, 0x71,
    0x76, 0x5B, 0x40, 0x45, 0x4A, 0x2F, 0x34, 0x39, 0x3E, 0x03, 0x08, 0x14,
];

/// Length of the Wi-Fi config block at 0x2C (GBATEK "DS Firmware Header").
const WIFI_CONFIG_LEN: usize = 0x138;

// CRC16 calculation using while loop, const compatible
/// CRC16 of a 0x70-byte user settings block.
const fn crc16(user: [u8; 0x70]) -> u16 {
    crc16_range(&user, 0, 0x70, 0xFFFF)
}

/// CRC16 (poly 0xA001) of `data[start..start + len]`, const compatible.
const fn crc16_range(data: &[u8], start: usize, len: usize, init: u16) -> u16 {
    let mut crc = init;
    let mut i = start;
    while i < start + len {
        crc ^= data[i] as u16;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        i += 1;
    }
    crc
}

/// Const-compatible firmware creation
const fn default_firmware<const N: usize>(model: DSType) -> [u8; N] {
    let mut firmware = [0_u8; N];

    // 0x04..0x07
    firmware[0x04] = 0x00;
    firmware[0x05] = 0xDB;
    firmware[0x06] = 0x1F;
    firmware[0x07] = 0x0F;

    // 0x08..0x0B "MACh"
    // TODO: optional customization
    firmware[0x08] = b'M';
    firmware[0x09] = b'A';
    firmware[0x0A] = b'C';
    firmware[0x0B] = 0x68;

    // 0x14..0x15
    let val14: u16 = ((N >> 17) << 12) as u16;
    firmware[0x14] = (val14 & 0xFF) as u8;
    firmware[0x15] = (val14 >> 8) as u8;

    // 0x18..0x1C
    firmware[0x18] = 0x00;
    firmware[0x19] = 0x00;
    firmware[0x1A] = 0x01;
    firmware[0x1B] = 0x01;
    firmware[0x1C] = 0x06;

    // 0x1D model-specific
    firmware[0x1D] = model.model_spec();

    // 0x1E..0x1F
    firmware[0x1E] = 0xFF;
    firmware[0x1F] = 0xFF;

    // 0x2C..0x163: Wi-Fi config (length, MAC, enabled channels), CRC at 0x2A
    firmware[0x2C] = (WIFI_CONFIG_LEN & 0xFF) as u8;
    firmware[0x2D] = (WIFI_CONFIG_LEN >> 8) as u8;
    let mut i = 0;
    while i < 6 {
        firmware[0x36 + i] = MAC_ADDRESS[i];
        i += 1;
    }
    firmware[0x3C] = 0xFE;
    firmware[0x3D] = 0x3F;

    // Wi-Fi calibration (melonDS' generated firmware). Without it the ARM7
    // wireless library cannot program the baseband/RF chips, WM_Enable fails
    // and games treat that as a communication error (Pokemon HeartGold then
    // stops reading the touch screen on its title menu).
    firmware[0x2F] = 6; // Wi-Fi version W006
    let mut i = 0;
    while i < 5 {
        firmware[0x30 + i] = 0xFF;
        i += 1;
    }
    firmware[0x3E] = 0xFF;
    firmware[0x3F] = 0xFF;
    firmware[0x40] = 3; // RF chip type 3
    firmware[0x41] = 0x94; // RF bits per entry
    firmware[0x42] = 0x29; // RF entries
    firmware[0x43] = 0x02;
    let mut i = 0;
    while i < WIFI_INIT_PORTS.len() {
        firmware[0x44 + i * 2] = (WIFI_INIT_PORTS[i] & 0xFF) as u8;
        firmware[0x45 + i * 2] = (WIFI_INIT_PORTS[i] >> 8) as u8;
        i += 1;
    }
    let mut i = 0;
    while i < WIFI_BB_INIT.len() {
        firmware[0x64 + i] = WIFI_BB_INIT[i];
        i += 1;
    }
    let mut i = 0;
    while i < WIFI_RF_INIT.len() {
        firmware[0xCE + i] = WIFI_RF_INIT[i];
        i += 1;
    }
    firmware[0xF7] = 0x02; // BB indices per channel
    let mut i = 0;
    while i < WIFI_CHANNEL_DATA.len() {
        firmware[0xF8 + i] = WIFI_CHANNEL_DATA[i];
        i += 1;
    }
    let mut i = 0x134;
    while i < 0x162 {
        firmware[i] = 0xFF;
        i += 1;
    }
    let wifi_crc = crc16_range(&firmware, 0x2C, WIFI_CONFIG_LEN, 0x0000);
    firmware[0x2A] = (wifi_crc & 0xFF) as u8;
    firmware[0x2B] = (wifi_crc >> 8) as u8;

    // 0x20..0x28 values
    let values: [u16; 5] = [((N - 0x200) >> 3) as u16, 0x0B51, 0x0DB3, 0x4F5D, 0xFFFF];
    let mut idx = 0;
    while idx < 5 {
        let val = values[idx];
        firmware[0x20 + idx * 2] = (val & 0xFF) as u8;
        firmware[0x21 + idx * 2] = (val >> 8) as u8;
        idx += 1;
    }

    // User settings for 2 users
    let mut u = 0;
    while u < 2 {
        let start = N - 0x200 + u * 0x100;
        let mut i = 0;
        while i < 0x100 {
            firmware[start + i] = 0;
            i += 1;
        }

        firmware[start] = 5;
        firmware[start + 0x02] = if u == 0 { 1 } else { 0 };
        firmware[start + 0x03] = 1;
        firmware[start + 0x04] = 1;

        // Name "Luna"
        firmware[start + 0x06] = b'L';
        firmware[start + 0x07] = 0;
        firmware[start + 0x08] = b'u';
        firmware[start + 0x09] = 0;
        firmware[start + 0x0A] = b'n';
        firmware[start + 0x0B] = 0;
        firmware[start + 0x0C] = b'a';
        firmware[start + 0x0D] = 0;

        // 0x58..0x63: touch screen calibration (GBATEK "DS Firmware User
        // Settings"): two points as 12-bit ADC values plus the screen pixel
        // they correspond to. The emulated TSC reports `pixel << 4`, so the
        // points match that mapping. All-zero calibration (as before) gives
        // the SDK's TP_CalcCalibrateParam nothing to work with.
        let mut c = 0;
        while c < TOUCH_CALIBRATION.len() {
            firmware[start + 0x58 + c] = TOUCH_CALIBRATION[c];
            c += 1;
        }

        // CRC16
        let mut crc: [u8; 112] = [0; 0x70];
        let mut i = 0;
        while i < 0x70 {
            crc[i] = firmware[start + i];
            i += 1;
        }
        let crc: u16 = crc16(crc);

        firmware[start + 0x72] = (crc & 0xFF) as u8;
        firmware[start + 0x73] = (crc >> 8) as u8;

        u += 1;
    }

    firmware
}
