// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! Software 3D rasterizer.
//!
//! A port of the melonDS software renderer (`GPU3D_Soft.cpp`,
//! GPL-3.0-or-later), which reproduces the hardware's edge slopes,
//! perspective-correct interpolation, depth tests, polygon ID rules for
//! translucency and shadows, edge marking, fog and anti-aliasing.
//!
//! The whole frame is rendered at once (after the buffer swap) into a
//! 256x192 color buffer that the 2D engine A reads as BG0. Colors are
//! stored as `r | g << 8 | b << 16 | a << 24` (6-bit RGB, 5-bit alpha).
//! Each buffer has a second layer holding the pixel below an edge, used
//! by anti-aliasing and edge marking.

mod band;
mod pixel;
mod slope;
mod texture;

use crate::engine_3d::state::{Polygon, RenderState, Vertex};
use crate::vram::{Region, Vram};
use band::Band;

const TEX_SIZE: usize = 512 * 1024;
const PAL_SIZE: usize = 128 * 1024;

const SCANLINE_WIDTH: usize = 258;
const NUM_SCANLINES: usize = 194;
const BUFFER_SIZE: usize = SCANLINE_WIDTH * NUM_SCANLINES;
const FIRST_PIXEL_OFFSET: usize = SCANLINE_WIDTH + 1;

/// Expands a BGR555 color to the 6-bit components used by the renderer.
#[inline]
fn expand6(c: u32) -> (u32, u32, u32) {
    let e = |v: u32| if v != 0 { v + 1 } else { 0 };
    (e((c << 1) & 0x3E), e((c >> 4) & 0x3E), e((c >> 9) & 0x3E))
}

/// Inputs of one frame.
struct Frame<'a> {
    st: &'a RenderState,
    polys: &'a [Polygon],
    verts: &'a [Vertex],
    order: &'a [usize],
    /// Flat copies of texture image and palette VRAM.
    tex: &'a [u8],
    pal: &'a [u8],
}

impl Frame<'_> {
    /// Vertex `i` of polygon `p`.
    #[inline]
    fn vtx(&self, p: &Polygon, i: usize) -> &Vertex {
        &self.verts[p.vertices[i]]
    }
}

/// Rasterizer buffers.
#[derive(Debug)]
pub struct Raster {
    color: Vec<u32>,
    depth: Vec<i32>,
    attr: Vec<u32>,
    /// Per-thread scanline bands (reused between frames).
    bands: Vec<Band>,
    tex: Vec<u8>,
    pal: Vec<u8>,
    /// Scratch buffers for the next frame's texture data.
    tex_next: Vec<u8>,
    pal_next: Vec<u8>,
    /// Whether the buffers hold a rendered frame.
    valid: bool,
}

impl Default for Raster {
    fn default() -> Self {
        Self {
            color: vec![0; BUFFER_SIZE * 2],
            depth: vec![0; BUFFER_SIZE * 2],
            attr: vec![0; BUFFER_SIZE * 2],
            bands: Vec::new(),
            tex: vec![0; TEX_SIZE],
            pal: vec![0; PAL_SIZE],
            tex_next: vec![0; TEX_SIZE],
            pal_next: vec![0; PAL_SIZE],
            valid: false,
        }
    }
}

impl Raster {
    /// One rendered line (256 pixels, layer 0).
    pub fn line(&self, y: usize) -> &[u32] {
        let start = FIRST_PIXEL_OFFSET + y.min(191) * SCANLINE_WIDTH;
        &self.color[start..start + 256]
    }

    /// Renders a frame. `order` lists the polygons to draw (indices into
    /// `polys`), sorted as the hardware does. When `changed` is false
    /// (same polygons and registers as the last frame) and the texture
    /// data is unchanged too, the previous frame is kept.
    ///
    /// ```text
    /// from: Gpu::render_3d_frame()
    ///   |
    ///   v
    /// render(state, polys, order, verts, vram, changed)
    ///   1. snapshot texture image / palette VRAM; changed?
    ///   2. nothing changed -> keep the last frame
    ///   3. render_frame():
    ///        clear_buffers()      clear color/depth/attr (or rear plane)
    ///        bands in parallel    Band::render: polygon edges (slope.rs),
    ///                             spans, texturing (texture.rs), depth
    ///                             test, blending (pixel.rs)
    ///        scanline_final_pass  edge marking, fog, anti-aliasing
    ///   |
    ///   v
    /// to:   line(y) is read by render_bg_3d() while composing BG0
    /// ```
    pub fn render(
        &mut self,
        st: &RenderState,
        polys: &[Polygon],
        order: &[usize],
        verts: &[Vertex],
        vram: &Vram,
        changed: bool,
    ) {
        vram.copy_region(Region::TexImage, &mut self.tex_next);
        vram.copy_region(Region::TexPal, &mut self.pal_next);
        let textures_changed = self.tex_next != self.tex || self.pal_next != self.pal;
        if textures_changed {
            std::mem::swap(&mut self.tex, &mut self.tex_next);
            std::mem::swap(&mut self.pal, &mut self.pal_next);
        }
        if self.valid && !changed && !textures_changed {
            return;
        }
        self.valid = true;

        let (tex, pal) = (std::mem::take(&mut self.tex), std::mem::take(&mut self.pal));
        let f = Frame {
            st,
            polys,
            verts,
            order,
            tex: &tex,
            pal: &pal,
        };
        self.render_frame(&f);
        self.tex = tex;
        self.pal = pal;
    }

