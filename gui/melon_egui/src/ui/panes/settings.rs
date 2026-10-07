//! The Config menu's settings dialogs.
//!
//! Each function takes `tr` from `app.translations` — the one field — rather
//! than calling `app.i18n()`, so the checkboxes can hold `&mut app.<setting>`
//! beside it.

use super::*;

pub(super) fn emu_settings(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let tr = app.translations.get(app.language);
    ui.checkbox(&mut app.limit_framerate, tr.t(K::LimitFramerate))
        .on_hover_text(tr.t(K::LimitFramerateHint));
    ui.checkbox(&mut app.audio_sync, tr.t(K::AudioSync)).on_hover_text(tr.t(K::AudioSyncSpeedHint));
    ui.separator();

    emulation_speed(app, ui);
    ui.separator();
    let tr = app.translations.get(app.language);
    ui.label(tr.t(K::ConsoleKind));
    ui.label(tr.t(K::NoOtherBootMode));
    ui.separator();
    ui.checkbox(&mut app.mic_static, tr.t(K::MicWhiteNoise))
        .on_hover_text(tr.t(K::MicWhiteNoiseHint));
}

/// The Emu settings pane's speed control.
///
/// The same setting the pad's left-stick click steps through, so the slider
/// moves when the pad is clicked and vice versa -- there is one value, and both
/// are views of it. See [`crate::speed`].
fn emulation_speed(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let tr = app.translations.get(app.language);
    ui.heading(tr.t(K::EmulationSpeed));

    let mut speed = app.speed;
    let slider = egui::Slider::new(&mut speed, crate::speed::MIN..=crate::speed::MAX)
        .step_by(0.05)
        .text(tr.t(K::Speed))
        .custom_formatter(|v, _| crate::speed::label(v as f32));
    if ui.add(slider).changed() {
        app.set_speed(speed);
    }

    // The steps the pad cycles through, offered as buttons so the two controls
    // cannot disagree about what a "step" is.
    ui.horizontal_wrapped(|ui| {
        for step in crate::speed::STEPS {
            let selected = (app.speed - step).abs() < 0.001;
            if ui.selectable_label(selected, crate::speed::label(step)).clicked() {
                app.set_speed(step);
            }
        }
    });

    let tr = app.translations.get(app.language);
    if app.speed_locked() {
        ui.colored_label(Severity::Warn.color(ui.visuals().dark_mode), tr.t(K::SpeedHeld));
    }
    ui.label(tr.t(K::SpeedExplained));
}

pub(super) fn preferences(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let tr = app.translations.get(app.language);
    ui.checkbox(&mut app.pause_when_unfocused, tr.t(K::PauseUnfocused));
    ui.checkbox(&mut app.confirm_on_quit, tr.t(K::ConfirmQuit));
    ui.separator();
    ui.label(tr.t(K::SettingsWrittenTo));
    ui.monospace(config::config_dir().display().to_string());
}

/// melonDS's Video settings dialog, control for control, one heading per
/// function below. The OpenGL choices are disabled (reason on hover) when no
/// usable GL context could be bound; see [`crate::video`].
pub(super) fn video_settings(app: &mut MelonEgui, ui: &mut egui::Ui) {
    renderer_choice(app, ui);
    ui.separator();
    opengl_options(app, ui);
    ui.separator();
    upscaling_2d(app, ui);
    ui.separator();
    display_scale(app, ui);
    ui.separator();
    display(app, ui);
    ui.separator();
    compositing(app, ui);
    ui.separator();
    aspect_ratio(app, ui);
}

/// The 3D renderer, and the software one's threading.
fn renderer_choice(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let gl_ok = app.gl_available();
    let tr = app.translations.get(app.language);
    ui.heading(tr.t(K::Renderer3d));
    let compute_ok = gl_ok && melonds::gl_supports_compute();
    for renderer in Renderer::ALL {
        let available = match renderer {
            Renderer::Software => true,
            Renderer::OpenGl => gl_ok,
            Renderer::Compute => compute_ok,
        };
        let button = ui.add_enabled(
            available,
            egui::RadioButton::new(app.video.renderer == renderer, renderer.label(tr)),
        );
        if available && button.clicked() {
            app.video.renderer = renderer;
        }
        if !available {
            // Why an OpenGL renderer cannot be selected on this machine, or
            // why the compute one in particular cannot.
            button.on_disabled_hover_text(tr.t(if gl_ok { K::NoCompute } else { K::NoGl }));
        }
    }
    // Threading is the software 3D rasteriser's own setting, so it is offered
    // with that renderer selected and no other.
    ui.add_enabled_ui(app.video.renderer == Renderer::Software, |ui| {
        ui.checkbox(&mut app.video.threaded_software, tr.t(K::ThreadedSoftware))
            .on_hover_text(tr.t(K::ThreadedSoftwareHint));
    });
}

