//! NDS Sound Processing Unit (SPU).
//!
//! The NDS has **16 hardware sound channels** (SOUNDxCNT at 4000400h + N×10h):
//! - Channels  0-7:  PCM8 / PCM16 / ADPCM (IMA-ADPCM).  Here: `base_channels`.
//! - Channels  8-13: PSG square-wave (six duty cycles) or PCM/ADPCM. `psg_channels`.
//! - Channels 14-15: Noise or PCM/ADPCM. `noise_channels`.
//!
//! Master control via **SOUNDCNT** (4000500h): master volume, output routing,
//! channel enable. **SOUNDBIAS** (4000504h) sets DC bias for the DAC.
//!
//! Output sample rate: nominally **32728 Hz** (= master clock / 1024).
//! The current implementation derives `clocks_per_sample` from the host audio
//! device rate and resamples; the TODO below tracks a missing 32728 Hz native
//! path.
//!
//! ADPCM uses the IMA-ADPCM standard with Nintendo's fixed step and index
//! tables (`ADPCM_TABLE` / `ADPCM_INDEX_TABLE`).
//!
//! GBATEK references:
//! - Sound overview: <https://problemkaputt.de/gbatek.htm#dssound>
//! - Channel registers (SOUNDxCNT/SAD/TMR/PNT/LEN):
//!   <https://problemkaputt.de/gbatek.htm#dssoundchannels015>
//! - Control registers (SOUNDCNT/SOUNDBIAS):
//!   <https://problemkaputt.de/gbatek.htm#dssoundcontrolregisters>
//! - Sound capture: <https://problemkaputt.de/gbatek.htm#dssoundcapture>
//! - Timing / mixing notes: <https://problemkaputt.de/gbatek.htm#dssoundnotes>

mod audio;
mod registers;

use super::{
    HW,
    mem::IORegister,
    scheduler::{Event, Scheduler},
};

use audio::Audio;
use registers::*;

/// NDS SPU state.
///
/// Sound is generated on the scheduler via [`Event::GenerateAudioSample`];
/// each individual channel step uses [`Event::StepAudioChannel`].
#[derive(emu_utils::Savestate)]
pub struct SPU {
    cnt: SoundControl,
    sound_bias: u16,
    captures: [Capture; 2],
    // Sound Generation
    #[savestate(skip)]
    audio: Audio,
    clocks_per_sample: usize,
    /// Running peak amplitude for the `mix` diagnostic probe.
    #[savestate(skip)]
    diag_peak: u16,
    /// Samples accumulated into `diag_peak` since the last report.
    #[savestate(skip)]
    diag_samples: u32,
    // Channels
    pub base_channels: [Channel<BaseChannel>; 8],
    pub psg_channels: [Channel<PSGChannel>; 6],
    pub noise_channels: [Channel<NoiseChannel>; 2],
}

macro_rules! create_channels {
    ($type:ident, $spec:ident, $( $num:expr ), *) => {
        [
            $(
                Channel::<$type>::new(ChannelSpec::$spec($num)),
            )*
        ]
    };
}

