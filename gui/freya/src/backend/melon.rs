//! The melonDS backend: the melonDS C++ core through `melonds-rs` (the
//! bindings `gui/melon_egui` uses).
//!
//! melonDS hands its audio over at 48 kHz (see `gui/melon_egui/src/audio.rs`,
//! `SPU_SAMPLE_RATE`); [`Resampler`] brings it to the rate the shared
//! `AudioOutput` expects.
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{Datelike as _, Timelike as _};
use ds_common::frontend::lock;
use melonds::{Cheat, Host, Nds};

use super::{Boot, Core, push_rgba};

/// melonDS's output rate (fixed by the bindings).
const MELON_RATE: f64 = 48_000.0;
/// Frames between writes of a changed save (about 1 s).
const SAVE_EVERY: u32 = 60;

pub fn boot(boot: &Boot) -> Result<Box<dyn Core>, String> {
    let rom = std::fs::read(&boot.rom).map_err(|e| e.to_string())?;
    let save = std::fs::read(&boot.save).ok();
    let pending = Arc::new(Mutex::new(None));
    let host = Box::new(SaveHost { pending: Arc::clone(&pending) });
    let mut nds = Nds::new(&rom, save.as_deref(), 0, host).map_err(|e| e.to_string())?;
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
    Ok(Box::new(Melon {
        nds,
        save_path: boot.save.clone(),
        pending,
        frames: 0,
        resampler: Resampler::new(MELON_RATE / ds_spu::OUTPUT_RATE_HZ),
        raw: Vec::new(),
    }))
}

/// The core's callbacks: only save writes are taken (no wireless).
struct SaveHost {
    pending: Arc<Mutex<Option<Vec<u8>>>>,
}

impl Host for SaveHost {
    /// `data` is the whole backup image.
    fn write_save(&self, data: &[u8], _offset: u32, _len: u32) {
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(data.to_vec());
    }
}

struct Melon {
    nds: Nds,
    save_path: std::path::PathBuf,
    /// The newest save image not written yet.
    pending: Arc<Mutex<Option<Vec<u8>>>>,
    frames: u32,
    resampler: Resampler,
    raw: Vec<i16>,
}

impl Core for Melon {
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
}

/// Linear resampler for interleaved stereo `i16`.
struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Position of the next output between `last` (0.0) and the first
    /// input frame (1.0).
    pos: f64,
    last: [i16; 2],
}

impl Resampler {
    const fn new(step: f64) -> Self {
        Self { step, pos: 1.0, last: [0; 2] }
    }

    fn process(&mut self, input: &[i16], out: &mut Vec<i16>) {
        let frames = input.as_chunks::<2>().0;
        // Frame `i` of the stream "last, frames..." is at index `i`.
        let at = |i: usize| if i == 0 { self.last } else { frames[i - 1] };
        let end = frames.len() as f64;
        #[allow(clippy::while_float)]
        while self.pos < end {
            let i = self.pos as usize;
            let t = self.pos - i as f64;
            let (a, b) = (at(i), at(i + 1));
            for c in 0..2 {
                let v = (f64::from(b[c]) - f64::from(a[c])).mul_add(t, f64::from(a[c]));
                out.push(v as i16);
            }
            self.pos += self.step;
        }
        if let Some(&last) = frames.last() {
            self.last = last;
            self.pos -= frames.len() as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Resampler;

    #[test]
    fn resamples_48k_to_the_spu_rate() {
        let mut r = Resampler::new(48_000.0 / 32_728.0);
        let mut out = Vec::new();
        // One second of a ramp, in 60 frame-sized chunks.
        let input: Vec<i16> = (0..48_000_i32).flat_map(|i| [(i % 1000) as i16; 2]).collect();
        for chunk in input.chunks(1600) {
            r.process(chunk, &mut out);
        }
        let frames = out.len() / 2;
        assert!((32_700..=32_760).contains(&frames), "{frames} frames");
        assert!(out.chunks(2).all(|f| f[0] == f[1]));
    }
}
