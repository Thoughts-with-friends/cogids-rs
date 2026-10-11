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

use crate::gpu_3d::structs::{Polygon, RenderState, Vertex};
use crate::vram::{Region, Vram};

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

// ===== Interpolation (melonDS `Interpolator`) =====

/// Perspective-correct interpolator along an edge (`DIR = true`, 9-bit
/// precision) or along a span (`DIR = false`, 8-bit precision).
#[derive(Debug, Clone, Copy, Default)]
struct Interp<const DIR: bool> {
    x0: i32,
    xdiff: i32,
    x: i32,
    shift: u32,
    linear: bool,
    wbuffer: bool,
    xrecip_z: i32,
    w0n: i32,
    w0d: i32,
    w1d: i32,
    yfactor: u32,
}

impl<const DIR: bool> Interp<DIR> {
    fn new(x0: i32, x1: i32, w0: i32, w1: i32, wbuffer: bool) -> Self {
        let mut s = Self::default();
        s.setup(x0, x1, w0, w1, wbuffer);
        s
    }

    fn setup(&mut self, x0: i32, x1: i32, w0: i32, w1: i32, wbuffer: bool) {
        self.x0 = x0;
        self.xdiff = x1.wrapping_sub(x0);
        self.wbuffer = wbuffer;
        self.xrecip_z = if self.xdiff != 0 {
            (1 << 22) / self.xdiff
        } else {
            0
        };
        let mask = if DIR { 0x7E } else { 0x7F };
        self.linear = w0 == w1 && w0 & mask == 0 && w1 & mask == 0;
        if DIR {
            // Along edges, the W values are reduced to 15 bits.
            self.w0n = w0 >> 1;
            self.w0d = (w0 + ((w0 & !w1) & 1)) >> 1;
            self.w1d = w1 >> 1;
            self.shift = 9;
        } else {
            self.w0n = w0;
            self.w0d = w0;
            self.w1d = w1;
            self.shift = 8;
        }
    }

    fn set_x(&mut self, x: i32) {
        let x = x - self.x0;
        self.x = x;
        if self.xdiff != 0 && (!self.linear || self.wbuffer) {
            let num = (x.wrapping_mul(self.w0n) as u32) << self.shift;
            let den = x
                .wrapping_mul(self.w0d)
                .wrapping_add((self.xdiff - x).wrapping_mul(self.w1d)) as u32;
            self.yfactor = num.checked_div(den).unwrap_or(0);
        }
    }

    fn interpolate(&self, y0: i32, y1: i32) -> i32 {
        if self.xdiff == 0 || y0 == y1 {
            return y0;
        }
        if self.linear {
            if y0 < y1 {
                y0 + (i64::from(y1 - y0) * i64::from(self.x) / i64::from(self.xdiff)) as i32
            } else {
                y1 + (i64::from(y0 - y1) * i64::from(self.xdiff - self.x) / i64::from(self.xdiff))
                    as i32
            }
        } else if y0 < y1 {
            y0.wrapping_add((((y1 - y0) as u32).wrapping_mul(self.yfactor) >> self.shift) as i32)
        } else {
            let f = (1u32 << self.shift).wrapping_sub(self.yfactor);
            y1.wrapping_add((((y0 - y1) as u32).wrapping_mul(f) >> self.shift) as i32)
        }
    }

    fn interpolate_z(&self, z0: i32, z1: i32) -> i32 {
        if self.xdiff == 0 || z0 == z1 {
            return z0;
        }
        if self.wbuffer {
            if z0 < z1 {
                z0 + ((i64::from(z1 - z0) * i64::from(self.yfactor)) >> self.shift) as i32
            } else {
                let f = (1u32 << self.shift).wrapping_sub(self.yfactor);
                z1 + ((i64::from(z0 - z1) * i64::from(f)) >> self.shift) as i32
            }
        } else {
            let (base, mut disp, factor) = if z0 < z1 {
                (z0, z1 - z0, self.x)
            } else {
                (z1, z0 - z1, self.xdiff - self.x)
            };
            let r = i64::from(self.xrecip_z);
            if DIR {
                let mut shift = 0;
                while disp > 0x3FF {
                    disp >>= 1;
                    shift += 1;
                }
                base.wrapping_add(
                    (((i64::from(disp) * i64::from(factor) * r) >> 22) << shift) as i32,
                )
            } else {
                disp >>= 9;
                base.wrapping_add(((i64::from(disp) * i64::from(factor) * r) >> 13) as i32)
            }
        }
    }
}