impl SPU {
    /// IMA-ADPCM index adjustment table (4-bit nibble high-3 bits → index delta).
    ///
    /// GBATEK "DS Sound Notes – ADPCM decoding":
    /// <https://problemkaputt.de/gbatek.htm#dssoundnotes>
    pub const ADPCM_INDEX_TABLE: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];
    /// IMA-ADPCM step-size table (89 entries, index 0–88).
    ///
    /// GBATEK "DS Sound Notes – AdpcmTable":
    /// <https://problemkaputt.de/gbatek.htm#dssoundnotes>
    pub const ADPCM_TABLE: [u16; 89] = [
        0x0007, 0x0008, 0x0009, 0x000A, 0x000B, 0x000C, 0x000D, 0x000E, 0x0010, 0x0011, 0x0013,
        0x0015, 0x0017, 0x0019, 0x001C, 0x001F, 0x0022, 0x0025, 0x0029, 0x002D, 0x0032, 0x0037,
        0x003C, 0x0042, 0x0049, 0x0050, 0x0058, 0x0061, 0x006B, 0x0076, 0x0082, 0x008F, 0x009D,
        0x00AD, 0x00BE, 0x00D1, 0x00E6, 0x00FD, 0x0117, 0x0133, 0x0151, 0x0173, 0x0198, 0x01C1,
        0x01EE, 0x0220, 0x0256, 0x0292, 0x02D4, 0x031C, 0x036C, 0x03C3, 0x0424, 0x048E, 0x0502,
        0x0583, 0x0610, 0x06AB, 0x0756, 0x0812, 0x08E0, 0x09C3, 0x0ABD, 0x0BD0, 0x0CFF, 0x0E4C,
        0x0FBA, 0x114C, 0x1307, 0x14EE, 0x1706, 0x1954, 0x1BDC, 0x1EA5, 0x21B6, 0x2515, 0x28CA,
        0x2CDF, 0x315B, 0x364B, 0x3BB9, 0x41B2, 0x4844, 0x4F7E, 0x5771, 0x602F, 0x69CE, 0x7462,
        0x7FFF,
    ];

    pub fn new(scheduler: &mut Scheduler) -> Self {
        let audio = Audio::new();
        // TODO: Sample at 32.768 kHz and resample to device sample rate
        let clocks_per_sample = crate::nds::NDS::CLOCK_RATE / audio.sample_rate();
        scheduler.schedule(
            Event::GenerateAudioSample,
            HW::generate_audio_sample,
            clocks_per_sample,
        );
        SPU {
            cnt: SoundControl::new(),
            sound_bias: 0,
            captures: [Capture::new(), Capture::new()],
            // Sound Generation
            audio,
            clocks_per_sample,
            diag_peak: 0,
            diag_samples: 0,
            // Channels
            base_channels: create_channels!(BaseChannel, Base, 0, 1, 2, 3, 4, 5, 6, 7),
            psg_channels: create_channels!(PSGChannel, PSG, 0, 1, 2, 3, 4, 5),
            noise_channels: create_channels!(NoiseChannel, Noise, 0, 1),
        }
    }

    fn generate_mixer(&self) -> ((i32, i32), (i32, i32), (i32, i32)) {
        let mut mixer = (0, 0);
        for i in (0..1).chain(2..3).chain(4..self.base_channels.len()) {
            self.base_channels[i].generate_sample(&mut mixer)
        }
        for channel in self.psg_channels.iter() {
            channel.generate_sample(&mut mixer)
        }
        for channel in self.noise_channels.iter() {
            channel.generate_sample(&mut mixer)
        }
        let (mut ch1, mut ch3) = ((0, 0), (0, 0));
        self.base_channels[1].generate_sample(&mut ch1);
        self.base_channels[3].generate_sample(&mut ch3);
        if self.cnt.output_1 {
            mixer.0 += ch1.0;
            mixer.1 += ch1.1
        }
        if self.cnt.output_3 {
            mixer.0 += ch3.0;
            mixer.1 += ch3.1
        }
        (mixer, ch1, ch3)
    }

    /// Mixes all 16 channels into one stereo output sample.
    ///
    /// Applies per-channel volume/pan, the SOUNDCNT left/right output
    /// selector (mixer / ch1 / ch3 / ch1+ch3), and master volume, then
    /// pushes the sample to the host audio ring buffer.
    ///
    /// GBATEK "SOUNDCNT – Left/Right Output Source, Master Volume":
    /// <https://problemkaputt.de/gbatek.htm#dssoundcontrolregisters>
    pub fn generate_sample(&mut self) {
        // SOUNDCNT bit 15 is the master enable: with it clear the SPU outputs
        // silence. The scheduler event keeps running so the host audio ring
        // buffer stays fed at a constant rate.
        if !self.cnt.enable {
            self.audio.push_sample(cpal::Sample::from::<i16>(&0), cpal::Sample::from::<i16>(&0));
            return;
        }
        let (mixer, ch1, ch3) = self.generate_mixer();
        // Each channel contributes `sample * volume_factor * pan_factor`, i.e.
        // two 0..128 factors, so the mixer accumulator carries 14 fractional
        // bits. Shifting by 16 here attenuated the whole mix by a further
        // factor of four; and the final cast has to saturate, since a loud mix
        // otherwise wraps around and turns into full-scale noise.
        const MIXER_FRAC_BITS: u8 = 14;
        let left_sample = match self.cnt.left_output {
            ChannelOutput::Mixer => mixer.0,
            ChannelOutput::Ch1 => ch1.0,
            ChannelOutput::Ch3 => ch3.0,
            ChannelOutput::Ch1Ch3 => ch1.0 + ch3.0,
        } >> MIXER_FRAC_BITS;
        let right_sample = match self.cnt.right_output {
            ChannelOutput::Mixer => mixer.1,
            ChannelOutput::Ch1 => ch1.1,
            ChannelOutput::Ch3 => ch3.1,
            ChannelOutput::Ch1Ch3 => ch1.1 + ch3.1,
        } >> MIXER_FRAC_BITS;
        let clamp = |sample: i32| {
            ((sample * self.cnt.master_volume()) >> 7).clamp(i16::MIN as i32, i16::MAX as i32)
                as i16
        };
        let final_sample = (clamp(left_sample), clamp(right_sample));
        self.log_output_level(final_sample);
        self.audio.push_sample(
            cpal::Sample::from::<i16>(&final_sample.0),
            cpal::Sample::from::<i16>(&final_sample.1),
        );
    }

    /// Diagnostic `mix`: reports the peak absolute amplitude of the mixed
    /// output roughly once per second of emulated audio, so "no audio" can be
    /// pinned to either the mixer or the host output path.
    ///
    /// See `docs/design/rendering-audio-fix-design.md` §3.
    fn log_output_level(&mut self, sample: (i16, i16)) {
        if !crate::hw::diag::probe("mix") {
            return;
        }
        const REPORT_EVERY: u32 = 32768;
        self.diag_peak = self.diag_peak.max(sample.0.unsigned_abs()).max(sample.1.unsigned_abs());
        self.diag_samples += 1;
        if self.diag_samples >= REPORT_EVERY {
            let active = self.base_channels.iter().filter(|c| c.is_busy()).count()
                + self.psg_channels.iter().filter(|c| c.is_busy()).count()
                + self.noise_channels.iter().filter(|c| c.is_busy()).count();
            let busy_state = self
                .base_channels
                .iter()
                .map(|c| c.diag_state())
                .chain(self.psg_channels.iter().map(|c| c.diag_state()))
                .chain(self.noise_channels.iter().map(|c| c.diag_state()))
                .enumerate()
                .filter(|(_, (busy, ..))| *busy)
                .map(|(i, (_, sample, vol, pan, timer))| {
                    format!("ch{i}(s={sample},v={vol},p={pan},t={timer})")
                })
                .collect::<Vec<_>>()
                .join(" ");
            crate::diag!(
                "mix",
                "peak={} master_volume={} enable={} active_channels={} {busy_state}",
                self.diag_peak,
                self.cnt.master_volume(),
                self.cnt.enable as u8,
                active
            );
            self.diag_peak = 0;
            self.diag_samples = 0;
        }
    }

    pub fn set_audio_volume(&mut self, volume_percent: f32) {
        self.audio.set_volume(volume_percent / 100.0);
    }

    pub fn capture_addr(&mut self, num: usize) -> Option<(u32, usize, bool)> {
        let capture_i = match num {
            1 => 0,
            3 => 1,
            _ => return None,
        };
        let capture = &mut self.captures[capture_i];
        if capture.num_bytes_left == 0 || !capture.cnt.busy {
            return None;
        }
        if capture.cnt.use_pcm8 {
            Some((capture.next_addr::<u8>(), capture_i, true))
        } else {
            Some((capture.next_addr::<u16>(), capture_i, false))
        }
    }

    /// Produces one captured sample for capture unit `capture_i`.
    ///
    /// Source select (SNDCAPxCNT bit 1) picks the raw output of channel 0 / 2
    /// instead of the left / right mixer output; the "add" bit (bit 0) is a
    /// hardware quirk that sums channel 1+2 (respectively 3+0) and is not
    /// modelled - the plain source is captured instead, which is audibly
    /// equivalent for the common echo/output-routing use.
    ///
    /// GBATEK "DS Sound Capture": <https://problemkaputt.de/gbatek.htm#dssoundcapture>
    pub fn capture_data<T: super::MemoryValue>(&self, capture_i: usize) -> T {
        let capture_value = if self.captures[capture_i].cnt.use_channel {
            let channel = if capture_i == 0 { 0 } else { 2 };
            self.base_channels[channel].sample() as u16
        } else {
            let (mixer, _, _) = self.generate_mixer();
            let mixer_value = (if capture_i == 0 { mixer.0 } else { mixer.1 } >> 16) as u16;
            if std::mem::size_of::<T>() == 1 { mixer_value >> 8 } else { mixer_value }
        };
        num_traits::cast(capture_value).unwrap()
    }

    pub fn read_channels(&self, addr: usize) -> u8 {
        let channel = (addr >> 4) & 0xF;
        let byte = addr & 0xF;
        match channel {
            0x0..=0x7 => self.base_channels[channel].read(byte),
            0x8..=0xD => self.psg_channels[channel - 0x8].read(byte),
            0xE..=0xF => self.noise_channels[channel - 0xE].read(byte),
            _ => unreachable!(),
        }
    }

    pub fn write_channels(&mut self, scheduler: &mut Scheduler, addr: usize, value: u8) {
        let channel = (addr >> 4) & 0xF;
        let byte = addr & 0xF;
        match channel {
            0x0..=0x7 => self.base_channels[channel].write(scheduler, byte, value),
            0x8..=0xD => self.psg_channels[channel - 0x8].write(scheduler, byte, value),
            0xE..=0xF => self.noise_channels[channel - 0xE].write(scheduler, byte, value),
            _ => unreachable!(),
        }
    }
}

