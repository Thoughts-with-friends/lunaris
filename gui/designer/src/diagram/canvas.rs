//! Drawing and interaction of the block design, Vivado style: blocks with
//! a coloured header and pins on both sides, orthogonally routed nets,
//! bus nets drawn thick, live nets animated.
//!
//! ```text
//!  drag block header/body  move the block        drag background  pan
//!  wheel / pinch           zoom about pointer    click block      select
//!  two-finger scroll       pan
//!  hover net               live values           click net        select
//! ```

use ds_emu::Emulator;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, Vec2, pos2, vec2};

use super::{
    Diagram,
    hw::{self, Category, Cpu, NetDef, NetKind},
};

pub const HEADER: f32 = 46.0;
pub const ROW: f32 = 18.0;

const fn category_color(c: Category) -> Color32 {
    match c {
        Category::Cpu => Color32::from_rgb(52, 120, 210),
        Category::Interconnect => Color32::from_rgb(90, 98, 112),
        Category::Memory => Color32::from_rgb(46, 150, 96),
        Category::Video => Color32::from_rgb(138, 84, 200),
        Category::Audio => Color32::from_rgb(222, 128, 36),
        Category::Io => Color32::from_rgb(40, 150, 160),
        Category::Expansion => Color32::from_rgb(210, 60, 80),
    }
}

const fn net_color(kind: NetKind) -> Color32 {
    match kind {
        NetKind::Bus => Color32::from_rgb(40, 90, 170),
        NetKind::Irq => Color32::from_rgb(220, 110, 30),
        NetKind::Video => Color32::from_rgb(130, 80, 190),
        NetKind::Audio => Color32::from_rgb(40, 150, 80),
        NetKind::Signal => Color32::from_rgb(110, 110, 120),
    }
}

/// Live state of an interrupt net: (pending & enabled, pending).
pub fn irq_state(emu: &Emulator, net: &NetDef) -> Option<(bool, bool)> {
    let (cpu, bits) = net.irq?;
    let regs = match cpu {
        Cpu::Arm9 => &emu.int9_reg,
        Cpu::Arm7 => &emu.int7_reg,
    };
    let pending = regs.irq_flags & bits != 0;
    Some((pending && regs.irq_enable & bits != 0, pending))
}

struct View {
    origin: Pos2,
    pan: Vec2,
    zoom: f32,
}

impl View {
    fn at(&self, p: Pos2) -> Pos2 {
        self.origin + self.pan + p.to_vec2() * self.zoom
    }
}

impl Diagram {
    fn block_rect(&self, view: &View, id: &str) -> Rect {
        let p = self.pos.get(id).copied().unwrap_or_default();
        Rect::from_min_size(view.at(p), self.block_size(id) * view.zoom)
    }

    fn pin(&self, view: &View, id: &str, port: &str, output: bool) -> Pos2 {
        let rect = self.block_rect(view, id);
        let list = self.ports.get(id).map(|p| if output { &p.outputs } else { &p.inputs });
        let row = list.and_then(|l| l.iter().position(|(n, _)| *n == port)).unwrap_or(0);
        let y = rect.min.y + (HEADER + ROW * (row as f32 + 0.5)) * view.zoom;
        pos2(if output { rect.max.x } else { rect.min.x }, y)
    }

    /// Orthogonal route of net `index`.
    fn route(&self, view: &View, index: usize, net: &NetDef) -> Vec<Pos2> {
        let a = self.pin(view, net.from.0, net.from.1, true);
        let b = self.pin(view, net.to.0, net.to.1, false);
        let lane = (index % 9) as f32 * 7.0 * view.zoom;
        let stub = 14.0 * view.zoom;
        if b.x - a.x > 2.0 * stub + lane {
            let xm = (a.x + stub + lane).min(b.x - stub);
            vec![a, pos2(xm, a.y), pos2(xm, b.y), b]
        } else {
            // Backwards: go round underneath both blocks.
            let below =
                self.block_rect(view, net.from.0).max.y.max(self.block_rect(view, net.to.0).max.y)
                    + 12.0 * view.zoom
                    + lane;
            let (x1, x2) = (a.x + stub + lane, b.x - stub - lane);
            vec![a, pos2(x1, a.y), pos2(x1, below), pos2(x2, below), pos2(x2, b.y), b]
        }
    }

