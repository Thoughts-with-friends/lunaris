//! The DS itself, painted from vector shapes (no bitmap assets): a white
//! DS Lite-style clamshell with both live screens in it.
//!
//! ```text
//!   +-----------------------------+   lid:   speakers L/R, top LCD
//!   |  ::  [  top LCD   ]  ::     |
//!   +=============o===========**==+   hinge: mic (o), power LEDs (**)
//!   |  +   [ bottom LCD ]   X     |   base:  D-pad, ABXY, START/SELECT
//!   | +++  [ (touch)    ]  Y A    |
//!   |  +                    B     |
//!   +-----------------------------+
//!          [ Slot-2 cart ]            only while an FPGA is inserted
//! ```
//!
//! Everything is laid out in millimetres of the real console (133 mm wide)
//! and scaled by `unit` points per millimetre. Each clickable [`Part`] maps
//! to a block of the block design.

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Shape, Stroke, TextureId, pos2, vec2};
use lunaris_gui_common::console::Key;

/// Width of the console in millimetres.
pub const WIDTH_MM: f32 = 133.0;
/// Height without / with a Slot-2 cartridge sticking out.
pub const HEIGHT_MM: f32 = 152.0;
pub const HEIGHT_WITH_CART_MM: f32 = 162.0;

/// A part of the console that leads into the block design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    TopScreen,
    BottomScreen,
    Speakers,
    DPad,
    Buttons,
    StartSelect,
    Mic,
    PowerLed,
    Slot2,
    Shell,
}

impl Part {
    pub const fn label(self) -> &'static str {
        match self {
            Self::TopScreen => "Top LCD (256x192)",
            Self::BottomScreen => "Bottom LCD + touch screen",
            Self::Speakers => "Stereo speakers",
            Self::DPad => "D-pad",
            Self::Buttons => "A / B / X / Y",
            Self::StartSelect => "START / SELECT",
            Self::Mic => "Microphone",
            Self::PowerLed => "Power / charge LEDs",
            Self::Slot2 => "Slot-2: FPGA cartridge",
            Self::Shell => "Nintendo DS (open the block design)",
        }
    }

    /// The block of the design this part opens.
    pub const fn block(self) -> Option<&'static str> {
        match self {
            Self::TopScreen => Some("lcd_top"),
            Self::BottomScreen => Some("tsc"),
            Self::Speakers => Some("speaker"),
            Self::DPad | Self::Buttons | Self::StartSelect => Some("keypad"),
            Self::Mic => Some("tsc"),
            Self::PowerLed => Some("pmic"),
            Self::Slot2 => Some("slot2"),
            Self::Shell => None,
        }
    }
}

/// Where the console is drawn: `origin` is its top-left corner.
#[derive(Debug, Clone, Copy)]
pub struct Placement {
    pub origin: Pos2,
    /// Points per millimetre.
    pub unit: f32,
}

impl Placement {
    /// Centres the console in `area` with a margin.
    pub fn fit(area: Rect, with_cart: bool) -> Self {
        let h = if with_cart { HEIGHT_WITH_CART_MM } else { HEIGHT_MM };
        let unit = ((area.width() * 0.9) / WIDTH_MM).min((area.height() * 0.92) / h);
        let size = vec2(WIDTH_MM, h) * unit;
        Self { origin: area.center() - size / 2.0, unit }
    }

