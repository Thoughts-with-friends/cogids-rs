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
//!
//! A raster can also work at N times the resolution ([`Raster::with_scale`],
//! used by [`hires`] for display only): vertex positions then come from the
//! sub-pixel `hires_position`, and every size below scales with N.

mod band;
pub mod hires;
mod pixel;
mod slope;
mod texture;

use crate::engine_3d::state::{Polygon, RenderState, Vertex};
use crate::vram::{Region, Vram};
use band::Band;

const TEX_SIZE: usize = 512 * 1024;
const PAL_SIZE: usize = 128 * 1024;

/// Buffer geometry at a resolution scale: the visible `width` x `height`
/// area with a one-pixel border all around (for edge marking).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Dims {
    scale: usize,
    width: usize,
    height: usize,
    /// Row pitch (`width + 2`).
    pitch: usize,
    /// Size of one buffer layer.
    layer: usize,
    /// Index of the visible pixel (0, 0).
    first: usize,
}

impl Dims {
    const fn new(scale: usize) -> Self {
        let (width, height) = (256 * scale, 192 * scale);
        let pitch = width + 2;
        Self {
            scale,
            width,
            height,
            pitch,
            layer: pitch * (height + 2),
            first: pitch + 1,
        }
    }
}

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
    /// Visible width in pixels (256 x scale).
    width: i32,
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
    dims: Dims,
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
    /// Counts the frames actually rendered (a new count means new pixels).
    generation: u64,
}

impl Default for Raster {
    fn default() -> Self {
        Self::with_scale(1)
    }
}

impl Raster {
    /// A raster working at `scale` times the DS resolution (1 = native).
    pub fn with_scale(scale: usize) -> Self {
        let dims = Dims::new(scale.max(1));
        // Only the native raster snapshots VRAM; scaled ones borrow it.
        let vram_size = |n: usize| if dims.scale == 1 { n } else { 0 };
        Self {
            dims,
            color: vec![0; dims.layer * 2],
            depth: vec![0; dims.layer * 2],
            attr: vec![0; dims.layer * 2],
            bands: Vec::new(),
            tex: vec![0; vram_size(TEX_SIZE)],
            pal: vec![0; vram_size(PAL_SIZE)],
            tex_next: vec![0; vram_size(TEX_SIZE)],
            pal_next: vec![0; vram_size(PAL_SIZE)],
            valid: false,
            generation: 0,
        }
    }

    /// The resolution scale.
    pub const fn scale(&self) -> usize {
        self.dims.scale
    }

    /// Frames rendered so far.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// One rendered line (256 x scale pixels, layer 0).
    pub fn line(&self, y: usize) -> &[u32] {
        let d = self.dims;
        let start = d.first + y.min(d.height - 1) * d.pitch;
        &self.color[start..start + d.width]
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
        self.generation += 1;

        let (tex, pal) = (std::mem::take(&mut self.tex), std::mem::take(&mut self.pal));
        let f = Frame {
            st,
            polys,
            verts,
            order,
            tex: &tex,
            pal: &pal,
            width: self.dims.width as i32,
        };
        self.render_frame(&f);
        self.tex = tex;
        self.pal = pal;
    }

    /// Texture image VRAM as last snapshotted (native raster).
    pub(super) fn tex(&self) -> &[u8] {
        &self.tex
    }

    /// Texture palette VRAM as last snapshotted (native raster).
    pub(super) fn pal(&self) -> &[u8] {
        &self.pal
    }

    /// Renders a frame with texture data from elsewhere (a scaled raster
    /// borrows the native one's VRAM snapshot).
    pub(super) fn render_with(
        &mut self,
        st: &RenderState,
        polys: &[Polygon],
        order: &[usize],
        verts: &[Vertex],
        tex: &[u8],
        pal: &[u8],
    ) {
        self.valid = true;
        self.generation += 1;
        let f = Frame {
            st,
            polys,
            verts,
            order,
            tex,
            pal,
            width: self.dims.width as i32,
        };
        self.render_frame(&f);
    }

