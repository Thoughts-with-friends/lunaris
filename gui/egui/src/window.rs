use crate::config::WindowConfig;

pub(crate) fn update_window_geometry(
    ctx: &egui::Context,
    viewport_id: egui::ViewportId,
    geometry: &mut WindowConfig,
) {
    // NOTE: Writing directly to `geometry` inside this closure would
    // deadlock (egui holds an internal lock during `input()`).
    let (pos, size, maximized) = ctx.input(|i| {
        let mut temp_pos = None;
        let mut temp_size = None;
        let mut temp_maximized = None;

        if let Some(info) = i.raw.viewports.get(&viewport_id) {
            temp_maximized = Some(info.maximized.unwrap_or(false));

            if let Some(inner_rect) = info.inner_rect {
                temp_size = Some(inner_rect.size());
            }

            if let Some(outer_rect) = info.outer_rect {
                temp_pos = Some(outer_rect.min);
            }
        }

        (temp_pos, temp_size, temp_maximized)
    });

    if !geometry.maximized {
        if let Some(pos) = pos {
            geometry.pos_x = pos.x;
            geometry.pos_y = pos.y;
        }

        if let Some(size) = size {
            geometry.width = size.x;
            geometry.height = size.y;
        }
    }

    if let Some(maximized) = maximized {
        geometry.maximized = maximized;
    }
}
