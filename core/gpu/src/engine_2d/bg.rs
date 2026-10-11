//! Background layers BG0-BG3: text, affine, extended (256-color /
//! bitmap), large bitmap, and the 3D engine's output as BG0.
//! See GBATEK "DS Video BG Modes / Control".
use super::compose::{BG_3D, OPAQUE, W};
use crate::Gpu;
use crate::vram::Region;

#[inline]
const fn sign_extend_28(v: i32) -> i32 {
    (v << 4) >> 4
}

impl Gpu {
    /// BG0 as the 3D layer: one line of the rendered 3D frame, scrolled by
    /// BG0HOFS. Pixels keep their 5-bit alpha in bits 16-20.
    pub(super) fn render_bg_3d(&self, out: &mut [u32; W], line: usize) {
        let src = self.engine_3d.raster.line(line);
        let xpos = usize::from(self.engine_upper.bghofs[0] & 0x1FF);
        for (x, px) in out.iter_mut().enumerate() {
            // Scrolling shifts the image left; uncovered pixels are empty.
            let sx = x + xpos;
            let sx = if xpos & 0x100 != 0 {
                sx.wrapping_sub(512)
            } else {
                sx
            };
            let Some(&c) = src.get(sx) else { continue };
            let alpha = (c >> 24) & 0x1F;
            if alpha == 0 {
                continue;
            }
            let r = (c >> 1) & 0x1F;
            let g = (c >> 9) & 0x1F;
            let b = (c >> 17) & 0x1F;
            *px = OPAQUE | BG_3D | (alpha << 16) | (b << 10) | (g << 5) | r;
        }
    }

    /// Text BG (GBATEK "BG Screen Data Formats", text mode).
    pub(super) fn render_bg_text(&self, a: bool, index: usize, line: usize, out: &mut [u32; W]) {
        let region = if a { Region::BgA } else { Region::BgB };
        let e = self.engine(a);
        let bgcnt = e.bgcnt[index];
        let hofs = u32::from(e.bghofs[index]);
        let vofs = u32::from(e.bgvofs[index]);
        let (char_base, screen_base) = if a {
            (
                e.dispcnt.char_base as u32 * 0x1_0000,
                e.dispcnt.screen_base as u32 * 0x1_0000,
            )
        } else {
            (0, 0)
        };
        let char_base = char_base + u32::from((bgcnt >> 2) & 0xF) * 0x4000;
        let screen_base = screen_base + u32::from((bgcnt >> 8) & 0x1F) * 0x800;
        let color_256 = bgcnt & (1 << 7) != 0;
        let size = (bgcnt >> 14) & 0x3;
        let ext_palette = color_256 && e.dispcnt.bg_extended_palette;
        let ext_slot = if index < 2 && bgcnt & (1 << 13) != 0 {
            index + 2
        } else {
            index
        } as u32;
        let ext_region = if a {
            Region::BgExtPalA
        } else {
            Region::BgExtPalB
        };

        let width_mask = if size & 1 != 0 { 511 } else { 255 };
        let height_mask = if size & 2 != 0 { 511 } else { 255 };
        let y = (vofs + line as u32) & height_mask;
        let fine_y = y & 7;
        let mut row_base = screen_base + ((y >> 3) & 31) * 64;
        if y >= 256 {
            row_base += if size == 3 { 0x1000 } else { 0x800 };
        }

        let mut cached_column = u32::MAX;
        let mut entry = 0u16;
        for (px, out_px) in out.iter_mut().enumerate() {
            let x = (hofs + px as u32) & width_mask;
            let column = x >> 3;
            if column != cached_column {
                cached_column = column;
                let block = if x >= 256 { 0x800 } else { 0 };
                entry = self
                    .vram
                    .read_u16(region, row_base + block + (column & 31) * 2);
            }
            let tile = u32::from(entry & 0x3FF);
            let tx = if entry & (1 << 10) != 0 {
                7 - (x & 7)
            } else {
                x & 7
            };
            let ty = if entry & (1 << 11) != 0 {
                7 - fine_y
            } else {
                fine_y
            };
            let pal_bank = u32::from(entry >> 12);

            let color = if color_256 {
                let ci = u32::from(
                    self.vram
                        .read_u8(region, char_base + tile * 64 + ty * 8 + tx),
                );
                if ci == 0 {
                    continue;
                }
                if ext_palette {
                    self.vram
                        .read_u16(ext_region, ext_slot * 0x2000 + pal_bank * 512 + ci * 2)
                } else {
                    self.bg_palette_color(a, ci)
                }
            } else {
                let byte = self
                    .vram
                    .read_u8(region, char_base + tile * 32 + ty * 4 + tx / 2);
                let ci = u32::from(if tx & 1 != 0 { byte >> 4 } else { byte & 0xF });
                if ci == 0 {
                    continue;
                }
                self.bg_palette_color(a, pal_bank * 16 + ci)
            };
            *out_px = OPAQUE | u32::from(color & 0x7FFF);
        }
    }