    fn fit(&mut self, size: Vec2) {
        let mut bounds = Rect::NOTHING;
        for b in hw::BLOCKS {
            let p = self.pos.get(b.id).copied().unwrap_or_default();
            bounds = bounds.union(Rect::from_min_size(p, self.block_size(b.id)));
        }
        let bounds = bounds.expand(60.0);
        self.zoom = (size.x / bounds.width()).min(size.y / bounds.height()).clamp(0.2, 2.0);
        self.pan = size / 2.0 - bounds.center().to_vec2() * self.zoom;
    }

    /// Draws the design and handles its input. `time` animates live nets.
    pub fn show(&mut self, ui: &mut egui::Ui, emu: &Emulator, time: f64) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        if self.fit_pending || (self.auto_fit && self.last_size != rect.size()) {
            self.fit(rect.size());
            self.fit_pending = false;
        }
        self.last_size = rect.size();
        let view = View { origin: rect.min, pan: self.pan, zoom: self.zoom };
        painter.rect_filled(rect, 0.0, Color32::from_rgb(250, 251, 253));
        draw_grid(&painter, rect, &view);

        let pointer = response.hover_pos();
        let hovered_block = pointer.and_then(|p| {
            hw::BLOCKS.iter().rev().find(|b| self.block_rect(&view, b.id).contains(p)).map(|b| b.id)
        });

        let routes: Vec<Vec<Pos2>> =
            hw::NETS.iter().enumerate().map(|(i, n)| self.route(&view, i, n)).collect();
        let hovered_net = match (pointer, hovered_block) {
            (Some(p), None) => routes.iter().position(|r| near_polyline(r, p, 5.0)),
            _ => None,
        };

        for (i, (net, route)) in hw::NETS.iter().zip(&routes).enumerate() {
            let highlighted = hovered_net == Some(i)
                || self.selected_net == Some(i)
                || self.selected.is_some_and(|s| s == net.from.0 || s == net.to.0);
            self.draw_net(&painter, &view, emu, net, route, highlighted, time);
        }
        for b in hw::BLOCKS {
            self.draw_block(&painter, &view, b, hovered_block == Some(b.id));
        }

        self.handle_input(ui, &response, &view, hovered_block, hovered_net);

