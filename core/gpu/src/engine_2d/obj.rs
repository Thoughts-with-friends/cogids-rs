//! Sprites (OBJ): normal, affine and bitmap OBJs, plus the OBJ window.
//! See GBATEK "DS Video OBJs".
use super::compose::{LineLayers, OBJ_BITMAP, OBJ_SEMI, OPAQUE, W};
use crate::Gpu;
use crate::vram::Region;

/// OBJ sizes in pixels: `[shape][size] = (width, height)`.
const OBJ_SIZES: [[(u32, u32); 4]; 4] = [
    [(8, 8), (16, 16), (32, 32), (64, 64)],
    [(16, 8), (32, 8), (32, 16), (64, 32)],
    [(8, 16), (8, 32), (16, 32), (32, 64)],
    [(8, 8), (8, 8), (8, 8), (8, 8)], // prohibited shape
];

impl Gpu {
    /// Renders every OBJ crossing `line` into `layers.obj` (and the OBJ
    /// window mask). Lower priority values win, ties go to the lower OAM
    /// index.
    pub(super) fn render_objs(&self, a: bool, line: usize, layers: &mut LineLayers) {
        let oam_base = if a { 0 } else { 0x400 };
        let region = if a { Region::ObjA } else { Region::ObjB };
        let ext_region = if a {
            Region::ObjExtPalA
        } else {
            Region::ObjExtPalB
        };
        let dispcnt = self.engine(a).dispcnt;
        let oam = |off: usize| {
            u16::from_le_bytes([self.oam[oam_base + off], self.oam[oam_base + off + 1]])
        };

        for i in (0..128usize).rev() {
            let attr0 = oam(i * 8);
            let attr1 = oam(i * 8 + 2);
            let attr2 = oam(i * 8 + 4);

            let affine = attr0 & (1 << 8) != 0;
            if !affine && attr0 & (1 << 9) != 0 {
                continue; // OBJ disabled
            }
            let obj_mode = (attr0 >> 10) & 3;
            let (width, height) = OBJ_SIZES[usize::from(attr0 >> 14)][usize::from(attr1 >> 14)];
            let double = affine && attr0 & (1 << 9) != 0;
            let (box_w, box_h) = if double {
                (width * 2, height * 2)
            } else {
                (width, height)
            };

            let y = u32::from(attr0 & 0xFF);
            let row = (line as u32).wrapping_sub(y) & 0xFF;
            if row >= box_h {
                continue;
            }
            let mut x0 = i32::from(attr1 & 0x1FF);
            if x0 >= 256 {
                x0 -= 512;
            }
            let priority = u32::from((attr2 >> 10) & 3);
            let tile = u32::from(attr2 & 0x3FF);
            let pal_bank = u32::from(attr2 >> 12);
            let color_256 = attr0 & (1 << 13) != 0;

            // Affine matrix (identity for normal OBJs, flips folded in).
            let (pa, pb, pc, pd) = if affine {
                let group = usize::from((attr1 >> 9) & 0x1F) * 32;
                (
                    i32::from(oam(group + 6) as i16),
                    i32::from(oam(group + 14) as i16),
                    i32::from(oam(group + 22) as i16),
                    i32::from(oam(group + 30) as i16),
                )
            } else {
                let h = if attr1 & (1 << 12) != 0 { -256 } else { 256 };
                let v = if attr1 & (1 << 13) != 0 { -256 } else { 256 };
                (h, 0, 0, v)
            };

            let half_w = box_w as i32 / 2;
            let half_h = box_h as i32 / 2;
            let ly = row as i32 - half_h;
            for bx in 0..box_w as i32 {
                let sx = x0 + bx;
                if !(0..W as i32).contains(&sx) {
                    continue;
                }
                let lx = bx - half_w;
                let (tx, ty) = if affine {
                    (
                        ((pa * lx + pb * ly) >> 8) + width as i32 / 2,
                        ((pc * lx + pd * ly) >> 8) + height as i32 / 2,
                    )
                } else {
                    let tx = if pa < 0 { width as i32 - 1 - bx } else { bx };
                    let ty = if pd < 0 {
                        height as i32 - 1 - row as i32
                    } else {
                        row as i32
                    };
                    (tx, ty)
                };
                if tx < 0 || ty < 0 || tx >= width as i32 || ty >= height as i32 {
                    continue;
                }
                let (tx, ty) = (tx as u32, ty as u32);

                let color = if obj_mode == 3 {
                    // Bitmap OBJ, 16-bit direct color.
                    if pal_bank == 0 {
                        continue;
                    }
                    let addr = if dispcnt.bitmap_obj_1d {
                        tile * (128 << dispcnt.bitmap_obj_1d_bound as u32) + (ty * width + tx) * 2
                    } else if dispcnt.bitmap_obj_square {
                        (tile & 0x1F) * 0x10 + (tile & 0x3E0) * 0x80 + ty * 512 + tx * 2
                    } else {
                        (tile & 0xF) * 0x10 + (tile & 0x3F0) * 0x80 + ty * 256 + tx * 2
                    };
                    let c = self.vram.read_u16(region, addr);
                    if c & 0x8000 == 0 {
                        continue;
                    }
                    OBJ_BITMAP | (pal_bank << 20) | u32::from(c & 0x7FFF)
                } else {
                    let tile_row = (ty / 8)
                        * if dispcnt.tile_obj_1d {
                            width / 8 * if color_256 { 2 } else { 1 }
                        } else {
                            32
                        };
                    let base = if dispcnt.tile_obj_1d {
                        tile << (5 + dispcnt.tile_obj_1d_bound as u32)
                    } else {
                        tile * 32
                    };
                    let ci = if color_256 {
                        let addr = base + (tile_row + (tx / 8) * 2) * 32 + (ty & 7) * 8 + (tx & 7);
                        u32::from(self.vram.read_u8(region, addr))
                    } else {
                        let addr = base + (tile_row + tx / 8) * 32 + (ty & 7) * 4 + (tx & 7) / 2;
                        let byte = self.vram.read_u8(region, addr);
                        u32::from(if tx & 1 != 0 { byte >> 4 } else { byte & 0xF })
                    };
                    if ci == 0 {
                        continue;
                    }
                    let c = if !color_256 {
                        self.obj_palette_color(a, pal_bank * 16 + ci)
                    } else if dispcnt.obj_extended_palette {
                        self.vram.read_u16(ext_region, pal_bank * 512 + ci * 2)
                    } else {
                        self.obj_palette_color(a, ci)
                    };
                    let semi = if obj_mode == 1 { OBJ_SEMI } else { 0 };
                    semi | u32::from(c & 0x7FFF)
                };

                let sx = sx as usize;
                if obj_mode == 2 {
                    layers.obj_window[sx] = true;
                    continue;
                }
                if !dispcnt.display_obj {
                    continue;
                }
                let existing = layers.obj[sx];
                if existing & OPAQUE == 0 || priority <= (existing >> 16) & 3 {
                    layers.obj[sx] = OPAQUE | (priority << 16) | color;
                }
            }
        }
    }
}
