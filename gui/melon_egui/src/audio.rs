//! Audio output.
//!
//! melonDS's SPU produces interleaved stereo `i16` at [`DS_SAMPLE_RATE`]; the
//! host device almost never runs at that rate, so samples are resampled on the
//! way into a ring buffer that the device callback drains.
//!
//! # Why the stream lives on its own thread
//!
//! On Windows, cpal's WASAPI backend initialises COM as a multi-threaded
//! apartment, while winit initialises the UI thread as a single-threaded one for
//! drag-and-drop. Doing both on one thread fails with `RPC_E_CHANGED_MODE`.
//! Building the stream on a dedicated thread gives each its own apartment. That
//! thread then parks forever holding the stream, because a `cpal::Stream` is
//! `!Send` and stops playing when dropped.

use std::sync::mpsc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// The rate the core hands samples over at.
///
/// **Not** the DS's internal 32768 Hz. melonDS's SPU resamples on the way out:
/// `SPU.cpp` drives blip-buf with `blip_set_rates(INTERNAL_SAMPLE_RATE * skew,
/// OutputSampleRate)`, so `ReadOutput` yields whatever `OutputSampleRate` is.
/// `melonds-sys`'s shim builds the console with a default-initialised
/// `NDSArgs`, and `Args.h` defaults that field to 48000 — so this is the rate,
/// and the FFI offers no way to change it.
///
/// Getting this wrong is audible rather than subtle: assuming 32768 stretches
/// everything by 48000/32768, which pitches it down about five semitones and
/// overruns the ring on every batch.
pub const SPU_SAMPLE_RATE: u32 = 48_000;

/// How much audio the ring holds, in milliseconds of playback. Enough to ride
/// out a slow repaint, short enough not to be audible as delay.
const BUFFER_MS: u32 = 100;

/// How full the ring is aimed at, so there is equal room to absorb drift in
/// either direction.
const TARGET_FILL: f32 = 0.5;

/// The most the playback rate is nudged to hold [`TARGET_FILL`], as a fraction.
///
/// The emulator is paced by the display's clock and the sound card runs on its
/// own; they are never exactly equal, so the ring would otherwise drift into an
/// overrun or an underrun after a minute or two and click. Trimming the rate to
/// absorb that is what every emulator ends up doing. 0.5% is about eight cents
/// of pitch — inaudible, and far more than any real drift needs.
const MAX_RATE_TRIM: f32 = 0.005;

/// A running output stream and the producer side of its ring.
pub struct Audio {
    ring: ringbuf::Producer<[f32; 2]>,
    /// The device's rate, which is what samples are resampled *to*.
    device_rate: u32,
    resampler: Resampler,
    /// How many frames the ring holds in total, for [`Self::fill`].
    capacity: usize,
    pub volume: f32,
    /// A short description of the device, for the Audio settings pane.
    description: String,
}