impl IORegister for SPU {
    fn read(&self, addr: usize) -> u8 {
        match addr {
            0x400..=0x4FF => self.read_channels(addr),
            0x500..=0x503 => self.cnt.read(addr & 0x3),
            0x504..=0x507 => HW::read_byte_from_value(&self.sound_bias, addr & 0x3),
            0x508..=0x509 => self.captures[addr & 0x1].cnt.read(),
            0x510..=0x51F => self.captures[addr >> 3 & 0x1].read(addr & 0xF),
            _ => {
                warn!("Ignoring SPU Register Read at 0x04000{:03X}", addr);
                0
            }
        }
    }

    fn write(&mut self, scheduler: &mut Scheduler, addr: usize, value: u8) {
        match addr {
            0x400..=0x4FF => {
                self.write_channels(scheduler, addr & 0xFF, value);
                // Diagnostic D-5: log a channel only when its control byte 3
                // (busy / format / repeat) is touched, which is what starts or
                // stops a voice.
                if addr & 0xF == 0x3 {
                    let channel = (addr >> 4) & 0xF;
                    crate::diag!(
                        "spu",
                        "ch{channel}: busy={} format={} repeat={} duty={}",
                        value >> 7 & 0x1,
                        value >> 5 & 0x3,
                        value >> 3 & 0x3,
                        value & 0x7
                    );
                }
            }
            0x500..=0x503 => {
                self.cnt.write(scheduler, addr & 0x3, value);
                crate::diag!(
                    "spu",
                    "SOUNDCNT: master_volume={} enable={} out1={} out3={}",
                    self.cnt.master_volume(),
                    self.cnt.enable as u8,
                    self.cnt.output_1 as u8,
                    self.cnt.output_3 as u8
                );
            }
            0x504..=0x507 => {
                HW::write_byte_to_value(&mut self.sound_bias, addr & 0x3, value);
                self.sound_bias &= 0x3FF;
            }
            0x508..=0x509 => {
                self.captures[addr & 0x1].write_cnt(value);
                crate::diag!(
                    "spu",
                    "SNDCAP{}: busy={} pcm8={} no_repeat={} use_channel={} add={}",
                    addr & 0x1,
                    value >> 7 & 0x1,
                    value >> 3 & 0x1,
                    value >> 2 & 0x1,
                    value >> 1 & 0x1,
                    value & 0x1
                );
            }
            0x510..=0x51F => self.captures[addr >> 3 & 0x1].write(addr & 0x7, value),
            _ => warn!("Ignoring SPU Register Write at 0x04000{:03X}", addr),
        }
    }
}