    /// Current affine parameters of BG2/BG3: (ref x, ref y, pa, pc).
    fn affine_params(&self, a: bool, index: usize) -> (i32, i32, i32, i32) {
        let e = self.engine(a);
        if index == 2 {
            (
                sign_extend_28(e.bg2x_internal),
                sign_extend_28(e.bg2y_internal),
                i32::from(e.bg2p[0] as i16),
                i32::from(e.bg2p[2] as i16),
            )
        } else {
            (
                sign_extend_28(e.bg3x_internal),
                sign_extend_28(e.bg3y_internal),
                i32::from(e.bg3p[0] as i16),
                i32::from(e.bg3p[2] as i16),
            )
        }
    }

    /// Walks the affine texture coordinates of one line, calling `f(px, x, y)`
    /// for every pixel inside the `width` x `height` area (wrapping when
    /// `wrap`).
    fn affine_walk(
        &self,
        a: bool,
        index: usize,
        width: i32,
        height: i32,
        wrap: bool,
        mut f: impl FnMut(usize, u32, u32),
    ) {
        let (mut rx, mut ry, pa, pc) = self.affine_params(a, index);
        for px in 0..W {
            let (mut x, mut y) = (rx >> 8, ry >> 8);
            rx = rx.wrapping_add(pa);
            ry = ry.wrapping_add(pc);
            if wrap {
                x &= width - 1;
                y &= height - 1;
            } else if x < 0 || y < 0 || x >= width || y >= height {
                continue;
            }
            f(px, x as u32, y as u32);
        }
    }

    /// Rotation/scaling BG with 8-bit map entries and 8bpp tiles.
    pub(super) fn render_bg_affine(&self, a: bool, index: usize, out: &mut [u32; W]) {
        let region = if a { Region::BgA } else { Region::BgB };
        let e = self.engine(a);
        let bgcnt = e.bgcnt[index];
        let (char_base, screen_base) = if a {
            (
                e.dispcnt.char_base as u32 * 0x1_0000,
                e.dispcnt.screen_base as u32 * 0x1_0000,
            )
        } else {
            (0, 0)
        };
        let char_base = char_base + u32::from((bgcnt >> 2) & 0xF) * 0x4000;
        let screen_base = screen_base + u32::from((bgcnt >> 8) & 0x1F) * 0x800;
        let size = 128 << ((bgcnt >> 14) & 3);
        let wrap = bgcnt & (1 << 13) != 0;
        let tiles_per_row = (size / 8) as u32;

        self.affine_walk(a, index, size, size, wrap, |px, x, y| {
            let tile = u32::from(
                self.vram
                    .read_u8(region, screen_base + (y / 8) * tiles_per_row + x / 8),
            );
            let ci = u32::from(
                self.vram
                    .read_u8(region, char_base + tile * 64 + (y & 7) * 8 + (x & 7)),
            );
            if ci != 0 {
                out[px] = OPAQUE | u32::from(self.bg_palette_color(a, ci) & 0x7FFF);
            }
        });
    }

