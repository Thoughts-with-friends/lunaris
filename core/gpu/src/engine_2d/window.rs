//! Windows: WIN0, WIN1, the OBJ window and the outside area decide which
//! layers and color effects show at each pixel. See GBATEK "DS Video
//! Windows".
//!
//! ```text
//!   x=0                                              x=255
//!   +-------------------------------------------------+
//!   | outside            (WINOUT bits 0-5)            |
//!   |    +--------------+                             |
//!   |    | WIN0         |                             |
//!   |    | (WININ 0-5)  |                             |
//!   |    |        +-----+-------+      .-"""-.        |
//!   |    |        |     |       |     /       \  OBJ window:
//!   |    +--------+-----+       |     \       /  the shape of mode-2
//!   |             | WIN1        |      '-...-'   sprites (WINOUT 8-13)
//!   |             | (WININ 8-13)|                     |
//!   |             +-------------+                     |
//!   +-------------------------------------------------+
//!
//!   WIN0 rectangle: X1..X2 = WIN0H bits 8-15 .. bits 0-7
//!                   Y1..Y2 = WIN0V bits 8-15 .. bits 0-7
//!                   X2 / Y2 are exclusive; X1 > X2 wraps around the screen
//!   where regions overlap:   WIN0  >  WIN1  >  OBJ window  >  outside
//!
//!   each region has 6 enable bits, turned into a per-pixel mask:
//!      bit 0-3  BG0-BG3 visible     bit 4  OBJ visible
//!      bit 5    color special effects (blending, fades) allowed
//!   no window enabled (DISPCNT bits 13-15 = 0): everything visible everywhere
//! ```
//!
//! Typical uses: a dialog box that hides a BG (WIN0), a spotlight that only
//! shows the scene inside a sprite's shape (OBJ window), a fade limited to
//! part of the screen (bit 5).
//! GBATEK: [LCD I/O Window Feature](https://problemkaputt.de/gbatek.htm#lcdiowindowfeature)
use super::compose::{LineLayers, W};
use crate::Gpu;

/// Whether `x` lies in the window span `[start, end)` (wrapping when
/// `start > end`, as the hardware does).
#[inline]
const fn in_span(x: u32, start: u32, end: u32) -> bool {
    if start <= end { x >= start && x < end } else { x >= start || x < end }
}

impl Gpu {
    /// Window enable mask (bits 0-3 BG0-3, bit 4 OBJ, bit 5 effects) for
    /// every pixel of the line.
    pub(super) fn window_masks(&self, a: bool, line: usize, layers: &LineLayers) -> [u8; W] {
        let e = self.engine(a);
        let d = e.dispcnt;
        if !(d.display_win0 || d.display_win1 || d.obj_win_display) {
            // LCD I/O Window Feature: https://problemkaputt.de/gbatek.htm#lcdiowindowfeature
            return [0x3F; W]; // no windows: all layers and effects enabled
        }
        let pack = |bg: [bool; 4], obj: bool, fx: bool| -> u8 {
            bg.iter().enumerate().fold(0u8, |m, (i, &on)| m | (u8::from(on) << i))
                | (u8::from(obj) << 4)
                | (u8::from(fx) << 5)
        };
        let win0 =
            pack(e.winin.win0_bg_enabled, e.winin.win0_obj_enabled, e.winin.win0_color_special);
        let win1 =
            pack(e.winin.win1_bg_enabled, e.winin.win1_obj_enabled, e.winin.win1_color_special);
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
        let v_in = |v: u16| in_span(line, u32::from(v >> 8), u32::from(v & 0xFF)); // WINxV: Y1 (bits 8-15), Y2 (bits 0-7)
        let w0_v = d.display_win0 && v_in(e.win0v);
        let w1_v = d.display_win1 && v_in(e.win1v);

        let mut masks = [outside; W];
        for (x, m) in masks.iter_mut().enumerate() {
            let x32 = x as u32;
            if w0_v && in_span(x32, u32::from(e.win0h >> 8), u32::from(e.win0h & 0xFF)) {
                // WIN0H: X1 (bits 8-15), X2 (bits 0-7)
                *m = win0;
            } else if w1_v && in_span(x32, u32::from(e.win1h >> 8), u32::from(e.win1h & 0xFF)) {
                // WIN1H: X1 (bits 8-15), X2 (bits 0-7)
                *m = win1;
            } else if d.obj_win_display && layers.obj_window[x] {
                *m = objwin;
            }
        }
        masks
    }
}
