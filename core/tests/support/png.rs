//! Minimal RGBA canvas for visual reports (no font/graphics crates needed).
//!
//! Provides exactly what the GPU reports need: blitting BGR555 images,
//! scaling, lines (wireframes), filled triangles, grids, legends and a 3×5
//! pixel font for labels such as tile numbers and polygon IDs.

use super::{bgr555_to_rgba, dist_path};

pub type Rgba = [u8; 4];

pub const BLACK: Rgba = [0, 0, 0, 255];
pub const WHITE: Rgba = [255, 255, 255, 255];
pub const GRAY: Rgba = [96, 96, 96, 255];
pub const RED: Rgba = [230, 40, 40, 255];
pub const GREEN: Rgba = [40, 200, 70, 255];
pub const BLUE: Rgba = [50, 90, 230, 255];
pub const YELLOW: Rgba = [240, 210, 40, 255];
pub const MAGENTA: Rgba = [230, 40, 230, 255];
pub const CYAN: Rgba = [40, 210, 230, 255];

/// Distinct, stable colour for index `i` (polygon ID, tile number, …).
pub fn palette(i: usize) -> Rgba {
    // Golden-ratio hue walk: neighbours always differ strongly.
    let h = (i as f32 * 0.618_034).fract();
    hsv(h, 0.75, 0.95)
}

pub fn hsv(h: f32, s: f32, v: f32) -> Rgba {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, 255]
}

/// Heat-map colour for `t` in 0..=1 (blue = near → red = far).
pub fn heat(t: f32) -> Rgba {
    hsv((1.0 - t.clamp(0.0, 1.0)) * 0.66, 0.9, 0.95)
}

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Canvas {
    pub fn new(w: usize, h: usize, bg: Rgba) -> Self {
        let mut px = Vec::with_capacity(w * h * 4);
        for _ in 0..w * h {
            px.extend_from_slice(&bg);
        }
        Canvas { w, h, px }
    }

    /// Canvas from a BGR555 image (`bit 15` ignored), scaled by `scale`.
    pub fn from_bgr555(src: &[u16], w: usize, h: usize, scale: usize) -> Self {
        let mut c = Canvas::new(w * scale, h * scale, BLACK);
        c.blit_bgr555(0, 0, src, w, h, scale);
        c
    }

    pub fn set(&mut self, x: i64, y: i64, c: Rgba) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 4;
        if c[3] == 255 {
            self.px[i..i + 4].copy_from_slice(&c);
        } else {
            // Simple alpha blend for overlays.
            let a = c[3] as u32;
            for k in 0..3 {
                self.px[i + k] =
                    ((self.px[i + k] as u32 * (255 - a) + c[k] as u32 * a) / 255) as u8;
            }
        }
    }

    pub fn get(&self, x: usize, y: usize) -> Rgba {
        let i = (y * self.w + x) * 4;
        [self.px[i], self.px[i + 1], self.px[i + 2], self.px[i + 3]]
    }

    pub fn fill_rect(&mut self, x: i64, y: i64, w: i64, h: i64, c: Rgba) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, c);
            }
        }
    }

    pub fn rect(&mut self, x: i64, y: i64, w: i64, h: i64, c: Rgba) {
        self.line(x, y, x + w - 1, y, c);
        self.line(x, y + h - 1, x + w - 1, y + h - 1, c);
        self.line(x, y, x, y + h - 1, c);
        self.line(x + w - 1, y, x + w - 1, y + h - 1, c);
    }

    /// Bresenham line.
    pub fn line(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: Rgba) {
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let (mut x, mut y, mut err) = (x0, y0, dx + dy);
        loop {
            self.set(x, y, c);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
            if (x - x0).abs() > 100_000 || (y - y0).abs() > 100_000 {
                break;
            }
        }
    }

    /// Filled triangle (edge-function rasteriser, top-left rule not needed
    /// for illustrations).
    pub fn fill_tri(&mut self, p: [(f32, f32); 3], c: Rgba) {
        let min_x = p.iter().map(|v| v.0).fold(f32::MAX, f32::min).floor().max(0.0) as i64;
        let max_x = p.iter().map(|v| v.0).fold(f32::MIN, f32::max).ceil().min(self.w as f32) as i64;
        let min_y = p.iter().map(|v| v.1).fold(f32::MAX, f32::min).floor().max(0.0) as i64;
        let max_y = p.iter().map(|v| v.1).fold(f32::MIN, f32::max).ceil().min(self.h as f32) as i64;
        let edge = |a: (f32, f32), b: (f32, f32), x: f32, y: f32| {
            (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0)
        };
        let area = edge(p[0], p[1], p[2].0, p[2].1);
        if area == 0.0 {
            return;
        }
        for y in min_y..max_y {
            for x in min_x..max_x {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = edge(p[1], p[2], fx, fy) / area;
                let w1 = edge(p[2], p[0], fx, fy) / area;
                let w2 = edge(p[0], p[1], fx, fy) / area;
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    self.set(x, y, c);
                }
            }
        }
    }

    /// Filled convex polygon as a triangle fan.
    pub fn fill_poly(&mut self, pts: &[(f32, f32)], c: Rgba) {
        for i in 1..pts.len().saturating_sub(1) {
            self.fill_tri([pts[0], pts[i], pts[i + 1]], c);
        }
    }

    /// Closed polygon outline.
    pub fn poly_outline(&mut self, pts: &[(f32, f32)], c: Rgba) {
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            self.line(
                a.0.round() as i64,
                a.1.round() as i64,
                b.0.round() as i64,
                b.1.round() as i64,
                c,
            );
        }
    }

    pub fn blit_bgr555(
        &mut self,
        x: usize,
        y: usize,
        src: &[u16],
        w: usize,
        h: usize,
        scale: usize,
    ) {
        for sy in 0..h {
            for sx in 0..w {
                let c = bgr555_to_rgba(src[sy * w + sx]);
                self.fill_rect(
                    (x + sx * scale) as i64,
                    (y + sy * scale) as i64,
                    scale as i64,
                    scale as i64,
                    c,
                );
            }
        }
    }

    /// Copies another canvas into this one at `(x, y)`.
    pub fn blit(&mut self, x: i64, y: i64, src: &Canvas) {
        for sy in 0..src.h {
            for sx in 0..src.w {
                self.set(x + sx as i64, y + sy as i64, src.get(sx, sy));
            }
        }
    }

    /// Grid lines every `step` pixels (used for 8×8 tile boundaries).
    pub fn grid(&mut self, step: usize, c: Rgba) {
        for x in (0..self.w).step_by(step) {
            self.line(x as i64, 0, x as i64, self.h as i64 - 1, c);
        }
        for y in (0..self.h).step_by(step) {
            self.line(0, y as i64, self.w as i64 - 1, y as i64, c);
        }
    }

    /// Draws text in the built-in 3×5 font, `scale` pixels per font pixel.
    /// Unknown characters render as a blank cell.
    pub fn text(&mut self, x: i64, y: i64, s: &str, scale: i64, c: Rgba) {
        let mut cx = x;
        for ch in s.chars() {
            let g = glyph(ch.to_ascii_uppercase());
            for (row, bits) in g.iter().enumerate() {
                for col in 0..3 {
                    if bits & (0b100 >> col) != 0 {
                        self.fill_rect(cx + col * scale, y + row as i64 * scale, scale, scale, c);
                    }
                }
            }
            cx += 4 * scale;
        }
    }

    /// Text with a 1-pixel dark backdrop, readable on any background.
    pub fn label(&mut self, x: i64, y: i64, s: &str, scale: i64, c: Rgba) {
        let w = s.chars().count() as i64 * 4 * scale;
        self.fill_rect(x - 1, y - 1, w + 1, 5 * scale + 2, [0, 0, 0, 170]);
        self.text(x, y, s, scale, c);
    }

    /// Width in pixels of `s` rendered by [`Canvas::text`].
    pub fn text_width(s: &str, scale: i64) -> i64 {
        s.chars().count() as i64 * 4 * scale
    }

    pub fn save(&self, rel: &str) {
        image::save_buffer(
            dist_path(rel),
            &self.px,
            self.w as u32,
            self.h as u32,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    }
}

