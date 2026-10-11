// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! Geometry engine: matrix stack, lighting, vertex transformation, clipping
//! and polygon setup.
//!
//! Ported from melonDS `GPU3D.cpp` (GPL-3.0-or-later); see also GBATEK
//! "DS 3D Matrix Load/Multiply", "DS 3D Polygon Definitions" and
//! "DS 3D Polygon Light Parameters".

use crate::gpu_3d::structs::{Gpu3D, IDENTITY, MAX_POLYGONS, MAX_VERTICES, Mtx, Polygon, Vertex};

/// Sign-extends a 10-bit field (bits `shift..shift+10` of `v`).
#[inline]
const fn sext10(v: u32, shift: u32) -> i16 {
    ((((v >> shift) & 0x3FF) << 6) as u16 as i16) >> 6
}

#[inline]
fn mul(a: i32, b: i32) -> i64 {
    i64::from(a) * i64::from(b)
}

/// `m = s * m` (4x4).
fn mtx_mult_4x4(m: &mut Mtx, s: &[i32]) {
    let t = *m;
    for r in 0..4 {
        for c in 0..4 {
            m[r * 4 + c] =
                ((0..4).map(|k| mul(s[r * 4 + k], t[k * 4 + c])).sum::<i64>() >> 12) as i32;
        }
    }
}

/// `m = s * m` with a 4x3 `s` (implicit last column 0,0,0,1).
fn mtx_mult_4x3(m: &mut Mtx, s: &[i32]) {
    let t = *m;
    for r in 0..4 {
        for c in 0..4 {
            let mut v: i64 = (0..3).map(|k| mul(s[r * 3 + k], t[k * 4 + c])).sum();
            if r == 3 {
                v += mul(0x1000, t[12 + c]);
            }
            m[r * 4 + c] = (v >> 12) as i32;
        }
    }
}

/// `m = s * m` with a 3x3 `s` (only the upper 3 rows change).
fn mtx_mult_3x3(m: &mut Mtx, s: &[i32]) {
    let t = *m;
    for r in 0..3 {
        for c in 0..4 {
            m[r * 4 + c] =
                ((0..3).map(|k| mul(s[r * 3 + k], t[k * 4 + c])).sum::<i64>() >> 12) as i32;
        }
    }
}

fn mtx_scale(m: &mut Mtx, s: &[i32]) {
    for r in 0..3 {
        for c in 0..4 {
            m[r * 4 + c] = (mul(s[r], m[r * 4 + c]) >> 12) as i32;
        }
    }
}

fn mtx_translate(m: &mut Mtx, s: &[i32]) {
    for c in 0..4 {
        let v = mul(s[0], m[c]) + mul(s[1], m[4 + c]) + mul(s[2], m[8 + c]);
        m[12 + c] = m[12 + c].wrapping_add((v >> 12) as i32);
    }
}

fn mtx_load_4x3(m: &mut Mtx, s: &[i32]) {
    *m = [
        s[0], s[1], s[2], 0, //
        s[3], s[4], s[5], 0, //
        s[6], s[7], s[8], 0, //
        s[9], s[10], s[11], 0x1000,
    ];
}

/// Transforms `(x, y, z, 1)` by `m`.
fn transform(v: [i32; 3], m: &Mtx) -> [i32; 4] {
    let v = [
        i64::from(v[0]),
        i64::from(v[1]),
        i64::from(v[2]),
        0x1000_i64,
    ];
    core::array::from_fn(|c| {
        ((v[0] * i64::from(m[c])
            + v[1] * i64::from(m[4 + c])
            + v[2] * i64::from(m[8 + c])
            + v[3] * i64::from(m[12 + c]))
            >> 12) as i32
    })
}

/// melonDS `ClipSegment`: places a vertex where the edge `vin`-`vout`
/// crosses the `plane * w` boundary of component `comp`.
fn clip_segment(comp: usize, plane: i32, attribs: bool, vin: &Vertex, vout: &Vertex) -> Vertex {
    let factor_num = i64::from(vin.position[3]) - i64::from(plane) * i64::from(vin.position[comp]);
    let factor_den = (factor_num
        - (i64::from(vout.position[3]) - i64::from(plane) * i64::from(vout.position[comp])))
        as i32;
    let interp = |a: i32, b: i32| -> i32 {
        if factor_den == 0 {
            return a;
        }
        // 128-bit: the product exceeds 64 bits for extreme geometry.
        (i128::from(a)
            + (i128::from(b) - i128::from(a)) * i128::from(factor_num) / i128::from(factor_den))
            as i32
    };

    let mut out = *vin;
    for i in 0..4 {
        if i != comp {
            out.position[i] = interp(vin.position[i], vout.position[i]);
        }
    }
    out.position[comp] = plane.wrapping_mul(out.position[3]);
    if attribs {
        for i in 0..3 {
            out.color[i] = interp(vin.color[i], vout.color[i]);
        }
        for i in 0..2 {
            out.texcoords[i] =
                interp(i32::from(vin.texcoords[i]), i32::from(vout.texcoords[i])) as i16;
        }
    }
    out.clipped = true;
    out
}