// ===== Edge slopes (melonDS `Slope`) =====

/// One polygon edge. `RIGHT` selects the right-edge rounding rules.
#[derive(Debug, Clone, Copy, Default)]
struct Slope<const RIGHT: bool> {
    increment: i32,
    negative: bool,
    x_major: bool,
    interp: Interp<true>,
    x0: i32,
    xmin: i32,
    xmax: i32,
    xlen: i32,
    ylen: i32,
    dx: i32,
    y: i32,
    xcov_incr: i32,
}

impl<const RIGHT: bool> Slope<RIGHT> {
    fn setup_dummy(&mut self, x0: i32, wbuffer: bool) -> i32 {
        self.dx = 0;
        self.x0 = x0;
        self.xmin = x0;
        self.xmax = x0;
        self.increment = 0;
        self.x_major = false;
        self.interp.setup(0, 0, 0, 0, wbuffer);
        self.interp.set_x(0);
        self.xcov_incr = 0;
        x0
    }

    #[expect(clippy::too_many_arguments)]
    fn setup(
        &mut self,
        x0: i32,
        x1: i32,
        y0: i32,
        y1: i32,
        w0: i32,
        w1: i32,
        y: i32,
        wbuffer: bool,
    ) -> i32 {
        self.x0 = x0;
        self.y = y;
        if x1 > x0 {
            self.xmin = x0;
            self.xmax = x1 - 1;
            self.negative = false;
        } else if x1 < x0 {
            self.xmin = x1;
            self.xmax = x0 - 1;
            self.negative = true;
        } else {
            self.xmin = x0;
            self.xmax = x0;
            self.negative = false;
        }
        self.xlen = self.xmax + 1 - self.xmin;
        self.ylen = y1 - y0;

        self.increment = if self.ylen == 0 {
            0
        } else if self.ylen == self.xlen && self.xlen != 1 {
            0x40000
        } else {
            let yrecip = (1 << 18) / self.ylen;
            ((x1 - x0).wrapping_mul(yrecip)).wrapping_abs()
        };
        self.x_major = self.increment > 0x40000;

        self.dx = if RIGHT {
            if self.x_major {
                if self.negative {
                    0x20000 + 0x40000
                } else {
                    self.increment - 0x20000
                }
            } else if self.increment != 0 {
                if self.negative { 0x40000 } else { 0 }
            } else {
                0
            }
        } else if self.x_major {
            if self.negative {
                (self.increment - 0x20000) + 0x40000
            } else {
                0x20000
            }
        } else if self.increment != 0 {
            if self.negative { 0x40000 } else { 0 }
        } else {
            0
        };
        self.dx = self.dx.wrapping_add((y - y0).wrapping_mul(self.increment));

        let x = self.x_val();
        let offset = i32::from(self.increment >= 0x40000 && (RIGHT ^ self.negative));
        self.interp.setup(y0 - offset, y1 - offset, w0, w1, wbuffer);
        self.interp.set_x(y);
        if self.x_major {
            self.xcov_incr = (self.ylen << 10) / self.xlen;
        }
        x
    }

    fn step(&mut self) -> i32 {
        self.dx = self.dx.wrapping_add(self.increment);
        self.y += 1;
        let x = self.x_val();
        self.interp.set_x(self.y);
        x
    }

    fn x_val(&self) -> i32 {
        let ret = if self.negative {
            self.x0 - (self.dx >> 18)
        } else {
            self.x0 + (self.dx >> 18)
        };
        ret.clamp(self.xmin, self.xmax.max(self.xmin))
    }