    /// The same placement magnified `zoom` times about `focus`.
    pub fn zoomed(self, focus: Pos2, zoom: f32) -> Self {
        Self { origin: focus + (self.origin - focus) * zoom, unit: self.unit * zoom }
    }

    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.unit
    }

    fn r(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(self.p(x0, y0), self.p(x1, y1))
    }

    fn cr(&self, mm: f32) -> CornerRadius {
        CornerRadius::same((mm * self.unit).clamp(0.0, 255.0) as u8)
    }

    pub fn top_screen(&self) -> Rect {
        self.r(35.5, 12.5, 97.5, 59.0)
    }

    pub fn bottom_screen(&self) -> Rect {
        self.r(35.5, 89.0, 97.5, 135.5)
    }

    /// The DS button under `pos` (D-pad by its dominant direction).
    pub fn button_at(&self, pos: Pos2) -> Option<Key> {
        let mm = (pos - self.origin) / self.unit;
        let (x, y) = (mm.x, mm.y);
        let (dx, dy) = (x - 17.0, y - 101.0);
        if dx.abs().max(dy.abs()) <= 6.5 && dx.abs().min(dy.abs()) <= 2.6 && dx.hypot(dy) > 0.8 {
            return Some(match (dx.abs() > dy.abs(), dx > 0.0, dy > 0.0) {
                (true, true, _) => Key::Right,
                (true, false, _) => Key::Left,
                (false, _, true) => Key::Down,
                (false, _, false) => Key::Up,
            });
        }
        let abxy =
            [(Key::X, 0.0, -6.2), (Key::Y, -6.2, 0.0), (Key::A, 6.2, 0.0), (Key::B, 0.0, 6.2)];
        if let Some((key, ..)) =
            abxy.iter().find(|(_, ox, oy)| (x - 116.0 - ox).hypot(y - 101.0 - oy) <= 3.6)
        {
            return Some(*key);
        }
        if (110.0..=125.0).contains(&x) {
            if (119.4..=122.6).contains(&y) {
                return Some(Key::Start);
            }
            if (126.4..=129.6).contains(&y) {
                return Some(Key::Select);
            }
        }
        if (80.0..=85.5).contains(&y) {
            if (1.0..=17.0).contains(&x) {
                return Some(Key::L);
            }
            if (116.0..=132.0).contains(&x) {
                return Some(Key::R);
            }
        }
        None
    }

    /// Hit areas, most specific first.
    fn parts(&self) -> [(Part, Rect); 10] {
        [
            (Part::TopScreen, self.top_screen()),
            (Part::BottomScreen, self.bottom_screen()),
            (Part::DPad, self.r(5.0, 89.0, 29.0, 113.0)),
            (Part::Buttons, self.r(104.0, 89.0, 128.0, 113.0)),
            (Part::StartSelect, self.r(108.0, 118.0, 126.0, 132.0)),
            (Part::Mic, self.r(63.0, 73.0, 70.0, 80.0)),
            (Part::PowerLed, self.r(112.0, 73.0, 126.0, 80.0)),
            (Part::Speakers, self.r(18.0, 26.0, 115.0, 46.0)),
            (Part::Slot2, self.r(38.0, 152.0, 95.0, 162.0)),
            (Part::Shell, self.r(0.0, 0.0, WIDTH_MM, HEIGHT_MM)),
        ]
    }

    /// The part under `pos`. Speakers only count left and right of the
    /// screen, the cartridge only while one is drawn.
    pub fn hit(&self, pos: Pos2, with_cart: bool) -> Option<Part> {
        self.parts().into_iter().find_map(|(part, rect)| {
            let ok = match part {
                Part::Speakers => {
                    rect.contains(pos) && !self.r(32.0, 6.0, 101.0, 66.0).contains(pos)
                }
                Part::Slot2 => with_cart && rect.contains(pos),
                _ => rect.contains(pos),
            };
            ok.then_some(part)
        })
    }

    pub fn part_rect(&self, part: Part) -> Rect {
        self.parts().into_iter().find(|(p, _)| *p == part).map_or(Rect::NOTHING, |(_, r)| r)
    }
}

/// Paints the console. `alpha` fades everything (for the zoom transition).
pub struct Painter<'a> {
    pub painter: &'a egui::Painter,
    pub at: Placement,
    pub alpha: f32,
}

const SHELL: Color32 = Color32::from_rgb(238, 240, 243);
const SHELL_EDGE: Color32 = Color32::from_rgb(196, 200, 207);
const GLASS: Color32 = Color32::from_rgb(18, 19, 23);
const BUTTON: Color32 = Color32::from_rgb(222, 225, 230);
const ACCENT: Color32 = Color32::from_rgb(64, 156, 255);