    /// Clears the buffers, renders the bands in parallel, then the final pass.
    fn render_frame(&mut self, f: &Frame) {
        self.clear_buffers(f);

        // Scanline bands rendered in parallel. Each band owns a copy of its
        // rows; polygon edges are set up directly at the band's first line.
        let threads = std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .clamp(1, 8);
        let lines = 192_usize.div_ceil(threads);
        self.bands.resize_with(threads, Band::default);
        for (i, band) in self.bands.iter_mut().enumerate() {
            let y0 = (i * lines).min(192);
            let y1 = ((i + 1) * lines).min(192);
            band.load(&self.color, &self.depth, &self.attr, y0, y1);
        }
        std::thread::scope(|scope| {
            let (first, rest) = self
                .bands
                .split_first_mut()
                .unwrap_or_else(|| unreachable!());
            for band in rest {
                scope.spawn(|| band.render(f));
            }
            first.render(f);
        });
        for band in &self.bands {
            band.store(&mut self.color, &mut self.depth, &mut self.attr);
        }

        // Edge marking, fog and anti-aliasing only read the finished frame.
        for y in 0..192 {
            self.scanline_final_pass(f.st, y);
        }
    }

    /// Clears color/depth/attributes to CLEAR_COLOR/DEPTH, or loads the rear-plane bitmap.
    fn clear_buffers(&mut self, f: &Frame) {
        let st = f.st;
        let clear_z = ((st.clear_attr2 & 0x7FFF) * 0x200 + 0x1FF) as i32;
        let poly_id = st.clear_attr1 & 0x3F00_0000;

        // Borders (outside the 256x192 area) only matter for edge marking.
        self.color[..BUFFER_SIZE].fill(0);
        self.depth[..BUFFER_SIZE].fill(clear_z);
        self.attr[..BUFFER_SIZE].fill(poly_id);

        if st.disp_cnt & (1 << 14) != 0 {
            // Rear-plane bitmap from texture slots 2 (color) and 3 (depth).
            let mut yoff = ((st.clear_attr2 >> 24) & 0xFF) as u8;
            for y in 0..192 {
                let mut xoff = ((st.clear_attr2 >> 16) & 0xFF) as u8;
                for x in 0..256 {
                    let ofs = (u32::from(yoff) << 9) + (u32::from(xoff) << 1);
                    let rd = |a: u32| {
                        let a = a as usize;
                        u32::from(u16::from_le_bytes([f.tex[a], f.tex[a + 1]]))
                    };
                    let val2 = rd(0x40000 + ofs);
                    let val3 = rd(0x60000 + ofs);
                    let (r, g, b) = expand6(val2);
                    let a = if val2 & 0x8000 != 0 { 0x1F00_0000 } else { 0 };
                    let addr = FIRST_PIXEL_OFFSET + y * SCANLINE_WIDTH + x;
                    self.color[addr] = r | (g << 8) | (b << 16) | a;
                    self.depth[addr] = ((val3 & 0x7FFF) * 0x200 + 0x1FF) as i32;
                    self.attr[addr] = poly_id | (val3 & 0x8000);
                    xoff = xoff.wrapping_add(1);
                }
                yoff = yoff.wrapping_add(1);
            }
        } else {
            let (r, g, b) = expand6(st.clear_attr1);
            let a = (st.clear_attr1 >> 16) & 0x1F;
            let color = r | (g << 8) | (b << 16) | (a << 24);
            let attr = poly_id | (st.clear_attr1 & 0x8000);
            for y in 0..192 {
                let start = FIRST_PIXEL_OFFSET + y * SCANLINE_WIDTH;
                self.color[start..start + 256].fill(color);
                self.attr[start..start + 256].fill(attr);
            }
        }
    }

    /// Blends a translucent pixel over the destination (when DISP3DCNT bit 3 enables blending).
    fn alpha_blend(st: &RenderState, src: u32, dst: u32, mut alpha: u32) -> u32 {
        let mut dst_alpha = dst >> 24;
        if dst_alpha == 0 {
            return src;
        }
        let (mut r, mut g, mut b) = (src & 0x3F, (src >> 8) & 0x3F, (src >> 16) & 0x3F);
        if st.disp_cnt & (1 << 3) != 0 {
            let (dr, dg, db) = (dst & 0x3F, (dst >> 8) & 0x3F, (dst >> 16) & 0x3F);
            alpha += 1;
            r = (r * alpha + dr * (32 - alpha)) >> 5;
            g = (g * alpha + dg * (32 - alpha)) >> 5;
            b = (b * alpha + db * (32 - alpha)) >> 5;
            alpha -= 1;
        }
        if alpha > dst_alpha {
            dst_alpha = alpha;
        }
        r | (g << 8) | (b << 16) | (dst_alpha << 24)
    }