/// The OpenGL renderers' own options (internal resolution etc.).
fn opengl_options(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let gl_ok = app.gl_available();
    let tr = app.translations.get(app.language);
    ui.heading(tr.t(K::OpenGlOptions));
    let on_gl = app.video.renderer.is_gl() && gl_ok;
    ui.add_enabled_ui(on_gl, |ui| {
        let mut scale = app.video.scale();
        egui::ComboBox::new("internal-resolution", tr.t(K::InternalResolution))
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
        ui.label(tr.t(K::SelectGlForResolution));
    }
    ui.label(tr.t(K::InternalResolutionExplained));
    // Each of these belongs to one of the two OpenGL renderers, exactly as in
    // melonDS: the core ignores the one its renderer has no use for, and this
    // says which is which instead of letting a dead checkbox look live.
    ui.add_enabled_ui(app.video.renderer == Renderer::OpenGl && gl_ok, |ui| {
        ui.checkbox(&mut app.video.better_polygons, tr.t(K::BetterPolygons))
            .on_hover_text(tr.t(K::BetterPolygonsHint));
    });
    ui.add_enabled_ui(app.video.renderer == Renderer::Compute && gl_ok, |ui| {
        ui.checkbox(&mut app.video.hires_coordinates, tr.t(K::HiresCoordinates))
            .on_hover_text(tr.t(K::HiresCoordinatesHint));
    });
    disabled_checkbox(ui, tr.t(K::GlDisplay), tr.t(K::GlDisplayHint));
}

/// xBRZ for the 2D layers, and which route applies it.
fn upscaling_2d(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let tr = app.translations.get(app.language);
    ui.heading(tr.t(K::Upscaling2d));
    // The one setting here that can improve a 2D layer: those come from tiles
    // at 256x192 whatever the renderer does, so the internal resolution above
    // cannot touch them.
    ui.horizontal(|ui| {
        for method in upscale::Method::ALL {
            ui.selectable_value(&mut app.video.upscale, method, method.label(tr));
        }
    });
    let mut factor = app.video.upscale_factor();
    let slider = egui::Slider::new(&mut factor, upscale::MIN_FACTOR..=upscale::MAX_FACTOR)
        .text(tr.t(K::Factor))
        .custom_formatter(|v, _| format!("{v}x"));
    let on = app.video.upscale != upscale::Method::None;
    if ui.add_enabled(on, slider).changed() {
        app.video.upscale_factor = factor;
    }
    ui.label(tr.t(K::XbrzExplained));

    if on {
        // Which of the two routes is in use is worth saying: the setting is
        // shared, but what it does — and what it costs — is not.
        if app.video.renderer == Renderer::Software {
            ui.label(tr.f(K::XbrzSoftwareRoute, &[&factor]));
        } else {
            ui.label(tr.t(K::XbrzGlRoute));
            let size = format!("{}x{}", crate::gl_screen::DS_WIDTH, crate::gl_screen::DS_HEIGHT);
            ui.label(tr.f(K::XbrzGlCost, &[&size]));
        }
    }
}

/// Draw at a fixed magnification instead of fitting the window.
fn display_scale(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let tr = app.translations.get(app.language);
    ui.heading(tr.t(K::DisplayScale));
    // `None` means "fit the window", which is the default and what the Screen
    // size menu entries assume.
    let mut fixed = app.view.display_scale.is_some();
    if ui.checkbox(&mut fixed, tr.t(K::FixedScale)).changed() {
        app.view.display_scale = fixed.then_some(2.0);
    }
    let mut scale = app.view.display_scale.unwrap_or(2.0);
    let slider = egui::Slider::new(&mut scale, 1.0..=8.0)
        .step_by(0.25)
        .text(tr.t(K::Scale))
        .custom_formatter(|v, _| format!("{v:.2}x"));
    if ui.add_enabled(fixed, slider).changed() {
        app.view.display_scale = Some(scale);
    }
    if fixed {
        let (w, h) = (
            (melonds::SCREEN_WIDTH as f32 * scale).round() as u32,
            (melonds::SCREEN_HEIGHT as f32 * scale).round() as u32,
        );
        ui.label(tr.f(K::EachScreenDrawnAt, &[&w, &h]));
        ui.label(tr.t(K::LargerCrops));
    } else {
        ui.label(tr.t(K::FittingWindow));
    }
}

/// VSync and screen filtering.
fn display(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let tr = app.translations.get(app.language);
    ui.heading(tr.t(K::Display));
    ui.checkbox(&mut app.video.vsync, "VSync").on_hover_text(tr.t(K::VsyncHint));
    ui.checkbox(&mut app.view.filtering, tr.t(K::ScreenFiltering))
        .on_hover_text(tr.t(K::ScreenFilteringHint));
}

/// "Render frames" and skipping hidden screens.
fn compositing(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let tr = app.translations.get(app.language);
    ui.heading(tr.t(K::Compositing));
    ui.checkbox(&mut app.video.render, tr.t(K::RenderFrames))
        .on_hover_text(tr.t(K::RenderFramesHint));
    ui.checkbox(&mut app.video.skip_hidden_screens, tr.t(K::SkipHiddenScreens))
        .on_hover_text(tr.t(K::SkipHiddenScreensHint));
}

/// Per-screen aspect ratio.
fn aspect_ratio(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let tr = app.translations.get(app.language);
    ui.heading(tr.t(K::AspectRatio));
    egui::Grid::new("video-aspect").show(ui, |ui| {
        // The id is the fixed English name, not the label: a language switch
        // must not turn the combo box into a different widget.
        for (id, label, aspect) in [
            ("Top", K::TopScreen, &mut app.view.aspect_top),
            ("Bottom", K::BottomScreen, &mut app.view.aspect_bottom),
        ] {
            ui.label(tr.t(label));
            egui::ComboBox::from_id_salt(id).selected_text(aspect.label(tr)).show_ui(ui, |ui| {
                for choice in AspectRatio::ALL {
                    ui.selectable_value(aspect, choice, choice.label(tr));
                }
            });
            ui.end_row();
        }
    });
}

pub(super) fn audio_settings(app: &mut MelonEgui, ui: &mut egui::Ui) {
    app.audio_status().show(ui);
    ui.separator();

    let tr = app.translations.get(app.language);
    let mut volume = app.volume();
    // Past 100% is a boost rather than a normalisation: the DS's own mix sits
    // some way below full scale, so unity is quieter than most other things on
    // the desktop. Loud material will clip up here, hence the hint below.
    let slider = egui::Slider::new(&mut volume, 0.0..=2.0)
        .text(tr.t(K::Volume))
        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0));
    if ui.add_enabled(app.has_audio(), slider).changed() {
        app.set_volume(volume);
    }
    let tr = app.translations.get(app.language);
    if volume > 1.0 {
        ui.label(tr.t(K::VolumeBoost));
    }
    ui.add_enabled(app.has_audio(), egui::Checkbox::new(&mut app.audio_sync, tr.t(K::AudioSync)))
        .on_hover_text(tr.t(K::AudioSyncHint));
    ui.separator();

    ui.label(tr.f(K::SourceRate, &[&crate::audio::SPU_SAMPLE_RATE]));
    ui.separator();
    ui.checkbox(&mut app.mic_static, tr.t(K::MicWhiteNoise));
}

