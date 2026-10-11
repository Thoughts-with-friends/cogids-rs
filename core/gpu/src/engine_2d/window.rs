//! Windows: WIN0, WIN1, the OBJ window and the outside area decide which
//! layers and color effects show at each pixel. See GBATEK "DS Video
//! Windows".
use super::compose::{LineLayers, W};
use crate::Gpu;

/// Whether `x` lies in the window span `[start, end)` (wrapping when
/// `start > end`, as the hardware does).
#[inline]
const fn in_span(x: u32, start: u32, end: u32) -> bool {
    if start <= end {
        x >= start && x < end
    } else {
        x >= start || x < end
    }
}

impl Gpu {
    /// Window enable mask (bits 0-3 BG0-3, bit 4 OBJ, bit 5 effects) for
    /// every pixel of the line.
    pub(super) fn window_masks(&self, a: bool, line: usize, layers: &LineLayers) -> [u8; W] {
        let e = self.engine(a);
        let d = e.dispcnt;
        if !(d.display_win0 || d.display_win1 || d.obj_win_display) {
            return [0x3F; W];
        }
        let pack = |bg: [bool; 4], obj: bool, fx: bool| -> u8 {
            bg.iter()
                .enumerate()
                .fold(0u8, |m, (i, &on)| m | (u8::from(on) << i))
                | (u8::from(obj) << 4)
                | (u8::from(fx) << 5)
        };
        let win0 = pack(
            e.winin.win0_bg_enabled,
            e.winin.win0_obj_enabled,
            e.winin.win0_color_special,
        );
        let win1 = pack(
            e.winin.win1_bg_enabled,
            e.winin.win1_obj_enabled,
            e.winin.win1_color_special,
        );
        let outside = pack(
            e.winout.outside_bg_enabled,
            e.winout.outside_obj_enabled,
            e.winout.outside_color_special,
        );
        let objwin = pack(
            e.winout.objwin_bg_enabled,
            e.winout.objwin_obj_enabled,
            e.winout.objwin_color_special,
        );

        let line = line as u32;
        let v_in = |v: u16| in_span(line, u32::from(v >> 8), u32::from(v & 0xFF));
        let w0_v = d.display_win0 && v_in(e.win0v);
        let w1_v = d.display_win1 && v_in(e.win1v);

        let mut masks = [outside; W];
        for (x, m) in masks.iter_mut().enumerate() {
            let x32 = x as u32;
            if w0_v && in_span(x32, u32::from(e.win0h >> 8), u32::from(e.win0h & 0xFF)) {
                *m = win0;
            } else if w1_v && in_span(x32, u32::from(e.win1h >> 8), u32::from(e.win1h & 0xFF)) {
                *m = win1;
            } else if d.obj_win_display && layers.obj_window[x] {
                *m = objwin;
            }
        }
        masks
    }
}
