//! The melonDS backend: the melonDS C++ core through `melonds-rs` (the
//! bindings `gui/melon_egui` uses).
//!
//! melonDS hands its audio over at 48 kHz (see `gui/melon_egui/src/audio.rs`,
//! `SPU_SAMPLE_RATE`); `Resampler` brings it to the rate the shared
//! `AudioOutput` expects.
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{Datelike as _, Timelike as _};
use ds_common::frontend::lock;
use melonds::{Cheat, Host, Nds, RenderSettings, Renderer};

use super::{Boot, Core, HdJob, PostScale, push_rgba, resample::Resampler};

/// melonDS's output rate (fixed by the bindings).
const MELON_RATE: f64 = 48_000.0;
/// Frames between writes of a changed save (about 1 s).
const SAVE_EVERY: u32 = 60;

pub fn boot(boot: &Boot) -> Result<Box<dyn Core>, String> {
    let link = lock(&boot.link).take();
    let link = match link {
        Some(link) => Some(
            *link
                .downcast::<crate::lan::MelonLink>()
                .map_err(|_| "Not a melonDS LAN link".to_owned())?,
        ),
        None => None,
    };
    let (network, pace) = link.map_or((None, None), |l| (Some(l.host), l.pace));
    Ok(Box::new(Melon::boot(&boot.rom, &boot.save, 0, network, pace)?))
}

impl Melon {
    /// Console `instance` (0, or 1 for Remote Desktop's second console:
    /// melonDS gives each its own MAC) on `rom`, its wireless on `network`.
    pub fn boot(
        rom: &std::path::Path,
        save_path: &std::path::Path,
        instance: u32,
        network: Option<Box<dyn Host>>,
        pace: Option<melon_lan::LinkPace>,
    ) -> Result<Self, String> {
        let rom = std::fs::read(rom).map_err(|e| e.to_string())?;
        let save = std::fs::read(save_path).ok();
        let pending = Arc::new(Mutex::new(None));
        let host = Box::new(SaveHost { pending: Arc::clone(&pending), network });
        let mut nds = Nds::new(&rom, save.as_deref(), instance, host).map_err(|e| e.to_string())?;
        let now = chrono::Local::now();
        nds.set_rtc(
            now.year(),
            now.month() as i32,
            now.day() as i32,
            now.hour() as i32,
            now.minute() as i32,
            now.second() as i32,
        );
        nds.boot();
        Ok(Self {
            nds,
            save_path: save_path.to_owned(),
            pending,
            frames: 0,
            resampler: Resampler::new(MELON_RATE / ds_spu::OUTPUT_RATE_HZ),
            raw: Vec::new(),
            pace,
            #[cfg(windows)]
            gl: None,
            gl_failed: false,
            render: (1, false),
            gl_pixels: Vec::new(),
        })
    }
}

/// A frame read back from OpenGL at `scale` x; with `xbrz`, its 2D put
/// through xBRZ (melon_egui's `gl_screen`, on the CPU).
struct Ready {
    rgba: Vec<u8>,
    width: usize,
    scale: usize,
    xbrz: bool,
}

impl HdJob for Ready {
    fn render(self: Box<Self>, rgba: &mut Vec<u8>) -> usize {
        let start = rgba.len();
        rgba.extend_from_slice(&self.rgba);
        if self.xbrz && self.scale > 1 {
            let screen = self.width * self.width * 3 / 4 * 4;
            for frame in rgba[start..].chunks_exact_mut(screen) {
                xbrz_2d(frame, self.scale);
            }
        }
        self.width
    }
}

/// How much a DS pixel's block may vary in luma (0-255) and still count as
/// 2D (melon_egui's `BLOCK_TOLERANCE`, 0.02).
const BLOCK_TOLERANCE: f32 = 0.02 * 255.0;