/// melonDS's Input dialog: one row per DS button, a keyboard column and a
/// controller column, and a click to rebind.
///
/// # How rebinding works
///
/// Clicking a cell arms it; the next key — or the next pad button — becomes the
/// binding. While a cell is armed the app stops handing key presses to the
/// console (see `MelonEgui::listening`), so binding `Start` does not also press
/// Start. `Escape` cancels and right-clicking a cell clears it, which is the
/// only way to leave a button deliberately unbound.
pub(super) fn input(app: &mut MelonEgui, ui: &mut egui::Ui) {
    use crate::bindings::{Device, DsInput};

    // Collected first and applied after the grid: the closures below borrow
    // `app` for the whole of it.
    let mut arm: Option<(DsInput, Device)> = None;
    let mut clear: Option<(DsInput, Device)> = None;
    let listening = app.listening;
    let tr = app.translations.get(app.language);

    egui::Grid::new("bindings").striped(true).num_columns(3).show(ui, |ui| {
        ui.label("");
        ui.strong(tr.t(K::Keyboard));
        ui.strong(tr.t(K::Controller));
        ui.end_row();

        for input in DsInput::ALL {
            let binding = app.bindings.get(input);
            ui.label(input.label(tr));
            for (device, bound) in
                [(Device::Keyboard, binding.key.clone()), (Device::Pad, binding.button.clone())]
            {
                let armed = listening == Some((input, device));
                let text = if armed {
                    tr.s(K::PressKey)
                } else {
                    bound.unwrap_or_else(|| "—".to_owned())
                };
                let cell = ui
                    .add_sized(
                        [130.0, 20.0],
                        egui::Button::new(text).selected(armed).min_size(egui::vec2(130.0, 20.0)),
                    )
                    .on_hover_text(tr.t(K::RebindHint));
                if cell.clicked() {
                    arm = Some((input, device));
                }
                if cell.secondary_clicked() {
                    clear = Some((input, device));
                }
            }
            ui.end_row();
        }

        ui.label(tr.t(K::Touch));
        ui.monospace(tr.t(K::TouchHow));
        ui.monospace("—");
        ui.end_row();
    });

    if let Some(armed) = arm {
        app.listening = Some(armed);
    }
    if let Some((input, device)) = clear {
        app.bindings.clear(input, device);
        app.listening = None;
        app.save_settings();
    }

    let tr = app.translations.get(app.language);
    if app.listening.is_some() {
        ui.label(tr.t(K::WaitingForPress));
    }
    if ui.button(tr.t(K::ResetBindings)).clicked() {
        app.bindings = crate::bindings::Bindings::default();
        app.listening = None;
        app.save_settings();
    }
    ui.separator();

    let tr = app.translations.get(app.language);
    ui.heading(tr.t(K::Controllers));
    match app.connected_pads() {
        [] => {
            ui.label(tr.t(K::NoControllers));
        }
        pads => {
            for pad in pads {
                ui.label(pad);
            }
        }
    }
    ui.label(tr.t(K::StickExplained));
}