    /// Fog density for the pixel at `addr` from its depth and the fog table.
    fn fog_density(&self, st: &RenderState, addr: usize) -> u32 {
        let z = self.depth[addr] as u32;
        let (id, frac) = if z < st.fog_offset {
            (0, 0)
        } else {
            let z = ((z - st.fog_offset) >> 2) << st.fog_shift;
            let id = (z >> 17) as usize;
            if id >= 32 { (32, 0) } else { (id, z & 0x1FFFF) }
        };
        let d = (u32::from(st.fog_density[id]) * (0x20000 - frac)
            + u32::from(st.fog_density[id + 1]) * frac)
            >> 17;
        if d >= 127 { 128 } else { d }
    }

    /// Mixes the fog color into the pixel at `addr` (pixels with the fog attribute bit only).
    fn apply_fog(&mut self, st: &RenderState, addr: usize) {
        if self.attr[addr] & (1 << 15) == 0 {
            return;
        }
        let density = self.fog_density(st, addr);
        let src = self.color[addr];
        let (mut r, mut g, mut b) = (src & 0x3F, (src >> 8) & 0x3F, (src >> 16) & 0x3F);
        let mut a = (src >> 24) & 0x1F;
        if st.disp_cnt & (1 << 6) == 0 {
            let (fr, fg, fb) = expand6(st.fog_color);
            r = (fr * density + r * (128 - density)) >> 7;
            g = (fg * density + g * (128 - density)) >> 7;
            b = (fb * density + b * (128 - density)) >> 7;
        }
        let fa = (st.fog_color >> 16) & 0x1F;
        a = (fa * density + a * (128 - density)) >> 7;
        self.color[addr] = r | (g << 8) | (b << 16) | (a << 24);
    }

    /// Edge marking, fog and anti-aliasing of one finished line.
    fn scanline_final_pass(&mut self, st: &RenderState, y: i32) {
        let row = FIRST_PIXEL_OFFSET + y as usize * SCANLINE_WIDTH;

        if st.disp_cnt & (1 << 5) != 0 {
            for x in 0..256 {
                let addr = row + x;
                let attr = self.attr[addr];
                if attr & 0xF == 0 {
                    continue;
                }
                let id = attr >> 24;
                let z = self.depth[addr];
                let differs = |a: usize| id != self.attr[a] >> 24 && z < self.depth[a];
                if differs(addr - 1)
                    || differs(addr + 1)
                    || differs(addr - SCANLINE_WIDTH)
                    || differs(addr + SCANLINE_WIDTH)
                {
                    let (r, g, b) = expand6(u32::from(st.edge_table[(id >> 3) as usize & 7]));
                    self.color[addr] = r | (g << 8) | (b << 16) | (self.color[addr] & 0xFF00_0000);
                    self.attr[addr] = (self.attr[addr] & 0xFFFF_E0FF) | 0x1000;
                }
            }
        }

        if st.disp_cnt & (1 << 7) != 0 {
            for x in 0..256 {
                let addr = row + x;
                let attr = self.attr[addr];
                self.apply_fog(st, addr);
                if attr & 0xF != 0 {
                    self.apply_fog(st, addr + BUFFER_SIZE);
                }
            }
        }

        if st.disp_cnt & (1 << 4) != 0 {
            for x in 0..256 {
                let addr = row + x;
                let attr = self.attr[addr];
                if attr & 0xF == 0 {
                    continue;
                }
                let mut coverage = (attr >> 8) & 0x1F;
                if coverage == 0x1F {
                    continue;
                }
                let bot = self.color[addr + BUFFER_SIZE];
                if coverage == 0 {
                    self.color[addr] = bot;
                    continue;
                }
                let top = self.color[addr];
                let ch = |c: u32, s: u32| (c >> s) & 0x3F;
                let (mut r, mut g, mut b) = (ch(top, 0), ch(top, 8), ch(top, 16));
                let mut a = (top >> 24) & 0x1F;
                let bot_a = (bot >> 24) & 0x1F;
                coverage += 1;
                if bot_a > 0 {
                    r = (r * coverage + ch(bot, 0) * (32 - coverage)) >> 5;
                    g = (g * coverage + ch(bot, 8) * (32 - coverage)) >> 5;
                    b = (b * coverage + ch(bot, 16) * (32 - coverage)) >> 5;
                }
                a = (a * coverage + bot_a * (32 - coverage)) >> 5;
                self.color[addr] = r | (g << 8) | (b << 16) | (a << 24);
            }
        }
    }
}
