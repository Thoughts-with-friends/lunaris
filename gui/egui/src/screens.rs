//! Owns the two egui textures the NDS screens are uploaded into every frame.
//!
//! See `docs/design/egui-migration-design.md` §5.2. Two separate textures
//! (rather than one stacked 256x384 texture, as the imgui front end uses)
//! so the Horizon (side-by-side) layout is a pure placement decision.

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use lunaris_gui_common::framebuffer::{SCREEN_HEIGHT, SCREEN_WIDTH, abgr1555_to_rgba8};
use nds_core::nds::NDS;

pub struct ScreenTextures {
    pub top: TextureHandle,
    pub bottom: TextureHandle,
}

impl ScreenTextures {
    pub fn new(ctx: &Context) -> Self {
        let placeholder = ColorImage::new(
            [SCREEN_WIDTH, SCREEN_HEIGHT],
            vec![egui::Color32::BLACK; SCREEN_WIDTH * SCREEN_HEIGHT],
        );
        ScreenTextures {
            top: ctx.load_texture("nds-top-screen", placeholder.clone(), TextureOptions::NEAREST),
            bottom: ctx.load_texture("nds-bottom-screen", placeholder, TextureOptions::NEAREST),
        }
    }

    /// Converts the current frame's pixels and re-uploads both textures.
    pub fn update(&mut self, nds: &NDS, options: TextureOptions) {
        let [top_pixels, bottom_pixels] = nds.get_screens();

        let top_rgba = abgr1555_to_rgba8(top_pixels);
        let top_image =
            ColorImage::from_rgba_unmultiplied([SCREEN_WIDTH, SCREEN_HEIGHT], &top_rgba);
        self.top.set(top_image, options);

        let bottom_rgba = abgr1555_to_rgba8(bottom_pixels);
        let bottom_image =
            ColorImage::from_rgba_unmultiplied([SCREEN_WIDTH, SCREEN_HEIGHT], &bottom_rgba);
        self.bottom.set(bottom_image, options);
    }
}