/// 3×5 bitmap font. Each row is 3 bits, MSB = leftmost pixel.
pub const fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        'A' => [0b010, 0b101, 0b111, 0b101, 0b101],
        'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
        'C' => [0b011, 0b100, 0b100, 0b100, 0b011],
        'D' => [0b110, 0b101, 0b101, 0b101, 0b110],
        'E' => [0b111, 0b100, 0b110, 0b100, 0b111],
        'F' => [0b111, 0b100, 0b110, 0b100, 0b100],
        'G' => [0b011, 0b100, 0b101, 0b101, 0b011],
        'H' => [0b101, 0b101, 0b111, 0b101, 0b101],
        'I' => [0b111, 0b010, 0b010, 0b010, 0b111],
        'J' => [0b001, 0b001, 0b001, 0b101, 0b010],
        'K' => [0b101, 0b101, 0b110, 0b101, 0b101],
        'L' => [0b100, 0b100, 0b100, 0b100, 0b111],
        'M' => [0b101, 0b111, 0b111, 0b101, 0b101],
        'N' => [0b110, 0b101, 0b101, 0b101, 0b101],
        'O' => [0b010, 0b101, 0b101, 0b101, 0b010],
        'P' => [0b110, 0b101, 0b110, 0b100, 0b100],
        'Q' => [0b010, 0b101, 0b101, 0b110, 0b011],
        'R' => [0b110, 0b101, 0b110, 0b101, 0b101],
        'S' => [0b011, 0b100, 0b010, 0b001, 0b110],
        'T' => [0b111, 0b010, 0b010, 0b010, 0b010],
        'U' => [0b101, 0b101, 0b101, 0b101, 0b111],
        'V' => [0b101, 0b101, 0b101, 0b101, 0b010],
        'W' => [0b101, 0b101, 0b111, 0b111, 0b101],
        'X' => [0b101, 0b101, 0b010, 0b101, 0b101],
        'Y' => [0b101, 0b101, 0b010, 0b010, 0b010],
        'Z' => [0b111, 0b001, 0b010, 0b100, 0b111],
        '-' => [0b000, 0b000, 0b111, 0b000, 0b000],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        '.' => [0b000, 0b000, 0b000, 0b000, 0b010],
        ':' => [0b000, 0b010, 0b000, 0b010, 0b000],
        '/' => [0b001, 0b001, 0b010, 0b100, 0b100],
        '=' => [0b000, 0b111, 0b000, 0b111, 0b000],
        '#' => [0b101, 0b111, 0b101, 0b111, 0b101],
        '(' => [0b010, 0b100, 0b100, 0b100, 0b010],
        ')' => [0b010, 0b001, 0b001, 0b001, 0b010],
        '<' => [0b001, 0b010, 0b100, 0b010, 0b001],
        '>' => [0b100, 0b010, 0b001, 0b010, 0b100],
        '_' => [0b000, 0b000, 0b000, 0b000, 0b111],
        _ => [0; 5],
    }
}
