//! What the speakers play: stereo waveform, spectrum, scrolling
//! spectrogram, and the 16 SPU channels decoded from SOUNDxCNT.
//!
//! ```text
//!  Console::audio_tap (interleaved L/R, ~32.7 kHz)
//!    -> waveform: last WAVE_FRAMES frames, L and R
//!    -> FFT (Hann, FFT_LEN) of the newest mono frames -> dB per bin
//!         -> spectrum line + one new spectrogram column per repaint
//! ```

use std::collections::VecDeque;

use ds_emu::Emulator;
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, pos2, vec2};

const FFT_LEN: usize = 512;
const BINS: usize = FFT_LEN / 2;
const HISTORY: usize = 300;
const WAVE_FRAMES: usize = 1024;

pub struct AudioView {
    columns: VecDeque<Vec<f32>>,
    texture: Option<egui::TextureHandle>,
    last_tap_len: usize,
    last_frame: u64,
}

impl AudioView {
    pub fn new() -> Self {
        Self { columns: VecDeque::new(), texture: None, last_tap_len: 0, last_frame: u64::MAX }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, tap: &VecDeque<i16>, emu: &Emulator, frame: u64) {
        let frames = tap.len() / 2;
        let stereo =
            |i: usize| (f32::from(tap[2 * i]) / 32768.0, f32::from(tap[2 * i + 1]) / 32768.0);

        ui.label(egui::RichText::new("Waveform (L blue, R orange)").strong());
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), 120.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(16, 18, 24));
        painter.line_segment(
            [rect.left_center(), rect.right_center()],
            Stroke::new(1.0_f32, Color32::from_gray(50)),
        );
        let n = frames.min(WAVE_FRAMES);
        if n > 1 {
            for (channel, color) in
                [(0, Color32::from_rgb(90, 160, 255)), (1, Color32::from_rgb(255, 160, 60))]
            {
                let points: Vec<Pos2> = (0..n)
                    .map(|i| {
                        let (l, r) = stereo(frames - n + i);
                        let v = if channel == 0 { l } else { r };
                        pos2(
                            rect.min.x + rect.width() * i as f32 / (n - 1) as f32,
                            rect.center().y - v * rect.height() * 0.48,
                        )
                    })
                    .collect();
                painter.add(egui::Shape::line(points, Stroke::new(1.0_f32, color)));
            }
        }

        // One spectrogram column per emulated frame with fresh samples.
        if frames >= FFT_LEN && (frame != self.last_frame || tap.len() != self.last_tap_len) {
            self.last_frame = frame;
            self.last_tap_len = tap.len();
            let mono: Vec<f32> = (frames - FFT_LEN..frames)
                .map(|i| {
                    let (l, r) = stereo(i);
                    (l + r) * 0.5
                })
                .collect();
            self.columns.push_back(spectrum_db(&mono));
            if self.columns.len() > HISTORY {
                self.columns.pop_front();
            }
            self.upload(ui.ctx());
        }

        let nyquist = ds_spu::OUTPUT_RATE_HZ / 2.0;
        ui.label(
            egui::RichText::new(format!("Spectrum (0 - {:.1} kHz)", nyquist / 1000.0)).strong(),
        );
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 100.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(16, 18, 24));
        if let Some(col) = self.columns.back() {
            let points: Vec<Pos2> = col
                .iter()
                .enumerate()
                .map(|(i, db)| {
                    let x = rect.min.x + rect.width() * i as f32 / (BINS - 1) as f32;
                    pos2(x, rect.max.y - rect.height() * ((db + 100.0) / 100.0).clamp(0.0, 1.0))
                })
                .collect();
            painter.add(egui::Shape::line(
                points,
                Stroke::new(1.2_f32, Color32::from_rgb(120, 230, 140)),
            ));
        }
        if let Some(p) = response.hover_pos() {
            let hz = (p.x - rect.min.x) / rect.width() * nyquist as f32;
            response.on_hover_text(format!("{hz:.0} Hz"));
        }

        ui.label(egui::RichText::new("Spectrogram (time ->, frequency ^)").strong());
        if let Some(tex) = &self.texture {
            let size = vec2(ui.available_width(), 160.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter().image(
                tex.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            ui.weak("waiting for audio...");
        }

        ui.separator();
        channels(ui, emu);
    }

    fn upload(&mut self, ctx: &egui::Context) {
        let mut image =
            egui::ColorImage::new([HISTORY, BINS], vec![Color32::BLACK; HISTORY * BINS]);
        let offset = HISTORY - self.columns.len();
        for (x, col) in self.columns.iter().enumerate() {
            for (bin, db) in col.iter().enumerate() {
                let y = BINS - 1 - bin;
                image.pixels[y * HISTORY + x + offset] =
                    heat(((db + 100.0) / 100.0).clamp(0.0, 1.0));
            }
        }
        match &mut self.texture {
            Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.texture =
                    Some(ctx.load_texture("spectrogram", image, egui::TextureOptions::LINEAR))
            }
        }
    }
}