        if let Some(i) = hovered_net {
            let net = &hw::NETS[i];
            response.on_hover_ui_at_pointer(|ui| net_tooltip(ui, emu, net));
        }
    }

    fn handle_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        view: &View,
        hovered_block: Option<&'static str>,
        hovered_net: Option<usize>,
    ) {
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.dragging = hovered_block;
            if let Some(id) = hovered_block {
                self.select(id);
            }
        }
        if response.dragged() {
            self.auto_fit = false;
            let delta = response.drag_delta();
            match self.dragging {
                Some(id) => {
                    if let Some(p) = self.pos.get_mut(id) {
                        *p += delta / self.zoom;
                    }
                }
                None => self.pan += delta,
            }
        }
        if response.drag_stopped() {
            self.dragging = None;
        }
        if response.clicked() {
            match (hovered_block, hovered_net) {
                (Some(id), _) => self.select(id),
                (None, Some(net)) => {
                    self.selected = None;
                    self.selected_net = Some(net);
                }
                (None, None) => {
                    self.selected = None;
                    self.selected_net = None;
                }
            }
        }
        if let (true, Some(p)) = (response.hovered(), response.hover_pos()) {
            let gesture = crate::nav::read(ui);
            if gesture.pan != Vec2::ZERO {
                self.auto_fit = false;
                self.pan += gesture.pan;
            }
            if gesture.zoom != 1.0 {
                self.auto_fit = false;
                let old = self.zoom;
                self.zoom = (old * gesture.zoom).clamp(0.15, 3.0);
                self.pan = crate::nav::pan_for_zoom(self.pan, p - view.origin, old, self.zoom);
            }
        }
    }

    #[expect(clippy::too_many_arguments)]
    fn draw_net(
        &self,
        painter: &egui::Painter,
        view: &View,
        emu: &Emulator,
        net: &NetDef,
        route: &[Pos2],
        highlighted: bool,
        time: f64,
    ) {
        let z = view.zoom;
        let mut color = net_color(net.kind);
        let irq = irq_state(emu, net);
        if let Some((raised, pending)) = irq {
            color = if raised {
                Color32::from_rgb(230, 40, 40)
            } else if pending {
                Color32::from_rgb(240, 170, 60)
            } else {
                color.gamma_multiply(0.55)
            };
        }
        let width = match net.kind {
            NetKind::Bus => 3.2,
            _ => 1.6,
        } * z.max(0.4);
        if highlighted {
            painter.add(Shape::line(
                route.to_vec(),
                Stroke::new(width + 5.0 * z, Color32::from_rgba_unmultiplied(255, 200, 0, 90)),
            ));
        }
        painter.add(Shape::line(route.to_vec(), Stroke::new(width, color)));

        let active = |id| self.activity.get(id).copied().unwrap_or(0.0);
        let level = active(net.from.0).max(active(net.to.0));
        let live = level > 0.05 || irq.is_some_and(|(r, _)| r);
        if live && z > 0.25 {
            // Packets travelling from source to sink.
            let len = polyline_len(route);
            let spacing = 26.0 * z;
            let offset = (time as f32 * 60.0 * z) % spacing;
            let mut d = offset;
            while d < len {
                if let Some(p) = point_along(route, d) {
                    painter.circle_filled(
                        p,
                        (width * 0.9).max(1.5),
                        Color32::WHITE.gamma_multiply(0.4 + 0.6 * level),
                    );
                    painter.circle_stroke(p, (width * 0.9).max(1.5), Stroke::new(1.0_f32, color));
                }
                d += spacing;
            }
        }
    }

    fn draw_block(&self, painter: &egui::Painter, view: &View, def: &hw::BlockDef, hovered: bool) {
        let z = view.zoom;
        let rect = self.block_rect(view, def.id);
        let accent = category_color(def.category);
        let selected = self.selected == Some(def.id);
        let radius = egui::CornerRadius::same((6.0 * z) as u8);

        let shadow = egui::epaint::Shadow {
            offset: [0, (3.0 * z) as i8],
            blur: (10.0 * z) as u8,
            spread: 0,
            color: Color32::from_black_alpha(40),
        };
        painter.add(shadow.as_shape(rect, radius));
        let border = if selected {
            Stroke::new(2.5_f32, Color32::from_rgb(255, 170, 0))
        } else if hovered {
            Stroke::new(1.5_f32, accent)
        } else {
            Stroke::new(1.0_f32, Color32::from_gray(170))
        };
        painter.rect(rect, radius, Color32::WHITE, border, egui::StrokeKind::Outside);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER * z - 6.0 * z));
        painter.rect_filled(
            header,
            egui::CornerRadius { nw: radius.nw, ne: radius.ne, sw: 0, se: 0 },
            accent.gamma_multiply(0.14),
        );
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(4.0 * z, rect.height())),
            egui::CornerRadius { nw: radius.nw, sw: radius.sw, ne: 0, se: 0 },
            accent,
        );

        let level = self.activity.get(def.id).copied().unwrap_or(0.0);
        if level > 0.0 {
            painter.circle_filled(
                rect.right_top() + vec2(-10.0 * z, 10.0 * z),
                4.0 * z,
                Color32::from_rgb(60, 200, 90).gamma_multiply(level),
            );
        }

        if z < 0.3 {
            return;
        }
        painter.text(
            rect.min + vec2(10.0 * z, 6.0 * z),
            egui::Align2::LEFT_TOP,
            def.title,
            egui::FontId::proportional(14.0 * z),
            Color32::from_gray(25),
        );
        painter.text(
            rect.min + vec2(10.0 * z, 24.0 * z),
            egui::Align2::LEFT_TOP,
            def.subtitle,
            egui::FontId::proportional(10.5 * z),
            Color32::from_gray(110),
        );

        let Some(ports) = self.ports.get(def.id) else { return };
        let font = egui::FontId::monospace(9.5 * z);
        for (output, list) in [(false, &ports.inputs), (true, &ports.outputs)] {
            for (name, kind) in list {
                let pin = self.pin(view, def.id, name, output);
                let color = net_color(*kind);
                let dir = if output { 1.0 } else { -1.0 };
                if *kind == NetKind::Bus {
                    // Interface pin: a circled "+", as Vivado draws AXI ports.
                    painter.circle(pin, 5.0 * z, Color32::WHITE, Stroke::new(1.5 * z, color));
                    painter.line_segment(
                        [pin - vec2(3.0 * z, 0.0), pin + vec2(3.0 * z, 0.0)],
                        Stroke::new(1.2 * z, color),
                    );
                    painter.line_segment(
                        [pin - vec2(0.0, 3.0 * z), pin + vec2(0.0, 3.0 * z)],
                        Stroke::new(1.2 * z, color),
                    );
                } else {
                    let tip = pin + vec2(dir * 5.0 * z, 0.0);
                    painter.add(Shape::convex_polygon(
                        vec![pin + vec2(0.0, -3.5 * z), tip, pin + vec2(0.0, 3.5 * z)],
                        color,
                        Stroke::NONE,
                    ));
                }
                let (align, at) = if output {
                    (egui::Align2::RIGHT_CENTER, pin - vec2(9.0 * z, 0.0))
                } else {
                    (egui::Align2::LEFT_CENTER, pin + vec2(9.0 * z, 0.0))
                };
                painter.text(at, align, *name, font.clone(), Color32::from_gray(60));
            }
        }
    }
}

