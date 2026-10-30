//! Lunaris Designer: one binary to play a game, open the console up as a
//! Vivado-style block design, and inspect or extend its hardware live.
//!
//! ```text
//!  [Emulate]  the game, as in gui/egui (keyboard + mouse stylus)
//!  [Design]   the DS drawn as a console (its buttons and touch screen
//!             play the game) --double-click--> zooms inside -->
//!             block design (drag blocks, pan, zoom, live nets)
//!               + properties panel: registers, audio, VRAM, memory,
//!                 CPU, Slot-2 FPGA (spec -> Verilog / SV / Rust, or
//!                 Verilog run by the built-in simulator)
//!  right edge: FPGA waveform window (signals scrolling as the ROM runs)
//! ```
//!
//! Emulation keeps running in both tabs. See gui/designer/README.md.

// Prevents an additional console window on Windows in release.
#![cfg_attr(feature = "release", windows_subsystem = "windows")]
#![allow(clippy::collapsible_if, reason = "as the workspace lints")]

mod diagram;
mod ds_body;
mod emu;
mod inspect;
mod nav;
mod wave;

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Pos2, RichText};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Emulate,
    Design,
}

/// The zoom between the console and its block design.
#[derive(Debug, Clone, Copy)]
struct Transition {
    start: Instant,
    /// `true`: console -> diagram.
    entering: bool,
    focus: Pos2,
}

impl Transition {
    const LENGTH: Duration = Duration::from_millis(650);

    /// 0..=1, eased.
    fn progress(&self) -> f32 {
        let t = (self.start.elapsed().as_secs_f32() / Self::LENGTH.as_secs_f32()).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    fn done(&self) -> bool {
        self.start.elapsed() >= Self::LENGTH
    }
}

struct DesignerApp {
    emu: emu::Emu,
    tab: Tab,
    /// In the Design tab: showing the block design (else the console).
    inside: bool,
    transition: Option<Transition>,
    diagram: diagram::Diagram,
    inspector: inspect::Inspector,
    epoch: Instant,
    last_repaint: Instant,
    /// Block to focus once the diagram has a size.
    pending_focus: Option<&'static str>,
    /// The user's pan / zoom of the drawn console (zoom 1 = fit).
    body_pan: egui::Vec2,
    body_zoom: f32,
    /// FPGA signals scrolling on the right (see `wave`).
    wave: wave::WaveView,
}

impl DesignerApp {
    fn new(ctx: &egui::Context) -> Self {
        let rom = std::env::args().nth(1).map(std::path::PathBuf::from).filter(|p| p.exists());
        let mut app = Self {
            emu: emu::Emu::new(ctx, rom),
            tab: Tab::Emulate,
            inside: false,
            transition: None,
            diagram: diagram::Diagram::new(),
            inspector: inspect::Inspector::new(),
            epoch: Instant::now(),
            last_repaint: Instant::now(),
            pending_focus: None,
            body_pan: egui::Vec2::ZERO,
            body_zoom: 1.0,
            wave: wave::WaveView::new(),
        };
        app.apply_start_view(&std::env::var("LUNARIS_DESIGNER_START").unwrap_or_default());
        app
    }

    /// Opens on a given view, for screenshots and quick debugging:
    /// `console`, `design`, `design:<block id>`, `fpga` (FPGA inserted,
    /// Slot-2 selected), `riscv` (RISC-V sample running).
    fn apply_start_view(&mut self, start: &str) {
        let (view, block) = start.split_once(':').unwrap_or((start, ""));
        match view {
            "console" => self.tab = Tab::Design,
            "riscv" => {
                // RISC-V sample running the counter demo, waveform open.
                self.tab = Tab::Design;
                self.inside = true;
                self.emu.set_fpga_inserted(true);
                if let Some(fpga) = self.emu.fpga() {
                    use ds_emu::slot2::{SLOT2_START, Slot2Device, Width};
                    use ds_fpga::samples::{counter, riscv_map as map, riscv_rv32i};
                    fpga.remove(0);
                    if fpga.add_spec(riscv_rv32i()).is_ok() {
                        for (i, w) in counter().iter().enumerate() {
                            fpga.write(SLOT2_START + map::IMEM + 4 * i as u32, *w, Width::Word);
                        }
                        fpga.write(SLOT2_START + map::CTRL, map::RESET_AND_RUN, Width::Word);
                    }
                }
                self.diagram.select("slot2");
            }
            "design" | "fpga" => {
                self.tab = Tab::Design;
                self.inside = true;
                if view == "fpga" {
                    self.emu.set_fpga_inserted(true);
                }
                let block = if view == "fpga" { "slot2" } else { block };
                if let Some(def) = diagram::hw::block_def(block) {
                    self.diagram.select(def.id);
                }
            }
            _ => {}
        }
    }