/// One screen of the OpenGL output (RGBA, 256n x 192n): every DS pixel
/// whose n x n block is uniform is 2D and is replaced by the xBRZ of the
/// screen's DS pixels; a block with detail inside is 3D and keeps what the
/// GPU drew (melon_egui's blit shader).
fn xbrz_2d(frame: &mut [u8], n: usize) {
    let (w, h) = (256, 192);
    let pitch = w * n * 4;
    let at = |frame: &[u8], x: usize, y: usize| {
        let i = y * pitch + x * 4;
        [frame[i], frame[i + 1], frame[i + 2]]
    };
    let luma = |a: [u8; 3], b: [u8; 3]| {
        let d = |c: usize| f32::from(a[c].abs_diff(b[c]));
        d(0).mul_add(0.2627, d(1).mul_add(0.6780, d(2) * 0.0593))
    };
    // One texel per DS pixel, from the middle of its block.
    let mut small = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = at(frame, x * n + n / 2, y * n + n / 2);
            small.extend_from_slice(&[r, g, b, 0xFF]);
        }
    }
    let sharp = xbrz::scale_rgba(&small, w, h, n);
    // Probes at 0.15 / 0.85 across the block, as the shader's.
    let (lo, hi) = ((n * 15) / 100, (n * 85) / 100);
    for y in 0..h {
        for x in 0..w {
            let q = |dx, dy| at(frame, x * n + dx, y * n + dy);
            let (q0, q1, q2, q3) = (q(lo, lo), q(hi, lo), q(lo, hi), q(hi, hi));
            let inside = luma(q0, q1).max(luma(q0, q2)).max(luma(q0, q3)).max(luma(q1, q2));
            if inside > BLOCK_TOLERANCE {
                continue;
            }
            for row in y * n..(y + 1) * n {
                let from = row * pitch + x * n * 4;
                frame[from..from + n * 4].copy_from_slice(&sharp[from..from + n * 4]);
            }
        }
    }
}

/// No wireless: the core's own defaults (nobody answers).
struct Unlinked;

impl Host for Unlinked {}

/// The core's callbacks: save writes, and the wireless of a LAN link
/// (`melon_lan`, as melon_egui's `HostBridge`).
struct SaveHost {
    pending: Arc<Mutex<Option<Vec<u8>>>>,
    network: Option<Box<dyn Host>>,
}

impl SaveHost {
    fn wifi(&self) -> &dyn Host {
        self.network.as_deref().unwrap_or(&Unlinked)
    }
}

impl Host for SaveHost {
    /// `data` is the whole backup image.
    fn write_save(&self, data: &[u8], _offset: u32, _len: u32) {
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(data.to_vec());
    }

    fn mp_begin(&self) {
        self.wifi().mp_begin();
    }

    fn mp_end(&self) {
        self.wifi().mp_end();
    }

    fn mp_send_packet(&self, data: &[u8], timestamp: u64) -> i32 {
        self.wifi().mp_send_packet(data, timestamp)
    }

    fn mp_send_cmd(&self, data: &[u8], timestamp: u64) -> i32 {
        self.wifi().mp_send_cmd(data, timestamp)
    }

    fn mp_send_reply(&self, data: &[u8], timestamp: u64, aid: u16) -> i32 {
        self.wifi().mp_send_reply(data, timestamp, aid)
    }

    fn mp_send_ack(&self, data: &[u8], timestamp: u64) -> i32 {
        self.wifi().mp_send_ack(data, timestamp)
    }

    fn mp_recv_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        self.wifi().mp_recv_packet(data, now, timestamp)
    }

    fn mp_recv_host_packet(&self, data: &mut [u8], now: u64, timestamp: &mut u64) -> Option<i32> {
        self.wifi().mp_recv_host_packet(data, now, timestamp)
    }

    fn mp_recv_replies(&self, data: &mut [u8], now: u64, timestamp: u64, aidmask: u16) -> u16 {
        self.wifi().mp_recv_replies(data, now, timestamp, aidmask)
    }

    fn mp_clock(&self, now: u64) {
        self.wifi().mp_clock(now);
    }
}

