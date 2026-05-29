use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;

pub struct Audio {
    config: cpal::StreamConfig,
    _stream: cpal::Stream,
    prod: ringbuf::Producer<[f32; 2]>,
    fraction: f32,
    history: VecDeque<[f32; 2]>, // history buffer for sinc
    volume: f32,
}

impl Audio {
    const SINC_WINDOW: usize = 16; // ±16 samples

    fn calculate_buffer_len(sample_rate: u32) -> usize {
        ((sample_rate as usize) * 100) / 1000
    }

    pub fn new() -> Self {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .expect("No audio output device available!");
        let config = device
            .default_output_config()
            .expect("No audio output config available!");

        match config.sample_format() {
            cpal::SampleFormat::F32 => Audio::init::<f32>(device, config.into()),
            cpal::SampleFormat::I16 => Audio::init::<i16>(device, config.into()),
            cpal::SampleFormat::U16 => Audio::init::<u16>(device, config.into()),
        }
    }

    fn init<T: cpal::Sample>(device: cpal::Device, config: cpal::StreamConfig) -> Self {
        let buffer_len = Audio::calculate_buffer_len(config.sample_rate.0);

        let main_buffer = ringbuf::RingBuffer::<[f32; 2]>::new(buffer_len);
        let (prod, mut cons) = main_buffer.split();

        let output_config = OutputConfig::from(config.channels);

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                    for frame in data.chunks_mut(output_config as usize) {
                        let samples = cons.pop().unwrap_or([0.0, 0.0]);
                        match output_config {
                            OutputConfig::Mono => {
                                let sample = (samples[0] + samples[1]) * 0.5;
                                frame[0] = cpal::Sample::from::<f32>(&sample.max(-1.0).min(1.0));
                            }
                            OutputConfig::Stereo => {
                                frame[0] =
                                    cpal::Sample::from::<f32>(&samples[0].max(-1.0).min(1.0));
                                frame[1] =
                                    cpal::Sample::from::<f32>(&samples[1].max(-1.0).min(1.0));
                            }
                        }
                    }
                },
                |err| error!("Audio Stream Error: {}", err),
            )
            .unwrap();
        stream.play().unwrap();

        Audio {
            config,
            _stream: stream,
            prod,
            fraction: 0.0,
            history: VecDeque::with_capacity(Self::SINC_WINDOW * 4),
            volume: 1.0,
        }
    }

    fn sinc(x: f32) -> f32 {
        if x.abs() < 1e-6 {
            1.0
        } else {
            (std::f32::consts::PI * x).sin() / (std::f32::consts::PI * x)
        }
    }

    fn sinc_interpolate(history: &VecDeque<[f32; 2]>, frac: f32, window: usize) -> [f32; 2] {
        let mut out = [0.0, 0.0];

        // Make sure we have enough samples
        let len = history.len();
        if len == 0 {
            return out;
        }

        // Pick last window*2 samples
        let start = len.saturating_sub(window * 2);
        let samples: Vec<_> = history.iter().skip(start).collect();

        let frac_clamped = frac.max(0.0).min((samples.len() - 1) as f32);

        for (n, &sample) in samples.iter().enumerate() {
            let x = n as f32 - frac_clamped;
            let s = Audio::sinc(x);
            out[0] += sample[0] * s;
            out[1] += sample[1] * s;
        }

        out
    }

    pub fn push_sample(&mut self, left_sample: f32, right_sample: f32) {
        let left_scaled = (left_sample * self.volume).clamp(-1.0, 1.0);
        let right_scaled = (right_sample * self.volume).clamp(-1.0, 1.0);

        // push to history
        if self.history.len() >= Self::SINC_WINDOW * 4 {
            self.history.pop_front();
        }
        self.history.push_back([left_scaled, right_scaled]);

        // let ratio = 32768.0 / self.config.sample_rate.0 as f32;
        let ratio = 1.0;
        self.fraction += ratio;

        // 32.768 kHz → device sample rate
        while self.fraction >= 1.0 {
            self.fraction -= 1.0;

            // wait if output buffer full
            let mut attempts = 0;
            while self.prod.is_full() {
                attempts += 1;
                if attempts > 1000 {
                    std::thread::yield_now();
                    attempts = 0;
                }
            }

            let pos = (self.history.len().saturating_sub(1)) as f32 - self.fraction;
            let pos = pos.max(0.0); // negative positions are invalid
            let interpolated = Self::sinc_interpolate(&self.history, pos, Self::SINC_WINDOW);
            let _ = self.prod.push(interpolated);
        }
    }

    pub fn sample_rate(&self) -> usize {
        self.config.sample_rate.0 as usize // 48kHz
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
    }
}

#[derive(Clone, Copy)]
enum OutputConfig {
    Mono = 1,
    Stereo = 2,
}

impl From<u16> for OutputConfig {
    fn from(value: u16) -> Self {
        match value {
            1 => OutputConfig::Mono,
            2 => OutputConfig::Stereo,
            _ => panic!("Only Mono and Stereo audio devices supported!"),
        }
    }
}
