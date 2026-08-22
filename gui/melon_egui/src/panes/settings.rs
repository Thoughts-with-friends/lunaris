//! The Config menu's settings dialogs.

use super::*;

pub(super) fn emu_settings(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.checkbox(&mut app.limit_framerate, "Limit framerate")
        .on_hover_text("Off runs the core as fast as it will go.");
    ui.checkbox(&mut app.audio_sync, "Audio sync");
    ui.separator();
    ui.label("Console: DS, direct boot, FreeBIOS + generated firmware.");
    ui.label("The shim offers no other boot mode, so there is nothing else to pick.");
    ui.separator();
    ui.checkbox(&mut app.mic_static, "Microphone: white noise")
        .on_hover_text("The only mic input this build has; carts wanting a breath hear static.");
}

pub(super) fn preferences(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.checkbox(&mut app.pause_when_unfocused, "Pause when the window loses focus");
    ui.checkbox(&mut app.confirm_on_quit, "Ask before quitting with a cart running");
    ui.separator();
    ui.label("Settings are written to:");
    ui.monospace(config::config_dir().display().to_string());
}

/// melonDS's Video settings dialog, control for control.
///
/// Every 3D-renderer control here is live: `melonds-sys` builds the core with
/// its OpenGL renderers and carries melonDS's whole `RendererSettings`. What a
/// given machine can select still depends on its driver — see
/// [`crate::video`] — so the OpenGL choices are disabled, with the reason on
/// hover, when the context could not be bound.
pub(super) fn video_settings(app: &mut MelonEgui, ui: &mut egui::Ui) {
    /// Why an OpenGL renderer cannot be selected on this machine.
    const NO_GL: &str = "No OpenGL context: melon_egui could not bind the GL entry \
                         points (or its blitter's shader would not build), so only \
                         the software renderer can draw.";
    /// Why the compute renderer in particular cannot.
    const NO_COMPUTE: &str = "This context is not OpenGL 4.3, which the compute-shader                               renderer needs.";

    ui.heading("3D renderer");
    let gl_ok = app.gl_available();
    let compute_ok = gl_ok && melonds::gl_supports_compute();
    for renderer in Renderer::ALL {
        let available = match renderer {
            Renderer::Software => true,
            Renderer::OpenGl => gl_ok,
            Renderer::Compute => compute_ok,
        };
        let button = ui.add_enabled(
            available,
            egui::RadioButton::new(app.video.renderer == renderer, renderer.label()),
        );
        if available && button.clicked() {
            app.video.renderer = renderer;
        }
        if !available {
            button.on_disabled_hover_text(if gl_ok { NO_COMPUTE } else { NO_GL });
        }
    }
    // Threading is the software 3D rasteriser's own setting, so it is offered
    // with that renderer selected and no other.
    ui.add_enabled_ui(app.video.renderer == Renderer::Software, |ui| {
        ui.checkbox(&mut app.video.threaded_software, "Threaded software renderer").on_hover_text(
            "Rasterise 3D on worker threads. Faster where there are cores to \
                 spare; melonDS ships it off, so this does too.",
        );
    });
    ui.separator();

    ui.heading("OpenGL options");
    let on_gl = app.video.renderer.is_gl() && gl_ok;
    ui.add_enabled_ui(on_gl, |ui| {
        let mut scale = app.video.scale();
        egui::ComboBox::from_label("Internal resolution")
            .selected_text(format!("{scale}x  ({} x {})", 256 * scale, 192 * scale))
            .show_ui(ui, |ui| {
                for choice in 1..=16u32 {
                    ui.selectable_value(
                        &mut scale,
                        choice,
                        format!("{choice}x  ({} x {})", 256 * choice, 192 * choice),
                    );
                }
            });
        if scale != app.video.internal_scale {
            app.video.internal_scale = scale;
        }
    });
    if !on_gl {
        ui.label("Select an OpenGL renderer above to change the internal resolution.");
    }
    ui.label(
        "This rasterises the 3D geometry itself at the higher resolution, so it adds real \
         detail — unlike Display scale below, which magnifies the finished picture.",
    );
    // Each of these belongs to one of the two OpenGL renderers, exactly as in
    // melonDS: the core ignores the one its renderer has no use for, and this
    // says which is which instead of letting a dead checkbox look live.
    ui.add_enabled_ui(app.video.renderer == Renderer::OpenGl && gl_ok, |ui| {
        ui.checkbox(&mut app.video.better_polygons, "Better polygons").on_hover_text(
            "Improved polygon splitting. Closes the seams upscaling opens in some \
                 geometry, for some speed. Regular OpenGL renderer only.",
        );
    });
    ui.add_enabled_ui(app.video.renderer == Renderer::Compute && gl_ok, |ui| {
        ui.checkbox(&mut app.video.hires_coordinates, "High-resolution coordinates").on_hover_text(
            "Keep the extra vertex precision upscaling makes visible instead of \
                 rounding to the DS's own grid. Compute-shader renderer only.",
        );
    });
    disabled_checkbox(
        ui,
        "GL display",
        "Always on: this front end composites through egui's OpenGL painter \
         whichever renderer the core draws with, so there is nothing to turn off.",
    );
    ui.separator();

    ui.heading("2D upscaling");
    // The one setting here that can improve a 2D layer: those come from tiles
    // at 256x192 whatever the renderer does, so the internal resolution above
    // cannot touch them.
    let on_software = app.video.renderer == Renderer::Software;
    ui.add_enabled_ui(on_software, |ui| {
        ui.horizontal(|ui| {
            for method in upscale::Method::ALL {
                ui.selectable_value(&mut app.video.upscale, method, method.label());
            }
        });
        let mut factor = app.video.upscale_factor();
        let slider = egui::Slider::new(&mut factor, upscale::MIN_FACTOR..=upscale::MAX_FACTOR)
            .text("Factor")
            .custom_formatter(|v, _| format!("{v}x"));
        if ui.add_enabled(app.video.upscale != upscale::Method::None, slider).changed() {
            app.video.upscale_factor = factor;
        }
    });
    if on_software {
        ui.label(
            "xBRZ redraws the finished picture edge by edge, which is what smooths              sprites and text rather than blurring them. It runs on the CPU, once              per screen per frame.",
        );
    } else {
        ui.label(
            "Software renderer only: an OpenGL renderer leaves its picture in a              texture, and filtering it would mean reading the whole thing back              every frame.",
        );
    }
    ui.separator();

    ui.heading("Display scale");
    // `None` means "fit the window", which is the default and what the Screen
    // size menu entries assume.
    let mut fixed = app.view.display_scale.is_some();
    if ui.checkbox(&mut fixed, "Draw at a fixed scale").changed() {
        app.view.display_scale = fixed.then_some(2.0);
    }
    let mut scale = app.view.display_scale.unwrap_or(2.0);
    let slider = egui::Slider::new(&mut scale, 1.0..=8.0)
        .step_by(0.25)
        .text("Scale")
        .custom_formatter(|v, _| format!("{v:.2}x"));
    if ui.add_enabled(fixed, slider).changed() {
        app.view.display_scale = Some(scale);
    }
    if fixed {
        let (w, h) = (
            (melonds::SCREEN_WIDTH as f32 * scale).round() as u32,
            (melonds::SCREEN_HEIGHT as f32 * scale).round() as u32,
        );
        ui.label(format!("Each screen drawn at {w} x {h} pixels."));
        ui.label("Larger than the window simply crops; the layout still centres it.");
    } else {
        ui.label("Fitting to the window (use Screen filtering to choose how it is sampled).");
    }
    ui.separator();

    ui.heading("Display");
    ui.checkbox(&mut app.video.vsync, "VSync").on_hover_text(
        "Takes effect the next time melon_egui starts: the surface's \
                        present mode is fixed when the window is created.",
    );
    ui.checkbox(&mut app.view.filtering, "Screen filtering")
        .on_hover_text("Smooth the picture when scaled, instead of square pixels.");
    ui.separator();

    ui.heading("Compositing");
    ui.checkbox(&mut app.video.render, "Render frames").on_hover_text(
        "Off, the console keeps running but stops composing a picture. Emulation \
             is unaffected -- melonDS documents it as bit-identical either way -- so \
             this only makes the window go still.",
    );
    ui.checkbox(&mut app.video.skip_hidden_screens, "Skip screens the layout hides").on_hover_text(
        "In the Top only / Bottom only sizings, tell the core not to compose the \
             screen nobody is looking at. Most of the 2D renderer's work, saved.",
    );
    ui.separator();

    ui.heading("Aspect ratio");
    egui::Grid::new("video-aspect").show(ui, |ui| {
        for (label, aspect) in
            [("Top", &mut app.view.aspect_top), ("Bottom", &mut app.view.aspect_bottom)]
        {
            ui.label(label);
            egui::ComboBox::from_id_salt(label).selected_text(aspect.label()).show_ui(ui, |ui| {
                for choice in AspectRatio::ALL {
                    ui.selectable_value(aspect, choice, choice.label());
                }
            });
            ui.end_row();
        }
    });
}