impl HW {
    /// Scheduler handler for [`Event::GenerateAudioSample`]; re-schedules
    /// itself every `clocks_per_sample` master-clock cycles.
    ///
    /// Hardware mixes at 32.768 kHz (one sample per 1024 cycles of the
    /// 33.554432 MHz sound clock); this implementation matches the host
    /// device rate instead.  GBATEK "DS Sound Notes – sample rate":
    /// <https://problemkaputt.de/gbatek.htm#dssoundnotes>
    pub fn generate_audio_sample(&mut self, _event: Event) {
        self.scheduler.schedule(
            Event::GenerateAudioSample,
            HW::generate_audio_sample,
            self.spu.clocks_per_sample,
        );
        self.spu.generate_sample();
    }

    /// Scheduler handler for [`Event::StepAudioChannel`]: fetches the next
    /// PCM8 / PCM16 / ADPCM sample (or advances PSG/noise state) for one
    /// channel via ARM7 bus reads.
    ///
    /// GBATEK "DS Sound Channels 0..15" (formats, ADPCM header word):
    /// <https://problemkaputt.de/gbatek.htm#dssoundchannels015>
    pub fn step_audio_channel(&mut self, event: Event) {
        let channel_spec = match event {
            Event::StepAudioChannel(channel_spec) => channel_spec,
            _ => unreachable!(),
        };
        match channel_spec {
            // TODO: Figure out how to avoid code duplication
            // TODO: Use SPU FIFO
            ChannelSpec::Base(num) => {
                let format = self.spu.base_channels[num].format();
                match format {
                    Format::PCM8 => {
                        let (addr, reset) = self.spu.base_channels[num].next_addr_pcm::<u8>();
                        self.spu.base_channels[num].schedule(&mut self.scheduler, reset);
                        let sample = self.arm7_read::<u8>(addr);
                        self.spu.base_channels[num].set_sample(sample);
                    }
                    Format::PCM16 => {
                        let (addr, reset) = self.spu.base_channels[num].next_addr_pcm::<u16>();
                        self.spu.base_channels[num].schedule(&mut self.scheduler, reset);
                        let sample = self.arm7_read::<u16>(addr);
                        self.spu.base_channels[num].set_sample(sample);
                    }
                    Format::ADPCM => {
                        let reset =
                            if let Some(addr) = self.spu.base_channels[num].initial_adpcm_addr() {
                                let value = self.arm7_read::<u32>(addr);
                                self.spu.base_channels[num].set_initial_adpcm(value);
                                false
                            } else {
                                let (addr, reset) = self.spu.base_channels[num].next_addr_adpcm();
                                let value = self.arm7_read(addr);
                                self.spu.base_channels[num].set_adpcm_data(value);
                                reset
                            };
                        self.spu.base_channels[num].schedule(&mut self.scheduler, reset);
                    }
                    // Format 3 has no meaning on channels 0-7 (PSG/noise start
                    // at channel 8). Hardware simply produces nothing rather
                    // than faulting, so keep the channel silent instead of
                    // panicking on a stray write.
                    Format::Special => self.spu.base_channels[num].reset_sample(),
                }
                // Sound capture: units 0/1 are clocked by channels 1/3 and write
                // the mixer output back into ARM7-visible memory. Games route
                // that buffer straight back into channels 1/3 and select them as
                // the SOUNDCNT output source, so leaving the write-back out
                // makes the whole mix inaudible even though every voice is
                // running.
                //
                // GBATEK "DS Sound Capture":
                // <https://problemkaputt.de/gbatek.htm#dssoundcapture>
                if let Some((addr, capture_i, use_pcm8)) = self.spu.capture_addr(num) {
                    if use_pcm8 {
                        let value: u8 = self.spu.capture_data(capture_i);
                        self.arm7_write::<u8>(addr, value);
                    } else {
                        let value: u16 = self.spu.capture_data(capture_i);
                        self.arm7_write::<u16>(addr, value);
                    }
                }
            }
            ChannelSpec::PSG(num) => {
                let format = self.spu.psg_channels[num].format();
                match format {
                    Format::PCM8 => {
                        let (addr, reset) = self.spu.psg_channels[num].next_addr_pcm::<u8>();
                        self.spu.psg_channels[num].schedule(&mut self.scheduler, reset);
                        let sample = self.arm7_read::<u8>(addr);
                        self.spu.psg_channels[num].set_sample(sample);
                    }
                    Format::PCM16 => {
                        let (addr, reset) = self.spu.psg_channels[num].next_addr_pcm::<u16>();
                        self.spu.psg_channels[num].schedule(&mut self.scheduler, reset);
                        let sample = self.arm7_read::<u16>(addr);
                        self.spu.psg_channels[num].set_sample(sample);
                    }
                    Format::ADPCM => {
                        let reset =
                            if let Some(addr) = self.spu.psg_channels[num].initial_adpcm_addr() {
                                let value = self.arm7_read::<u32>(addr);
                                self.spu.psg_channels[num].set_initial_adpcm(value);
                                false
                            } else {
                                let (addr, reset) = self.spu.psg_channels[num].next_addr_adpcm();
                                let value = self.arm7_read(addr);
                                self.spu.psg_channels[num].set_adpcm_data(value);
                                reset
                            };
                        self.spu.psg_channels[num].schedule(&mut self.scheduler, reset);
                    }
                    Format::Special => {
                        self.spu.psg_channels[num].schedule(&mut self.scheduler, false);
                        self.spu.psg_channels[num].step_psg();
                    }
                }
            }
            ChannelSpec::Noise(num) => {
                let format = self.spu.noise_channels[num].format();
                match format {
                    Format::PCM8 => {
                        let (addr, reset) = self.spu.noise_channels[num].next_addr_pcm::<u8>();
                        self.spu.noise_channels[num].schedule(&mut self.scheduler, reset);
                        let sample = self.arm7_read::<u8>(addr);
                        self.spu.noise_channels[num].set_sample(sample);
                    }
                    Format::PCM16 => {
                        let (addr, reset) = self.spu.noise_channels[num].next_addr_pcm::<u16>();
                        self.spu.noise_channels[num].schedule(&mut self.scheduler, reset);
                        let sample = self.arm7_read::<u16>(addr);
                        self.spu.noise_channels[num].set_sample(sample);
                    }
                    Format::ADPCM => {
                        let reset =
                            if let Some(addr) = self.spu.noise_channels[num].initial_adpcm_addr() {
                                let value = self.arm7_read::<u32>(addr);
                                self.spu.noise_channels[num].set_initial_adpcm(value);
                                false
                            } else {
                                let (addr, reset) = self.spu.noise_channels[num].next_addr_adpcm();
                                let value = self.arm7_read(addr);
                                self.spu.noise_channels[num].set_adpcm_data(value);
                                reset
                            };
                        self.spu.noise_channels[num].schedule(&mut self.scheduler, reset);
                    }
                    Format::Special => {
                        self.spu.noise_channels[num].schedule(&mut self.scheduler, false);
                        self.spu.noise_channels[num].step_noise();
                    }
                }
            }
        }
    }