    fn time(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open ROM...").clicked() {
                        self.emu.open_rom_dialog();
                        ui.close();
                    }
                    if ui.button("Reset").clicked() {
                        self.emu.boot(None);
                        ui.close();
                    }
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Emulation", |ui| {
                    ui.checkbox(&mut self.emu.paused, "Pause");
                    let mut inserted = self.emu.fpga().is_some();
                    if ui.checkbox(&mut inserted, "FPGA cartridge in Slot-2").changed() {
                        self.emu.set_fpga_inserted(inserted);
                    }
                    ui.checkbox(&mut self.wave.open, "Waveform window");
                });
                ui.separator();
                for (tab, label) in [(Tab::Emulate, "Emulate"), (Tab::Design, "Design")] {
                    if ui.selectable_label(self.tab == tab, RichText::new(label).strong()).clicked()
                    {
                        self.tab = tab;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.monospace(format!("{:5.1} FPS", self.emu.fps));
                    ui.label(self.emu.rom_name());
                    if self.emu.paused {
                        ui.colored_label(Color32::from_rgb(240, 170, 60), "paused");
                    }
                });
            });
        });
    }

    fn status_bar(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            let hint = match (self.tab, self.inside) {
                (Tab::Emulate, _) => "X/Z = A/B, S/A = X/Y, Q/W = L/R, Enter = Start, Backspace = Select, arrows = D-pad, mouse = stylus",
                (Tab::Design, false) => "Click the buttons / touch screen to play; drag to move, wheel or pinch to zoom, two-finger scroll to move; double-click the console to open its block design",
                (Tab::Design, true) => "Drag blocks to move, drag the background to pan, wheel or pinch to zoom, two-finger scroll to pan; click a block or a net to inspect it",
            };
            ui.weak(hint);
        });
    }

    fn emulate_tab(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(
            ctx,
            |ui| {
                let area = ui.max_rect();
                let scale = (area.width() / 256.0).min(area.height() / 392.0);
                let size = egui::vec2(256.0, 192.0) * scale;
                let top = egui::Rect::from_min_size(
                    egui::pos2(
                        area.center().x - size.x / 2.0,
                        area.center().y - size.y - 4.0 * scale,
                    ),
                    size,
                );
                let bottom = top.translate(egui::vec2(0.0, size.y + 8.0 * scale));
                let response = ui.allocate_rect(area, egui::Sense::click_and_drag());
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(self.emu.screens[0].id(), top, uv, Color32::WHITE);
                ui.painter().image(self.emu.screens[1].id(), bottom, uv, Color32::WHITE);
                self.emu.stylus =
                    match (response.is_pointer_button_down_on(), response.interact_pointer_pos()) {
                        (true, Some(pos)) => emu::Emu::touch_at(bottom, pos),
                        _ => None,
                    };
                if !self.emu.nds.has_rom() {
                    ui.painter().text(
                        area.center(),
                        egui::Align2::CENTER_CENTER,
                        "File > Open ROM...",
                        egui::FontId::proportional(22.0),
                        Color32::GRAY,
                    );
                }
            },
        );
    }

    /// The drawn console: its buttons and touch screen play the game, a
    /// drag elsewhere moves it, wheel / pinch zooms, two-finger scroll moves,
    /// a double-click (off the controls) opens the block design.
    fn console_view(&mut self, ui: &mut egui::Ui, alpha: f32, zoom: Option<(Pos2, f32)>) {
        let area = ui.max_rect();
        let with_cart = self.emu.nds.emulator().slot2.device().is_some();
        let fit = ds_body::Placement::fit(area, with_cart);
        let center = area.center();
        let mut at = ds_body::Placement {
            origin: center + (fit.origin - center) * self.body_zoom + self.body_pan,
            unit: fit.unit * self.body_zoom,
        };
        if let Some((focus, z)) = zoom {
            at = at.zoomed(focus, z);
        }
        let response = ui.allocate_rect(area, egui::Sense::click_and_drag());
        let interactive = self.transition.is_none();

        let mut pressed = Vec::new();
        if interactive {
            self.console_input(ui, &response, at, &mut pressed);
        }
        let hovered = if interactive && pressed.is_empty() {
            response.hover_pos().and_then(|p| at.hit(p, with_cart))
        } else {
            None
        };
        let painter = ui.painter_at(area);
        ds_body::Painter { painter: &painter, at, alpha }.paint(
            [self.emu.screens[0].id(), self.emu.screens[1].id()],
            with_cart,
            hovered,
            &pressed,
        );
        if interactive {
            if ui
                .put(
                    egui::Rect::from_min_size(
                        area.min + egui::vec2(8.0, 8.0),
                        egui::vec2(90.0, 24.0),
                    ),
                    egui::Button::new("Reset view"),
                )
                .clicked()
            {
                self.body_pan = egui::Vec2::ZERO;
                self.body_zoom = 1.0;
            }
        }
        if let (Some(part), Some(p)) = (hovered, response.hover_pos()) {
            let action = if at.button_at(p).is_some() {
                "click: press"
            } else if part == ds_body::Part::BottomScreen {
                "click / drag: touch"
            } else {
                "double-click: block design"
            };
            response.clone().on_hover_text(format!("{}  ({action})", part.label()));
        }
    }

    fn console_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        at: ds_body::Placement,
        pressed: &mut Vec<lunaris_gui_common::console::Key>,
    ) {
        let with_cart = self.emu.nds.emulator().slot2.device().is_some();
        let bottom = at.bottom_screen();
        let origin = ui.input(|i| i.pointer.press_origin());
        let on_control = |p: Pos2| at.button_at(p).is_some() || bottom.contains(p);

        // What the press started on decides what the drag does: hold a
        // button (sliding follows the pointer, e.g. across the D-pad), draw
        // on the touch screen, or move the console.
        match (response.is_pointer_button_down_on(), origin, response.interact_pointer_pos()) {
            (true, Some(o), Some(p)) if at.button_at(o).is_some() => {
                if let Some(key) = at.button_at(p) {
                    pressed.push(key);
                }
            }
            (true, Some(o), Some(p)) if bottom.contains(o) => {
                self.emu.stylus = emu::Emu::touch_at(bottom, p);
            }
            _ if response.dragged() => self.body_pan += response.drag_delta(),
            _ => {}
        }
        self.emu.mouse_keys.clone_from(pressed);

        if let Some(p) = response.hover_pos() {
            let gesture = nav::read(ui);
            self.body_pan += gesture.pan;
            if gesture.zoom != 1.0 {
                let old = self.body_zoom;
                self.body_zoom = (old * gesture.zoom).clamp(0.3, 8.0);
                self.body_pan = nav::pan_for_zoom(
                    self.body_pan,
                    p - ui.max_rect().center(),
                    old,
                    self.body_zoom,
                );
            }
        }

        if response.double_clicked() {
            if let Some(pos) = response.interact_pointer_pos().filter(|p| !on_control(*p)) {
                let part = at.hit(pos, with_cart);
                self.pending_focus = part.and_then(ds_body::Part::block);
                if let Some(id) = self.pending_focus {
                    self.diagram.select(id);
                }
                self.transition =
                    Some(Transition { start: Instant::now(), entering: true, focus: pos });
            }
        }
    }

    fn design_tab(&mut self, ctx: &egui::Context) {
        let time = self.time();
        let transition = self.transition;
        let progress = transition.map_or(1.0, |t| t.progress());
        let show_inside = self.inside || transition.is_some_and(|t| t.entering);

        if self.inside && transition.is_none() {
            self.inspector_panel(ctx, time);
        }

        let background = Color32::from_rgb(28, 31, 38);
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(background)).show(ctx, |ui| {
            if self.inside && transition.is_none() {
                self.diagram_toolbar(ui);
            }
            match transition {
                Some(t) => {
                    // Console zooms towards the click and fades; the design fades in.
                    let k = if t.entering { progress } else { 1.0 - progress };
                    let z = 1.0 + 9.0 * k * k;
                    if show_inside || !t.entering {
                        let mut child =
                            ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
                        child.multiply_opacity(k);
                        self.diagram.show(&mut child, self.emu.nds.emulator(), time);
                    }
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
                    self.console_view(&mut child, (1.0 - k).powi(2), Some((t.focus, z)));
                }
                None if self.inside => {
                    if let Some(id) = self.pending_focus.take() {
                        self.diagram.zoom = 1.0;
                        self.diagram.focus(id, ui.available_size());
                    }
                    self.diagram.show(ui, self.emu.nds.emulator(), time);
                }
                None => self.console_view(ui, 1.0, None),
            }
        });

        if let Some(t) = transition {
            if t.done() {
                self.inside = t.entering;
                self.transition = None;
            }
        }
    }

    fn diagram_toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Frame::NONE.fill(Color32::from_rgb(40, 44, 54)).inner_margin(6.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                let back = egui::Button::new(RichText::new("<  Back to DS").strong())
                    .fill(Color32::from_rgb(52, 110, 200));
                if ui.add(back).on_hover_text("Return to the console view").clicked() {
                    let focus = ui.ctx().content_rect().center();
                    self.transition =
                        Some(Transition { start: Instant::now(), entering: false, focus });
                }
                ui.separator();
                ui.label(RichText::new("Nintendo DS  >  Block Design").strong());
                ui.separator();
                if ui.button("Fit").clicked() {
                    self.diagram.fit_pending = true;
                    self.diagram.auto_fit = true;
                }
                if ui.button("Reset layout").clicked() {
                    self.diagram.reset_layout();
                }
                if ui.button("Save layout").clicked() {
                    self.diagram.save_layout();
                }
                egui::ComboBox::from_id_salt("find_block").selected_text("Find block...").show_ui(
                    ui,
                    |ui| {
                        for b in diagram::hw::BLOCKS {
                            if ui
                                .selectable_label(self.diagram.selected == Some(b.id), b.title)
                                .clicked()
                            {
                                self.diagram.select(b.id);
                                self.pending_focus = Some(b.id);
                            }
                        }
                    },
                );
                ui.label(format!("zoom {:.0}%", self.diagram.zoom * 100.0));
            });
        });
    }

    fn inspector_panel(&mut self, ctx: &egui::Context, time: f64) {
        egui::SidePanel::right("inspector")
            .resizable(true)
            .default_width(440.0)
            .min_width(300.0)
            .show(ctx, |ui| {
                if let Some(id) = self.diagram.selected {
                    self.inspector.show_block(ui, id, &mut self.emu, time);
                } else if let Some(net) =
                    self.diagram.selected_net.and_then(|i| diagram::hw::NETS.get(i))
                {
                    self.inspector.show_net(ui, net, &self.emu, time);
                } else {
                    ui.heading("Block Properties");
                    ui.label("Select a block or a net in the design.");
                    ui.separator();
                    ui.label(RichText::new("Legend").strong());
                    for (text, color) in [
                        ("bus (memory mapped)", Color32::from_rgb(40, 90, 170)),
                        ("interrupt (red = IF & IE set)", Color32::from_rgb(220, 110, 30)),
                        ("video", Color32::from_rgb(130, 80, 190)),
                        ("audio", Color32::from_rgb(40, 150, 80)),
                        ("signal", Color32::from_rgb(110, 110, 120)),
                    ] {
                        ui.colored_label(color, format!("--- {text}"));
                    }
                    ui.weak(
                        "Moving dots: the registers of one of the net's blocks changed recently.",
                    );
                }
            });
    }
}