/// melonDS `ClipAgainstPlane`.
fn clip_against_plane(
    comp: usize,
    attribs: bool,
    far_intersect: bool,
    vertices: &mut [Vertex; 10],
    nverts: usize,
    clip_start: usize,
) -> usize {
    let mut temp = [Vertex::default(); 10];
    let mut c = clip_start;
    if clip_start == 2 {
        temp[0] = vertices[0];
        temp[1] = vertices[1];
    }

    for i in clip_start..nverts {
        let prev = if i == 0 { nverts - 1 } else { i - 1 };
        let next = if i + 1 >= nverts { 0 } else { i + 1 };
        let vtx = vertices[i];
        if vtx.position[comp] > vtx.position[3] {
            if comp == 2 && !far_intersect {
                return 0;
            }
            let vprev = vertices[prev];
            if vprev.position[comp] <= vprev.position[3] && c < 10 {
                temp[c] = clip_segment(comp, 1, attribs, &vtx, &vprev);
                c += 1;
            }
            let vnext = vertices[next];
            if vnext.position[comp] <= vnext.position[3] && c < 10 {
                temp[c] = clip_segment(comp, 1, attribs, &vtx, &vnext);
                c += 1;
            }
        } else if c < 10 {
            temp[c] = vtx;
            c += 1;
        }
    }

    let nverts = c;
    c = clip_start;
    for i in clip_start..nverts {
        let prev = if i == 0 { nverts - 1 } else { i - 1 };
        let next = if i + 1 >= nverts { 0 } else { i + 1 };
        let vtx = temp[i];
        if i64::from(vtx.position[comp]) < -i64::from(vtx.position[3]) {
            let vprev = temp[prev];
            if i64::from(vprev.position[comp]) >= -i64::from(vprev.position[3]) && c < 10 {
                vertices[c] = clip_segment(comp, -1, attribs, &vtx, &vprev);
                c += 1;
            }
            let vnext = temp[next];
            if i64::from(vnext.position[comp]) >= -i64::from(vnext.position[3]) && c < 10 {
                vertices[c] = clip_segment(comp, -1, attribs, &vtx, &vnext);
                c += 1;
            }
        } else if c < 10 {
            vertices[c] = vtx;
            c += 1;
        }
    }

    for v in &mut vertices[..c] {
        for col in &mut v.color {
            *col = (*col & !0xFFF) + 0xFFF;
        }
    }
    c
}

/// Clips against the Z, Y and X planes (melonDS `ClipPolygon`).
fn clip_polygon(
    attribs: bool,
    far_intersect: bool,
    vertices: &mut [Vertex; 10],
    nverts: usize,
    clip_start: usize,
) -> usize {
    let n = clip_against_plane(2, attribs, far_intersect, vertices, nverts, clip_start);
    let n = clip_against_plane(1, attribs, far_intersect, vertices, n, clip_start);
    clip_against_plane(0, attribs, far_intersect, vertices, n, clip_start)
}

impl Default for Gpu3D {
    fn default() -> Self {
        Self::new()
    }
}

impl Gpu3D {
    pub fn new() -> Self {
        let mut s = Self {
            cycles: 0,
            gxfifo: Default::default(),
            gxpipe: Default::default(),
            cmd_params: [0; 32],
            cmd_param_count: 0,
            param_count: 0,
            cmd_count: 0,
            total_params: 0,
            current_cmd: 0,
            disp_cnt: 0,
            alpha_ref_val: 0,
            clear_attr1: 0,
            clear_attr2: 0,
            fog_color: 0,
            fog_offset: 0,
            fog_density: [0; 32],
            edge_table: [0; 8],
            toon_table: [0; 32],
            zero_dot_w_limit: 0,
            gxstat: 0,
            matrix_mode: 0,
            proj_matrix: IDENTITY,
            pos_matrix: IDENTITY,
            vec_matrix: IDENTITY,
            tex_matrix: IDENTITY,
            clip_matrix: IDENTITY,
            clip_matrix_dirty: true,
            proj_stack: IDENTITY,
            tex_stack: IDENTITY,
            pos_stack: [IDENTITY; 32],
            vec_stack: [IDENTITY; 32],
            proj_stack_ptr: 0,
            tex_stack_ptr: 0,
            pos_stack_ptr: 0,
            viewport: Default::default(),
            polygon_attr: 0,
            cur_polygon_attr: 0,
            tex_param: 0,
            tex_palette: 0,
            polygon_mode: 0,
            cur_vertex: [0; 3],
            vertex_color: [31; 3],
            normal: [0; 3],
            raw_texcoords: [0; 2],
            texcoords: [0; 2],
            light_direction: [[0; 3]; 4],
            light_color: [[0; 3]; 4],
            spec_recip: [0; 4],
            mat_diffuse: [0; 3],
            mat_ambient: [0; 3],
            mat_specular: [0; 3],
            mat_emission: [0; 3],
            use_shininess_table: false,
            shininess_table: [0; 128],
            pos_test_result: [0; 4],
            vec_test_result: [0; 3],
            temp_vertices: [Vertex::default(); 4],
            vertex_num_in_poly: 0,
            num_consecutive_polygons: 0,
            last_strip_polygon: None,
            vertex_ram: [
                vec![Vertex::default(); MAX_VERTICES],
                vec![Vertex::default(); MAX_VERTICES],
            ],
            polygon_ram: [
                vec![Polygon::default(); MAX_POLYGONS],
                vec![Polygon::default(); MAX_POLYGONS],
            ],
            cur_bank: 0,
            num_vertices: 0,
            num_polygons: 0,
            num_opaque_polygons: 0,
            swap_buffers: false,
            flush_attributes: 0,
            render_polygons: Vec::with_capacity(MAX_POLYGONS),
            render_bank: 1,
            render: Default::default(),
            render_dirty: true,
            rend_poly_count: 0,
            rend_vert_count: 0,
            raster: Default::default(),
        };
        // Viewport 0,0-255,191 (melonDS reset value).
        s.set_viewport(0xBFFF_0000);
        s
    }