    pub fn reset_audio_channel(&mut self, event: Event) {
        let channel_spec = match event {
            Event::ResetAudioChannel(channel_spec) => channel_spec,
            _ => unreachable!(),
        };
        match channel_spec {
            ChannelSpec::Base(num) => self.spu.base_channels[num].reset_sample(),
            ChannelSpec::PSG(num) => self.spu.psg_channels[num].reset_sample(),
            ChannelSpec::Noise(num) => self.spu.noise_channels[num].reset_sample(),
        }
    }
}

/// One of the 16 sound channels (registers SOUNDxCNT / SOUNDxSAD /
/// SOUNDxTMR / SOUNDxPNT / SOUNDxLEN at 40004x0h).
///
/// The channel timer counts up at 2× the master clock from `timer_val` to
/// 10000h, i.e. one sample every `(10000h - timer_val) * 2` cycles.
///
/// GBATEK "DS Sound Channels 0..15":
/// <https://problemkaputt.de/gbatek.htm#dssoundchannels015>
#[derive(emu_utils::Savestate)]
#[load(in_place_only)]
pub struct Channel<T: ChannelType> {
    // Registers
    cnt: ChannelControl<T>,
    src_addr: u32,
    timer_val: u16,
    loop_start: u16,
    len: u32,
    // Sample Generation
    spec: ChannelSpec,
    addr: u32,
    num_bytes_left: usize,
    sample: i16,
    // PSG / Noise
    /// Position within the 8-step PSG duty cycle (channels 8-13).
    ///
    /// Skipped by the savestate (like `noise_lfsr` below) so that states
    /// written before PSG/noise generation existed still load; both are
    /// re-seeded whenever a channel is started.
    #[savestate(skip)]
    psg_pos: u8,
    /// 15-bit noise LFSR (channels 14-15), seeded to 0x7FFF on channel start.
    #[savestate(skip)]
    noise_lfsr: u16,
    // ADPCM
    adpcm_in_header: bool,
    adpcm_low_nibble: bool,
    adpcm_index: i32,
    adpcm_value: i16,
    initial_adpcm_index: i32,
    initial_adpcm_value: i16,
}

