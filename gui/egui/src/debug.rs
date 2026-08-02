//! Debug inspector windows (palettes, maps, tiles, VRAM banks, perf stats).
//!
//! Equivalent to the imgui front end's `gui/src/debug.rs` +
//! `gui/src/debug/windows.rs`, ported onto egui per
//! `docs/design/egui-migration-design.md` §6.3. The generic
//! `DebugWindow<S: DebugWindowState>` pattern is preserved unchanged; only
//! the widget calls and the texture backing store differ.

mod windows;

use egui::{ColorImage, TextureHandle, TextureOptions};
use lunaris_gui_common::framebuffer::abgr1555_to_rgba8;
use nds_core::nds::{Engine, GraphicsType, NDS};
pub use windows::{
    MapsWindowState, PalettesWindowState, StatsWindow, TilesWindowState, VRAMWindowState,
};

/// Per-window state: what to render and the controls that configure it.
///
/// Implemented once per debug view (palettes/maps/tiles/VRAM); the shared
/// [`DebugWindow`] wrapper handles the egui `Window`, texture upload, and
/// zoom hotkeys around it.
pub trait DebugWindowState {
    fn new() -> Self;
    /// Draws the view's controls (combos/sliders/checkboxes) above the image.
    fn render(&mut self, ui: &mut egui::Ui);
    /// Renders the current selection into a raw ABGR1555 pixel buffer.
    fn get_pixels(&self, nds: &mut NDS) -> (Vec<u16>, usize, usize);

    const ENGINES: [Engine; 2] = [Engine::A, Engine::B];
    const GRAPHICS_TYPES: [GraphicsType; 2] = [GraphicsType::BG, GraphicsType::OBJ];
}

/// A togglable debug window showing one [`DebugWindowState`]'s pixels,
/// zoomable with `=`/`-` while hovered.
pub struct DebugWindow<S: DebugWindowState> {
    title: String,
    texture: Option<TextureHandle>,
    tex_size: (usize, usize),
    opened: bool,
    scale: f32,
    state: S,
}

impl<S: DebugWindowState> DebugWindow<S> {
    const SCALE_OFFSET: f32 = 0.1;

    pub fn new(title: &str) -> Self {
        DebugWindow {
            title: title.to_owned(),
            texture: None,
            tex_size: (0, 0),
            opened: false,
            scale: 1.0,
            state: S::new(),
        }
    }

    /// Draws this window's toggle as a menu entry (checkmark = open).
    pub fn menu_item(&mut self, ui: &mut egui::Ui) {
        if ui.selectable_label(self.opened, &self.title).clicked() {
            self.opened = !self.opened;
        }
    }

    pub fn render(&mut self, ctx: &egui::Context, nds: &mut NDS) {
        if !self.opened {
            return;
        }

        let (pixels, width, height) = self.state.get_pixels(nds);
        self.upload_texture(ctx, &pixels, width, height);

        let mut opened = self.opened;
        let title = self.title.clone();
        let inner = egui::Window::new(&title).open(&mut opened).resizable(false).show(ctx, |ui| {
            self.state.render(ui);
            if let Some(texture) = &self.texture {
                let size = egui::vec2(
                    self.tex_size.0 as f32 * self.scale,
                    self.tex_size.1 as f32 * self.scale,
                );
                ui.image((texture.id(), size));
            }
        });

        // Zoom hotkeys only apply while this window is hovered, so they
        // don't fight over `=`/`-` with other open debug windows.
        if inner.is_some_and(|inner| inner.response.hovered()) {
            ctx.input(|i| {
                if i.key_pressed(egui::Key::Equals) {
                    self.scale += Self::SCALE_OFFSET;
                }
                if i.key_pressed(egui::Key::Minus) {
                    self.scale = (self.scale - Self::SCALE_OFFSET).max(Self::SCALE_OFFSET);
                }
            });
        }

        self.opened = opened;
    }

    fn upload_texture(&mut self, ctx: &egui::Context, pixels: &[u16], width: usize, height: usize) {
        let rgba = abgr1555_to_rgba8(pixels);
        let image = ColorImage::from_rgba_unmultiplied([width, height], &rgba);
        if self.tex_size == (width, height) {
            if let Some(texture) = &mut self.texture {
                texture.set(image, TextureOptions::NEAREST);
                return;
            }
        }
        self.texture = Some(ctx.load_texture(self.title.clone(), image, TextureOptions::NEAREST));
        self.tex_size = (width, height);
    }
}