pub(super) fn audio_settings(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.label(app.audio_status());
    ui.separator();

    let mut volume = app.volume();
    // Past 100% is a boost rather than a normalisation: the DS's own mix sits
    // some way below full scale, so unity is quieter than most other things on
    // the desktop. Loud material will clip up here, hence the hint below.
    let slider = egui::Slider::new(&mut volume, 0.0..=2.0)
        .text("Volume")
        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0));
    if ui.add_enabled(app.has_audio(), slider).changed() {
        app.set_volume(volume);
    }
    if volume > 1.0 {
        ui.label("Above 100% is a boost; loud passages may clip.");
    }
    ui.add_enabled(app.has_audio(), egui::Checkbox::new(&mut app.audio_sync, "Audio sync"))
        .on_hover_text("Pace emulation against the sound card instead of the clock.");
    ui.separator();

    ui.label(format!(
        "Source rate: {} Hz (the core's SPU output; fixed by the bindings)",
        crate::audio::SPU_SAMPLE_RATE,
    ));
    ui.separator();
    ui.checkbox(&mut app.mic_static, "Microphone: white noise");
}

pub(super) fn input(app: &MelonEgui, ui: &mut egui::Ui) {
    ui.label("Bindings are fixed in this front end.");
    ui.separator();

    ui.heading("Controllers");
    match app.connected_pads() {
        [] => {
            ui.label("None connected. A pad is picked up as soon as it is plugged in.");
        }
        pads => {
            for pad in pads {
                ui.label(pad);
            }
        }
    }
    ui.label(
        "Face buttons map by position, as the DS lays them out: A is the right-hand          button and B the bottom one. Shoulders are L and R; the D-pad and the left          stick both steer. Pad and keyboard are merged, so either works at any time.",
    );
    ui.separator();
    egui::Grid::new("bindings").striped(true).show(ui, |ui| {
        for (key, _, name) in BINDINGS {
            ui.label(*name);
            ui.monospace(key.name());
            ui.end_row();
        }
        ui.label("Touch");
        ui.monospace("click the bottom screen");
        ui.end_row();
    });
}