pub struct Melon {
    nds: Nds,
    save_path: std::path::PathBuf,
    /// The newest save image not written yet.
    pending: Arc<Mutex<Option<Vec<u8>>>>,
    frames: u32,
    resampler: Resampler,
    raw: Vec<i16>,
    /// The LAN link's sustainable frame rate.
    pace: Option<melon_lan::LinkPace>,
    /// The OpenGL context of the OpenGL renderer (made on first use).
    #[cfg(windows)]
    gl: Option<super::melon_gl::GlContext>,
    /// OpenGL could not be set up: upscaling stays on the CPU.
    gl_failed: bool,
    /// The renderer settings in use.
    render: (usize, bool),
    /// One screen read back from the OpenGL renderer.
    gl_pixels: Vec<u32>,
}

impl Melon {
    /// Makes the OpenGL context current (once) and binds melonDS to it.
    #[cfg(windows)]
    fn ensure_gl(&mut self) -> bool {
        if self.gl.is_none() && !self.gl_failed {
            match super::melon_gl::GlContext::new() {
                Ok(gl) if melonds::gl_load(None) => {
                    log::info!("OpenGL renderer ready: {}", melonds::gl_info().unwrap_or_default());
                    self.gl = Some(gl);
                }
                Ok(_) => {
                    log::error!("melonDS could not bind OpenGL; upscaling on the CPU");
                    self.gl_failed = true;
                }
                Err(e) => {
                    log::error!("{e}; upscaling on the CPU");
                    self.gl_failed = true;
                }
            }
        }
        self.gl.is_some()
    }

    #[cfg(not(windows))]
    fn ensure_gl(&mut self) -> bool {
        false
    }
}

impl Core for Melon {
    /// Above 1x: the OpenGL renderer at that internal resolution (the 3D
    /// gains detail, as melon_egui; xBRZ then goes over the 2D, see
    /// [`Ready`]). Without OpenGL: the software renderer, scaled afterwards.
    fn set_render_scale(&mut self, scale: usize, xbrz: bool) {
        if self.render == (scale, xbrz) {
            return;
        }
        self.render = (scale, xbrz);
        let gl = scale > 1 && self.ensure_gl();
        let wanted = if gl {
            RenderSettings {
                renderer: Renderer::OpenGl,
                scale: scale as u32,
                better_polygons: true,
                ..RenderSettings::default()
            }
        } else {
            RenderSettings::default()
        };
        let installed = self.nds.set_render_settings(wanted);
        if installed != wanted.renderer {
            log::warn!("melonDS fell back to {installed:?} (asked for {:?})", wanted.renderer);
        }
    }

    fn hd_job(&mut self, scale: usize, xbrz: bool) -> Option<Box<dyn HdJob>> {
        if let Some(output) = self.nds.gl_output() {
            let (w, h) = (output.width as usize, output.height as usize);
            self.gl_pixels.resize(w * h, 0);
            let mut rgba = Vec::with_capacity(w * h * 8);
            for screen in 0..2 {
                if self.nds.gl_read_output(screen, &mut self.gl_pixels) == 0 {
                    return None;
                }
                // The read-back is bottom-up (GL's origin), on this AMD
                // driver at least, whatever melonds-rs documents.
                for row in self.gl_pixels.chunks_exact(w).rev() {
                    push_rgba(row, &mut rgba);
                }
            }
            return Some(Box::new(Ready { rgba, width: w, scale: w / 256, xbrz }));
        }
        let mut rgba = Vec::with_capacity(256 * 384 * 4);
        self.frame(&mut rgba);
        Some(Box::new(PostScale { rgba, scale, xbrz }))
    }

    fn run_frame(&mut self, keys: u32, touch: Option<(i32, i32)>) {
        // Same bit layout as `emu_thread::key` (melonds::keys).
        self.nds.set_keys(keys);
        match touch {
            Some((x, y)) => self.nds.touch(x as u16, y as u16),
            None => self.nds.release_screen(),
        }
        self.nds.run_frame();
        self.frames += 1;
        if self.frames.is_multiple_of(SAVE_EVERY)
            && let Err(e) = self.flush_save()
        {
            log::error!("{e}");
        }
    }