    pub fn power_on(&mut self) {
        *self = Self::new();
    }

    fn update_clip_matrix(&mut self) {
        if self.clip_matrix_dirty {
            self.clip_matrix = self.current_clip_matrix();
            self.clip_matrix_dirty = false;
        }
    }

    /// Projection * position matrix (CLIPMTX_RESULT).
    pub(crate) fn current_clip_matrix(&self) -> Mtx {
        if !self.clip_matrix_dirty {
            return self.clip_matrix;
        }
        let mut m = self.proj_matrix;
        mtx_mult_4x4(&mut m, &self.pos_matrix);
        m
    }

    fn set_viewport(&mut self, param: u32) {
        let v = &mut self.viewport;
        v.x0 = (param & 0xFF) as i32;
        v.y0 = (191 - ((param >> 8) & 0xFF) as i32) & 0xFF;
        v.x1 = ((param >> 16) & 0xFF) as i32;
        v.y1 = (191 - (param >> 24) as i32) & 0xFF;
        v.width = (v.x1 - v.x0 + 1) & 0x1FF;
        v.height = (v.y0 - v.y1 + 1) & 0xFF;
    }

    /// Executes one complete GX command whose parameters are in
    /// `cmd_params`.
    #[expect(clippy::too_many_lines)]
    pub fn execute_command(&mut self, cmd: u8) {
        let p = self.cmd_params;
        let p0 = p[0];
        let params: [i32; 32] = p.map(|v| v as i32);
        let mode = self.matrix_mode;
        match cmd {
            0x10 => self.matrix_mode = p0 & 3,
            0x11 => match mode {
                0 => {
                    if self.proj_stack_ptr > 0 {
                        self.gxstat |= 1 << 15;
                    }
                    self.proj_stack = self.proj_matrix;
                    self.proj_stack_ptr = (self.proj_stack_ptr + 1) & 1;
                }
                3 => {
                    if self.tex_stack_ptr > 0 {
                        self.gxstat |= 1 << 15;
                    }
                    self.tex_stack = self.tex_matrix;
                    self.tex_stack_ptr = (self.tex_stack_ptr + 1) & 1;
                }
                _ => {
                    if self.pos_stack_ptr > 30 {
                        self.gxstat |= 1 << 15;
                    }
                    let i = (self.pos_stack_ptr & 0x1F) as usize;
                    self.pos_stack[i] = self.pos_matrix;
                    self.vec_stack[i] = self.vec_matrix;
                    self.pos_stack_ptr = (self.pos_stack_ptr + 1) & 0x3F;
                }
            },
            0x12 => match mode {
                0 => {
                    if self.proj_stack_ptr == 0 {
                        self.gxstat |= 1 << 15;
                    }
                    self.proj_stack_ptr = self.proj_stack_ptr.wrapping_sub(1) & 1;
                    self.proj_matrix = self.proj_stack;
                    self.clip_matrix_dirty = true;
                }
                3 => {
                    if self.tex_stack_ptr == 0 {
                        self.gxstat |= 1 << 15;
                    }
                    self.tex_stack_ptr = self.tex_stack_ptr.wrapping_sub(1) & 1;
                    self.tex_matrix = self.tex_stack;
                }
                _ => {
                    let offset = ((p0 << 26) as i32) >> 26;
                    self.pos_stack_ptr =
                        (self.pos_stack_ptr as i32).wrapping_sub(offset) as u32 & 0x3F;
                    if self.pos_stack_ptr > 30 {
                        self.gxstat |= 1 << 15;
                    }
                    let i = (self.pos_stack_ptr & 0x1F) as usize;
                    self.pos_matrix = self.pos_stack[i];
                    self.vec_matrix = self.vec_stack[i];
                    self.clip_matrix_dirty = true;
                }
            },
            0x13 => match mode {
                0 => self.proj_stack = self.proj_matrix,
                3 => self.tex_stack = self.tex_matrix,
                _ => {
                    let addr = (p0 & 0x1F) as usize;
                    if addr > 30 {
                        self.gxstat |= 1 << 15;
                    }
                    self.pos_stack[addr] = self.pos_matrix;
                    self.vec_stack[addr] = self.vec_matrix;
                }
            },
            0x14 => match mode {
                0 => {
                    self.proj_matrix = self.proj_stack;
                    self.clip_matrix_dirty = true;
                }
                3 => self.tex_matrix = self.tex_stack,
                _ => {
                    let addr = (p0 & 0x1F) as usize;
                    if addr > 30 {
                        self.gxstat |= 1 << 15;
                    }
                    self.pos_matrix = self.pos_stack[addr];
                    self.vec_matrix = self.vec_stack[addr];
                    self.clip_matrix_dirty = true;
                }
            },
            0x15 => match mode {
                0 => {
                    self.proj_matrix = IDENTITY;
                    self.clip_matrix_dirty = true;
                }
                3 => self.tex_matrix = IDENTITY,
                _ => {
                    self.pos_matrix = IDENTITY;
                    if mode == 2 {
                        self.vec_matrix = IDENTITY;
                    }
                    self.clip_matrix_dirty = true;
                }
            },
            0x16..=0x1C => self.matrix_command(cmd, &params),
            0x20 => {
                self.vertex_color = [
                    (p0 & 0x1F) as u8,
                    ((p0 >> 5) & 0x1F) as u8,
                    ((p0 >> 10) & 0x1F) as u8,
                ];
            }
            0x21 => {
                self.normal = [sext10(p0, 0), sext10(p0, 10), sext10(p0, 20)];
                self.calculate_lighting();
            }
            0x22 => {
                self.raw_texcoords = [(p0 & 0xFFFF) as i16, (p0 >> 16) as i16];
                if self.tex_param >> 30 == 1 {
                    let (s, t) = (
                        i32::from(self.raw_texcoords[0]),
                        i32::from(self.raw_texcoords[1]),
                    );
                    let m = &self.tex_matrix;
                    self.texcoords = core::array::from_fn(|i| {
                        ((s.wrapping_mul(m[i]))
                            .wrapping_add(t.wrapping_mul(m[4 + i]))
                            .wrapping_add(m[8 + i])
                            .wrapping_add(m[12 + i])
                            >> 12) as i16
                    });
                } else {
                    self.texcoords = self.raw_texcoords;
                }
            }
            0x23 => {
                self.cur_vertex = [(p0 & 0xFFFF) as i16, (p0 >> 16) as i16, p[1] as i16];
                self.submit_vertex();
            }
            0x24 => {
                self.cur_vertex = [
                    ((p0 & 0x3FF) << 6) as i16,
                    ((p0 & 0x000F_FC00) >> 4) as i16,
                    ((p0 & 0x3FF0_0000) >> 14) as i16,
                ];
                self.submit_vertex();
            }
            0x25 => {
                self.cur_vertex[0] = p0 as i16;
                self.cur_vertex[1] = (p0 >> 16) as i16;
                self.submit_vertex();
            }
            0x26 => {
                self.cur_vertex[0] = p0 as i16;
                self.cur_vertex[2] = (p0 >> 16) as i16;
                self.submit_vertex();
            }
            0x27 => {
                self.cur_vertex[1] = p0 as i16;
                self.cur_vertex[2] = (p0 >> 16) as i16;
                self.submit_vertex();
            }
            0x28 => {
                let v = &mut self.cur_vertex;
                v[0] = v[0].wrapping_add(sext10(p0, 0));
                v[1] = v[1].wrapping_add(sext10(p0, 10));
                v[2] = v[2].wrapping_add(sext10(p0, 20));
                self.submit_vertex();
            }
            0x29 => self.polygon_attr = p0,
            0x2A => self.tex_param = p0,
            0x2B => self.tex_palette = p0 & 0x1FFF,
            0x30 => {
                self.mat_diffuse = [
                    (p0 & 0x1F) as u8,
                    ((p0 >> 5) & 0x1F) as u8,
                    ((p0 >> 10) & 0x1F) as u8,
                ];
                self.mat_ambient = [
                    ((p0 >> 16) & 0x1F) as u8,
                    ((p0 >> 21) & 0x1F) as u8,
                    ((p0 >> 26) & 0x1F) as u8,
                ];
                if p0 & 0x8000 != 0 {
                    self.vertex_color = self.mat_diffuse;
                }
            }
            0x31 => {
                self.mat_specular = [
                    (p0 & 0x1F) as u8,
                    ((p0 >> 5) & 0x1F) as u8,
                    ((p0 >> 10) & 0x1F) as u8,
                ];
                self.mat_emission = [
                    ((p0 >> 16) & 0x1F) as u8,
                    ((p0 >> 21) & 0x1F) as u8,
                    ((p0 >> 26) & 0x1F) as u8,
                ];
                self.use_shininess_table = p0 & 0x8000 != 0;
            }
            0x32 => {
                let l = (p0 >> 30) as usize;
                let dir = [
                    i32::from(sext10(p0, 0)),
                    i32::from(sext10(p0, 10)),
                    i32::from(sext10(p0, 20)),
                ];
                let m = &self.vec_matrix;
                let dot = |c: usize| {
                    dir[0]
                        .wrapping_mul(m[c])
                        .wrapping_add(dir[1].wrapping_mul(m[4 + c]))
                        .wrapping_add(dir[2].wrapping_mul(m[8 + c]))
                };
                for c in 0..3 {
                    self.light_direction[l][c] = ((-(dot(c) >> 12)) << 21 >> 21) as i16;
                }
                let den = -((dot(2) << 9) >> 21) + (1 << 9);
                self.spec_recip[l] = if den == 0 { 0 } else { (1 << 18) / den };
            }
            0x33 => {
                let l = (p0 >> 30) as usize;
                self.light_color[l] = [
                    (p0 & 0x1F) as u8,
                    ((p0 >> 5) & 0x1F) as u8,
                    ((p0 >> 10) & 0x1F) as u8,
                ];
            }
            0x34 => {
                for (chunk, word) in self
                    .shininess_table
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(p)
                {
                    *chunk = word.to_le_bytes();
                }
            }
            0x40 => {
                self.polygon_mode = p0 & 3;
                self.vertex_num_in_poly = 0;
                self.num_consecutive_polygons = 0;
                self.last_strip_polygon = None;
                self.cur_polygon_attr = self.polygon_attr;
            }
            0x41 => {}
            0x50 => {
                self.swap_buffers = true;
                self.flush_attributes = p0 & 3;
            }
            0x60 => self.set_viewport(p0),
            0x70 => self.box_test(&p),
            0x71 => {
                self.cur_vertex = [(p0 & 0xFFFF) as i16, (p0 >> 16) as i16, p[1] as i16];
                self.update_clip_matrix();
                let v = self.cur_vertex.map(i32::from);
                self.pos_test_result = transform(v, &self.clip_matrix);
            }
            0x72 => {
                let n = [
                    i32::from(sext10(p0, 0)),
                    i32::from(sext10(p0, 10)),
                    i32::from(sext10(p0, 20)),
                ];
                let m = &self.vec_matrix;
                for c in 0..3 {
                    let mut r = (n[0]
                        .wrapping_mul(m[c])
                        .wrapping_add(n[1].wrapping_mul(m[4 + c]))
                        .wrapping_add(n[2].wrapping_mul(m[8 + c])))
                        >> 9;
                    if r & 0x1000 != 0 {
                        r |= 0xF000;
                    }
                    self.vec_test_result[c] = r as i16;
                }
            }
            _ => {}
        }
    }

