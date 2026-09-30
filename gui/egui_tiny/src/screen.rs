//! Draws the two DS screens and turns clicks on the lower one into stylus
//! input.

use core_base::{HEIGHT, Nds, WIDTH};

/// Owns one egui texture per screen, reuploaded every frame.
#[derive(Default)]
pub struct Screens {
    handles: Option<[egui::TextureHandle; 2]>,
}

impl Screens {
    /// Renders both screens into `ui`, scaled to the available width.
    pub fn show(&mut self, ui: &mut egui::Ui, nds: &mut Nds) {
        let (top, bottom) = nds.framebuffer();
        let images = [to_image(top), to_image(bottom)];
        let handles = self.handles.get_or_insert_with(|| {
            [
                ui.ctx().load_texture("top", images[0].clone(), egui::TextureOptions::NEAREST),
                ui.ctx().load_texture("bottom", images[1].clone(), egui::TextureOptions::NEAREST),
            ]
        });
        for (handle, image) in handles.iter_mut().zip(images) {
            handle.set(image, egui::TextureOptions::NEAREST);
        }

        // Both screens share one scale so they stay aligned.
        let scale = (ui.available_width() / WIDTH as f32)
            .min(ui.available_height() / (HEIGHT * 2) as f32)
            .max(1.0);
        let size = egui::vec2(WIDTH as f32 * scale, HEIGHT as f32 * scale);
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(egui::Image::new(&handles[0]).fit_to_exact_size(size));
            let lower = ui.add(
                egui::Image::new(&handles[1]).fit_to_exact_size(size).sense(egui::Sense::drag()),
            );
            nds.set_touch(touch_position(&lower, scale));
        });
    }
}

/// Converts a pointer position over the lower screen into DS pixel
/// coordinates, or `None` when the stylus is not down.
fn touch_position(lower: &egui::Response, scale: f32) -> Option<(u8, u8)> {
    let pointer = lower.interact_pointer_pos()?;
    let local = pointer - lower.rect.min;
    let (x, y) = (local.x / scale, local.y / scale);
    if (0.0..WIDTH as f32).contains(&x) && (0.0..HEIGHT as f32).contains(&y) {
        Some((x as u8, y as u8))
    } else {
        None
    }
}

/// Wraps a framebuffer as an egui image without copying the pixel data twice.
fn to_image(buffer: &[u32]) -> egui::ColorImage {
    let pixels = buffer
        .iter()
        .map(|p| egui::Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, *p as u8))
        .collect();
    egui::ColorImage {
        size: [WIDTH, HEIGHT],
        source_size: egui::vec2(WIDTH as f32, HEIGHT as f32),
        pixels,
    }
}