impl<T: ChannelType> IORegister for Channel<T> {
    fn read(&self, byte: usize) -> u8 {
        match byte {
            0x0..=0x3 => self.cnt.read(byte & 0x3),
            0x4..=0x7 => {
                warn!("Reading from Write-Only SPU Register: Src Addr");
                0
            }
            0x8..=0x9 => {
                warn!("Reading from Write-Only SPU Register: Timer");
                0
            }
            0xA..=0xB => {
                warn!("Reading from Write-Only SPU Register: Loop Start");
                0
            }
            0xC..=0xF => {
                warn!("Reading from Write-Only SPU Register: Len");
                0
            }
            _ => unreachable!(),
        }
    }

    fn write(&mut self, scheduler: &mut super::scheduler::Scheduler, byte: usize, value: u8) {
        let shift16 = 8 * (byte & 0x1);
        let shift32 = 8 * (byte & 0x3);
        let mask16 = 0xFF << shift16;
        let mask32 = 0xFF << shift32;
        let value16 = (value as u16) << shift16;
        let value32 = (value as u32) << shift32;
        // TODO: Fix inaccurate scheduling timing for maxmod interpolated mode
        match byte {
            0x0..=0x2 => self.cnt.write(scheduler, byte & 0x3, value),
            0x3 => {
                let prev_busy = self.cnt.busy;
                self.cnt.write(scheduler, byte & 0x3, value);
                if !prev_busy && self.cnt.busy {
                    self.adpcm_in_header = true;
                    self.adpcm_low_nibble = true;
                    self.psg_pos = 0;
                    self.noise_lfsr = Channel::<T>::NOISE_LFSR_INIT;
                    self.schedule(scheduler, false);
                } else if !self.cnt.busy {
                    scheduler.remove(Event::StepAudioChannel(self.spec));
                }
            }
            0x4..=0x7 => {
                self.src_addr = (self.src_addr & !mask32 | value32) & 0x3FF_FFFF;
                self.addr = self.src_addr;
                // TODO: Behavior when channel has already started
            }
            0x8..=0x9 => {
                self.timer_val = self.timer_val & !mask16 | value16;
                if self.cnt.busy {
                    self.schedule(scheduler, false)
                }
            }
            0xA..=0xB => {
                self.loop_start = self.loop_start & !mask16 | value16;
                self.num_bytes_left = (self.loop_start as usize + self.len as usize) * 4;
                if self.cnt.busy {
                    self.schedule(scheduler, false)
                }
            }
            0xC..=0xF => {
                self.len = (self.len & !mask32 | value32) & 0x3F_FFFF;
                self.num_bytes_left = (self.loop_start as usize + self.len as usize) * 4;
                if self.cnt.busy {
                    self.schedule(scheduler, false)
                }
            }
            _ => unreachable!(),
        }
    }
}

impl<T: ChannelType> Channel<T> {
    /// Seed value of the 15-bit noise LFSR (GBATEK "DS Sound Channels 0..15",
    /// noise generator): <https://problemkaputt.de/gbatek.htm#dssoundchannels015>
    const NOISE_LFSR_INIT: u16 = 0x7FFF;
    /// Peak amplitude of the PSG square wave and of the noise generator.
    const PSG_AMPLITUDE: i16 = 0x7FFF;

    /// Advances the PSG square wave by one step and latches the new sample.
    ///
    /// `SOUNDxCNT` bits 24-26 select the duty cycle: the wave is high for
    /// `wave_duty + 1` of the 8 steps, giving 12.5% .. 87.5% for values 0-6
    /// and a constant level for value 7.
    ///
    /// GBATEK "DS Sound Channels 0..15":
    /// <https://problemkaputt.de/gbatek.htm#dssoundchannels015>
    pub fn step_psg(&mut self) {
        self.psg_pos = (self.psg_pos + 1) & 0x7;
        self.sample = if self.psg_pos <= self.cnt.wave_duty {
            Channel::<T>::PSG_AMPLITUDE
        } else {
            -Channel::<T>::PSG_AMPLITUDE
        };
    }

    /// Advances the 15-bit noise LFSR by one step and latches the new sample.
    ///
    /// The register is shifted right each step; when the shifted-out bit was
    /// set it is XORed with 6000h and the output is negative, otherwise the
    /// output is positive.
    ///
    /// GBATEK "DS Sound Channels 0..15":
    /// <https://problemkaputt.de/gbatek.htm#dssoundchannels015>
    pub fn step_noise(&mut self) {
        let carry = self.noise_lfsr & 0x1 != 0;
        self.noise_lfsr >>= 1;
        if carry {
            self.noise_lfsr ^= 0x6000;
            self.sample = -Channel::<T>::PSG_AMPLITUDE;
        } else {
            self.sample = Channel::<T>::PSG_AMPLITUDE;
        }
    }

    pub fn new(spec: ChannelSpec) -> Self {
        Channel {
            // Registers
            cnt: ChannelControl::new(),
            src_addr: 0,
            timer_val: 0,
            loop_start: 0,
            len: 0,
            // Sound Generation
            spec,
            addr: 0,
            num_bytes_left: 0,
            sample: 0,
            // PSG / Noise
            psg_pos: 0,
            noise_lfsr: Channel::<T>::NOISE_LFSR_INIT,
            // ADPCM
            adpcm_in_header: true,
            adpcm_low_nibble: true,
            adpcm_index: 0,
            adpcm_value: 0,
            initial_adpcm_index: 0,
            initial_adpcm_value: 0,
        }
    }