    /// Matrix load/multiply commands 0x16-0x1C.
    fn matrix_command(&mut self, cmd: u8, p: &[i32; 32]) {
        let apply = |m: &mut Mtx| match cmd {
            0x16 => m.copy_from_slice(&p[..16]),
            0x17 => mtx_load_4x3(m, p),
            0x18 => mtx_mult_4x4(m, p),
            0x19 => mtx_mult_4x3(m, p),
            0x1A => mtx_mult_3x3(m, p),
            0x1B => mtx_scale(m, p),
            _ => mtx_translate(m, p),
        };
        match self.matrix_mode {
            0 => {
                apply(&mut self.proj_matrix);
                self.clip_matrix_dirty = true;
            }
            3 => apply(&mut self.tex_matrix),
            mode => {
                apply(&mut self.pos_matrix);
                // SCALE never touches the vector matrix.
                if mode == 2 && cmd != 0x1B {
                    apply(&mut self.vec_matrix);
                }
                self.clip_matrix_dirty = true;
            }
        }
    }

    /// melonDS `CalculateLighting`.
    fn calculate_lighting(&mut self) {
        let n = self.normal.map(i32::from);
        if self.tex_param >> 30 == 2 {
            let m = &self.tex_matrix;
            for i in 0..2 {
                let v = (mul(n[0], m[i]) + mul(n[1], m[4 + i]) + mul(n[2], m[8 + i])) >> 21;
                self.texcoords[i] = self.raw_texcoords[i].wrapping_add(v as i16);
            }
        }

        let m = &self.vec_matrix;
        let nt: [i32; 3] = core::array::from_fn(|c| {
            (n[0]
                .wrapping_mul(m[c])
                .wrapping_add(n[1].wrapping_mul(m[4 + c]))
                .wrapping_add(n[2].wrapping_mul(m[8 + c])))
                << 9
                >> 21
        });

        let mut c = 0_u32;
        let mut buf: [u32; 3] = self.mat_emission.map(|e| u32::from(e) << 14);
        for i in 0..4 {
            if self.cur_polygon_attr & (1 << i) == 0 {
                continue;
            }
            let ld = self.light_direction[i].map(i32::from);
            let lc = self.light_color[i].map(u32::from);
            let mut dot = ((ld[0] * nt[0]) >> 9) + ((ld[1] * nt[1]) >> 9) + ((ld[2] * nt[2]) >> 9);

            let mut shine: i32 = 0;
            if dot > 0 {
                let diff = (dot << 21) >> 21;
                for k in 0..3 {
                    let d = (u32::from(self.mat_diffuse[k]) * lc[k]).wrapping_mul(diff as u32);
                    buf[k] = buf[k].wrapping_add(d & 0xFFFFF);
                }
                dot += nt[2];
                dot = (dot << 21) >> 21;
                dot = ((dot * dot) >> 10) & 0x3FF;
                shine = ((dot * self.spec_recip[i]) >> 8) - (1 << 9);
                if shine < 0 {
                    shine = 0;
                } else {
                    shine = (shine << 18) >> 18;
                    shine = shine.clamp(0, 0x1FF);
                }
            }
            if self.use_shininess_table {
                shine = i32::from(self.shininess_table[(shine >> 2) as usize & 0x7F]) << 1;
            }
            for k in 0..3 {
                let v = (u32::from(self.mat_specular[k]) * shine as u32
                    + (u32::from(self.mat_ambient[k]) << 9))
                    * lc[k];
                buf[k] = buf[k].wrapping_add(v);
            }
            c += 1;
        }
        self.vertex_color = buf.map(|v| (v >> 14).min(31) as u8);
        let _ = c;
    }