fn channels(ui: &mut egui::Ui, emu: &Emulator) {
    ui.label(egui::RichText::new("SPU channels").strong());
    egui::Grid::new("spu_channels").striped(true).num_columns(6).show(ui, |ui| {
        for h in ["ch", "busy", "format", "volume", "pan", "repeat"] {
            ui.strong(h);
        }
        ui.end_row();
        for ch in 0..16u32 {
            let cnt = emu.spu.read32(0x0400_0400 + 16 * ch);
            let busy = cnt >> 31 != 0;
            let format = match (cnt >> 29) & 3 {
                0 => "PCM8",
                1 => "PCM16",
                2 => "ADPCM",
                _ if ch >= 14 => "noise",
                _ => "PSG",
            };
            let text = |s: String| {
                if busy {
                    egui::RichText::new(s).color(Color32::from_rgb(90, 220, 120))
                } else {
                    egui::RichText::new(s).weak()
                }
            };
            ui.label(text(ch.to_string()));
            ui.label(text(if busy { "on" } else { "-" }.to_owned()));
            ui.label(text(format.to_owned()));
            let volume = cnt & 0x7F;
            ui.add(
                egui::ProgressBar::new(volume as f32 / 127.0)
                    .desired_width(70.0)
                    .text(volume.to_string()),
            );
            ui.label(text(format!("{}", ((cnt >> 16) & 0x7F) as i32 - 64)));
            ui.label(text(
                match (cnt >> 27) & 3 {
                    1 => "loop",
                    2 => "one-shot",
                    _ => "manual",
                }
                .to_owned(),
            ));
            ui.end_row();
        }
    });
}

/// Black -> purple -> orange -> yellow.
fn heat(t: f32) -> Color32 {
    let stops = [
        (0.0, [0, 0, 4]),
        (0.35, [80, 18, 123]),
        (0.65, [222, 73, 104]),
        (0.85, [254, 159, 109]),
        (1.0, [252, 253, 191]),
    ];
    for w in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (w[0], w[1]);
        if t <= t1 {
            let k = (t - t0) / (t1 - t0);
            let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * k) as u8;
            return Color32::from_rgb(mix(c0[0], c1[0]), mix(c0[1], c1[1]), mix(c0[2], c1[2]));
        }
    }
    Color32::from_rgb(252, 253, 191)
}

/// Magnitude spectrum in dBFS of `samples` (length FFT_LEN), Hann windowed.
fn spectrum_db(samples: &[f32]) -> Vec<f32> {
    let n = samples.len();
    let mut re: Vec<f32> = samples
        .iter()
        .enumerate()
        .map(|(i, s)| s * (0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()))
        .collect();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    (0..n / 2)
        .map(|k| {
            let mag = (re[k] * re[k] + im[k] * im[k]).sqrt() / (n as f32 / 4.0);
            20.0 * mag.max(1e-6).log10()
        })
        .collect()
}

/// In-place iterative radix-2 FFT (`re.len()` must be a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * std::f32::consts::PI / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (angle * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sine_peaks_at_its_bin() {
        let bin = 37;
        let samples: Vec<f32> = (0..FFT_LEN)
            .map(|i| (2.0 * std::f32::consts::PI * bin as f32 * i as f32 / FFT_LEN as f32).sin())
            .collect();
        let db = spectrum_db(&samples);
        let peak = db.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i);
        assert_eq!(peak, Some(bin));
        assert!(db[bin] > -1.0 && db[bin] < 1.0, "full-scale sine ~ 0 dBFS, got {}", db[bin]);
    }
}
