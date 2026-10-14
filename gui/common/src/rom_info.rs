//! System > ROM info: the cartridge header and banner (GBATEK "DS
//! Cartridge Header", "DS Cartridge Icon/Title").
//!
//! ```text
//!  ROM file  0x000 title, 0x00C game code, 0x010 maker, 0x01E version,
//!            0x020/0x030 ARM9/ARM7 offset, entry, RAM address, size,
//!            0x068 banner offset
//!  banner    0x020 icon (32x32, 4bpp 8x8 tiles), 0x220 palette (BGR555),
//!            0x240 + n*0x100 titles (UTF-16; 0 Japanese, 1 English)
//! ```
use serde::Serialize;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

/// One CPU's program in the header.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Binary {
    pub rom_offset: u32,
    pub entry: u32,
    pub ram_address: u32,
    pub size: u32,
}

/// What ROM info shows.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RomInfo {
    pub path: String,
    /// Header title (ASCII, 12 characters).
    pub title: String,
    pub game_code: String,
    pub maker_code: String,
    pub version: u8,
    /// File size in bytes.
    pub file_size: u64,
    /// Chip capacity from the header (bytes).
    pub capacity: u64,
    pub arm9: Binary,
    pub arm7: Binary,
    /// Banner titles (lines separated by `\n`), if the ROM has a banner.
    pub title_ja: Option<String>,
    pub title_en: Option<String>,
    /// 32x32 RGBA icon, empty without a banner.
    pub icon_rgba: Vec<u8>,
}

/// Icon size in pixels (square).
pub const ICON_SIZE: usize = 32;

fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4)
        .map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn ascii(b: &[u8]) -> String {
    b.iter()
        .take_while(|&&c| c != 0)
        .map(|&c| {
            if c.is_ascii_graphic() || c == b' ' {
                c as char
            } else {
                '?'
            }
        })
        .collect()
}

fn utf16_title(b: &[u8]) -> Option<String> {
    let units: Vec<u16> = b
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_le_bytes(*p))
        .take_while(|&u| u != 0)
        .collect();
    let text = String::from_utf16_lossy(&units).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Decodes the 4bpp tiled icon with its BGR555 palette (colour 0 is
/// transparent).
fn icon(bitmap: &[u8], palette: &[u8]) -> Vec<u8> {
    let colour = |i: usize| -> [u8; 4] {
        if i == 0 {
            return [0; 4];
        }
        let c = u16::from_le_bytes([palette[i * 2], palette[i * 2 + 1]]);
        let ch = |shift: u16| {
            let v = ((c >> shift) & 0x1F) as u8;
            (v << 3) | (v >> 2)
        };
        [ch(0), ch(5), ch(10), 0xFF]
    };
    let mut rgba = vec![0; ICON_SIZE * ICON_SIZE * 4];
    for (i, byte) in bitmap.iter().enumerate().take(512) {
        let tile = i / 32;
        let (tx, ty) = (tile % 4, tile / 4);
        let within = i % 32;
        let (row, col) = (within / 4, (within % 4) * 2);
        for (half, index) in [(0, byte & 0xF), (1, byte >> 4)] {
            let x = tx * 8 + col + half;
            let y = ty * 8 + row;
            let at = (y * ICON_SIZE + x) * 4;
            rgba[at..at + 4].copy_from_slice(&colour(usize::from(index)));
        }
    }
    rgba
}

impl RomInfo {
    /// Reads the header and banner of the ROM at `path`.
    ///
    /// # Errors
    /// If the file cannot be read or is shorter than a header.
    pub fn read(path: &Path) -> std::io::Result<Self> {
        let mut file = std::fs::File::open(path)?;
        let file_size = file.metadata()?.len();
        let mut header = vec![0; 0x200];
        file.read_exact(&mut header)?;

        let binary = |at: usize| Binary {
            rom_offset: u32_at(&header, at),
            entry: u32_at(&header, at + 4),
            ram_address: u32_at(&header, at + 8),
            size: u32_at(&header, at + 12),
        };
        let mut info = Self {
            path: path.display().to_string(),
            title: ascii(&header[0x00..0x0C]),
            game_code: ascii(&header[0x0C..0x10]),
            maker_code: ascii(&header[0x10..0x12]),
            version: header[0x1E],
            file_size,
            capacity: (128 * 1024) << u64::from(header[0x14].min(15)),
            arm9: binary(0x20),
            arm7: binary(0x30),
            title_ja: None,
            title_en: None,
            icon_rgba: Vec::new(),
        };

        let banner_at = u64::from(u32_at(&header, 0x68));
        if banner_at != 0 && banner_at + 0x840 <= file_size {
            let mut banner = vec![0; 0x840];
            file.seek(SeekFrom::Start(banner_at))?;
            file.read_exact(&mut banner)?;
            info.icon_rgba = icon(&banner[0x20..0x220], &banner[0x220..0x240]);
            info.title_ja = utf16_title(&banner[0x240..0x340]);
            info.title_en = utf16_title(&banner[0x340..0x440]);
        }
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_header_and_banner() {
        let mut rom = vec![0_u8; 0x1000];
        rom[..10].copy_from_slice(b"HELLOWORLD");
        rom[0x0C..0x10].copy_from_slice(b"ABCJ");
        rom[0x10..0x12].copy_from_slice(b"01");
        rom[0x14] = 0;
        rom[0x1E] = 2;
        rom[0x20..0x24].copy_from_slice(&0x4000_u32.to_le_bytes());
        rom[0x68..0x6C].copy_from_slice(&0x400_u32.to_le_bytes());
        // Banner: icon pixel (0, 0) = colour 1 = pure red; Japanese title.
        rom[0x400 + 0x20] = 0x01;
        rom[0x400 + 0x222..0x400 + 0x224].copy_from_slice(&0x001F_u16.to_le_bytes());
        for (i, u) in "テスト\nNintendo".encode_utf16().enumerate() {
            rom[0x400 + 0x240 + i * 2..0x400 + 0x242 + i * 2].copy_from_slice(&u.to_le_bytes());
        }
        let path = std::env::temp_dir().join(format!("rom_info_{}.nds", std::process::id()));
        std::fs::write(&path, &rom).ok();
        let info = RomInfo::read(&path);
        let _ = std::fs::remove_file(&path);
        let Ok(info) = info else {
            panic!("read failed");
        };

        assert_eq!(info.title, "HELLOWORLD");
        assert_eq!(info.game_code, "ABCJ");
        assert_eq!(info.version, 2);
        assert_eq!(info.capacity, 128 * 1024);
        assert_eq!(info.arm9.rom_offset, 0x4000);
        assert_eq!(info.title_ja.as_deref(), Some("テスト\nNintendo"));
        assert_eq!(info.title_en, None);
        assert_eq!(&info.icon_rgba[..4], &[0xFF, 0, 0, 0xFF]);
        assert_eq!(
            info.icon_rgba[7], 0,
            "pixel (1, 0) is colour 0: transparent"
        );
    }
}