    /// melonDS `SubmitVertex`.
    fn submit_vertex(&mut self) {
        self.update_clip_matrix();
        let v = self.cur_vertex.map(i32::from);
        let idx = self.vertex_num_in_poly;
        let mut vt = Vertex {
            position: transform(v, &self.clip_matrix),
            color: self.vertex_color.map(|c| (i32::from(c) << 12) + 0xFFF),
            ..Vertex::default()
        };
        if self.tex_param >> 30 == 3 {
            let m = &self.tex_matrix;
            for i in 0..2 {
                let t = (mul(v[0], m[i]) + mul(v[1], m[4 + i]) + mul(v[2], m[8 + i])) >> 24;
                vt.texcoords[i] = (t as i16).wrapping_add(self.raw_texcoords[i]);
            }
        } else {
            vt.texcoords = self.texcoords;
        }
        self.temp_vertices[idx] = vt;
        self.vertex_num_in_poly += 1;

        match self.polygon_mode {
            0 => {
                if self.vertex_num_in_poly == 3 {
                    self.vertex_num_in_poly = 0;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                }
            }
            1 => {
                if self.vertex_num_in_poly == 4 {
                    self.vertex_num_in_poly = 0;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                }
            }
            2 => {
                if self.num_consecutive_polygons & 1 != 0 {
                    self.temp_vertices.swap(0, 1);
                    self.vertex_num_in_poly = 2;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                    self.temp_vertices[1] = self.temp_vertices[2];
                } else if self.vertex_num_in_poly == 3 {
                    self.vertex_num_in_poly = 2;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                    self.temp_vertices[0] = self.temp_vertices[1];
                    self.temp_vertices[1] = self.temp_vertices[2];
                }
            }
            _ => {
                if self.vertex_num_in_poly == 4 {
                    self.temp_vertices.swap(2, 3);
                    self.vertex_num_in_poly = 2;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                    self.temp_vertices[0] = self.temp_vertices[3];
                    self.temp_vertices[1] = self.temp_vertices[2];
                }
            }
        }
    }

