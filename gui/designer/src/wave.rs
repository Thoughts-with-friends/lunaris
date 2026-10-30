//! The waveform window: FPGA signals scrolling live on the right of the
//! UI while the ROM runs, like a simulator's wave view (Vivado's
//! behavioural simulation).
//!
//! ```text
//!  | block [v] [x] follow  window 64 clk   [-][+]  filter ____  |
//!  |            0      64      128     192     256  (FPGA clock) |
//!  | clk        _|‾|_|‾|_|‾|_|‾|_|‾|_|‾|_|‾|_|‾|_                |
//!  | led   1    ______|‾‾‾‾‾‾‾‾‾‾‾‾‾‾|______                     |
//!  | pc    1C   <0018><001C><0020><0024>...                      |
//! ```
//!
//! Data: `ds_fpga::trace::Trace` of each block (value changes per FPGA
//! clock; Verilog blocks: every signal, spec blocks: every register).
//! Wheel zooms, drag pans (and stops following), double-click resumes
//! following, the hover line shows every value at that clock.

use std::collections::HashSet;

use ds_fpga::trace::Trace;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, pos2, vec2};

use crate::emu::Emu;

const ROW: f32 = 22.0;
const NAMES: f32 = 150.0;
const BG: Color32 = Color32::from_rgb(10, 12, 14);
const WAVE: Color32 = Color32::from_rgb(70, 230, 100);
const BUS: Color32 = Color32::from_rgb(90, 210, 230);
const CURSOR: Color32 = Color32::from_rgb(255, 210, 60);

pub struct WaveView {
    pub open: bool,
    block: usize,
    /// Visible clocks.
    window: f64,
    follow: bool,
    /// Right edge, in clocks, while not following.
    end: f64,
    filter: String,
    hidden: HashSet<String>,
    tracing: bool,
}

impl WaveView {
    pub fn new() -> Self {
        Self {
            open: true,
            block: 0,
            window: 64.0,
            follow: true,
            end: 0.0,
            filter: String::new(),
            hidden: HashSet::new(),
            tracing: false,
        }
    }