    fn generate_sample(&self, sample: &mut (i32, i32)) {
        // TODO: Use volume and panning
        sample.0 += ((self.sample as i32) >> self.cnt.volume_shift())
            * self.cnt.volume_factor()
            * (128 - self.cnt.pan_factor());
        sample.1 += ((self.sample as i32) >> self.cnt.volume_shift())
            * self.cnt.volume_factor()
            * (self.cnt.pan_factor());
    }

    pub fn next_addr_pcm<M: super::MemoryValue>(&mut self) -> (u32, bool) {
        assert!(self.num_bytes_left > 0);
        let return_addr = self.addr;
        self.addr += std::mem::size_of::<M>() as u32;
        self.num_bytes_left -= std::mem::size_of::<M>();
        let reset = if self.num_bytes_left == 0 { self.handle_end() } else { false };
        (return_addr, reset)
    }

    /// Handles reaching the end of sample data according to the repeat mode
    /// (manual / loop-to-PNT / one-shot).
    ///
    /// GBATEK "SOUNDxCNT Bit 27-28 Repeat Mode":
    /// <https://problemkaputt.de/gbatek.htm#dssoundchannels015>
    fn handle_end(&mut self) -> bool {
        // TODO: Verify out timing of busy bit for other modes
        let (reset, new_busy) = match self.cnt.repeat_mode {
            RepeatMode::Manual => (true, true),
            RepeatMode::Loop => {
                self.addr = self.src_addr + self.loop_start as u32 * 4;
                self.adpcm_low_nibble = true;
                self.num_bytes_left = self.len as usize * 4;
                (false, true)
            }
            RepeatMode::OneShot => (true, false),
        };
        self.cnt.busy = new_busy;
        reset
    }

    pub fn reset_sample(&mut self) {
        self.sample = 0;
        self.cnt.busy = false;
    }

    pub fn set_sample<M: super::MemoryValue>(&mut self, sample: M) {
        let sample = num_traits::cast::<M, u16>(sample).unwrap();
        self.sample = if std::mem::size_of::<M>() == 1 { sample << 8 } else { sample } as i16;
    }

    pub fn initial_adpcm_addr(&mut self) -> Option<u32> {
        if self.adpcm_in_header {
            assert_eq!(self.src_addr, self.addr);
            self.adpcm_in_header = false;
            let return_addr = self.addr;
            self.addr += std::mem::size_of::<u32>() as u32;
            self.num_bytes_left -= std::mem::size_of::<u32>();
            Some(return_addr)
        } else {
            None
        }
    }

    pub fn next_addr_adpcm(&mut self) -> (u32, bool) {
        assert!(self.num_bytes_left > 0);
        let return_addr = self.addr;
        let reset = if self.adpcm_low_nibble {
            false
        } else {
            self.addr += 1;
            self.num_bytes_left -= 1;
            if self.num_bytes_left == 0 { self.handle_end() } else { false }
        };
        (return_addr, reset)
    }

    /// Decodes one 4-bit IMA-ADPCM nibble into the running sample value,
    /// following the reference decode algorithm (diff accumulation with
    /// saturation, index clamped to 0..88).
    ///
    /// GBATEK "DS Sound Notes – ADPCM pseudo-code":
    /// <https://problemkaputt.de/gbatek.htm#dssoundnotes>
    pub fn set_adpcm_data(&mut self, value: u8) {
        let data = if self.adpcm_low_nibble { value & 0xF } else { value >> 4 & 0xF };
        self.adpcm_low_nibble = !self.adpcm_low_nibble;
        let table_val = SPU::ADPCM_TABLE[self.adpcm_index as usize];
        let mut diff = table_val / 8;
        if data & 0x1 != 0 {
            diff += table_val / 4
        }
        if data & 0x2 != 0 {
            diff += table_val / 2
        }
        if data & 0x4 != 0 {
            diff += table_val
        }
        if data & 0x8 == 0 {
            self.adpcm_value = self.adpcm_value.saturating_add(diff as i16);
        } else {
            self.adpcm_value = self.adpcm_value.saturating_sub(diff as i16);
        }
        self.adpcm_index += SPU::ADPCM_INDEX_TABLE[data as usize & 0x7];
        self.adpcm_index = self.adpcm_index.clamp(0, 88);

        self.sample = self.adpcm_value;
    }

    pub fn set_initial_adpcm(&mut self, value: u32) {
        self.initial_adpcm_index = (value >> 16 & 0x7F).clamp(0, 88) as i32;
        self.initial_adpcm_value = value as u16 as i16;
        self.reset_adpcm();
    }

    pub fn reset_adpcm(&mut self) {
        self.adpcm_index = self.initial_adpcm_index;
        self.adpcm_value = self.initial_adpcm_value;
    }

    /// The most recently decoded sample of this channel.
    pub fn sample(&self) -> i16 {
        self.sample
    }

    /// Whether the channel is currently playing, for diagnostics.
    pub fn is_busy(&self) -> bool {
        self.cnt.busy
    }

    /// `(busy, latched sample, volume factor, pan factor, timer reload)` for
    /// the `mix` diagnostic probe.
    pub fn diag_state(&self) -> (bool, i16, i32, i32, u16) {
        (
            self.cnt.busy,
            self.sample,
            self.cnt.volume_factor(),
            self.cnt.pan_factor(),
            self.timer_val,
        )
    }