impl Painter<'_> {
    fn c(&self, color: Color32) -> Color32 {
        color.gamma_multiply(self.alpha)
    }

    fn shell_half(&self, rect: Rect) {
        let at = &self.at;
        let shadow = egui::epaint::Shadow {
            offset: [0, (2.5 * at.unit) as i8],
            blur: (6.0 * at.unit).min(255.0) as u8,
            spread: 0,
            color: self.c(Color32::from_black_alpha(90)),
        };
        self.painter.add(shadow.as_shape(rect, at.cr(8.0)));
        self.painter.rect(
            rect,
            at.cr(8.0),
            self.c(SHELL),
            Stroke::new(at.unit * 0.5, self.c(SHELL_EDGE)),
            egui::StrokeKind::Inside,
        );
        // Soft top sheen.
        let sheen = Rect::from_min_max(
            rect.min + vec2(at.unit, at.unit),
            pos2(rect.max.x - at.unit, rect.center().y),
        );
        self.painter.rect_filled(sheen, at.cr(7.0), self.c(Color32::from_white_alpha(70)));
    }

    fn glass(&self, rect: Rect) {
        let at = &self.at;
        self.painter.rect_filled(rect, at.cr(3.0), self.c(GLASS));
        // Diagonal reflection across the upper-left of the panel.
        let a = rect.left_top() + vec2(rect.width() * 0.02, rect.height() * 0.02);
        let points = vec![
            a,
            a + vec2(rect.width() * 0.45, 0.0),
            a + vec2(rect.width() * 0.25, rect.height() * 0.5),
            a + vec2(0.0, rect.height() * 0.5),
        ];
        self.painter.add(Shape::convex_polygon(
            points,
            self.c(Color32::from_white_alpha(10)),
            Stroke::NONE,
        ));
    }

    fn screen(&self, rect: Rect, texture: TextureId) {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        self.painter.image(texture, rect, uv, self.c(Color32::WHITE));
        self.painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(self.at.unit * 0.4, self.c(Color32::from_gray(50))),
            egui::StrokeKind::Outside,
        );
    }

    fn speaker(&self, x0: f32, y0: f32) {
        for row in 0..5 {
            for col in 0..3 {
                let p = self.at.p(x0 + col as f32 * 2.6, y0 + row as f32 * 2.6);
                self.painter.circle_filled(p, self.at.unit * 0.65, self.c(Color32::from_gray(70)));
            }
        }
    }

    const fn face(down: bool) -> Color32 {
        if down { Color32::from_rgb(170, 176, 186) } else { BUTTON }
    }

    fn round_button(&self, center: Pos2, radius: f32, label: &str, down: bool) {
        let u = self.at.unit;
        let center = if down { center + vec2(0.0, 0.4 * u) } else { center };
        self.painter.circle_filled(
            center + vec2(0.0, 0.5 * u),
            radius,
            self.c(Color32::from_black_alpha(40)),
        );
        self.painter.circle(
            center,
            radius,
            self.c(Self::face(down)),
            Stroke::new(0.35 * u, self.c(SHELL_EDGE)),
        );
        self.painter.circle_filled(
            center - vec2(0.0, radius * 0.35),
            radius * 0.55,
            self.c(Color32::from_white_alpha(60)),
        );
        self.painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(3.0 * u),
            self.c(Color32::from_gray(140)),
        );
    }

    fn dpad(&self, pressed: &[Key]) {
        let (u, c) = (self.at.unit, self.at.p(17.0, 101.0));
        let arm = 11.0 * u;
        let w = 3.8 * u;
        for rect in
            [Rect::from_center_size(c, vec2(arm, w)), Rect::from_center_size(c, vec2(w, arm))]
        {
            self.painter.rect_filled(
                rect.translate(vec2(0.0, 0.5 * u)),
                self.at.cr(0.8),
                self.c(Color32::from_black_alpha(40)),
            );
            self.painter.rect(
                rect,
                self.at.cr(0.8),
                self.c(BUTTON),
                Stroke::new(0.35 * u, self.c(SHELL_EDGE)),
                egui::StrokeKind::Inside,
            );
        }
        self.painter.circle_filled(c, 1.0 * u, self.c(Color32::from_gray(200)));
        // The pressed arm darkens.
        for (key, dir) in [
            (Key::Up, vec2(0.0, -1.0)),
            (Key::Down, vec2(0.0, 1.0)),
            (Key::Left, vec2(-1.0, 0.0)),
            (Key::Right, vec2(1.0, 0.0)),
        ] {
            if pressed.contains(&key) {
                let arm_rect = Rect::from_center_size(
                    c + dir * 3.6 * u,
                    if dir.x == 0.0 { vec2(w, 3.6 * u) } else { vec2(3.6 * u, w) },
                );
                self.painter.rect_filled(
                    arm_rect,
                    self.at.cr(0.6),
                    self.c(Color32::from_black_alpha(60)),
                );
            }
        }
    }

    fn shoulder(&self, rect: Rect, label: &str, down: bool) {
        let u = self.at.unit;
        let rect = if down { rect.translate(vec2(0.0, 0.4 * u)) } else { rect };
        self.painter.rect(
            rect,
            self.at.cr(1.5),
            self.c(Self::face(down)),
            Stroke::new(0.35 * u, self.c(SHELL_EDGE)),
            egui::StrokeKind::Inside,
        );
        self.painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(2.4 * u),
            self.c(Color32::from_gray(140)),
        );
    }

    fn pill(&self, center: Pos2, label: &str, down: bool) {
        let u = self.at.unit;
        let rect = Rect::from_center_size(center, vec2(5.5 * u, 2.4 * u));
        self.painter.rect(
            rect,
            self.at.cr(1.2),
            self.c(Self::face(down)),
            Stroke::new(0.3 * u, self.c(SHELL_EDGE)),
            egui::StrokeKind::Inside,
        );
        self.painter.text(
            center + vec2(5.0 * u, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(1.9 * u),
            self.c(Color32::from_gray(150)),
        );
    }

    fn hinge(&self) {
        let at = &self.at;
        let rect = at.r(5.0, 72.0, 128.0, 81.0);
        // A cylinder: darker rims around a lighter centre band.
        self.painter.rect_filled(rect, at.cr(4.5), self.c(Color32::from_rgb(214, 217, 222)));
        self.painter.rect_filled(
            at.r(6.0, 75.0, 127.0, 78.0),
            at.cr(1.5),
            self.c(Color32::from_rgb(232, 234, 238)),
        );
        self.painter.circle_filled(at.p(66.5, 76.5), 0.9 * at.unit, self.c(Color32::from_gray(60))); // mic
        self.painter.circle_filled(
            at.p(116.0, 76.5),
            1.0 * at.unit,
            self.c(Color32::from_rgb(70, 220, 110)),
        ); // power
        self.painter.circle_filled(
            at.p(116.0, 76.5),
            2.2 * at.unit,
            self.c(Color32::from_rgba_unmultiplied(70, 220, 110, 40)),
        );
        self.painter.circle_filled(
            at.p(122.0, 76.5),
            1.0 * at.unit,
            self.c(Color32::from_rgb(90, 90, 90)),
        ); // charge
    }

    fn cartridge(&self) {
        let at = &self.at;
        let rect = at.r(40.0, 147.0, 93.0, 161.0);
        self.painter.rect(
            rect,
            at.cr(1.5),
            self.c(Color32::from_rgb(34, 40, 52)),
            Stroke::new(0.4 * at.unit, self.c(ACCENT)),
            egui::StrokeKind::Inside,
        );
        let chip = at.r(43.0, 154.0, 52.0, 159.5);
        self.painter.rect_filled(chip, at.cr(0.5), self.c(Color32::from_rgb(60, 66, 80)));
        self.painter.text(
            at.p(70.0, 157.0),
            egui::Align2::CENTER_CENTER,
            "LUNARIS FPGA",
            egui::FontId::monospace(2.6 * at.unit),
            self.c(ACCENT),
        );
    }

    /// Paints the whole console with `screens` (top, bottom).
    /// `pressed`: buttons drawn held down.
    pub fn paint(
        &self,
        screens: [TextureId; 2],
        with_cart: bool,
        hovered: Option<Part>,
        pressed: &[Key],
    ) {
        let at = &self.at;
        if with_cart {
            self.cartridge();
        }
        self.shell_half(at.r(0.0, 0.0, WIDTH_MM, 74.0));
        self.shell_half(at.r(0.0, 79.0, WIDTH_MM, HEIGHT_MM));
        self.hinge();
        self.shoulder(at.r(1.0, 80.0, 17.0, 85.5), "L", pressed.contains(&Key::L));
        self.shoulder(at.r(116.0, 80.0, 132.0, 85.5), "R", pressed.contains(&Key::R));

        self.glass(at.r(14.0, 6.0, 119.0, 66.0));
        self.speaker(21.0, 30.0);
        self.speaker(106.0, 30.0);
        self.screen(at.top_screen(), screens[0]);
        self.painter.text(
            at.p(66.5, 62.5),
            egui::Align2::CENTER_CENTER,
            "Nintendo DS lite",
            egui::FontId::proportional(2.4 * at.unit),
            self.c(Color32::from_gray(150)),
        );

        self.glass(at.r(32.0, 85.5, 101.0, 139.0));
        self.screen(at.bottom_screen(), screens[1]);

        self.dpad(pressed);
        let abxy = at.p(116.0, 101.0);
        let (o, r) = (6.2 * at.unit, 3.3 * at.unit);
        self.round_button(abxy + vec2(0.0, -o), r, "X", pressed.contains(&Key::X));
        self.round_button(abxy + vec2(-o, 0.0), r, "Y", pressed.contains(&Key::Y));
        self.round_button(abxy + vec2(o, 0.0), r, "A", pressed.contains(&Key::A));
        self.round_button(abxy + vec2(0.0, o), r, "B", pressed.contains(&Key::B));
        self.pill(at.p(114.0, 121.0), "START", pressed.contains(&Key::Start));
        self.pill(at.p(114.0, 128.0), "SELECT", pressed.contains(&Key::Select));

        if let Some(part) = hovered {
            let rect = if part == Part::Shell {
                at.r(0.0, 0.0, WIDTH_MM, HEIGHT_MM)
            } else {
                at.part_rect(part)
            };
            let stroke = Stroke::new(2.0_f32, self.c(ACCENT));
            self.painter.rect_stroke(
                rect.expand(2.0),
                at.cr(2.0),
                stroke,
                egui::StrokeKind::Outside,
            );
            self.painter.rect_filled(
                rect,
                at.cr(2.0),
                self.c(Color32::from_rgba_unmultiplied(64, 156, 255, 24)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_are_hit_where_they_are_drawn() {
        let at = Placement { origin: pos2(0.0, 0.0), unit: 1.0 };
        let cases = [
            ((17.0, 96.0), Some(Key::Up)),
            ((17.0, 106.0), Some(Key::Down)),
            ((12.0, 101.0), Some(Key::Left)),
            ((22.0, 101.0), Some(Key::Right)),
            ((17.0, 101.0), None),
            ((116.0, 94.8), Some(Key::X)),
            ((109.8, 101.0), Some(Key::Y)),
            ((122.2, 101.0), Some(Key::A)),
            ((116.0, 107.2), Some(Key::B)),
            ((114.0, 121.0), Some(Key::Start)),
            ((114.0, 128.0), Some(Key::Select)),
            ((8.0, 82.0), Some(Key::L)),
            ((125.0, 82.0), Some(Key::R)),
            ((66.0, 110.0), None),
        ];
        for ((x, y), key) in cases {
            assert_eq!(at.button_at(pos2(x, y)), key, "({x}, {y})");
        }
    }
}