    /// Draws the panel when an FPGA block exists (and keeps recording on
    /// exactly while the panel is shown).
    pub fn panel(&mut self, ctx: &egui::Context, emu: &mut Emu) {
        let Some(fpga) = emu.fpga() else {
            self.tracing = false;
            return;
        };
        let show = self.open && !fpga.blocks().is_empty();
        if show {
            fpga.ensure_tracing();
        } else if self.tracing {
            fpga.set_tracing(false);
        }
        self.tracing = show;
        if !show {
            return;
        }
        self.block = self.block.min(fpga.blocks().len() - 1);
        let names: Vec<String> = fpga.blocks().iter().map(|b| b.spec.name.clone()).collect();
        let block = &fpga.blocks()[self.block];
        let Some(trace) = block.core.trace() else { return };
        let title = format!("{} ({})", block.spec.name, block.backend);

        egui::SidePanel::right("waveform")
            .resizable(true)
            .default_width(560.0)
            .min_width(320.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Waveform");
                    egui::ComboBox::from_id_salt("wave_block").selected_text(&title).show_ui(
                        ui,
                        |ui| {
                            for (i, name) in names.iter().enumerate() {
                                ui.selectable_value(&mut self.block, i, name);
                            }
                        },
                    );
                    if ui.button("x").on_hover_text("close (Emulation > Waveform window)").clicked()
                    {
                        self.open = false;
                    }
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.follow, "follow");
                    if ui.button("-").on_hover_text("zoom out").clicked() {
                        self.window = (self.window * 2.0).min(1e9);
                    }
                    if ui.button("+").on_hover_text("zoom in").clicked() {
                        self.window = (self.window / 2.0).max(8.0);
                    }
                    ui.weak(format!("{:.0} clk  |  now {}", self.window, trace.now));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("filter")
                            .desired_width(90.0),
                    );
                });
                ui.separator();
                self.waves(ui, trace);
            });
    }

    fn visible(&self, trace: &Trace) -> Vec<usize> {
        let filter = self.filter.to_ascii_lowercase();
        (0..trace.signals.len())
            .filter(|&i| {
                let name = &trace.signals[i].name;
                name != "clk"
                    && !self.hidden.contains(name)
                    && name.to_ascii_lowercase().contains(&filter)
            })
            .collect()
    }

    fn waves(&mut self, ui: &mut egui::Ui, trace: &Trace) {
        let rows = self.visible(trace);
        if self.follow {
            self.end = trace.now as f64;
        }
        let t1 = self.end;
        let t0 = t1 - self.window;
        let height = ROW * (rows.len() + 2) as f32;
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, BG);
            let waves = Rect::from_min_max(pos2(rect.min.x + NAMES, rect.min.y), rect.max);
            let window = self.window;
            let x_of = |t: f64| waves.min.x + ((t - t0) / window) as f32 * waves.width();
            let t_of = |x: f32| t0 + f64::from((x - waves.min.x) / waves.width()) * window;

            // Interaction.
            if response.dragged() {
                self.follow = false;
                self.end -= f64::from(response.drag_delta().x / waves.width()) * self.window;
            }
            if response.double_clicked() {
                self.follow = true;
            }
            let cursor = response
                .hover_pos()
                .filter(|p| p.x > waves.min.x)
                .map(|p| t_of(p.x).floor().max(0.0) as u64);
            if response.hovered() {
                let zoom = ui.input(|i| {
                    let lines: f32 = i
                        .raw
                        .events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                            _ => None,
                        })
                        .sum();
                    i.zoom_delta() * (lines * 0.15).exp()
                });
                if (zoom - 1.0).abs() > f32::EPSILON {
                    self.window = (self.window / f64::from(zoom)).clamp(8.0, 1e9);
                }
            }

            ruler(
                &painter,
                Rect::from_min_size(waves.min, vec2(waves.width(), ROW)),
                t0,
                window,
                x_of,
            );
            let row_rect = |r: usize| {
                Rect::from_min_size(
                    pos2(rect.min.x, rect.min.y + ROW * (r + 1) as f32),
                    vec2(rect.width(), ROW),
                )
            };

            // clk: drawn, not recorded (it toggles twice per clock).
            let clk = row_rect(0);
            name_cell(&painter, clk, "clk", None);
            draw_clock(
                &painter,
                Rect::from_min_max(pos2(waves.min.x, clk.min.y), clk.max),
                t0,
                window,
            );

            for (r, &sig) in rows.iter().enumerate() {
                let row = row_rect(r + 1);
                let s = &trace.signals[sig];
                let at = cursor.unwrap_or(trace.now);
                name_cell(
                    &painter,
                    row,
                    &s.name,
                    trace.value_at(sig, at).map(|v| fmt_value(v, s.width)),
                );
                let area = Rect::from_min_max(
                    pos2(waves.min.x, row.min.y + 3.0),
                    pos2(waves.max.x, row.max.y - 3.0),
                );
                draw_signal(&painter, area, trace, sig, t0, t1, &x_of);
                painter.line_segment(
                    [pos2(rect.min.x, row.max.y), pos2(rect.max.x, row.max.y)],
                    Stroke::new(1.0_f32, Color32::from_gray(28)),
                );
                let hide = ui.interact(
                    Rect::from_min_size(row.min, vec2(NAMES, ROW)),
                    ui.id().with(("hide", sig)),
                    Sense::click(),
                );
                if hide.on_hover_text("click: hide this signal").clicked() {
                    self.hidden.insert(s.name.clone());
                }
            }
            painter.line_segment(
                [pos2(waves.min.x, rect.min.y), pos2(waves.min.x, rect.max.y)],
                Stroke::new(1.0_f32, Color32::from_gray(60)),
            );

            if let Some(t) = cursor {
                let x = x_of(t as f64);
                painter.line_segment(
                    [pos2(x, rect.min.y), pos2(x, rect.max.y)],
                    Stroke::new(1.0_f32, CURSOR),
                );
                painter.text(
                    pos2(x + 3.0, rect.min.y + 2.0),
                    Align2::LEFT_TOP,
                    t.to_string(),
                    FontId::monospace(10.0),
                    CURSOR,
                );
            }
        });
        if !self.hidden.is_empty()
            && ui.button(format!("show {} hidden", self.hidden.len())).clicked()
        {
            self.hidden.clear();
        }
    }
}

fn ruler(painter: &egui::Painter, rect: Rect, t0: f64, window: f64, x_of: impl Fn(f64) -> f32) {
    // A tick every power-of-two clocks so labels sit ~80 px apart.
    let per_px = window / f64::from(rect.width().max(1.0));
    let step = (per_px * 80.0).max(1.0).log2().ceil().exp2();
    let mut t = (t0 / step).ceil() * step;
    while t <= t0 + window {
        let x = x_of(t);
        painter.line_segment(
            [pos2(x, rect.max.y - 5.0), pos2(x, rect.max.y)],
            Stroke::new(1.0_f32, Color32::GRAY),
        );
        if t >= 0.0 {
            painter.text(
                pos2(x + 2.0, rect.min.y + 3.0),
                Align2::LEFT_TOP,
                format!("{t:.0}"),
                FontId::monospace(10.0),
                Color32::GRAY,
            );
        }
        t += step;
    }
}