    pub fn format(&self) -> Format {
        self.cnt.format
    }

    pub fn schedule(&mut self, scheduler: &mut Scheduler, reset: bool) {
        // PSG / noise channels generate their samples procedurally, so they
        // have no sample data and games legitimately leave LEN and LOOPSTART
        // at zero; requiring a non-zero length there would never schedule them.
        let has_data = self.len + self.loop_start as u32 != 0 || self.cnt.format == Format::Special;
        if self.timer_val != 0 && has_data {
            if reset {
                scheduler.schedule(
                    Event::ResetAudioChannel(self.spec),
                    HW::reset_audio_channel,
                    // (-(self.timer_val as i16) as u16) as usize,
                    ((0x10000u32 - self.timer_val as u32) * 2) as usize,
                );
            } else {
                scheduler.schedule(
                    Event::StepAudioChannel(self.spec),
                    HW::step_audio_channel,
                    // (-(self.timer_val as i16) as u16) as usize,
                    ((0x10000u32 - self.timer_val as u32) * 2) as usize,
                );
            }
        }
    }
}

/// Sound capture unit 0/1 (SNDCAPxCNT 4000508h, SNDCAPxDAD/LEN 4000510h+).
///
/// Captures the mixer (or a channel) output back into ARM7-visible memory;
/// capture 0 pairs with channel 1, capture 1 with channel 3.
///
/// GBATEK "DS Sound Capture":
/// <https://problemkaputt.de/gbatek.htm#dssoundcapture>
#[derive(emu_utils::Savestate)]
struct Capture {
    // Registers
    cnt: CaptureControl,
    dest_addr: u32,
    len: usize,
    // Sound Capturing
    addr: u32,
    num_bytes_left: usize,
}

impl Capture {
    pub fn new() -> Self {
        Capture {
            // Registers
            cnt: CaptureControl::new(),
            dest_addr: 0,
            len: 0,
            // Sound Capturing
            addr: 0,
            num_bytes_left: 0,
        }
    }

    /// Returns the next destination address and advances the capture cursor.
    ///
    /// When the buffer is full the unit either wraps back to `SNDCAPxDAD` and
    /// keeps running (repeat mode) or stops and clears its busy flag
    /// (SNDCAPxCNT bit 2 = "one-shot").
    ///
    /// GBATEK "DS Sound Capture": <https://problemkaputt.de/gbatek.htm#dssoundcapture>
    pub fn next_addr<T: super::MemoryValue>(&mut self) -> u32 {
        assert!(self.num_bytes_left > 0);
        let return_addr = self.addr;
        self.num_bytes_left -= std::mem::size_of::<T>();
        self.addr += std::mem::size_of::<T>() as u32;
        if self.num_bytes_left == 0 {
            if self.cnt.no_repeat {
                self.cnt.busy = false;
            } else {
                self.num_bytes_left = self.len * 4;
                self.addr = self.dest_addr;
            }
        }
        return_addr
    }

    pub fn read(&self, byte: usize) -> u8 {
        let shift = (byte & 0x3) * 8;
        match byte {
            0x0..=0x3 => (self.addr >> shift) as u8,
            0x4..=0x7 => {
                warn!("Reading from Write-Only Sound Capture Register: Dest Addr");
                0
            }
            _ => unreachable!(),
        }
    }

    pub fn write_cnt(&mut self, value: u8) {
        let prev_busy = self.cnt.busy;
        self.cnt.write(value);
        if !prev_busy && self.cnt.busy {
            self.num_bytes_left = self.len * 4;
            self.addr = self.dest_addr;
        }
    }

    pub fn write(&mut self, byte: usize, value: u8) {
        let shift = (byte & 0x3) * 8;
        let mask = 0xFF << shift;
        let value = (value as u32) << shift;
        match byte {
            0x0..=0x3 => {
                // SNDCAPxDAD is assembled from four byte writes: the previous
                // value has to be preserved outside the written byte, otherwise
                // every byte write resets the destination to that byte alone
                // and the capture lands at a garbage address.
                self.dest_addr = (self.dest_addr & !mask | value) & 0x7FF_FFFF;
                self.addr = self.dest_addr;
            }
            0x4..=0x7 => {
                self.len = (self.len & !(mask as usize) | (value as usize)) as u16 as usize;
                self.num_bytes_left = self.len * 4;
            }
            _ => unreachable!(),
        }
    }
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChannelSpec {
    Base(usize),
    PSG(usize),
    Noise(usize),
}

#[expect(unused)]
pub trait ChannelType {
    fn supports_psg() -> bool;
    fn supports_noise() -> bool;
}
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy)]
pub struct BaseChannel {}
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy)]
pub struct PSGChannel {}
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy)]
pub struct NoiseChannel {}

impl ChannelType for BaseChannel {
    fn supports_psg() -> bool {
        false
    }
    fn supports_noise() -> bool {
        false
    }
}

impl ChannelType for PSGChannel {
    fn supports_psg() -> bool {
        true
    }
    fn supports_noise() -> bool {
        false
    }
}

impl ChannelType for NoiseChannel {
    fn supports_psg() -> bool {
        false
    }
    fn supports_noise() -> bool {
        true
    }
}