    /// melonDS `SubmitPolygon`: culling, clipping, viewport transform and
    /// polygon setup.
    #[expect(clippy::too_many_lines)]
    fn submit_polygon(&mut self) {
        let mut nverts = if self.polygon_mode & 1 != 0 { 4 } else { 3 };
        let attr = self.cur_polygon_attr;

        // Culling (facing test in homogeneous coordinates).
        let [v0, v1, v2, _] = self.temp_vertices;
        // Differences wrap in 32 bits, products are 64-bit (as in melonDS).
        let d =
            |a: &Vertex, b: &Vertex, i: usize| i64::from(a.position[i].wrapping_sub(b.position[i]));
        let cross = |a0: i64, b0: i64, a1: i64, b1: i64| (a0 * b0).wrapping_sub(a1 * b1);
        let mut nx = cross(
            d(&v0, &v1, 1),
            d(&v2, &v1, 3),
            d(&v0, &v1, 3),
            d(&v2, &v1, 1),
        );
        let mut ny = cross(
            d(&v0, &v1, 3),
            d(&v2, &v1, 0),
            d(&v0, &v1, 0),
            d(&v2, &v1, 3),
        );
        let mut nz = cross(
            d(&v0, &v1, 0),
            d(&v2, &v1, 1),
            d(&v0, &v1, 1),
            d(&v2, &v1, 0),
        );
        let big = |n: i64| (n >> 31) ^ (n >> 63) != 0;
        while big(nx) || big(ny) || big(nz) {
            nx >>= 4;
            ny >>= 4;
            nz >>= 4;
        }
        let dot = i64::from(v1.position[0])
            .wrapping_mul(nx)
            .wrapping_add(i64::from(v1.position[1]).wrapping_mul(ny))
            .wrapping_add(i64::from(v1.position[3]).wrapping_mul(nz));
        let facing_view = dot <= 0;
        if (dot < 0 && attr & (1 << 7) == 0) || (dot > 0 && attr & (1 << 6) == 0) {
            self.last_strip_polygon = None;
            return;
        }

        // Strips: reuse the two shared vertices of the previous polygon.
        let bank = self.cur_bank;
        let mut clipped = [Vertex::default(); 10];
        let mut clip_start = 0;
        let mut reused = [0usize; 2];
        let mut last_poly_verts = 0;
        if self.polygon_mode >= 2
            && let Some(last) = self.last_strip_polygon
        {
            let (id0, id1) = if self.polygon_mode == 2 {
                last_poly_verts = 3;
                if self.num_consecutive_polygons & 1 != 0 {
                    (2, 1)
                } else {
                    (0, 2)
                }
            } else {
                last_poly_verts = 4;
                (3, 2)
            };
            let lp = &self.polygon_ram[bank][last];
            let vr = &self.vertex_ram[bank];
            if lp.num_vertices == last_poly_verts
                && !vr[lp.vertices[id0]].clipped
                && !vr[lp.vertices[id1]].clipped
            {
                reused = [lp.vertices[id0], lp.vertices[id1]];
                clipped[0] = vr[reused[0]];
                clipped[1] = vr[reused[1]];
                clip_start = 2;
            }
        }
        clipped[clip_start..nverts].copy_from_slice(&self.temp_vertices[clip_start..nverts]);

        nverts = clip_polygon(
            true,
            attr & (1 << 12) != 0,
            &mut clipped,
            nverts,
            clip_start,
        );
        if nverts == 0 {
            self.last_strip_polygon = None;
            return;
        }
        if self.num_polygons >= MAX_POLYGONS || self.num_vertices + nverts > MAX_VERTICES {
            self.last_strip_polygon = None;
            self.disp_cnt |= 1 << 13;
            return;
        }

        // Viewport transform (32-bit divider, as on hardware).
        let vp = self.viewport;
        for vtx in &mut clipped[clip_start..nverts] {
            vtx.position[3] &= 0x00FF_FFFF;
            let w = vtx.position[3] as u32;
            let (pos_x, pos_y) = if w == 0 {
                (0, 0)
            } else {
                let mut px = (vtx.position[0] as u32).wrapping_add(w);
                let mut py = (vtx.position[1] as u32).wrapping_neg().wrapping_add(w);
                let mut den = w;
                if w > 0xFFFF {
                    px >>= 1;
                    py >>= 1;
                    den >>= 1;
                }
                den <<= 1;
                (
                    (px.wrapping_mul(vp.width as u32) / den).wrapping_add(vp.x0 as u32),
                    (py.wrapping_mul(vp.height as u32) / den).wrapping_add(vp.y1 as u32),
                )
            };
            vtx.final_position = [(pos_x & 0x1FF) as i32, (pos_y & 0xFF) as i32];
        }

        // Zero-dot polygons far behind are rejected unless bit 13 is set.
        if attr & (1 << 13) == 0 {
            let first = clipped[0].final_position;
            let mut zero_dot = true;
            let mut all_behind = true;
            for v in &clipped[..nverts] {
                if v.final_position != first {
                    zero_dot = false;
                    break;
                }
                if v.position[3] <= self.zero_dot_w_limit {
                    all_behind = false;
                    break;
                }
            }
            if zero_dot && all_behind {
                self.last_strip_polygon = None;
                return;
            }
        }

        let tex_fmt = (self.tex_param >> 26) & 7;
        let alpha = (attr >> 16) & 0x1F;
        let mut poly = Polygon {
            attr,
            tex_param: self.tex_param,
            tex_palette: self.tex_palette,
            facing_view,
            translucent: tex_fmt == 1 || tex_fmt == 6 || (alpha > 0 && alpha < 31),
            is_shadow_mask: attr & 0x3F00_0030 == 0x30,
            ..Polygon::default()
        };
        poly.is_shadow = attr & 0x30 == 0x30 && !poly.is_shadow_mask;

        let vram = &mut self.vertex_ram[bank];
        if self.last_strip_polygon.is_some() && clip_start > 0 {
            if nverts == last_poly_verts {
                poly.vertices[0] = reused[0];
                poly.vertices[1] = reused[1];
            } else {
                for k in 0..2 {
                    vram[self.num_vertices] = vram[reused[k]];
                    poly.vertices[k] = self.num_vertices;
                    self.num_vertices += 1;
                }
            }
            poly.num_vertices = 2;
        }
        for (i, src) in clipped.iter().enumerate().take(nverts).skip(clip_start) {
            let mut vtx = *src;
            for k in 0..3 {
                let c = vtx.color[k] >> 12;
                vtx.final_color[k] = if c != 0 { (c << 4) + 0xF } else { 0 };
            }
            vram[self.num_vertices] = vtx;
            poly.vertices[i] = self.num_vertices;
            self.num_vertices += 1;
            poly.num_vertices += 1;
        }

        // Bounds, W normalization and depth values.
        let (mut ytop, mut ybot, mut xtop, mut xbot) = (192, 0, 256, 0);
        let mut wsize = 0u32;
        for i in 0..nverts {
            let v = &vram[poly.vertices[i]];
            let [fx, fy] = v.final_position;
            if fy < ytop {
                xtop = fx;
                ytop = fy;
                poly.v_top = i;
            }
            if fy > ybot || (fy == ybot && fx > xbot) {
                xbot = fx;
                ybot = fy;
                poly.v_bottom = i;
            }
            let w = v.position[3] as u32;
            if w == 0 {
                poly.degenerate = true;
            }
            while wsize < 32 && (w >> wsize) != 0 {
                wsize += 4;
            }
        }
        poly.y_top = ytop;
        poly.y_bottom = ybot;
        poly.x_top = xtop;
        poly.x_bottom = xbot;
        if ybot > 192 {
            poly.degenerate = true;
        }
        poly.sort_key = ((ybot as u32) << 8) | ytop as u32;
        if poly.translucent {
            poly.sort_key |= 0x10000;
        }
        poly.w_buffer = self.flush_attributes & 2 != 0;

        for i in 0..nverts {
            let v = &vram[poly.vertices[i]];
            let (w, wshifted) = if wsize < 16 {
                let w = v.position[3] << (16 - wsize);
                (w, w >> (16 - wsize))
            } else {
                let w = v.position[3] >> (wsize - 16);
                (w, w << (wsize - 16))
            };
            let z = if poly.w_buffer {
                i64::from(wshifted)
            } else if v.position[3] != 0 {
                (i64::from(v.position[2]) * 0x4000 / i64::from(v.position[3]) + 0x3FFF) * 0x200
            } else {
                0x7F_FE00
            };
            poly.final_z[i] = z.clamp(0, 0xFF_FFFF) as i32;
            poly.final_w[i] = w;
        }

        if !poly.translucent {
            self.num_opaque_polygons += 1;
        }
        let index = self.num_polygons;
        self.polygon_ram[bank][index] = poly;
        self.num_polygons += 1;
        self.last_strip_polygon = (self.polygon_mode >= 2).then_some(index);
    }