    /// Clears the buffers, renders the bands in parallel, then the final pass.
    fn render_frame(&mut self, f: &Frame) {
        self.clear_buffers(f);

        // Scanline bands rendered in parallel. Each band owns a copy of its
        // rows; polygon edges are set up directly at the band's first line.
        let threads = std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .clamp(1, 8);
        let d = self.dims;
        let lines = d.height.div_ceil(threads);
        self.bands.resize_with(threads, Band::default);
        for (i, band) in self.bands.iter_mut().enumerate() {
            let y0 = (i * lines).min(d.height);
            let y1 = ((i + 1) * lines).min(d.height);
            band.load(d, &self.color, &self.depth, &self.attr, y0, y1);
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
        for y in 0..d.height {
            self.scanline_final_pass(f.st, y);
        }
    }

    /// Clears color/depth/attributes to CLEAR_COLOR/DEPTH, or loads the rear-plane bitmap.
    fn clear_buffers(&mut self, f: &Frame) {
        let st = f.st;
        let d = self.dims;
        let clear_z = ((st.clear_attr2 & 0x7FFF) * 0x200 + 0x1FF) as i32;
        let poly_id = st.clear_attr1 & 0x3F00_0000;

        // Borders (outside the visible area) only matter for edge marking.
        self.color[..d.layer].fill(0);
        self.depth[..d.layer].fill(clear_z);
        self.attr[..d.layer].fill(poly_id);

        if st.disp_cnt & (1 << 14) != 0 {
            // Rear-plane bitmap from texture slots 2 (color) and 3 (depth),
            // one DS pixel per scale x scale block.
            let yoff0 = ((st.clear_attr2 >> 24) & 0xFF) as u8;
            let xoff0 = ((st.clear_attr2 >> 16) & 0xFF) as u8;
            for y in 0..d.height {
                let yoff = yoff0.wrapping_add((y / d.scale) as u8);
                for x in 0..d.width {
                    let xoff = xoff0.wrapping_add((x / d.scale) as u8);
                    let ofs = (u32::from(yoff) << 9) + (u32::from(xoff) << 1);
                    let rd = |a: u32| {
                        let a = a as usize;
                        u32::from(u16::from_le_bytes([f.tex[a], f.tex[a + 1]]))
                    };
                    let val2 = rd(0x40000 + ofs);
                    let val3 = rd(0x60000 + ofs);
                    let (r, g, b) = expand6(val2);
                    let a = if val2 & 0x8000 != 0 { 0x1F00_0000 } else { 0 };
                    let addr = d.first + y * d.pitch + x;
                    self.color[addr] = r | (g << 8) | (b << 16) | a;
                    self.depth[addr] = ((val3 & 0x7FFF) * 0x200 + 0x1FF) as i32;
                    self.attr[addr] = poly_id | (val3 & 0x8000);
                }
            }
        } else {
            let (r, g, b) = expand6(st.clear_attr1);
            let a = (st.clear_attr1 >> 16) & 0x1F;
            let color = r | (g << 8) | (b << 16) | (a << 24);
            let attr = poly_id | (st.clear_attr1 & 0x8000);
            for y in 0..d.height {
                let start = d.first + y * d.pitch;
                self.color[start..start + d.width].fill(color);
                self.attr[start..start + d.width].fill(attr);
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
    fn scanline_final_pass(&mut self, st: &RenderState, y: usize) {
        let d = self.dims;
        let row = d.first + y * d.pitch;

        if st.disp_cnt & (1 << 5) != 0 {
            for x in 0..d.width {
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
                    || differs(addr - d.pitch)
                    || differs(addr + d.pitch)
                {
                    let (r, g, b) = expand6(u32::from(st.edge_table[(id >> 3) as usize & 7]));
                    self.color[addr] = r | (g << 8) | (b << 16) | (self.color[addr] & 0xFF00_0000);
                    self.attr[addr] = (self.attr[addr] & 0xFFFF_E0FF) | 0x1000;
                }
            }
        }

        if st.disp_cnt & (1 << 7) != 0 {
            for x in 0..d.width {
                let addr = row + x;
                let attr = self.attr[addr];
                self.apply_fog(st, addr);
                if attr & 0xF != 0 {
                    self.apply_fog(st, addr + d.layer);
                }
            }
        }

        if st.disp_cnt & (1 << 4) != 0 {
            for x in 0..d.width {
                let addr = row + x;
                let attr = self.attr[addr];
                if attr & 0xF == 0 {
                    continue;
                }
                let mut coverage = (attr >> 8) & 0x1F;
                if coverage == 0x1F {
                    continue;
                }
                let bot = self.color[addr + d.layer];
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