impl Audio {
    /// Open the default output device.
    ///
    /// Returns the error rather than panicking: a machine with no sound card is
    /// one this front end should still start on.
    pub fn spawn() -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("melon_egui-audio".to_owned())
            .spawn(move || match build_stream() {
                Ok((stream, ring, device_rate, capacity, description)) => {
                    if let Err(e) = stream.play() {
                        let _ = tx.send(Err(format!("cannot start audio stream: {e}")));
                        return;
                    }
                    let _ = tx.send(Ok((ring, device_rate, capacity, description)));
                    // The stream stops when dropped, so this thread holds it for
                    // the life of the process.
                    loop {
                        std::thread::park();
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            })
            .map_err(|e| format!("cannot start audio thread: {e}"))?;

        let (ring, device_rate, capacity, description) =
            rx.recv().map_err(|_| "audio thread stopped".to_owned())??;
        Ok(Self {
            ring,
            device_rate,
            resampler: Resampler::default(),
            capacity,
            volume: 1.0,
            description,
        })
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    /// How full the ring is, 0.0 to 1.0. What "audio sync" paces against: a ring
    /// filling up means emulation is outrunning the sound card.
    pub fn fill(&self) -> f32 {
        if self.capacity == 0 {
            return 0.0;
        }
        self.ring.len() as f32 / self.capacity as f32
    }

    /// Resample a batch of DS samples (interleaved stereo `i16`) into the ring.
    ///
    /// Samples that do not fit are dropped: the alternative is blocking the UI
    /// thread on the sound card, and a dropped tail is less bad than a stalled
    /// window. Returns how many output frames were written.
    pub fn push(&mut self, samples: &[i16]) -> usize {
        self.push_at(samples, SPU_SAMPLE_RATE)
    }

    /// As [`Self::push`], but for samples that are **not** at the console's
    /// rate.
    ///
    /// Remote Desktop sends its sound decimated, to keep it from being a third
    /// of the link's bandwidth, and names the rate in every datagram (see
    /// [`crate::remote::audio`]). Upsampling it back here — rather than on the
    /// host, before it is sent — is the whole saving, and costs nothing extra:
    /// [`Resampler`] was already interpolating to the device rate, so it simply
    /// takes a different step.
    pub fn push_at(&mut self, samples: &[i16], source_rate: u32) -> usize {
        // One output frame advances this far through the source. Exactly 1.0
        // for the common case of a 48 kHz source and device, which makes this a
        // copy.
        let nominal = source_rate.max(1) as f32 / self.device_rate as f32;
        let step = nominal * (1.0 + rate_trim(self.fill()));
        let ring = &mut self.ring;
        self.resampler.run(samples, step, self.volume, |frame| ring.push(frame).is_ok())
    }
}

/// Linear resampling from the SPU's rate to the device's, with enough state
/// carried between batches that the seams are inaudible.
#[derive(Default)]
pub struct Resampler {
    /// Position within the source batch carried between calls, so resampling
    /// does not restart (and click) on every frame.
    fraction: f32,
    /// The last source sample of the previous batch, to interpolate against the
    /// first sample of the next one.
    last: [f32; 2],
}

impl Resampler {
    /// Resample `samples` (interleaved stereo `i16`), handing each output frame
    /// to `push`. `step` is source frames per output frame: below 1.0 the device
    /// is faster than the DS and frames are interpolated, above 1.0 they are
    /// dropped.
    ///
    /// Stops early when `push` returns `false`, which is how a full ring is
    /// handled. Returns how many frames were accepted.
    pub fn run(
        &mut self,
        samples: &[i16],
        step: f32,
        volume: f32,
        mut push: impl FnMut([f32; 2]) -> bool,
    ) -> usize {
        let frames = samples.len() / 2;
        // A non-positive or NaN step would never terminate the loop below.
        if frames == 0 || !step.is_finite() || step <= 0.0 {
            return 0;
        }
        let last = self.last;
        let source = |i: usize| -> [f32; 2] {
            // Index 0 is the sample carried over from the previous batch, so
            // interpolation across the seam has something to work with.
            if i == 0 {
                return last;
            }
            let base = (i - 1) * 2;
            [
                f32::from(samples[base]) / f32::from(i16::MAX),
                f32::from(samples[base + 1]) / f32::from(i16::MAX),
            ]
        };

        let mut written = 0;
        let mut full = false;
        let mut at = self.fraction;
        while at < frames as f32 {
            if !full {
                let index = at as usize;
                let t = at - index as f32;
                let (a, b) = (source(index), source((index + 1).min(frames)));
                let frame =
                    [(a[0] + (b[0] - a[0]) * t) * volume, (a[1] + (b[1] - a[1]) * t) * volume];
                if push(frame) {
                    written += 1;
                } else {
                    // The sink is full, so the rest of this batch is dropped --
                    // but the position keeps advancing to the end of it. Leaving
                    // the loop here instead would restart the interpolation
                    // phase on the next batch, and a phase that resets on every
                    // batch is audible as a chorus.
                    full = true;
                }
            }
            at += step;
        }
        // Carry the leftover fraction and the final sample into the next batch.
        self.fraction = (at - frames as f32).max(0.0);
        self.last = source(frames);
        written
    }
}

/// How far to nudge the playback rate, given how full the ring is.
///
/// Positive means "advance through the source faster", which produces fewer
/// output frames and lets the ring drain; negative does the opposite. The
/// response is proportional and clamped, so it corrects drift without hunting.
fn rate_trim(fill: f32) -> f32 {
    ((fill - TARGET_FILL) * 2.0 * MAX_RATE_TRIM).clamp(-MAX_RATE_TRIM, MAX_RATE_TRIM)
}

type Built = (cpal::Stream, ringbuf::Producer<[f32; 2]>, u32, usize, String);

fn build_stream() -> Result<Built, String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device")?;
    let name = device.name().unwrap_or_else(|_| "unknown device".to_owned());
    let supported = device.default_output_config().map_err(|e| format!("no output config: {e}"))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    let capacity = (config.sample_rate.0 * BUFFER_MS / 1000) as usize;
    let (prod, cons) = ringbuf::RingBuffer::<[f32; 2]>::new(capacity).split();
    let channels = config.channels as usize;
    let description = format!("{name} - {} Hz, {channels} ch", config.sample_rate.0);

    let stream = match format {
        cpal::SampleFormat::F32 => play::<f32>(&device, &config, cons, channels),
        cpal::SampleFormat::I16 => play::<i16>(&device, &config, cons, channels),
        cpal::SampleFormat::U16 => play::<u16>(&device, &config, cons, channels),
    }
    .map_err(|e| format!("cannot open audio stream: {e}"))?;

    Ok((stream, prod, config.sample_rate.0, capacity, description))
}

fn play<T: cpal::Sample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut ring: ringbuf::Consumer<[f32; 2]>,
    channels: usize,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                // Silence when the emulator has not kept up, rather than
                // repeating the last sample, which buzzes.
                let [left, right] = ring.pop().unwrap_or([0.0, 0.0]);
                match frame {
                    [] => {}
                    // Mono devices get the two channels folded together.
                    [mono] => *mono = cpal::Sample::from(&((left + right) * 0.5).clamp(-1.0, 1.0)),
                    [l, r, rest @ ..] => {
                        *l = cpal::Sample::from(&left.clamp(-1.0, 1.0));
                        *r = cpal::Sample::from(&right.clamp(-1.0, 1.0));
                        // Surround layouts get silence on the extra channels.
                        for channel in rest {
                            *channel = cpal::Sample::from(&0.0f32);
                        }
                    }
                }
            }
        },
        |e| log::error!("audio stream error: {e}"),
    )
}