    /// Extended BG: affine 16-bit map entries, 256-color or direct bitmap.
    pub(super) fn render_bg_extended(&self, a: bool, index: usize, out: &mut [u32; W]) {
        let region = if a { Region::BgA } else { Region::BgB };
        let e = self.engine(a);
        let bgcnt = e.bgcnt[index];
        let wrap = bgcnt & (1 << 13) != 0;
        let size_bits = (bgcnt >> 14) & 3;

        if bgcnt & (1 << 7) == 0 {
            // Affine tiled BG with text-style 16-bit entries, 8bpp tiles.
            let (char_base, screen_base) = if a {
                (
                    e.dispcnt.char_base as u32 * 0x1_0000,
                    e.dispcnt.screen_base as u32 * 0x1_0000,
                )
            } else {
                (0, 0)
            };
            let char_base = char_base + u32::from((bgcnt >> 2) & 0xF) * 0x4000;
            let screen_base = screen_base + u32::from((bgcnt >> 8) & 0x1F) * 0x800;
            let size = 128 << size_bits;
            let tiles_per_row = (size / 8) as u32;
            let ext_palette = e.dispcnt.bg_extended_palette;
            let ext_region = if a {
                Region::BgExtPalA
            } else {
                Region::BgExtPalB
            };

            self.affine_walk(a, index, size, size, wrap, |px, x, y| {
                let entry = self
                    .vram
                    .read_u16(region, screen_base + ((y / 8) * tiles_per_row + x / 8) * 2);
                let tile = u32::from(entry & 0x3FF);
                let tx = if entry & (1 << 10) != 0 {
                    7 - (x & 7)
                } else {
                    x & 7
                };
                let ty = if entry & (1 << 11) != 0 {
                    7 - (y & 7)
                } else {
                    y & 7
                };
                let ci = u32::from(
                    self.vram
                        .read_u8(region, char_base + tile * 64 + ty * 8 + tx),
                );
                if ci == 0 {
                    return;
                }
                let color = if ext_palette {
                    let bank = u32::from(entry >> 12);
                    self.vram
                        .read_u16(ext_region, index as u32 * 0x2000 + bank * 512 + ci * 2)
                } else {
                    self.bg_palette_color(a, ci)
                };
                out[px] = OPAQUE | u32::from(color & 0x7FFF);
            });
            return;
        }

        // Bitmap BG: data at screen base * 16 KiB.
        let base = u32::from((bgcnt >> 8) & 0x1F) * 0x4000;
        let (width, height) = match size_bits {
            0 => (128, 128),
            1 => (256, 256),
            2 => (512, 256),
            _ => (512, 512),
        };
        let direct = bgcnt & (1 << 2) != 0;
        self.affine_walk(a, index, width, height, wrap, |px, x, y| {
            let offset = y * width as u32 + x;
            if direct {
                let c = self.vram.read_u16(region, base + offset * 2);
                if c & 0x8000 != 0 {
                    out[px] = OPAQUE | u32::from(c & 0x7FFF);
                }
            } else {
                let ci = u32::from(self.vram.read_u8(region, base + offset));
                if ci != 0 {
                    out[px] = OPAQUE | u32::from(self.bg_palette_color(a, ci) & 0x7FFF);
                }
            }
        });
    }

    /// BG mode 6 large 256-color bitmap (engine A BG2, 512x1024 / 1024x512).
    pub(super) fn render_bg_large(&self, out: &mut [u32; W]) {
        let bgcnt = self.engine_upper.bgcnt[2];
        let wrap = bgcnt & (1 << 13) != 0;
        let (width, height) = if (bgcnt >> 14) & 1 != 0 {
            (1024, 512)
        } else {
            (512, 1024)
        };
        self.affine_walk(true, 2, width, height, wrap, |px, x, y| {
            let ci = u32::from(self.vram.read_u8(Region::BgA, y * width as u32 + x));
            if ci != 0 {
                out[px] = OPAQUE | u32::from(self.bg_palette_color(true, ci) & 0x7FFF);
            }
        });
    }
}
