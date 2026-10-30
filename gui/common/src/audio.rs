//! Audio output: plays the SPU's stereo stream (~32728 Hz) on the default
//! output device (via `cpal`), resampled to the device rate.
//!
//! The emulator produces audio in bursts (one emulated frame at a time,
//! paced by a timer or the UI's repaints), while the device consumes it
//! continuously. To play it without gaps the queue:
//!
//! - fills up to a target latency before playback starts, and again after
//!   an underrun (instead of playing the stream as it trickles in);
//! - nudges the resampling ratio by up to ±0.5% so the fill level stays
//!   at the target (dynamic rate control): the emulator's pacing clock and
//!   the sound card's clock never match exactly;
//! - only drops audio when far above the target (e.g. after a stall).
//!
//! [`AudioOutput::wait_for_room`] lets a caller pace itself to the device
//! (the egui GUI's "audio sync" at native speed).
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::Instant,
};

use cpal::{
    FromSample, SizedSample,
    traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _},
};

/// Source rate of the SPU output.
const SOURCE_RATE: f64 = ds_spu::OUTPUT_RATE_HZ;
/// Queue level aimed for, in source frames (~70 ms, a bit over 4 emulated
/// frames, so frame batching and scheduling jitter never empty it).
const TARGET_FRAMES: usize = 2300;
/// Above this level the oldest audio is dropped back to the target.
const MAX_FRAMES: usize = TARGET_FRAMES * 3;
/// Maximum resampling ratio correction.
const MAX_RATE_ADJUST: f64 = 0.005;

#[derive(Default)]
struct Queue {
    frames: VecDeque<[f32; 2]>,
    /// Fractional read position between `frames[0]` and `frames[1]`.
    pos: f64,
    /// Last output sample, faded to silence while (re)buffering.
    last: [f32; 2],
    /// Playing (true) or buffering up to the target (false).
    primed: bool,
}

/// Counters for diagnosing glitches (logged at `debug` level).
#[derive(Default)]
struct Stats {
    underruns: AtomicU64,
    dropped_frames: AtomicU64,
}

/// Cheap to clone: clones share the same output stream and queue.
#[derive(Clone)]
pub struct AudioOutput {
    queue: Arc<Mutex<Queue>>,
    /// Volume 0.0..=1.0 as `f32` bits.
    volume: Arc<AtomicU32>,
    stats: Arc<Stats>,
    last_report: Arc<Mutex<Instant>>,
    /// A device stream is running (else [`Self::wait_for_room`] never
    /// waits).
    playing: Arc<std::sync::atomic::AtomicBool>,
}

impl AudioOutput {
    /// Starts audio output on the default device.
    ///
    /// The `cpal` stream is created and kept on its own thread: on Windows
    /// WASAPI initialises COM in multi-threaded mode, which on the UI thread
    /// conflicts with the window's single-threaded COM setup and made the
    /// window close right after start-up. If no device is usable the
    /// emulator simply runs silently (logged).
    pub fn new(volume: f32) -> Self {
        let out = Self {
            queue: Arc::new(Mutex::new(Queue::default())),
            volume: Arc::new(AtomicU32::new(volume.clamp(0.0, 1.0).to_bits())),
            stats: Arc::new(Stats::default()),
            last_report: Arc::new(Mutex::new(Instant::now())),
            playing: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        if std::env::var_os("LUNARIS_NO_AUDIO").is_some() {
            return out;
        }
        let (q, v, s, playing) = (
            Arc::clone(&out.queue),
            Arc::clone(&out.volume),
            Arc::clone(&out.stats),
            Arc::clone(&out.playing),
        );
        let spawned = std::thread::Builder::new().name("audio".to_owned()).spawn(move || {
            // Keep the stream alive for the lifetime of the process.
            if let Some(_stream) = open_stream(&q, &v, &s) {
                playing.store(true, Ordering::Relaxed);
                loop {
                    std::thread::park();
                }
            }
        });
        if let Err(e) = spawned {
            log::error!("Failed to start audio thread: {e}");
        }
        out
    }

    /// Queues interleaved stereo `i16` samples from the emulator.
    pub fn push(&self, samples: &[i16]) {
        let Ok(mut q) = self.queue.lock() else {
            return;
        };
        q.frames.extend(
            samples
                .as_chunks::<2>()
                .0
                .iter()
                .map(|s| [f32::from(s[0]) / 32768.0, f32::from(s[1]) / 32768.0]),
        );
        if q.frames.len() > MAX_FRAMES {
            let excess = q.frames.len() - TARGET_FRAMES;
            q.frames.drain(..excess);
            self.stats.dropped_frames.fetch_add(excess as u64, Ordering::Relaxed);
        }
        let queued = q.frames.len();
        drop(q);
        self.report(queued);
    }

    /// Logs the queue state about once per second.
    fn report(&self, queued: usize) {
        let Ok(mut last) = self.last_report.lock() else {
            return;
        };
        if last.elapsed().as_secs() >= 1 {
            *last = Instant::now();
            log::debug!(
                target: "audio",
                "queued={queued} underruns={} dropped={}",
                self.stats.underruns.load(Ordering::Relaxed),
                self.stats.dropped_frames.load(Ordering::Relaxed),
            );
        }
    }

    /// Source frames waiting to be played.
    pub fn queued(&self) -> usize {
        self.queue.lock().map_or(0, |q| q.frames.len())
    }

    /// Blocks (at most `max_wait`) while more than twice the target latency
    /// is queued, so a producer running ahead of the device is held back
    /// instead of having its audio dropped. Returns at once when no device
    /// is playing.
    pub fn wait_for_room(&self, max_wait: std::time::Duration) {
        let deadline = Instant::now() + max_wait;
        while self.playing.load(Ordering::Relaxed)
            && self.queued() > TARGET_FRAMES * 2
            && Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Total (underruns, dropped source frames) since start.
    pub fn stats(&self) -> (u64, u64) {
        (
            self.stats.underruns.load(Ordering::Relaxed),
            self.stats.dropped_frames.load(Ordering::Relaxed),
        )
    }

    /// Drops queued audio (on stop / pause); playback re-buffers.
    pub fn clear(&self) {
        if let Ok(mut q) = self.queue.lock() {
            q.frames.clear();
            q.pos = 0.0;
            q.primed = false;
        }
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
}

fn open_stream(
    queue: &Arc<Mutex<Queue>>,
    vol: &Arc<AtomicU32>,
    stats: &Arc<Stats>,
) -> Option<cpal::Stream> {
    let host = cpal::default_host();
    let device = host.default_output_device().or_else(|| {
        log::error!("No audio output device");
        None
    })?;
    let config = device
        .default_output_config()
        .inspect_err(|e| log::error!("Audio output config: {e}"))
        .ok()?;
    log::info!(
        "Audio output: {} ({} Hz, {} ch, {:?})",
        device.name().unwrap_or_default(),
        config.sample_rate().0,
        config.channels(),
        config.sample_format()
    );

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config.into(), queue, vol, stats),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config.into(), queue, vol, stats),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config.into(), queue, vol, stats),
        other => {
            log::error!("Unsupported audio sample format {other:?}");
            return None;
        }
    }?;
    stream.play().inspect_err(|e| log::error!("Audio play: {e}")).ok()?;
    Some(stream)
}