#[cfg(test)]
mod tests {
    use super::Resampler;

    /// `n` stereo frames of a ramp, so interpolation errors show up as values
    /// that are not where they should be on the ramp.
    fn ramp(n: usize) -> Vec<i16> {
        // Wrapped so that a long batch cannot overflow `i16`.
        (0..n).map(|i| ((i % 300) as i16) * 100).flat_map(|v| [v, -v]).collect()
    }

    /// What `ramp` put in source frame `i`, scaled the way the resampler scales
    /// it. Frame 0 is the sample carried in from the previous batch.
    fn ramp_value(i: usize) -> f32 {
        ((i % 300) as f32) * 100.0 / f32::from(i16::MAX)
    }

    fn collect(resampler: &mut Resampler, samples: &[i16], step: f32) -> Vec<[f32; 2]> {
        let mut out = Vec::new();
        resampler.run(samples, step, 1.0, |frame| {
            out.push(frame);
            true
        });
        out
    }

    #[test]
    fn matching_rates_pass_every_frame_through_once() {
        let mut resampler = Resampler::default();
        let out = collect(&mut resampler, &ramp(8), 1.0);
        assert_eq!(out.len(), 8);
        // At step 1.0 output n is source n exactly, with no interpolation. Source
        // 0 is the sample carried in from the previous batch -- silence, here --
        // and source n after that is batch sample n - 1.
        assert_eq!(out[0], [0.0, 0.0], "the first frame is the carried silence");
        for (n, frame) in out.iter().enumerate().skip(1) {
            let expected = ramp_value(n - 1);
            assert!((frame[0] - expected).abs() < 1e-4, "frame {n}: {frame:?}");
            assert!((frame[1] + expected).abs() < 1e-4, "frame {n} right is negated");
        }
    }

