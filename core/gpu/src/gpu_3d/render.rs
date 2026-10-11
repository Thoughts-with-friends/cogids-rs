// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! 3D engine registers (0x04000060, 0x04000320-0x040003BF,
//! 0x04000600-0x040006A3) and the VBlank buffer swap.
//!
//! Follows melonDS `GPU3D::Read32/Write*` and `GPU3D::VBlank`.

use crate::gpu_3d::structs::Gpu3D;

impl Gpu3D {
    /// Whether `address` is a 3D register handled by [`Self::write_reg`] /
    /// [`Self::read_reg`]. (GXSTAT, GXFIFO and the command ports are
    /// handled by the emulator, which drives the FIFO.)
    pub const fn is_3d_reg(address: u32) -> bool {
        matches!(address,
            0x0400_0320..=0x0400_03BF
            | 0x0400_0604..=0x0400_0607
            | 0x0400_0610..=0x0400_0611
            | 0x0400_0620..=0x0400_06A3)
    }

    /// Writes the bytes of `value` selected by `mask` to the word at
    /// `address & !3`.
    pub fn write_reg(&mut self, address: u32, value: u32, mask: u32) {
        let word = address & !3;
        let merge = |old: u32| (old & !mask) | (value & mask);
        match word {
            0x0400_0330..=0x0400_033F => {
                let i = ((word - 0x0400_0330) >> 1) as usize;
                let old = u32::from(self.edge_table[i]) | (u32::from(self.edge_table[i + 1]) << 16);
                let v = merge(old);
                self.edge_table[i] = v as u16;
                self.edge_table[i + 1] = (v >> 16) as u16;
            }
            0x0400_0340 => self.alpha_ref_val = merge(self.alpha_ref_val) & 0x1F,
            0x0400_0350 => self.clear_attr1 = merge(self.clear_attr1),
            0x0400_0354 => self.clear_attr2 = merge(self.clear_attr2),
            0x0400_0358 => self.fog_color = merge(self.fog_color),
            0x0400_035C => self.fog_offset = merge(self.fog_offset) & 0x7FFF,
            0x0400_0360..=0x0400_037F => {
                let i = (word - 0x0400_0360) as usize;
                let old =
                    u32::from_le_bytes(self.fog_density[i..i + 4].try_into().unwrap_or_default());
                let v = merge(old).to_le_bytes();
                for (d, b) in self.fog_density[i..i + 4].iter_mut().zip(v) {
                    *d = b & 0x7F;
                }
            }
            0x0400_0380..=0x0400_03BF => {
                let i = ((word - 0x0400_0380) >> 1) as usize;
                let old = u32::from(self.toon_table[i]) | (u32::from(self.toon_table[i + 1]) << 16);
                let v = merge(old);
                self.toon_table[i] = v as u16;
                self.toon_table[i + 1] = (v >> 16) as u16;
            }
            0x0400_0610 if mask & 0xFFFF != 0 => {
                let v = merge(0) & 0x7FFF;
                self.zero_dot_w_limit = (v * 0x200 + 0x1FF) as i32;
            }
            _ => {}
        }
    }

    /// 32-bit read of the 3D register word at `address & !3` (0 if
    /// unmapped / write-only).
    pub fn read_reg(&self, address: u32) -> u32 {
        let word = address & !3;
        match word {
            // RDLINES_COUNT: the renderer never runs out of time.
            0x0400_0320 => 46,
            0x0400_0604 => self.num_polygons as u32 | ((self.num_vertices as u32) << 16),
            0x0400_0620..=0x0400_062F => self.pos_test_result[((word >> 2) & 3) as usize] as u32,
            0x0400_0630..=0x0400_0637 => {
                let i = ((word - 0x0400_0630) >> 1) as usize;
                let lo = self.vec_test_result[i] as u16;
                let hi = self.vec_test_result.get(i + 1).map_or(0, |&v| v as u16);
                u32::from(lo) | (u32::from(hi) << 16)
            }
            0x0400_0640..=0x0400_067F => {
                self.current_clip_matrix()[((word >> 2) & 0xF) as usize] as u32
            }
            0x0400_0680..=0x0400_06A3 => {
                let i = ((word - 0x0400_0680) >> 2) as usize;
                self.vec_matrix[(i / 3) * 4 + i % 3] as u32
            }
            _ => 0,
        }
    }

    // ===== DISP3DCNT / GXSTAT =====

    pub fn get_disp3dcnt(&self) -> u16 {
        self.disp_cnt as u16
    }