fn net_tooltip(ui: &mut egui::Ui, emu: &Emulator, net: &NetDef) {
    ui.strong(net.name);
    ui.label(format!("{}.{}  ->  {}.{}", net.from.0, net.from.1, net.to.0, net.to.1));
    if let Some((cpu, bits)) = net.irq {
        let regs = if cpu == Cpu::Arm9 { &emu.int9_reg } else { &emu.int7_reg };
        ui.monospace(format!("mask  {bits:08X}"));
        ui.monospace(format!("IF    {:08X}", regs.irq_flags & bits));
        ui.monospace(format!("IE    {:08X}", regs.irq_enable & bits));
        ui.monospace(format!("IME   {}", regs.ime & 1));
    }
    ui.weak("click to inspect");
}

fn draw_grid(painter: &egui::Painter, rect: Rect, view: &View) {
    let step = 40.0 * view.zoom;
    if step < 8.0 {
        return;
    }
    let color = Color32::from_rgb(228, 232, 238);
    let start = rect.min + vec2(view.pan.x.rem_euclid(step), view.pan.y.rem_euclid(step));
    let mut x = start.x;
    while x < rect.max.x {
        painter
            .line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], Stroke::new(1.0_f32, color));
        x += step;
    }
    let mut y = start.y;
    while y < rect.max.y {
        painter
            .line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], Stroke::new(1.0_f32, color));
        y += step;
    }
}

fn segment_distance(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() == 0.0 {
        0.0
    } else {
        ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0)
    };
    (a + ab * t).distance(p)
}

fn near_polyline(points: &[Pos2], p: Pos2, tolerance: f32) -> bool {
    points.windows(2).any(|w| segment_distance(w[0], w[1], p) <= tolerance)
}

fn polyline_len(points: &[Pos2]) -> f32 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

fn point_along(points: &[Pos2], mut d: f32) -> Option<Pos2> {
    for w in points.windows(2) {
        let len = w[0].distance(w[1]);
        if d <= len {
            return Some(w[0] + (w[1] - w[0]) * (d / len.max(f32::EPSILON)));
        }
        d -= len;
    }
    None
}