fn fmt_value(v: u64, width: u32) -> String {
    if width == 1 { v.to_string() } else { format!("{v:0w$X}", w = width.div_ceil(4) as usize) }
}

fn name_cell(painter: &egui::Painter, row: Rect, name: &str, value: Option<String>) {
    painter.text(
        pos2(row.min.x + 4.0, row.center().y),
        Align2::LEFT_CENTER,
        name,
        FontId::monospace(11.0),
        Color32::from_gray(230),
    );
    if let Some(v) = value {
        painter.text(
            pos2(row.min.x + NAMES - 4.0, row.center().y),
            Align2::RIGHT_CENTER,
            v,
            FontId::monospace(10.0),
            BUS,
        );
    }
}

fn draw_clock(painter: &egui::Painter, area: Rect, t0: f64, window: f64) {
    let per_clock = f64::from(area.width()) / window;
    let (hi, lo) = (area.min.y + 4.0, area.max.y - 4.0);
    if per_clock < 4.0 {
        // Too dense to draw edges: a band, as wave viewers do.
        painter.rect_filled(
            Rect::from_min_max(pos2(area.min.x, hi), pos2(area.max.x, lo)),
            0.0,
            WAVE.gamma_multiply(0.35),
        );
        return;
    }
    let mut points = Vec::new();
    let mut t = t0.floor();
    while t <= t0 + window {
        let x = |t: f64| area.min.x + ((t - t0) * per_clock) as f32;
        // Rising edge at each clock, falling half way.
        points.extend([pos2(x(t), lo), pos2(x(t), hi), pos2(x(t + 0.5), hi), pos2(x(t + 0.5), lo)]);
        t += 1.0;
    }
    painter.add(egui::Shape::line(points, Stroke::new(1.0_f32, WAVE)));
}

fn draw_signal(
    painter: &egui::Painter,
    area: Rect,
    trace: &Trace,
    sig: usize,
    t0: f64,
    t1: f64,
    x_of: &impl Fn(f64) -> f32,
) {
    let s = &trace.signals[sig];
    let changes = &s.changes;
    let start = changes.partition_point(|&(t, _)| (t as f64) <= t0.max(0.0)).saturating_sub(1);
    let clamp = |x: f32| x.clamp(area.min.x, area.max.x);
    // Segments: (from, to, value).
    let mut segments: Vec<(f32, f32, u64)> = Vec::new();
    for (i, &(t, v)) in changes.iter().enumerate().skip(start) {
        if t as f64 > t1 {
            break;
        }
        let next = changes.get(i + 1).map_or(trace.now as f64, |&(nt, _)| nt as f64).min(t1);
        let (a, b) = (clamp(x_of(t as f64)), clamp(x_of(next)));
        segments.push((a, b.max(a), v));
    }
    let (hi, lo, mid) = (area.min.y, area.max.y, area.center().y);
    if s.width == 1 {
        let mut points: Vec<Pos2> = Vec::new();
        for &(a, b, v) in &segments {
            let y = if v != 0 { hi } else { lo };
            points.push(pos2(a, y));
            points.push(pos2(b, y));
        }
        painter.add(egui::Shape::line(points, Stroke::new(1.2_f32, WAVE)));
        return;
    }
    for &(a, b, v) in &segments {
        let slant = ((b - a) / 2.0).min(3.0);
        let shape = vec![
            pos2(a, mid),
            pos2(a + slant, hi),
            pos2(b - slant, hi),
            pos2(b, mid),
            pos2(b - slant, lo),
            pos2(a + slant, lo),
        ];
        painter.add(egui::Shape::closed_line(shape, Stroke::new(1.0_f32, BUS)));
        let text = fmt_value(v, s.width);
        let room = b - a - 2.0 * slant;
        if room > 7.0 * text.len() as f32 {
            painter.text(
                pos2((a + b) / 2.0, mid),
                Align2::CENTER_CENTER,
                text,
                FontId::monospace(10.0),
                Color32::WHITE,
            );
        }
    }
}
