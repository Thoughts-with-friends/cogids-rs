//! Matrix math (20.12 fixed point) and the matrix commands
//! (MTX_LOAD / MTX_MULT / MTX_SCALE / MTX_TRANS).
//! See GBATEK "DS 3D Matrix Load/Multiply".
use super::mul;
use crate::engine_3d::state::{Gpu3D, Mtx};

/// `m = s * m` (4x4).
pub(super) fn mtx_mult_4x4(m: &mut Mtx, s: &[i32]) {
    let t = *m;
    for r in 0..4 {
        for c in 0..4 {
            m[r * 4 + c] =
                ((0..4).map(|k| mul(s[r * 4 + k], t[k * 4 + c])).sum::<i64>() >> 12) as i32;
        }
    }
}

/// `m = s * m` with a 4x3 `s` (implicit last column 0,0,0,1).
pub(super) fn mtx_mult_4x3(m: &mut Mtx, s: &[i32]) {
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
pub(super) fn mtx_mult_3x3(m: &mut Mtx, s: &[i32]) {
    let t = *m;
    for r in 0..3 {
        for c in 0..4 {
            m[r * 4 + c] =
                ((0..3).map(|k| mul(s[r * 3 + k], t[k * 4 + c])).sum::<i64>() >> 12) as i32;
        }
    }
}

/// MTX_SCALE: scales the first three rows by x, y, z.
pub(super) fn mtx_scale(m: &mut Mtx, s: &[i32]) {
    for r in 0..3 {
        for c in 0..4 {
            m[r * 4 + c] = (mul(s[r], m[r * 4 + c]) >> 12) as i32;
        }
    }
}

/// MTX_TRANS: `row3 += x*row0 + y*row1 + z*row2`.
pub(super) fn mtx_translate(m: &mut Mtx, s: &[i32]) {
    for c in 0..4 {
        let v = mul(s[0], m[c]) + mul(s[1], m[4 + c]) + mul(s[2], m[8 + c]);
        m[12 + c] = m[12 + c].wrapping_add((v >> 12) as i32);
    }
}

/// MTX_LOAD_4x3: loads a 4x3 matrix (last column 0,0,0,1).
pub(super) fn mtx_load_4x3(m: &mut Mtx, s: &[i32]) {
    *m = [
        s[0], s[1], s[2], 0, //
        s[3], s[4], s[5], 0, //
        s[6], s[7], s[8], 0, //
        s[9], s[10], s[11], 0x1000,
    ];
}

/// Transforms `(x, y, z, 1)` by `m`.
pub(super) fn transform(v: [i32; 3], m: &Mtx) -> [i32; 4] {
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

impl Gpu3D {
    /// Matrix load/multiply commands 0x16-0x1C.
    pub(super) fn matrix_command(&mut self, cmd: u8, p: &[i32; 32]) {
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
}