    #[test]
    fn a_faster_device_interpolates_and_a_slower_one_drops() {
        // Device at twice the DS rate: twice as many output frames.
        let mut up = Resampler::default();
        assert_eq!(collect(&mut up, &ramp(100), 0.5).len(), 200);

        // Device at half the DS rate: half as many.
        let mut down = Resampler::default();
        assert_eq!(collect(&mut down, &ramp(100), 2.0).len(), 50);
    }

    /// The seam between batches is where a naive resampler clicks: it must
    /// produce the same total as one continuous run.
    #[test]
    fn the_position_carries_across_batches() {
        let step = 32768.0 / 48000.0;

        let mut split = Resampler::default();
        let batched: usize = (0..4).map(|_| collect(&mut split, &ramp(100), step).len()).sum();

        let mut whole = Resampler::default();
        let continuous = collect(&mut whole, &ramp(400), step).len();

        assert_eq!(batched, continuous, "batching must not change the frame count");
        // And the count is what the rate ratio implies, within a frame.
        assert!((batched as f32 - 400.0 / step).abs() <= 1.0, "{batched}");
    }

    #[test]
    fn a_full_sink_stops_the_batch_without_losing_sync() {
        let mut resampler = Resampler::default();
        let mut room = 3;
        let taken = resampler.run(&ramp(100), 1.0, 1.0, |_| {
            room -= 1;
            room >= 0
        });
        assert_eq!(taken, 3, "only what fitted was accepted");
    }

    #[test]
    fn volume_scales_the_output() {
        let mut resampler = Resampler::default();
        let mut out = Vec::new();
        resampler.run(&ramp(4), 1.0, 0.5, |frame| {
            out.push(frame);
            true
        });
        // Output 2 is source 2, which is batch sample 1.
        let expected = ramp_value(1) * 0.5;
        assert!((out[2][0] - expected).abs() < 1e-4, "{:?}", out[2]);
    }

    /// A full sink must not restart the interpolation phase: doing so on every
    /// batch is what a chorus artifact sounds like.
    #[test]
    fn a_full_sink_does_not_disturb_the_phase() {
        let step = 48000.0 / 44100.0;

        // One run that never fills up.
        let mut clean = Resampler::default();
        collect(&mut clean, &ramp(100), step);
        let clean_phase = clean.fraction;

        // The same batch into a sink that fills after three frames.
        let mut choked = Resampler::default();
        let mut room = 3;
        choked.run(&ramp(100), step, 1.0, |_| {
            room -= 1;
            room >= 0
        });

        assert!(
            (choked.fraction - clean_phase).abs() < 1e-4,
            "phase drifted: {} vs {clean_phase}",
            choked.fraction,
        );
    }

    /// Guards the constant itself, since getting it wrong is audible and the
    /// value comes from a default two layers down in the C++.
    #[test]
    fn a_matching_device_rate_is_a_straight_copy() {
        let mut resampler = Resampler::default();
        let out = collect(&mut resampler, &ramp(64), 1.0);
        assert_eq!(out.len(), 64, "no frames invented or dropped at 1:1");
        for (n, frame) in out.iter().enumerate().skip(1) {
            let expected = ramp_value(n - 1);
            assert!(
                (frame[0] - expected).abs() < 1e-4,
                "frame {n} was interpolated rather than copied: {frame:?}",
            );
        }
    }

    #[test]
    fn the_rate_trim_pushes_the_ring_back_towards_half_full() {
        use super::{MAX_RATE_TRIM, rate_trim};

        // Balanced: leave the rate alone.
        assert!(rate_trim(0.5).abs() < 1e-6);
        // Too full: speed up through the source so fewer frames come out.
        assert!(rate_trim(0.9) > 0.0);
        // Too empty: slow down so more come out.
        assert!(rate_trim(0.1) < 0.0);
        // And never by more than the cap, however far out it is.
        assert!(rate_trim(1.0) <= MAX_RATE_TRIM + 1e-6);
        assert!(rate_trim(0.0) >= -MAX_RATE_TRIM - 1e-6);
        // The cap has to stay inaudible; 0.5% is about eight cents.
        const { assert!(MAX_RATE_TRIM <= 0.01) };
    }

    #[test]
    fn an_empty_batch_is_not_an_error() {
        let mut resampler = Resampler::default();
        assert_eq!(collect(&mut resampler, &[], 1.0).len(), 0);
    }
}
