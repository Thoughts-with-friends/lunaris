//! The auxiliary windows behind the menu's dialog entries.
//!
//! melonDS opens each of these as a modal Qt dialog; here they are ordinary
//! egui windows, so several can be open at once and none of them blocks
//! emulation.

use egui::Context;

use crate::{
    app::{BINDINGS, MelonEgui},
    config,
    mp::Kind,
    upscale,
    video::Renderer,
    view::AspectRatio,
};

/// One auxiliary window.
///
/// Serialisable so that whichever dialogs were open are reopened next run, the
/// way a docked tool window would be.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Pane {
    RomInfo,
    Power,
    Cheats,
    RamSearch,
    DateTime,
    Input,
    EmuSettings,
    Preferences,
    VideoSettings,
    AudioSettings,
    Wireless,
    Interface,
    Paths,
    About,
}

impl Pane {
    /// The window title, which is also its egui identity.
    pub const fn title(self) -> &'static str {
        match self {
            Self::RomInfo => "ROM info",
            Self::Power => "Power management",
            Self::Cheats => "Cheat codes",
            Self::RamSearch => "RAM search",
            Self::DateTime => "Date and time",
            Self::Input => "Input and hotkeys",
            Self::EmuSettings => "Emu settings",
            Self::Preferences => "Preferences",
            Self::VideoSettings => "Video settings",
            Self::AudioSettings => "Audio settings",
            Self::Wireless => "Wireless status",
            Self::Interface => "Interface settings",
            Self::Paths => "Path settings",
            Self::About => "About melon_egui",
        }
    }
}

/// Draw every open pane, closing any whose window was dismissed.
pub fn show(app: &mut MelonEgui, ctx: &Context) {
    for pane in app.open_panes() {
        let mut open = true;
        egui::Window::new(pane.title())
            .open(&mut open)
            .resizable(matches!(pane, Pane::RamSearch | Pane::Wireless | Pane::Cheats))
            .default_width(if matches!(pane, Pane::Wireless) { 460.0 } else { 260.0 })
            .show(ctx, |ui| body(app, pane, ui));
        if !open {
            app.close_pane(pane);
        }
    }
}

fn body(app: &mut MelonEgui, pane: Pane, ui: &mut egui::Ui) {
    match pane {
        Pane::RomInfo => rom_info(app, ui),
        Pane::Power => power(app, ui),
        Pane::Cheats => cheat_codes(app, ui),
        Pane::RamSearch => ram_search(app, ui),
        Pane::DateTime => date_time(app, ui),
        Pane::Input => input(app, ui),
        Pane::EmuSettings => emu_settings(app, ui),
        Pane::Preferences => preferences(app, ui),
        Pane::VideoSettings => video_settings(app, ui),
        Pane::AudioSettings => audio_settings(app, ui),
        Pane::Wireless => wireless(app, ui),
        Pane::Interface => interface(app, ui),
        Pane::Paths => paths(app, ui),
        Pane::About => about(ui),
    }
}

/// A checkbox present for shape but not usable, with the reason on hover.
fn disabled_checkbox(ui: &mut egui::Ui, label: &str, why: &str) {
    let mut off = false;
    ui.add_enabled(false, egui::Checkbox::new(&mut off, label)).on_disabled_hover_text(why);
}

/// melonDS's **System ▸ Power management**: the lid switch and what the
/// power-management chip says about the battery.
///
/// Both are inputs to the console rather than settings of the front end, so
/// they are read back from the core each frame instead of being mirrored here
/// — a cart that opens the lid itself is then visible in the dialog.
fn power(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let Some((lid, battery)) = app.power_state() else {
        ui.label("No cart running.");
        return;
    };

    let mut closed = lid;
    if ui.checkbox(&mut closed, "Lid closed").changed() {
        app.set_lid_closed(closed);
    }
    ui.label("Closing the lid raises the lid IRQ, which is how a cart is told to sleep.");
    ui.separator();

    let mut okay = battery;
    ui.label("Battery level");
    let mut changed = ui.radio_value(&mut okay, true, "Okay").changed();
    changed |= ui.radio_value(&mut okay, false, "Low").changed();
    if changed {
        app.set_battery_okay(okay);
    }
    ui.label(
        "What SPI's power-management chip reports; \"Low\" is what a cart's low-battery          warning reads.",
    );
}