    /// Edge length on this scanline and anti-aliasing coverage
    /// (bit 31 set: x-major, bits 12-21 start coverage, 0-9 increment).
    fn edge_params(&self, swapped: bool) -> (i32, u32) {
        if self.x_major {
            let mut length = 1;
            if !swapped || RIGHT {
                length = if RIGHT ^ self.negative {
                    (self.dx >> 18) - (self.dx.wrapping_sub(self.increment) >> 18)
                } else {
                    (self.dx.wrapping_add(self.increment) >> 18) - (self.dx >> 18)
                };
            }
            let mut startx = self.dx >> 18;
            if self.negative {
                startx = self.xlen - startx;
            }
            if RIGHT {
                startx = startx - length + 1;
            }
            let startcov = (((startx << 10) + 0x1FF) * self.ylen) / self.xlen;
            let coverage =
                (1 << 31) | (((startcov & 0x3FF) as u32) << 12) | (self.xcov_incr & 0x3FF) as u32;
            (if swapped { 1 } else { length }, coverage)
        } else {
            let coverage = if self.increment == 0 {
                if swapped { 0 } else { 31 }
            } else {
                let mut cov = ((self.dx >> 9) + (self.increment >> 10)) >> 4;
                if (cov >> 5) != (self.dx >> 18) {
                    cov = 31;
                }
                cov &= 0x1F;
                let flip = if swapped {
                    RIGHT ^ self.negative
                } else {
                    !(RIGHT ^ self.negative)
                };
                if flip {
                    cov = 0x1F - cov;
                }
                cov as u32
            };
            (1, coverage)
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct RendererPolygon {
    poly: usize,
    slope_l: Slope<false>,
    slope_r: Slope<true>,
    xl: i32,
    xr: i32,
    cur_vl: usize,
    cur_vr: usize,
    next_vl: usize,
    next_vr: usize,
}

#[derive(Clone, Copy)]
enum DepthTest {
    EqualZ,
    EqualW,
    Less,
    LessFrontFacing,
}

impl DepthTest {
    fn for_poly(p: &Polygon) -> Self {
        if p.attr & (1 << 14) != 0 {
            if p.w_buffer {
                Self::EqualW
            } else {
                Self::EqualZ
            }
        } else if p.facing_view {
            Self::LessFrontFacing
        } else {
            Self::Less
        }
    }

    #[inline]
    fn test(self, dstz: i32, z: i32, dstattr: u32) -> bool {
        match self {
            Self::EqualZ => (dstz.wrapping_sub(z).wrapping_add(0x200) as u32) <= 0x400,
            Self::EqualW => (dstz.wrapping_sub(z).wrapping_add(0xFF) as u32) <= 0x1FE,
            Self::Less => z < dstz,
            Self::LessFrontFacing => {
                // Opaque, back-facing destination: <= instead of <.
                if dstattr & 0x0040_0010 == 0x10 {
                    z <= dstz
                } else {
                    z < dstz
                }
            }
        }
    }
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
    #[inline]
    fn vtx(&self, p: &Polygon, i: usize) -> &Vertex {
        &self.verts[p.vertices[i]]
    }

    /// Texel color (BGR555) and alpha (0-31), melonDS `TextureLookup`.
    fn texture_lookup(&self, texparam: u32, texpal: u32, s: i16, t: i16) -> (u32, u32) {
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

    /// Shades one pixel (melonDS `RenderPixel`).
    fn render_pixel(
        &self,
        p: &Polygon,
        mut vr: u32,
        mut vg: u32,
        mut vb: u32,
        s: i16,
        t: i16,
    ) -> u32 {
        let st = self.st;
        let blend = (p.attr >> 4) & 3;
        let poly_alpha = (p.attr >> 16) & 0x1F;
        let wireframe = poly_alpha == 0;
        let highlight = st.disp_cnt & (1 << 1) != 0;

        if blend == 2 {
            if highlight {
                vg = vr;
                vb = vr;
            } else {
                (vr, vg, vb) = expand6(u32::from(st.toon_table[(vr >> 1) as usize & 31]));
            }
        }

        let (mut r, mut g, mut b, mut a);
        if st.disp_cnt & 1 != 0 && (p.tex_param >> 26) & 7 != 0 {
            let (tcolor, talpha) = self.texture_lookup(p.tex_param, p.tex_palette, s, t);
            let (tr, tg, tb) = expand6(tcolor);
            if blend & 1 != 0 {
                // Decal
                (r, g, b) = match talpha {
                    0 => (vr, vg, vb),
                    31 => (tr, tg, tb),
                    _ => (
                        (tr * talpha + vr * (31 - talpha)) >> 5,
                        (tg * talpha + vg * (31 - talpha)) >> 5,
                        (tb * talpha + vb * (31 - talpha)) >> 5,
                    ),
                };
                a = poly_alpha;
            } else {
                // Modulate
                r = ((tr + 1) * (vr + 1) - 1) >> 6;
                g = ((tg + 1) * (vg + 1) - 1) >> 6;
                b = ((tb + 1) * (vb + 1) - 1) >> 6;
                a = ((talpha + 1) * (poly_alpha + 1) - 1) >> 5;
            }
        } else {
            (r, g, b, a) = (vr, vg, vb, poly_alpha);
        }

        if blend == 2 && highlight {
            let (hr, hg, hb) = expand6(u32::from(st.toon_table[(vr >> 1) as usize & 31]));
            r = (r + hr).min(63);
            g = (g + hg).min(63);
            b = (b + hb).min(63);
        }
        if wireframe {
            a = 31;
        }
        r | (g << 8) | (b << 16) | (a << 24)
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

/// Per-scanline span parameters shared by the three span segments.
struct Span {
    xstart: i32,
    xend: i32,
    l_edgelen: i32,
    r_edgelen: i32,
    l_edgecov: u32,
    r_edgecov: u32,
    l_filledge: bool,
    r_filledge: bool,
    zl: i32,
    zr: i32,
    interp_x: Interp<false>,
    yedge: u32,
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

/// A horizontal band of the frame rendered by one thread: its rows of both
/// buffer layers (each row keeps the one-pixel border of the full buffers)
/// and its own shadow stencil.
#[derive(Debug, Default)]
struct Band {
    color: Vec<u32>,
    depth: Vec<i32>,
    attr: Vec<u32>,
    y0: usize,
    y1: usize,
    /// Size of one layer of this band.
    layer: usize,
    stencil: Vec<u8>,
    prev_is_shadow_mask: bool,
    polys: Vec<RendererPolygon>,
}

impl Band {
    /// Copies rows `y0..y1` of both layers out of the frame buffers.
    fn load(&mut self, color: &[u32], depth: &[i32], attr: &[u32], y0: usize, y1: usize) {
        self.y0 = y0;
        self.y1 = y1;
        self.layer = (y1 - y0) * SCANLINE_WIDTH;
        let n = self.layer * 2;
        self.color.resize(n, 0);
        self.depth.resize(n, 0);
        self.attr.resize(n, 0);
        self.stencil.clear();
        self.stencil.resize(512, 0);
        self.prev_is_shadow_mask = false;
        for k in 0..2 {
            let src = k * BUFFER_SIZE + SCANLINE_WIDTH + y0 * SCANLINE_WIDTH;
            let dst = k * self.layer;
            let len = self.layer;
            self.color[dst..dst + len].copy_from_slice(&color[src..src + len]);
            self.depth[dst..dst + len].copy_from_slice(&depth[src..src + len]);
            self.attr[dst..dst + len].copy_from_slice(&attr[src..src + len]);
        }
    }

    /// Writes the band's rows back.
    fn store(&self, color: &mut [u32], depth: &mut [i32], attr: &mut [u32]) {
        for k in 0..2 {
            let dst = k * BUFFER_SIZE + SCANLINE_WIDTH + self.y0 * SCANLINE_WIDTH;
            let src = k * self.layer;
            let len = self.layer;
            color[dst..dst + len].copy_from_slice(&self.color[src..src + len]);
            depth[dst..dst + len].copy_from_slice(&self.depth[src..src + len]);
            attr[dst..dst + len].copy_from_slice(&self.attr[src..src + len]);
        }
    }

    /// Index of pixel 0 of line `y` (layer 0).
    #[inline]
    fn row(&self, y: i32) -> usize {
        (y as usize - self.y0) * SCANLINE_WIDTH + 1
    }

    fn render(&mut self, f: &Frame) {
        let (y0, y1) = (self.y0 as i32, self.y1 as i32);
        self.polys.clear();
        for &i in f.order {
            let p = &f.polys[i];
            if p.degenerate || p.y_top >= y1 {
                continue;
            }
            // Same visibility rule as `render_scanline`.
            let last = if p.y_bottom == p.y_top {
                p.y_top
            } else {
                p.y_bottom - 1
            };
            if last < y0 {
                continue;
            }
            let mut rp = RendererPolygon {
                poly: i,
                ..RendererPolygon::default()
            };
            Self::setup_polygon(f, &mut rp);
            if p.y_top < y0 && p.y_top != p.y_bottom {
                Self::setup_left_edge(f, &mut rp, y0);
                Self::setup_right_edge(f, &mut rp, y0);
            }
            self.polys.push(rp);
        }
        for y in y0..y1 {
            self.render_scanline(f, y);
        }
    }

    fn next_vertex(p: &Polygon, cur: usize, forward: bool) -> usize {
        let n = p.num_vertices;
        if forward {
            if cur + 1 >= n { 0 } else { cur + 1 }
        } else if cur == 0 {
            n - 1
        } else {
            cur - 1
        }
    }

    fn setup_left_edge(f: &Frame, rp: &mut RendererPolygon, y: i32) {
        let p = &f.polys[rp.poly];
        while y >= f.vtx(p, rp.next_vl).final_position[1] && rp.cur_vl != p.v_bottom {
            rp.cur_vl = rp.next_vl;
            rp.next_vl = Self::next_vertex(p, rp.cur_vl, p.facing_view);
        }
        let (a, b) = (f.vtx(p, rp.cur_vl), f.vtx(p, rp.next_vl));
        rp.xl = rp.slope_l.setup(
            a.final_position[0],
            b.final_position[0],
            a.final_position[1],
            b.final_position[1],
            p.final_w[rp.cur_vl],
            p.final_w[rp.next_vl],
            y,
            p.w_buffer,
        );
    }

    fn setup_right_edge(f: &Frame, rp: &mut RendererPolygon, y: i32) {
        let p = &f.polys[rp.poly];
        while y >= f.vtx(p, rp.next_vr).final_position[1] && rp.cur_vr != p.v_bottom {
            rp.cur_vr = rp.next_vr;
            rp.next_vr = Self::next_vertex(p, rp.cur_vr, !p.facing_view);
        }
        let (a, b) = (f.vtx(p, rp.cur_vr), f.vtx(p, rp.next_vr));
        rp.xr = rp.slope_r.setup(
            a.final_position[0],
            b.final_position[0],
            a.final_position[1],
            b.final_position[1],
            p.final_w[rp.cur_vr],
            p.final_w[rp.next_vr],
            y,
            p.w_buffer,
        );
    }

    fn setup_polygon(f: &Frame, rp: &mut RendererPolygon) {
        let p = &f.polys[rp.poly];
        rp.cur_vl = p.v_top;
        rp.cur_vr = p.v_top;
        rp.next_vl = Self::next_vertex(p, p.v_top, p.facing_view);
        rp.next_vr = Self::next_vertex(p, p.v_top, !p.facing_view);

        if p.y_bottom == p.y_top {
            // Flat polygon: span from the leftmost to the rightmost vertex.
            let x = |i: usize| f.vtx(p, i).final_position[0];
            let (mut vtop, mut vbot) = (0, 0);
            for i in [1, p.num_vertices - 1] {
                if x(i) < x(vtop) {
                    vtop = i;
                }
                if x(i) > x(vbot) {
                    vbot = i;
                }
            }
            rp.cur_vl = vtop;
            rp.next_vl = vtop;
            rp.cur_vr = vbot;
            rp.next_vr = vbot;
            rp.xl = rp.slope_l.setup_dummy(x(vtop), p.w_buffer);
            rp.xr = rp.slope_r.setup_dummy(x(vbot), p.w_buffer);
        } else {
            Self::setup_left_edge(f, rp, p.y_top);
            Self::setup_right_edge(f, rp, p.y_top);
        }
    }

    fn render_scanline(&mut self, f: &Frame, y: i32) {
        for i in 0..self.polys.len() {
            let mut rp = self.polys[i];
            let p = &f.polys[rp.poly];
            if y >= p.y_top && (y < p.y_bottom || (y == p.y_top && p.y_bottom == p.y_top)) {
                if p.is_shadow_mask {
                    self.render_shadow_mask_scanline(f, &mut rp, y);
                } else {
                    self.render_polygon_scanline(f, &mut rp, y);
                }
                self.polys[i] = rp;
            }
        }
    }

    /// Advances the edges to scanline `y` and computes the span (shared
    /// by polygons and shadow masks). Also returns whether the span is
    /// swapped (the right edge is left of the left edge).
    fn setup_span(f: &Frame, rp: &mut RendererPolygon, y: i32) -> (Span, bool) {
        let p = &f.polys[rp.poly];
        if p.y_top != p.y_bottom {
            if y >= f.vtx(p, rp.next_vl).final_position[1] && rp.cur_vl != p.v_bottom {
                Self::setup_left_edge(f, rp, y);
            }
            if y >= f.vtx(p, rp.next_vr).final_position[1] && rp.cur_vr != p.v_bottom {
                Self::setup_right_edge(f, rp, y);
            }
        }

        let poly_alpha = (p.attr >> 16) & 0x1F;
        let wireframe = poly_alpha == 0;
        let mut xstart = rp.xl;
        let mut xend = rp.xr;
        let mut wl = rp
            .slope_l
            .interp
            .interpolate(p.final_w[rp.cur_vl], p.final_w[rp.next_vl]);
        let mut wr = rp
            .slope_r
            .interp
            .interpolate(p.final_w[rp.cur_vr], p.final_w[rp.next_vr]);
        let mut zl = rp
            .slope_l
            .interp
            .interpolate_z(p.final_z[rp.cur_vl], p.final_z[rp.next_vl]);
        let mut zr = rp
            .slope_r
            .interp
            .interpolate_z(p.final_z[rp.cur_vr], p.final_z[rp.next_vr]);

        // Right edges are exclusive, except for vertical-ish spans.
        if rp.slope_r.increment == 0 && (rp.slope_l.increment != 0 || xstart != xend) && xend != 0 {
            xend -= 1;
        }

        let fill_all = f.st.disp_cnt & ((1 << 4) | (1 << 5)) != 0
            || (poly_alpha < 31 && f.st.disp_cnt & (1 << 3) != 0)
            || wireframe;
        let last_line = y == p.y_bottom - 1;
        let swapped = xstart > xend;
        let (l_edgelen, l_edgecov, r_edgelen, r_edgecov, l_filledge, r_filledge);
        if swapped {
            let (vlnext, vrnext) = (f.vtx(p, rp.next_vr), f.vtx(p, rp.next_vl));
            let differ = vlnext.final_position[0] != vrnext.final_position[0];
            (l_edgelen, l_edgecov) = rp.slope_r.edge_params(true);
            (r_edgelen, r_edgecov) = rp.slope_l.edge_params(true);
            std::mem::swap(&mut xstart, &mut xend);
            std::mem::swap(&mut wl, &mut wr);
            std::mem::swap(&mut zl, &mut zr);
            let (sl, sr) = (&rp.slope_l, &rp.slope_r);
            if fill_all {
                (l_filledge, r_filledge) = (true, true);
            } else {
                l_filledge = (sr.negative || !sr.x_major) || (last_line && sr.x_major && differ);
                r_filledge = (!sl.negative && sl.x_major)
                    || (!(sl.negative && sl.x_major) && sr.increment == 0)
                    || (last_line && sl.x_major && differ);
            }
        } else {
            let (vlnext, vrnext) = (f.vtx(p, rp.next_vl), f.vtx(p, rp.next_vr));
            let differ = vlnext.final_position[0] != vrnext.final_position[0];
            (l_edgelen, l_edgecov) = rp.slope_l.edge_params(false);
            (r_edgelen, r_edgecov) = rp.slope_r.edge_params(false);
            let (sl, sr) = (&rp.slope_l, &rp.slope_r);
            if fill_all {
                (l_filledge, r_filledge) = (true, true);
            } else {
                l_filledge = ((sl.negative || !sl.x_major) || (last_line && sl.x_major && differ))
                    || (sl.increment == sr.increment && xstart + l_edgelen == xend + 1);
                r_filledge = (!sr.negative && sr.x_major)
                    || sr.increment == 0
                    || (last_line && sr.x_major && differ);
            }
        }

        let yedge = if y == p.y_top {
            0x4
        } else if last_line {
            0x8
        } else {
            0
        };
        (
            Span {
                xstart,
                xend,
                l_edgelen,
                r_edgelen,
                l_edgecov,
                r_edgecov,
                l_filledge,
                r_filledge,
                zl,
                zr,
                interp_x: Interp::new(xstart, xend + 1, wl, wr, p.w_buffer),
                yedge,
            },
            swapped,
        )
    }

    /// Calls `f(x, edge_flags, segment)` for the pixels of the three span
    /// segments (0 = left edge, 1 = interior, 2 = right edge), honoring the
    /// fill rules.
    fn for_span(span: &Span, wireframe: bool, mut f: impl FnMut(i32, u32, u32)) {
        let mut x = span.xstart.max(0);

        let mut xlimit = (span.xstart + span.l_edgelen).min(span.xend + 1).min(256);
        if span.l_filledge {
            while x < xlimit {
                f(x, span.yedge | 0x1, 0);
                x += 1;
            }
        } else {
            x = x.max(xlimit);
        }

        xlimit = (span.xend - span.r_edgelen + 1).min(span.xend + 1).min(256);
        if wireframe && span.yedge == 0 {
            x = x.max(xlimit);
        } else {
            while x < xlimit {
                f(x, span.yedge, 1);
                x += 1;
            }
        }

        xlimit = (span.xend + 1).min(256);
        if span.r_filledge {
            while x < xlimit {
                f(x, span.yedge | 0x2, 2);
                x += 1;
            }
        }
    }

    fn render_shadow_mask_scanline(&mut self, f: &Frame, rp: &mut RendererPolygon, y: i32) {
        let p = &f.polys[rp.poly];
        let depth_test = DepthTest::for_poly(p);
        let poly_alpha = (p.attr >> 16) & 0x1F;
        let wireframe = poly_alpha == 0;

        let sbase = 256 * (y as usize & 1);
        if !self.prev_is_shadow_mask {
            self.stencil[sbase..sbase + 256].fill(0);
        }
        self.prev_is_shadow_mask = true;

        let layer = self.layer;
        let (span, _) = Self::setup_span(f, rp, y);
        let alpha = if wireframe { 31 } else { poly_alpha };
        if alpha > f.st.alpha_ref {
            let (zl, zr) = (span.zl, span.zr);
            let mut ix = span.interp_x;
            let row = self.row(y);
            let (depth, attr, stencil) = (&self.depth, &self.attr, &mut self.stencil);
            Self::for_span(&span, wireframe, |x, _, _| {
                let addr = row + x as usize;
                ix.set_x(x);
                let z = ix.interpolate_z(zl, zr);
                let dstattr = attr[addr];
                if !depth_test.test(depth[addr], z, dstattr) {
                    stencil[sbase + x as usize] = 1;
                }
                if dstattr & 0xF != 0 {
                    let a2 = addr + layer;
                    if !depth_test.test(depth[a2], z, attr[a2]) {
                        stencil[sbase + x as usize] |= 2;
                    }
                }
            });
        }

        rp.xl = rp.slope_l.step();
        rp.xr = rp.slope_r.step();
    }

    #[expect(clippy::too_many_lines)]
    fn render_polygon_scanline(&mut self, f: &Frame, rp: &mut RendererPolygon, y: i32) {
        let p = &f.polys[rp.poly];
        let mut polyattr = p.attr & 0x3F00_8000;
        if !p.facing_view {
            polyattr |= 1 << 4;
        }
        let poly_alpha = (p.attr >> 16) & 0x1F;
        let wireframe = poly_alpha == 0;
        let depth_test = DepthTest::for_poly(p);
        self.prev_is_shadow_mask = false;

        let layer = self.layer;
        let (span, swapped) = Self::setup_span(f, rp, y);

        // Edge attribute interpolation.
        let (vlcur, vlnext, vrcur, vrnext, istart, iend) = if swapped {
            (
                f.vtx(p, rp.cur_vr),
                f.vtx(p, rp.next_vr),
                f.vtx(p, rp.cur_vl),
                f.vtx(p, rp.next_vl),
                rp.slope_r.interp,
                rp.slope_l.interp,
            )
        } else {
            (
                f.vtx(p, rp.cur_vl),
                f.vtx(p, rp.next_vl),
                f.vtx(p, rp.cur_vr),
                f.vtx(p, rp.next_vr),
                rp.slope_l.interp,
                rp.slope_r.interp,
            )
        };
        let cl: [i32; 3] = core::array::from_fn(|k| {
            istart.interpolate(vlcur.final_color[k], vlnext.final_color[k])
        });
        let tl: [i32; 2] = core::array::from_fn(|k| {
            istart.interpolate(
                i32::from(vlcur.texcoords[k]),
                i32::from(vlnext.texcoords[k]),
            )
        });
        let cr: [i32; 3] =
            core::array::from_fn(|k| iend.interpolate(vrcur.final_color[k], vrnext.final_color[k]));
        let tr: [i32; 2] = core::array::from_fn(|k| {
            iend.interpolate(
                i32::from(vrcur.texcoords[k]),
                i32::from(vrnext.texcoords[k]),
            )
        });

        let aa = f.st.disp_cnt & (1 << 4) != 0;
        let alpha_ref = f.st.alpha_ref;
        let row = self.row(y);
        let sbase = 256 * (y as usize & 1);
        let (zl, zr) = (span.zl, span.zr);
        let mut ix = span.interp_x;

        let start_cov = |c: u32| {
            if c & (1 << 31) != 0 {
                let v = (c >> 12) & 0x3FF;
                if v == 0x3FF { 0 } else { v }
            } else {
                0
            }
        };
        let mut xcov = start_cov(span.l_edgecov);
        let mut right_started = false;

        Self::for_span(&span, wireframe, |x, edge, seg| {
            if seg == 2 && !right_started {
                right_started = true;
                xcov = start_cov(span.r_edgecov);
            }
            let mut addr = row + x as usize;
            let mut dstattr = self.attr[addr];

            if p.is_shadow {
                let stencil = self.stencil[sbase + x as usize];
                if stencil == 0 {
                    return;
                }
                if stencil & 1 == 0 {
                    addr += layer;
                }
                if stencil & 2 == 0 {
                    dstattr &= !0xF;
                }
            }

            ix.set_x(x);
            let mut z = ix.interpolate_z(zl, zr);

            if !depth_test.test(self.depth[addr], z, dstattr) {
                if dstattr & 0xF == 0 || addr >= layer {
                    return;
                }
                addr += layer;
                dstattr = self.attr[addr];
                if !depth_test.test(self.depth[addr], z, dstattr) {
                    return;
                }
            }

            let vr = ix.interpolate(cl[0], cr[0]) as u32;
            let vg = ix.interpolate(cl[1], cr[1]) as u32;
            let vb = ix.interpolate(cl[2], cr[2]) as u32;
            let s = ix.interpolate(tl[0], tr[0]) as i16;
            let t = ix.interpolate(tl[1], tr[1]) as i16;

            let color = f.render_pixel(
                p,
                (vr >> 3) & 0x3F,
                (vg >> 3) & 0x3F,
                (vb >> 3) & 0x3F,
                s,
                t,
            );
            let alpha = color >> 24;
            if alpha <= alpha_ref {
                return;
            }

            if alpha == 31 {
                let mut attr = polyattr | edge;
                // Interior pixels only take part in AA on the top/bottom line.
                if aa && (seg != 1 || attr & 0xF != 0) {
                    let cov = match seg {
                        0 => {
                            let c = span.l_edgecov;
                            if c & (1 << 31) != 0 {
                                let v = (xcov >> 5).min(31);
                                xcov += c & 0x3FF;
                                v
                            } else {
                                c
                            }
                        }
                        2 => {
                            let c = span.r_edgecov;
                            if c & (1 << 31) != 0 {
                                let v = 0x1F_u32.saturating_sub(xcov >> 5);
                                xcov += c & 0x3FF;
                                v
                            } else {
                                c
                            }
                        }
                        _ => 0x1F,
                    };
                    attr |= cov << 8;
                    if addr < layer {
                        self.color[addr + layer] = self.color[addr];
                        self.depth[addr + layer] = self.depth[addr];
                        self.attr[addr + layer] = self.attr[addr];
                    }
                }
                self.depth[addr] = z;
                self.color[addr] = color;
                self.attr[addr] = attr;
            } else {
                if p.attr & (1 << 11) == 0 {
                    z = -1;
                }
                self.plot_translucent_pixel(f.st, addr, color, z, polyattr, p.is_shadow);
                if dstattr & 0xF != 0 && addr < layer {
                    self.plot_translucent_pixel(
                        f.st,
                        addr + layer,
                        color,
                        z,
                        polyattr,
                        p.is_shadow,
                    );
                }
            }
        });

        rp.xl = rp.slope_l.step();
        rp.xr = rp.slope_r.step();
    }

    fn plot_translucent_pixel(
        &mut self,
        st: &RenderState,
        addr: usize,
        color: u32,
        z: i32,
        polyattr: u32,
        shadow: bool,
    ) {
        let dstattr = self.attr[addr];
        let mut attr = (polyattr & 0xE0F0)
            | ((polyattr >> 8) & 0xFF_0000)
            | (1 << 22)
            | (dstattr & 0xFF00_1F0F);

        if shadow {
            if dstattr & (1 << 22) != 0 {
                if dstattr & 0x007F_0000 == attr & 0x007F_0000 {
                    return;
                }
            } else if dstattr & 0x3F00_0000 == polyattr & 0x3F00_0000 {
                return;
            }
        } else if dstattr & 0x007F_0000 == attr & 0x007F_0000 {
            // Same translucent polygon ID already drawn here.
            return;
        }

        if dstattr & (1 << 15) == 0 {
            attr &= !(1 << 15);
        }

        let blended = Raster::alpha_blend(st, color, self.color[addr], color >> 24);
        if z != -1 {
            self.depth[addr] = z;
        }
        self.color[addr] = blended;
        self.attr[addr] = attr;
    }
}