    pub fn set_disp3dcnt(&mut self, value: u16) {
        let v = u32::from(value);
        self.disp_cnt = (v & 0x4FFF) | (self.disp_cnt & 0x3000);
        // Bits 12/13 are acknowledged by writing 1.
        self.disp_cnt &= !(v & 0x3000);
    }

    /// GXSTAT without the FIFO bits, which the emulator adds.
    pub fn get_gxstat(&self) -> u32 {
        let fifo = self.gxfifo.len() as u32;
        let busy = !self.gxfifo.is_empty()
            || !self.gxpipe.is_empty()
            || self.swap_buffers
            || self.cycles > 0;
        let test_busy = self
            .gxpipe
            .front()
            .is_some_and(|c| (0x70..=0x72).contains(&c.command));
        let stack_busy = self
            .gxpipe
            .front()
            .is_some_and(|c| c.command == 0x11 || c.command == 0x12);
        (self.gxstat & 0xC000_8002)
            | u32::from(test_busy)
            | ((self.pos_stack_ptr & 0x1F) << 8)
            | ((self.proj_stack_ptr & 1) << 13)
            | (u32::from(stack_busy) << 14)
            | (fifo.min(256) << 16)
            | (u32::from(fifo < 128) << 25)
            | (u32::from(fifo == 0) << 26)
            | (u32::from(busy) << 27)
    }

    /// GXSTAT write: bit 15 acknowledges a matrix stack error, bits 30-31
    /// select the FIFO IRQ mode.
    pub fn write_gxstat(&mut self, value: u32, mask: u32) {
        let value = value & mask;
        if value & 0x8000 != 0 {
            self.gxstat &= !0x8000;
            self.proj_stack_ptr = 0;
            self.tex_stack_ptr = 0;
        }
        if mask & 0xC000_0000 != 0 {
            self.gxstat = (self.gxstat & 0x3FFF_FFFF) | (value & 0xC000_0000);
        }
    }

    /// FIFO IRQ mode (GXSTAT bits 30-31).
    pub const fn gxfifo_irq_mode(&self) -> u32 {
        self.gxstat >> 30
    }

    pub fn get_vert_count(&self) -> u16 {
        self.num_vertices as u16
    }

    pub fn get_poly_count(&self) -> u16 {
        self.num_polygons as u16
    }

    /// VBlank (melonDS `GPU3D::VBlank`): on a pending SWAP_BUFFERS the
    /// polygon list becomes the render list; the rendering registers are
    /// latched every frame.
    pub fn end_of_frame(&mut self) {
        if self.swap_buffers {
            let bank = self.cur_bank;
            let polys = &self.polygon_ram[bank][..self.num_polygons];
            self.render_polygons.clear();
            self.render_polygons
                .extend((0..polys.len()).filter(|&i| !polys[i].translucent));
            self.render_polygons
                .extend((0..polys.len()).filter(|&i| polys[i].translucent));
            // Y-sorting (stable): all polygons, or only the opaque ones
            // when SWAP_BUFFERS bit 0 (manual translucent sort) is set.
            let sorted = if self.flush_attributes & 1 != 0 {
                self.num_opaque_polygons
            } else {
                polys.len()
            };
            self.render_polygons[..sorted].sort_by_key(|&i| polys[i].sort_key);
            self.render_bank = bank;
            self.rend_poly_count = self.num_polygons as i32;
            self.rend_vert_count = self.num_vertices as i32;

            self.cur_bank ^= 1;
            self.num_vertices = 0;
            self.num_polygons = 0;
            self.num_opaque_polygons = 0;
            self.last_strip_polygon = None;
            self.swap_buffers = false;
            self.render_dirty = true;
        }

        let old = self.render.clone();
        let r = &mut self.render;
        r.disp_cnt = self.disp_cnt;
        r.alpha_ref = if self.disp_cnt & (1 << 2) != 0 {
            self.alpha_ref_val
        } else {
            0
        };
        r.clear_attr1 = self.clear_attr1;
        r.clear_attr2 = self.clear_attr2;
        r.fog_color = self.fog_color;
        r.fog_offset = self.fog_offset * 0x200;
        r.fog_shift = (self.disp_cnt >> 8) & 0xF;
        r.fog_density[0] = self.fog_density[0];
        r.fog_density[1..33].copy_from_slice(&self.fog_density);
        r.fog_density[33] = self.fog_density[31];
        r.edge_table = self.edge_table;
        r.toon_table = self.toon_table;
        if *r != old {
            self.render_dirty = true;
        }
    }
}
