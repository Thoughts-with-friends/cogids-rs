//! Texture sampling: wrap / mirror / clamp and the 7 texture formats
//! (melonDS `TextureLookup`). See GBATEK "DS 3D Texture Formats".
use super::{Frame, PAL_SIZE, TEX_SIZE};

impl Frame<'_> {
    /// Texel color (BGR555) and alpha (0-31), melonDS `TextureLookup`.
    pub(super) fn texture_lookup(&self, texparam: u32, texpal: u32, s: i16, t: i16) -> (u32, u32) {
        let tex = |a: u32| self.tex[a as usize & (TEX_SIZE - 1)];
        let tex16 = |a: u32| {
            let a = a as usize & (TEX_SIZE - 2);
            u32::from(u16::from_le_bytes([self.tex[a], self.tex[a + 1]]))
        };
        let pal = |a: u32| {
            let a = a as usize & (PAL_SIZE - 2);
            u32::from(u16::from_le_bytes([self.pal[a], self.pal[a + 1]]))
        };

        let mut vramaddr = (texparam & 0xFFFF) << 3;
        let width = 8_i32 << ((texparam >> 20) & 7);
        let height = 8_i32 << ((texparam >> 23) & 7);
        let mut s = i32::from(s >> 4);
        let mut t = i32::from(t >> 4);

        if texparam & (1 << 16) != 0 {
            if texparam & (1 << 18) != 0 && s & width != 0 {
                s = (width - 1) - (s & (width - 1));
            } else {
                s &= width - 1;
            }
        } else {
            s = s.clamp(0, width - 1);
        }
        if texparam & (1 << 17) != 0 {
            if texparam & (1 << 19) != 0 && t & height != 0 {
                t = (height - 1) - (t & (height - 1));
            } else {
                t &= height - 1;
            }
        } else {
            t = t.clamp(0, height - 1);
        }
        let (s, t, width) = (s as u32, t as u32, width as u32);

        let alpha0 = if texparam & (1 << 29) != 0 { 0 } else { 31 };
        match (texparam >> 26) & 7 {
            1 => {
                // A3I5
                let pixel = u32::from(tex(vramaddr + t * width + s));
                let color = pal((texpal << 4) + ((pixel & 0x1F) << 1));
                (color, ((pixel >> 3) & 0x1C) + (pixel >> 6))
            }
            2 => {
                // 4-color palette
                let pixel =
                    (u32::from(tex(vramaddr + ((t * width + s) >> 2))) >> ((s & 3) << 1)) & 3;
                let color = pal((texpal << 3) + (pixel << 1));
                (color, if pixel == 0 { alpha0 } else { 31 })
            }
            3 => {
                // 16-color palette
                let byte = u32::from(tex(vramaddr + ((t * width + s) >> 1)));
                let pixel = if s & 1 != 0 { byte >> 4 } else { byte & 0xF };
                let color = pal((texpal << 4) + (pixel << 1));
                (color, if pixel == 0 { alpha0 } else { 31 })
            }
            4 => {
                // 256-color palette
                let pixel = u32::from(tex(vramaddr + t * width + s));
                let color = pal((texpal << 4) + (pixel << 1));
                (color, if pixel == 0 { alpha0 } else { 31 })
            }
            5 => {
                // 4x4 compressed
                vramaddr += (t & 0x3FC) * (width >> 2) + (s & 0x3FC);
                vramaddr += t & 3;
                vramaddr &= 0x7FFFF;
                let mut slot1 = 0x20000 + ((vramaddr & 0x1FFFC) >> 1);
                if vramaddr >= 0x40000 {
                    slot1 += 0x10000;
                }
                let val = if (0x20000..0x40000).contains(&vramaddr) {
                    0
                } else {
                    u32::from(tex(vramaddr)) >> (2 * (s & 3))
                };
                let palinfo = tex16(slot1);
                let base = (texpal << 4) + ((palinfo & 0x3FFF) << 2);
                let mix = |w0: u32, w1: u32| {
                    let (c0, c1) = (pal(base), pal(base + 2));
                    let r = ((c0 & 0x1F) * w0 + (c1 & 0x1F) * w1) >> 3;
                    let g = (((c0 & 0x3E0) * w0 + (c1 & 0x3E0) * w1) >> 3) & 0x3E0;
                    let b = (((c0 & 0x7C00) * w0 + (c1 & 0x7C00) * w1) >> 3) & 0x7C00;
                    r | g | b
                };
                match (val & 3, palinfo >> 14) {
                    (0, _) => (pal(base), 31),
                    (1, _) => (pal(base + 2), 31),
                    (2, 1) => (mix(4, 4), 31),
                    (2, 3) => (mix(5, 3), 31),
                    (2, _) => (pal(base + 4), 31),
                    (3, 2) => (pal(base + 6), 31),
                    (3, 3) => (mix(3, 5), 31),
                    _ => (0, 0),
                }
            }
            6 => {
                // A5I3
                let pixel = u32::from(tex(vramaddr + t * width + s));
                let color = pal((texpal << 4) + ((pixel & 7) << 1));
                (color, pixel >> 3)
            }
            7 => {
                // Direct color
                let color = tex16(vramaddr + ((t * width + s) << 1));
                (color, if color & 0x8000 != 0 { 31 } else { 0 })
            }
            _ => (0, 31),
        }
    }
}