/// Resampling step for the current queue level (dynamic rate control):
/// consume slightly faster above the target, slower below it.
fn rate_step(base_step: f64, queued: usize) -> f64 {
    let error = (queued as f64 - TARGET_FRAMES as f64) / TARGET_FRAMES as f64;
    base_step * (1.0 + (error * MAX_RATE_ADJUST).clamp(-MAX_RATE_ADJUST, MAX_RATE_ADJUST))
}

impl Queue {
    /// Produces the next output frame, advancing by `step` source frames.
    fn next(&mut self, step: f64, stats: &Stats) -> [f32; 2] {
        if !self.primed {
            if self.frames.len() >= TARGET_FRAMES {
                self.primed = true;
            } else {
                return self.fade();
            }
        }
        if self.frames.len() < 2 {
            // Underrun: fade out and buffer up to the target again.
            self.primed = false;
            stats.underruns.fetch_add(1, Ordering::Relaxed);
            return self.fade();
        }

        // Linear interpolation between the two oldest frames.
        let t = self.pos as f32;
        let (a, b) = (self.frames[0], self.frames[1]);
        let s = [f32::mul_add(b[0] - a[0], t, a[0]), f32::mul_add(b[1] - a[1], t, a[1])];
        self.pos += step;
        while self.pos >= 1.0 && self.frames.len() >= 2 {
            self.pos -= 1.0;
            self.frames.pop_front();
        }
        self.last = s;
        s
    }

    /// Decays the last sample to silence (no click when audio stops).
    fn fade(&mut self) -> [f32; 2] {
        self.last = self.last.map(|v| v * 0.995);
        self.last
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: &Arc<Mutex<Queue>>,
    volume: &Arc<AtomicU32>,
    stats: &Arc<Stats>,
) -> Option<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    let base_step = SOURCE_RATE / f64::from(config.sample_rate.0);
    let queue = Arc::clone(queue);
    let volume = Arc::clone(volume);
    let stats = Arc::clone(stats);

    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                let vol = f32::from_bits(volume.load(Ordering::Relaxed));
                let Ok(mut q) = queue.lock() else {
                    out.fill(T::from_sample(0.0));
                    return;
                };
                let step = rate_step(base_step, q.frames.len());

                for frame in out.chunks_mut(channels) {
                    let sample = q.next(step, &stats);
                    for (i, out) in frame.iter_mut().enumerate() {
                        let v = if channels == 1 {
                            (sample[0] + sample[1]) * 0.5
                        } else {
                            sample[i.min(1)]
                        };
                        *out = T::from_sample(v * vol);
                    }
                }
            },
            |e| log::error!("Audio stream error: {e}"),
            None,
        )
        .inspect_err(|e| log::error!("Audio stream: {e}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds the queue in emulated-frame bursts with jitter and consumes
    /// it at a device rate: after start-up there must be no underruns.
    #[test]
    fn bursty_producer_does_not_underrun() {
        let stats = Stats::default();
        let mut q = Queue::default();
        let device_rate = 48_000.0;
        let base_step = SOURCE_RATE / device_rate;
        // 547 frames per emulated frame, delivered 0-2 frames at a time.
        let pattern = [1, 0, 2, 1, 1, 0, 2, 1];
        let mut produced = 0.0_f64;
        let mut consumed_time = 0.0_f64;
        for tick in 0..6000 {
            for _ in 0..pattern[tick % pattern.len()] {
                produced += SOURCE_RATE / 59.826_098;
                let n = produced as usize;
                produced -= n as f64;
                q.frames.extend(std::iter::repeat_n([0.1, -0.1], n));
            }
            // One 60 Hz device tick of output.
            consumed_time += device_rate / 59.826_098;
            let n = consumed_time as usize;
            consumed_time -= n as f64;
            let step = rate_step(base_step, q.frames.len());
            for _ in 0..n {
                q.next(step, &stats);
            }
            if tick == 100 {
                stats.underruns.store(0, Ordering::Relaxed);
            }
        }
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 0);
        assert!(q.frames.len() < MAX_FRAMES);
    }
}