    /// melonDS `BoxTest`: whether any face of the box is inside the view
    /// volume.
    fn box_test(&mut self, p: &[u32; 32]) {
        self.gxstat &= !(1 << 1);
        let x0 = p[0] as i16;
        let y0 = (p[0] >> 16) as i16;
        let z0 = p[1] as i16;
        let x1 = ((p[1] >> 16) as i16).wrapping_add(x0);
        let y1 = (p[2] as i16).wrapping_add(y0);
        let z1 = ((p[2] >> 16) as i16).wrapping_add(z0);
        let corners = [
            [x0, y0, z0],
            [x1, y0, z0],
            [x1, y1, z0],
            [x0, y1, z0],
            [x0, y1, z1],
            [x0, y0, z1],
            [x1, y0, z1],
            [x1, y1, z1],
        ];
        self.update_clip_matrix();
        let cube: [Vertex; 8] = corners.map(|c| Vertex {
            position: transform(c.map(i32::from), &self.clip_matrix),
            ..Vertex::default()
        });
        const FACES: [[usize; 4]; 6] = [
            [0, 1, 2, 3],
            [4, 5, 6, 7],
            [0, 3, 4, 5],
            [1, 2, 7, 6],
            [0, 1, 6, 5],
            [2, 3, 4, 7],
        ];
        let far = self.cur_polygon_attr & (1 << 12) != 0;
        for f in FACES {
            let mut face = [Vertex::default(); 10];
            for (k, &i) in f.iter().enumerate() {
                face[k] = cube[i];
            }
            if clip_polygon(false, far, &mut face, 4, 0) > 0 {
                self.gxstat |= 1 << 1;
                return;
            }
        }
    }
}