/// melonDS's **System ▸ Setup cheat codes**.
///
/// The list is the cart's `.mch` — melonDS's own file, in its own format — so
/// codes written here open there and vice versa. Editing is deliberately plain:
/// a name and the `%08X %08X` lines every published code list is written in.
fn cheat_codes(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let mut enabled = app.cheats_enabled;
    if ui
        .checkbox(&mut enabled, "Enable cheats")
        .on_hover_text(
            "Off hands the console an empty list, so the codes cost nothing at all \
             rather than merely doing nothing.",
        )
        .clicked()
    {
        app.cheats_enabled = enabled;
    }
    match app.cheat_file() {
        Some(path) => ui.label(format!("File: {}", path.display())),
        None => ui.label("No cart running; codes load with one."),
    };
    ui.separator();

    // Applied after the loop: removing an entry mid-iteration would renumber
    // the rest under the widgets already drawn.
    let mut remove = None;
    let mut edit = None;
    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
        for (i, cheat) in app.cheats.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut cheat.enabled, "");
                let label = if cheat.category.is_empty() {
                    cheat.name.clone()
                } else {
                    format!("{} / {}", cheat.category, cheat.name)
                };
                let mut response = ui.label(label);
                if !cheat.description.is_empty() {
                    response = response.on_hover_text(&cheat.description);
                }
                if !cheat.is_well_formed() {
                    response.on_hover_text("This code has an odd number of words.");
                }
                if ui.small_button("Edit").clicked() {
                    edit = Some(i);
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
    });
    if let Some(i) = remove {
        app.cheats.remove(i);
    }
    if let Some(i) = edit {
        let cheat = &app.cheats[i];
        app.cheat_draft = (cheat.name.clone(), cheat.text());
        app.cheats.remove(i);
    }
    if app.cheats.is_empty() {
        ui.label("No codes. Paste one below, or read a melonDS .mch file.");
    }
    ui.separator();

    ui.heading("Add a code");
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut app.cheat_draft.0);
    });
    ui.add(
        egui::TextEdit::multiline(&mut app.cheat_draft.1)
            .hint_text("020F5CE4 000003E7")
            .desired_rows(3)
            .font(egui::TextStyle::Monospace),
    );
    ui.horizontal(|ui| {
        if ui.button("Add").clicked() {
            app.add_cheat_from_draft();
        }
        if ui.button("Clear").clicked() {
            app.cheat_draft = (String::new(), String::new());
        }
    });
    ui.separator();

    ui.horizontal(|ui| {
        if ui.add_enabled(app.cheat_file().is_some(), egui::Button::new("Save")).clicked() {
            app.save_cheats();
        }
        if ui.button("Open .mch...").clicked()
            && let Some(path) =
                rfd::FileDialog::new().add_filter("melonDS cheats", &["mch"]).pick_file()
        {
            app.import_cheats(&path);
        }
    });
}

fn rom_info(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let Some(info) = app.cart_info() else {
        ui.label("no cart loaded");
        return;
    };
    egui::Grid::new("rom-info").show(ui, |ui| {
        for (label, value) in info {
            ui.label(label);
            ui.label(value);
            ui.end_row();
        }
    });
}