impl eframe::App for DesignerApp {
    /// `LUNARIS_DESIGNER_NO_INPUT=1` ignores the host mouse and keyboard, so
    /// unattended screenshots show exactly the start view.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if std::env::var_os("LUNARIS_DESIGNER_NO_INPUT").is_some() {
            raw_input.events.clear();
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        let dt = self.last_repaint.elapsed().as_secs_f32();
        self.last_repaint = Instant::now();

        let keyboard = !ctx.wants_keyboard_input();
        self.emu.update(ctx, keyboard);
        // The view drawn below sets these again while the mouse holds them.
        self.emu.stylus = None;
        self.emu.mouse_keys.clear();
        self.diagram.update_activity(self.emu.nds.emulator(), &self.inspector.catalog, dt);

        self.top_bar(ctx);
        self.status_bar(ctx);
        // Outermost right panel, so the inspector sits to its left.
        self.wave.panel(ctx, &mut self.emu);
        match self.tab {
            Tab::Emulate => self.emulate_tab(ctx),
            Tab::Design => self.design_tab(ctx),
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.diagram.save_layout();
        self.emu.shutdown();
    }
}

fn main() -> eframe::Result<()> {
    let (rgba, [width, height]) = icon();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Lunaris Designer")
        .with_icon(egui::IconData { rgba, width, height })
        .with_inner_size([1280.0, 780.0])
        // The NDS opens its audio device (COM, multithreaded) before winit
        // registers drag-and-drop; see gui/egui/src/main.rs.
        .with_drag_and_drop(false);
    // Unattended screenshots stay off the desktop (PrintWindow still renders it).
    if std::env::var_os("LUNARIS_DESIGNER_NO_INPUT").is_some() {
        viewport = viewport.with_position([-6000.0, 0.0]).with_taskbar(false);
    }
    eframe::run_native(
        "Lunaris Designer",
        eframe::NativeOptions { viewport, ..Default::default() },
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(DesignerApp::new(&cc.egui_ctx)))
        }),
    )
}

#[expect(clippy::unwrap_used, reason = "bundled icon")]
fn icon() -> (Vec<u8>, [u32; 2]) {
    let ico =
        ico::IconDir::read(std::io::Cursor::new(include_bytes!("../../../docs/icons/icon.ico")))
            .unwrap();
    let image = ico.entries().first().unwrap().decode().unwrap();
    (image.rgba_data().to_vec(), [image.width(), image.height()])
}