    fn link_rate(&self) -> Option<f64> {
        self.pace.as_ref().map(melon_lan::LinkPace::frame_rate).filter(|fps| *fps > 0.0)
    }

    fn take_audio(&mut self, out: &mut Vec<i16>) {
        let queued = self.nds.audio_queued();
        if queued == 0 {
            return;
        }
        self.raw.resize(queued * 2, 0);
        let pairs = self.nds.read_audio(&mut self.raw);
        self.raw.truncate(pairs * 2);
        self.resampler.process(&self.raw, out);
    }

    fn frame(&mut self, rgba: &mut Vec<u8>) {
        match self.nds.framebuffers() {
            Some((top, bottom)) => {
                push_rgba(top, rgba);
                push_rgba(bottom, rgba);
            }
            None => rgba.resize(256 * 192 * 2 * 4, 0),
        }
    }

    fn set_cheats(&mut self, codes: Vec<Vec<u32>>) {
        let cheats: Vec<Cheat> = codes
            .into_iter()
            .map(|code| Cheat { name: String::new(), code, enabled: true })
            .collect();
        self.nds.set_cheats(&cheats);
    }

    fn flush_save(&mut self) -> Result<(), String> {
        let Some(data) = lock(&self.pending).take() else {
            return Ok(());
        };
        std::fs::write(&self.save_path, data)
            .map_err(|e| format!("Failed to write {}: {e}", self.save_path.display()))
    }

    /// melonDS's own format (`.mlN`, as melon_egui writes them).
    fn save_state(&mut self, path: &std::path::Path) -> Result<(), String> {
        let mut buffer = Vec::new();
        self.nds.save_state(&mut buffer).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, &buffer).map_err(|e| format!("Cannot write {}: {e}", path.display()))
    }

    fn load_state(&mut self, path: &std::path::Path) -> Result<(), String> {
        let buffer =
            std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        self.nds.load_state(&buffer).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod gl_tests {
    use super::*;

    /// Above 1x the OpenGL renderer draws, with xBRZ on too (it then goes
    /// over the 2D only): the frame is the GPU's, at the internal size.
    #[test]
    #[ignore = "requires a ROM and OpenGL"]
    fn opengl_renders_above_1x_with_and_without_xbrz() {
        let rom = std::path::Path::new("D:/GAME/Emulator/DS/Roms/Newスーパーマリオブラザーズ.nds");
        let save = std::env::temp_dir().join("freya_gl_test.sav");
        let mut m = Melon::boot(rom, &save, 0, None, None).expect("boots");
        for xbrz in [false, true] {
            m.set_render_scale(2, xbrz);
            for _ in 0..30 {
                m.run_frame(0, None);
            }
            let out = m.nds.gl_output();
            assert!(out.is_some_and(|o| o.width == 512), "xbrz {xbrz}: {out:?}");
            let mut rgba = Vec::new();
            assert_eq!(m.hd_job(2, xbrz).expect("a frame").render(&mut rgba), 512);
            assert_eq!(rgba.len(), 512 * 384 * 2 * 4);
        }
    }

    /// A uniform block is 2D (replaced by xBRZ), a varied one 3D (kept).
    #[test]
    fn xbrz_replaces_only_uniform_blocks() {
        let n = 2;
        let mut frame = vec![0_u8; 512 * 384 * 4];
        // DS pixel (0, 0): a 3D block with detail inside.
        frame[..4].copy_from_slice(&[255, 255, 255, 255]);
        let before = frame.clone();
        xbrz_2d(&mut frame, n);
        assert_eq!(frame[..8], before[..8], "the 3D block is kept");
        assert_eq!(frame.len(), before.len());
    }
}