/// A cut-down version of melonDS's RAM search: scan main RAM for a value, then
/// narrow the surviving addresses as the value changes.
///
/// The narrowing is what makes it useful — a first scan of 4 MB finds far too
/// many addresses to read, and only repeated scans while the number on screen
/// changes isolate the one that matters.
fn ram_search(app: &mut MelonEgui, ui: &mut egui::Ui) {
    if !app.is_loaded() {
        ui.label("no cart loaded");
        return;
    }

    ui.horizontal(|ui| {
        ui.label("Value:");
        ui.text_edit_singleline(&mut app.ram_search.needle);
        egui::ComboBox::from_id_salt("ram-width")
            .selected_text(app.ram_search.width.label())
            .show_ui(ui, |ui| {
                for width in SearchWidth::ALL {
                    ui.selectable_value(&mut app.ram_search.width, width, width.label());
                }
            });
    });

    let parsed = app.ram_search.parse_needle();
    ui.horizontal(|ui| {
        if ui.add_enabled(parsed.is_some(), egui::Button::new("First scan")).clicked() {
            app.ram_first_scan();
        }
        let can_narrow = parsed.is_some() && !app.ram_search.hits.is_empty();
        if ui.add_enabled(can_narrow, egui::Button::new("Narrow")).clicked() {
            app.ram_narrow();
        }
        if ui.button("Clear").clicked() {
            app.ram_search.hits.clear();
        }
    });
    if parsed.is_none() && !app.ram_search.needle.is_empty() {
        ui.colored_label(egui::Color32::from_rgb(0xE0, 0x80, 0x60), "not a number");
    }

    ui.separator();
    ui.label(format!("{} matching addresses", app.ram_search.hits.len()));
    // Only a window's worth is listed: a first scan can match millions, and
    // nobody reads past the first screenful anyway.
    let shown: Vec<_> = app.ram_search.hits.iter().take(200).copied().collect();
    egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
        egui::Grid::new("ram-hits").striped(true).show(ui, |ui| {
            for addr in shown {
                ui.monospace(format!("{addr:08X}"));
                ui.monospace(format!("{}", app.ram_read(addr)));
                ui.end_row();
            }
        });
    });
    if app.ram_search.hits.len() > 200 {
        ui.label("(first 200 shown — narrow the search to see fewer)");
    }
}

/// How wide a value the RAM search looks for.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchWidth {
    Byte,
    Half,
    #[default]
    Word,
}

impl SearchWidth {
    pub const ALL: [Self; 3] = [Self::Byte, Self::Half, Self::Word];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Byte => "8-bit",
            Self::Half => "16-bit",
            Self::Word => "32-bit",
        }
    }

    /// Bytes per value, which is also the scan's stride: a value is only looked
    /// for where it could be aligned.
    pub const fn size(self) -> usize {
        match self {
            Self::Byte => 1,
            Self::Half => 2,
            Self::Word => 4,
        }
    }
}

/// The RAM search's state, kept between repaints.
#[derive(Default)]
pub struct RamSearch {
    pub needle: String,
    pub width: SearchWidth,
    /// Addresses still matching, narrowed by each scan.
    pub hits: Vec<u32>,
}

impl RamSearch {
    /// The value being searched for, accepting decimal or `0x`-prefixed hex.
    pub fn parse_needle(&self) -> Option<u32> {
        let text = self.needle.trim();
        let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            Some(hex) => u32::from_str_radix(hex, 16),
            None => text.parse(),
        }
        .ok()?;
        // A value too wide for the chosen width could never be found.
        let fits = match self.width {
            SearchWidth::Byte => parsed <= u32::from(u8::MAX),
            SearchWidth::Half => parsed <= u32::from(u16::MAX),
            SearchWidth::Word => true,
        };
        fits.then_some(parsed)
    }
}

