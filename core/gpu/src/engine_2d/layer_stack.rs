//! The top two visible layers of every pixel of a line.
//!
//! Compositing only ever looks at the frontmost visible pixel and the one
//! right behind it (for blending). Instead of searching the layers in
//! priority order for each pixel, the layers are painted back to front, one
//! whole layer at a time, keeping the last two painted (melonDS
//! `GPU2D_Soft` keeps the same two-entry stack per pixel):
//!
//! ```text
//!   paint order (back -> front)          per pixel, if the layer is visible:
//!     prio 3: BG3 BG2 BG1 BG0 OBJ(p=3)      second = top
//!     prio 2:  ...            OBJ(p=2)      top    = this layer's pixel
//!     prio 1:  ...            OBJ(p=1)
//!     prio 0:  ...            OBJ(p=0)    start: top = second = backdrop
//!
//!   a lower BG index / OBJ over BG at the same priority is painted later,
//!   so it ends up in front, as on the hardware
//! ```
//!
//! Scope: this only picks layers. Window masks, blending and colour
//! conversion stay in `compose.rs`.
use super::compose::{LineLayers, OPAQUE, W};

/// Layer ids stored next to the pixels (0-3 BG, 4 OBJ, 5 backdrop).
pub(super) const LAYER_OBJ: u8 = 4;
pub(super) const LAYER_BACKDROP: u8 = 5;

/// Frontmost and second visible pixel of each x, with their layer ids.
pub(super) struct LayerStack {
    pub(super) top: [u32; W],
    pub(super) top_layer: [u8; W],
    pub(super) second: [u32; W],
    pub(super) second_layer: [u8; W],
}

impl LayerStack {
    /// Paints the visible layers of one line back to front.
    ///
    /// * `bg_prio[i]`: priority (BGxCNT bits 0-1) of BGi, `None` when it is
    ///   not displayed;
    /// * `masks[x]`: window enable bits (0-3 BGs, 4 OBJ);
    /// * `backdrop`: pixel shown where no layer is (layer id 5).
    pub(super) fn build(
        layers: &LineLayers,
        masks: &[u8; W],
        bg_prio: [Option<u8>; 4],
        backdrop: u32,
    ) -> Self {
        let mut s = Self {
            top: [backdrop; W],
            top_layer: [LAYER_BACKDROP; W],
            second: [backdrop; W],
            second_layer: [LAYER_BACKDROP; W],
        };
        for prio in (0..4u8).rev() {
            for bg in (0..4usize).rev() {
                if bg_prio[bg] == Some(prio) {
                    s.paint_bg(&layers.bg[bg], masks, bg);
                }
            }
            s.paint_obj(&layers.obj, masks, prio);
        }
        s
    }

    /// Pushes `px` (layer `layer`) in front of pixel `x`.
    #[inline(always)]
    const fn push(&mut self, x: usize, px: u32, layer: u8) {
        self.second[x] = self.top[x];
        self.second_layer[x] = self.top_layer[x];
        self.top[x] = px;
        self.top_layer[x] = layer;
    }

    /// One BG layer: opaque pixels where the window shows this BG.
    fn paint_bg(&mut self, line: &[u32; W], masks: &[u8; W], bg: usize) {
        let bit = 1 << bg;
        for x in 0..W {
            let px = line[x];
            if px & OPAQUE != 0 && masks[x] & bit != 0 {
                self.push(x, px, bg as u8);
            }
        }
    }

    /// The OBJ pixels of priority `prio` (bits 16-17 of the OBJ line).
    fn paint_obj(&mut self, line: &[u32; W], masks: &[u8; W], prio: u8) {
        for x in 0..W {
            let px = line[x];
            // window mask bit 4: OBJ enabled
            if px & OPAQUE != 0 && masks[x] & 0x10 != 0 && (px >> 16) & 3 == u32::from(prio) {
                self.push(x, px, LAYER_OBJ);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The per-pixel search compose.rs used before: walk priorities 0..3,
    /// OBJ first at each, then the BGs by index; keep the first two hits.
    fn reference(
        layers: &LineLayers,
        masks: &[u8; W],
        bg_prio: [Option<u8>; 4],
        backdrop: u32,
        x: usize,
    ) -> [(u8, u32); 2] {
        let mut found = [(LAYER_BACKDROP, backdrop); 2];
        let mut count = 0;
        let mask = masks[x];
        let obj = layers.obj[x];
        let obj_prio = if obj & OPAQUE != 0 && mask & 0x10 != 0 {
            Some(((obj >> 16) & 3) as u8)
        } else {
            None
        };
        'search: for prio in 0..4u8 {
            if obj_prio == Some(prio) {
                found[count] = (LAYER_OBJ, obj);
                count += 1;
                if count == 2 {
                    break 'search;
                }
            }
            for bg in 0..4 {
                let px = layers.bg[bg][x];
                if bg_prio[bg] == Some(prio) && px & OPAQUE != 0 && mask & (1 << bg) != 0 {
                    found[count] = (bg as u8, px);
                    count += 1;
                    if count == 2 {
                        break 'search;
                    }
                }
            }
        }
        found
    }

    #[test]
    fn matches_the_per_pixel_search() {
        let mut seed = 0x1234_5678_u32;
        let mut rand = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed >> 8
        };
        for _ in 0..200 {
            let mut layers = LineLayers { bg: [[0; W]; 4], obj: [0; W], obj_window: [false; W] };
            let mut masks = [0u8; W];
            for x in 0..W {
                for bg in 0..4 {
                    // about half opaque, distinct colors per layer
                    if rand() & 1 != 0 {
                        layers.bg[bg][x] = OPAQUE | (bg as u32) << 12 | (rand() & 0x7FF);
                    }
                }
                if rand() & 1 != 0 {
                    layers.obj[x] = OPAQUE | ((rand() & 3) << 16) | (rand() & 0x7FFF);
                }
                masks[x] = (rand() & 0x3F) as u8 | if rand() & 3 != 0 { 0x1F } else { 0 };
            }
            let bg_prio: [Option<u8>; 4] =
                core::array::from_fn(|_| (rand() % 5 != 0).then(|| (rand() & 3) as u8));
            let backdrop = rand() & 0x7FFF;

            let s = LayerStack::build(&layers, &masks, bg_prio, backdrop);
            for x in 0..W {
                let want = reference(&layers, &masks, bg_prio, backdrop, x);
                assert_eq!((s.top_layer[x], s.top[x]), want[0], "top x={x} {bg_prio:?}");
                assert_eq!((s.second_layer[x], s.second[x]), want[1], "second x={x}");
            }
        }
    }
}