fn date_time(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.label("The DS clock is set at boot and runs on emulated time from there.");
    ui.separator();
    let clock = &mut app.clock;
    egui::Grid::new("datetime").show(ui, |ui| {
        for (label, value, range) in [
            ("Year", &mut clock.year, 2000..=2099),
            ("Month", &mut clock.month, 1..=12),
            ("Day", &mut clock.day, 1..=31),
            ("Hour", &mut clock.hour, 0..=23),
            ("Minute", &mut clock.minute, 0..=59),
            ("Second", &mut clock.second, 0..=59),
        ] {
            ui.label(label);
            ui.add(egui::DragValue::new(value).range(range));
            ui.end_row();
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked() {
            app.apply_clock();
        }
        if ui.button("Now (UTC)").clicked() {
            app.clock = crate::emu::utc_clock();
        }
    });
    ui.label(&app.clock_note);
}

fn input(app: &MelonEgui, ui: &mut egui::Ui) {
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

fn emu_settings(app: &mut MelonEgui, ui: &mut egui::Ui) {
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

fn preferences(app: &mut MelonEgui, ui: &mut egui::Ui) {
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
fn video_settings(app: &mut MelonEgui, ui: &mut egui::Ui) {
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

fn audio_settings(app: &mut MelonEgui, ui: &mut egui::Ui) {
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

/// Everything the shared airwaves have seen, in the detail needed to compare
/// this run against lunaris's own wireless trace.
///
/// The headline is deliberately the CMD count. DS local play only starts once
/// the host begins sending CMD frames — association succeeding is *not* the same
/// thing — and "association fine, no CMD ever sent" is exactly where lunaris
/// currently stops (`docs/design/review_mp_local2.md` §4). So the one number
/// that says whether this is working is how many CMD frames went out.
fn wireless(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let counters = app.airwaves.counters();
    let connected = app.airwaves.connected();
    let live: Vec<usize> =
        connected.iter().enumerate().filter_map(|(i, on)| on.then_some(i)).collect();

    let cmds: u64 = counters.iter().map(|c| c.sent_cmd).sum();
    let replies: u64 = counters.iter().map(|c| c.sent_reply).sum();
    let acks: u64 = counters.iter().map(|c| c.sent_ack).sum();
    let generic: u64 = counters.iter().map(|c| c.sent_generic).sum();

    // -- the verdict ----------------------------------------------------
    ui.heading("Status");
    if live.is_empty() {
        ui.label(
            "No console is on the air yet. A cart only joins when it opens its \
             wireless menu, so this stays empty until then.",
        );
    } else if cmds == 0 {
        ui.colored_label(
            egui::Color32::from_rgb(0xE0, 0xA0, 0x40),
            format!(
                "{} console(s) on the air, {generic} frames exchanged, but no CMD frame \
                 has been sent.",
                live.len()
            ),
        );
        ui.label(
            "Beacons and the association handshake are ordinary frames; local play only \
             begins when the host starts an MP round with a CMD. This is the exact point \
             lunaris does not get past.",
        );
    } else {
        ui.colored_label(
            egui::Color32::from_rgb(0x60, 0xC0, 0x60),
            format!("MP rounds are running: {cmds} CMD, {replies} replies, {acks} ACK."),
        );
        if replies == 0 {
            ui.colored_label(
                egui::Color32::from_rgb(0xE0, 0xA0, 0x40),
                "The host is asking but no client has answered.",
            );
        }
    }
    ui.separator();

    // -- per console ----------------------------------------------------
    ui.heading("Per console");
    egui::ScrollArea::horizontal().id_salt("mp-counters").show(ui, |ui| {
        egui::Grid::new("mp-grid").striped(true).show(ui, |ui| {
            for heading in [
                "#",
                "on air",
                "wifi clock",
                "sent pkt",
                "CMD",
                "reply",
                "ACK",
                "recv pkt",
                "recv CMD",
                "recv reply",
                "stale",
                "AID mask",
            ] {
                ui.strong(heading);
            }
            ui.end_row();

            for (i, c) in counters.iter().enumerate() {
                // Consoles that never joined and never sent anything are noise.
                if !connected[i] && c.sent_generic == 0 && c.recv_generic == 0 {
                    continue;
                }
                ui.monospace(i.to_string());
                ui.monospace(if connected[i] { "yes" } else { "no" });
                ui.monospace(c.clock.to_string());
                ui.monospace(c.sent_generic.to_string());
                ui.monospace(c.sent_cmd.to_string());
                ui.monospace(c.sent_reply.to_string());
                ui.monospace(c.sent_ack.to_string());
                ui.monospace(c.recv_generic.to_string());
                ui.monospace(c.recv_cmd.to_string());
                ui.monospace(c.recv_reply.to_string());
                ui.monospace(c.stale_replies.to_string());
                ui.monospace(format!("{:04b}", c.last_reply_mask));
                ui.end_row();
            }
        });
    });
    ui.label(
        "\"stale\" counts replies discarded for arriving outside the host's round, and \
         \"AID mask\" is what the last reply collection returned - a host asking and \
         getting 0000 is a host nobody answered.",
    );
    ui.separator();

    // -- the traffic log ------------------------------------------------
    ui.horizontal(|ui| {
        ui.heading("Traffic");
        if ui.button("Clear").clicked() {
            app.airwaves.clear_log();
        }
    });
    let log = app.airwaves.log();
    if log.is_empty() {
        ui.label("(nothing yet)");
        return;
    }
    // Newest last, scrolled to the bottom, so it reads like a trace.
    egui::ScrollArea::vertical().id_salt("mp-log").max_height(220.0).stick_to_bottom(true).show(
        ui,
        |ui| {
            for event in &log {
                let kind = match event.kind {
                    Kind::Reply(aid) => format!("reply aid={aid}"),
                    other => other.label().to_owned(),
                };
                ui.monospace(format!(
                    "inst {}  t={:<12} {:<12} {} bytes",
                    event.sender, event.timestamp, kind, event.len
                ));
            }
        },
    );
}

fn interface(app: &mut MelonEgui, ui: &mut egui::Ui) {
    let mut dark = app.dark_theme;
    if ui.checkbox(&mut dark, "Dark theme").changed() {
        app.set_theme(ui.ctx(), dark);
    }
    ui.separator();
    ui.add(
        egui::Slider::new(&mut app.ui_scale, 0.75..=2.0)
            .text("UI scale")
            .custom_formatter(|value, _| format!("{value:.2}x")),
    );
    if ui.button("Apply UI scale").clicked() {
        ui.ctx().set_zoom_factor(app.ui_scale);
    }
    ui.separator();
    ui.checkbox(&mut app.view.show_osd, "Show OSD");
}

fn paths(app: &mut MelonEgui, ui: &mut egui::Ui) {
    ui.label("Empty means \"beside the ROM\", which is melonDS's behaviour.");
    ui.separator();
    for (label, dir) in [("Save files", &mut app.save_dir), ("Savestates", &mut app.state_dir)] {
        ui.horizontal(|ui| {
            ui.label(label);
            let shown = dir
                .as_ref()
                .map_or_else(|| "(beside the ROM)".to_owned(), |d| d.display().to_string());
            ui.monospace(shown);
        });
        ui.horizontal(|ui| {
            if ui.button(format!("Choose {}...", label.to_lowercase())).clicked()
                && let Some(picked) = rfd::FileDialog::new().pick_folder()
            {
                *dir = Some(picked);
            }
            if ui.add_enabled(dir.is_some(), egui::Button::new("Reset")).clicked() {
                *dir = None;
            }
        });
        ui.separator();
    }
    ui.label("These take effect for the next cart loaded.");
}

fn about(ui: &mut egui::Ui) {
    ui.label("melon_egui");
    ui.label(concat!("version ", env!("CARGO_PKG_VERSION")));
    ui.separator();
    ui.label(
        "An egui front end for the melonDS core, through the melonds-rs bindings. \
         Built as a reference picture to compare lunaris against.",
    );
    ui.separator();
    ui.label("GPL-3.0-or-later, as is the melonDS core it embeds.");
}

#[cfg(test)]
mod tests {
    use super::{RamSearch, SearchWidth};

    #[test]
    fn the_needle_accepts_decimal_and_hex() {
        let mut search = RamSearch { needle: "255".into(), ..Default::default() };
        assert_eq!(search.parse_needle(), Some(255));
        search.needle = "0xFF".into();
        assert_eq!(search.parse_needle(), Some(255));
        search.needle = "  0x10  ".into();
        assert_eq!(search.parse_needle(), Some(16));
    }

    #[test]
    fn the_needle_rejects_nonsense_and_values_too_wide_for_the_width() {
        let mut search = RamSearch { needle: "abc".into(), ..Default::default() };
        assert_eq!(search.parse_needle(), None);

        search.needle = "300".into();
        search.width = SearchWidth::Byte;
        assert_eq!(search.parse_needle(), None, "300 does not fit in 8 bits");
        search.width = SearchWidth::Half;
        assert_eq!(search.parse_needle(), Some(300));
    }
}
