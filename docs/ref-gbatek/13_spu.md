# 13. The Sound Processing Unit

Sixteen hardware channels, mixed to one stereo pair, driven entirely from the
scheduler. Audio is also the one subsystem where the emulator has to meet the
_host_ on the host's terms, which shapes several design decisions here.

GBATEK references:
[Sound overview](https://problemkaputt.de/gbatek.htm#dssound) ·
[Channels 0-15](https://problemkaputt.de/gbatek.htm#dssoundchannels015) ·
[Control registers](https://problemkaputt.de/gbatek.htm#dssoundcontrolregisters) ·
[Sound capture](https://problemkaputt.de/gbatek.htm#dssoundcapture) ·
[Sound notes (ADPCM tables)](https://problemkaputt.de/gbatek.htm#dssoundnotes)

The SPU belongs to the **ARM7**. All sample fetches go through `arm7_read`.

---

## 13.1 Sixteen channels, three kinds

```text
   channel   type in Lunaris             formats available
   ───────   ─────────────────────────   ──────────────────────────────
    0 -  7   Channel<BaseChannel>        PCM8, PCM16, IMA-ADPCM
    8 - 13   Channel<PSGChannel>         + square wave, 6 duty cycles
   14 - 15   Channel<NoiseChannel>       + LFSR noise

   Registers per channel at 4000400h + N×10h:
     SOUNDxCNT  volume, pan, format, repeat mode, enable
     SOUNDxSAD  source address in ARM7-visible memory
     SOUNDxTMR  timer reload — this IS the pitch
     SOUNDxPNT  loop start point
     SOUNDxLEN  length
```

The kind is a type parameter, so PSG state does not exist on channels 0-7
([spu.rs:44-63](core/src/hw/spu.rs#L44-L63)):

```rust
pub struct SPU {
    cnt: SoundControl,
    sound_bias: u16,
    captures: [Capture; 2],
    // Sound Generation
    #[savestate(skip)]
    audio: Audio,
    clocks_per_sample: usize,
    // ...
    pub base_channels: [Channel<BaseChannel>; 8],
    pub psg_channels: [Channel<PSGChannel>; 6],
    pub noise_channels: [Channel<NoiseChannel>; 2],
}
```

---

## 13.2 Two clocks, two events

This is the key structural idea, and it is worth internalising before reading
the code:

```text
   ┌───────────────────────────────────────────────────────────────────┐
   │ PER-CHANNEL CLOCK — Event::StepAudioChannel(spec)                 │
   │                                                                   │
   │   period = (0x10000 − SOUNDxTMR) × 2  master cycles               │
   │   fires:  fetch the next sample byte/word from memory             │
   │   rate:   different for every channel; that IS the pitch          │
   └───────────────────────────────────────────────────────────────────┘

   ┌───────────────────────────────────────────────────────────────────┐
   │ OUTPUT CLOCK — Event::GenerateAudioSample                         │
   │                                                                   │
   │   period = clocks_per_sample                                      │
   │   fires:  mix all 16 channels' CURRENT samples, push to host      │
   │   rate:   one, fixed, matching the host audio device              │
   └───────────────────────────────────────────────────────────────────┘

   channel 0 ──●────●────●────●────●────●────●──►   (high pitch)
   channel 1 ──●─────────●─────────●──────────►    (low pitch)
   channel 2 ──●──●──●──●──●──●──●──●──●──●───►
                 │     │     │     │     │
   output    ────▼─────▼─────▼─────▼─────▼─────►   (fixed rate; samples
                                                    whatever each channel
                                                    currently holds)
```

Channel period ([spu.rs:876-899](core/src/hw/spu.rs#L876-L899)):

```rust
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
                    ((0x10000u32 - self.timer_val as u32) * 2) as usize,
                );
            } else {
                scheduler.schedule(
                    Event::StepAudioChannel(self.spec),
                    HW::step_audio_channel,
                    ((0x10000u32 - self.timer_val as u32) * 2) as usize,
                );
            }
        }
    }
```

Output period ([spu.rs:97-105](core/src/hw/spu.rs#L97-L105)):

```rust
    pub fn new(scheduler: &mut Scheduler) -> Self {
        let audio = Audio::new();
        // TODO: Sample at 32.768 kHz and resample to device sample rate
        let clocks_per_sample = crate::nds::NDS::CLOCK_RATE / audio.sample_rate();
        scheduler.schedule(
            Event::GenerateAudioSample,
            HW::generate_audio_sample,
            clocks_per_sample,
        );
```

> **Divergence:** the DS SPU natively runs at 32728 Hz (master clock ÷ 1024)
> and the console resamples in hardware. Lunaris instead derives its sample
> period from the _host_ device rate, which means the emulated output rate
> changes with the audio device. The TODO tracks the missing native path.
> melonDS mixes at 32823 Hz in `src/SPU.cpp` and resamples afterwards.

---

## 13.3 Fetching samples

`step_audio_channel` reads through the ARM7 bus — sample data lives in main RAM
or ARM7 WRAM, not in a dedicated sound memory
([spu.rs:396-437](core/src/hw/spu.rs#L396-L437)):

```rust
            ChannelSpec::Base(num) => {
                let format = self.spu.base_channels[num].format();
                match format {
                    Format::PCM8 => {
                        let (addr, reset) = self.spu.base_channels[num].next_addr_pcm::<u8>();
                        self.spu.base_channels[num].schedule(&mut self.scheduler, reset);
                        let sample = self.arm7_read::<u8>(addr);
                        self.spu.base_channels[num].set_sample(sample);
                    }
                    Format::PCM16 => { /* same with u16 */ }
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
```

Note that `schedule` is called **before** the read, so the next event is queued
regardless of what the fetch returns.

Format 3 on a base channel is meaningless, and the handling is another instance
of the project's rule ([spu.rs:433-437](core/src/hw/spu.rs#L433-L437)):

```rust
                    // Format 3 has no meaning on channels 0-7 (PSG/noise start
                    // at channel 8). Hardware simply produces nothing rather
                    // than faulting, so keep the channel silent instead of
                    // panicking on a stray write.
                    Format::Special => self.spu.base_channels[num].reset_sample(),
```

### IMA-ADPCM

```text
   ADPCM block layout in memory

   ┌──────────────────────────────────┐
   │ header word (32 bits)            │  ← read once, via initial_adpcm_addr
   │  bits 0..15  initial sample      │
   │  bits 16..22 initial table index │
   └──────────────────────────────────┘
   ┌────┬────┬────┬────┬────┬────┬────┐
   │ n1 │ n0 │ n3 │ n2 │ …             │  ← 4 bits per sample, low nibble first
   └────┴────┴────┴────┴────┴────┴────┘

   decode step:
     step  = ADPCM_TABLE[index]
     diff  = step/8 + (n&4)*step/1 + (n&2)*step/2 + (n&1)*step/4
     sample += (n&8) ? −diff : +diff        (clamped to i16)
     index  = clamp(index + ADPCM_INDEX_TABLE[n&7], 0, 88)
```

Both tables are transcribed verbatim from GBATEK
([spu.rs:73-91](core/src/hw/spu.rs#L73-L91)):

```rust
    /// IMA-ADPCM index adjustment table (4-bit nibble high-3 bits → index delta).
    pub const ADPCM_INDEX_TABLE: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];
    /// IMA-ADPCM step-size table (89 entries, index 0–88).
    pub const ADPCM_TABLE: [u16; 89] = [
        0x0007, 0x0008, 0x0009, 0x000A, 0x000B, 0x000C, 0x000D, 0x000E, 0x0010, 0x0011, 0x0013,
        // ... 89 entries ...
        0x7FFF,
    ];
```

ADPCM is stateful, which is why it cannot be seeked: a looping ADPCM sample
must restore the sample/index pair captured at the loop point, not re-read the
header. That is what `reset_adpcm` and `Event::ResetAudioChannel` exist for.

---

## 13.4 Mixing

Channels 1 and 3 are special: they can be routed to the output _instead of_ the
mixer, so they are accumulated separately
([spu.rs:122-145](core/src/hw/spu.rs#L122-L145)):

```rust
    fn generate_mixer(&self) -> ((i32, i32), (i32, i32), (i32, i32)) {
        let mut mixer = (0, 0);
        for i in (0..1).chain(2..3).chain(4..self.base_channels.len()) {
            self.base_channels[i].generate_sample(&mut mixer)
        }
        // ... psg and noise channels into mixer ...
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
```

Note `(0..1).chain(2..3).chain(4..)` — that is "every base channel except 1 and
3", spelled with iterators.

```text
   SOUNDCNT output routing

   ┌────────────────┐
   │ channels 0,2,4-15 ─────────────┐
   │ channel 1  ──┬─(output_1?)────►├─► mixer ──┐
   │ channel 3  ──┼─(output_3?)────►┘           │
   └──────────────┼─────────────────────────────┼──┐
                  │                             │  │
                  └── ch1 ────────┐             │  │
                  └── ch3 ────────┤             │  │
                                  ▼             ▼  ▼
                          left_output  ∈ { Mixer, Ch1, Ch3, Ch1+Ch3 }
                          right_output ∈ { Mixer, Ch1, Ch3, Ch1+Ch3 }
                                  │
                                  ▼  × master_volume >> 7
                                  ▼  saturating cast to i16
                              host ring buffer
```

The fixed-point detail carries a fixed bug
([spu.rs:155-190](core/src/hw/spu.rs#L155-L190)):

```rust
        // Each channel contributes `sample * volume_factor * pan_factor`, i.e.
        // two 0..128 factors, so the mixer accumulator carries 14 fractional
        // bits. Shifting by 16 here attenuated the whole mix by a further
        // factor of four; and the final cast has to saturate, since a loud mix
        // otherwise wraps around and turns into full-scale noise.
        const MIXER_FRAC_BITS: u8 = 14;
```

Two failure modes in one comment: shift too far and everything is quiet; fail
to saturate and a loud passage becomes white noise. Both are audible
immediately, and both are easy to introduce.

Master-enable produces _silence_, not a stopped event
([spu.rs:156-163](core/src/hw/spu.rs#L156-L163)):

```rust
        // SOUNDCNT bit 15 is the master enable: with it clear the SPU outputs
        // silence. The scheduler event keeps running so the host audio ring
        // buffer stays fed at a constant rate.
        if !self.cnt.enable {
            self.audio.push_sample(cpal::Sample::from::<i16>(&0), cpal::Sample::from::<i16>(&0));
            return;
        }
```

Starving the ring buffer instead would cause underrun crackle every time a game
muted its audio.

---

## 13.5 Sound capture

Two capture units write the mixer output _back into memory_, clocked by
channels 1 and 3 ([spu.rs:901-960](core/src/hw/spu.rs#L901-L960)):

```text
   ┌─────────┐                          ┌─────────────────┐
   │ mixer   │ ──► capture unit 0 ─────► │ ARM7 memory     │
   └─────────┘     (clocked by ch 1)     │ SNDCAP0DAD..LEN │
                                         └─────────────────┘
                                                  │
                        a channel can then PLAY that buffer back
                                                  │
                                                  ▼
                                     echo / reverb / mic effects
```

This is the DS's only "DSP-ish" feature, and it is how games implement echo
without a DSP.

---

## 13.6 Meeting the host clock

The frontend can run the emulator faster or slower than real time. The SPU has
to be told, because the audio ring buffer is what would otherwise pin it
([nds.rs:80-87](core/src/nds.rs#L80-L87)):

```rust
    /// Enables/disables pacing emulation to the host audio clock.
    ///
    /// Must be disabled whenever the frontend drives emulation at a speed
    /// other than 1.0x, otherwise the SPU blocks on a full output ring buffer
    /// and pins the emulator to real time. See `Audio::blocking`.
    #[inline]
    pub fn set_audio_sync(&mut self, sync: bool) {
        self.hw.set_audio_sync(sync);
    }
```

```text
   audio_sync = true  (normal play)
   ─────────────────────────────────
   emulator produces samples ──► ring buffer ──► host device
                              ▲                    │
                              └── blocks when full ┘
   Result: the host audio clock paces emulation at exactly 1.0×,
           which is more stable than pacing on a 59.8262 Hz timer.

   audio_sync = false (fast-forward, frame advance, rewind)
   ─────────────────────────────────
   emulator produces samples ──► ring buffer ──► host device
                                   (drops excess, never blocks)
```

Pacing on audio rather than on a wall-clock timer is the standard trick: a
frame-timer-paced emulator drifts against the sound card and produces periodic
clicks, whereas an audio-paced one is glitch-free by construction.

`Audio` itself is `#[savestate(skip)]` — a host device handle has no meaning in
a savestate ([spu.rs:49-51](core/src/hw/spu.rs#L49-L51)).

---

## 13.7 The `mix` diagnostic

"No audio" has many possible causes; this probe splits the space in half
([spu.rs:194-200](core/src/hw/spu.rs#L194-L200)):

```rust
    /// Diagnostic `mix`: reports the peak absolute amplitude of the mixed
    /// output roughly once per second of emulated audio, so "no audio" can be
    /// pinned to either the mixer or the host output path.
    fn log_output_level(&mut self, sample: (i16, i16)) {
```

```text
   peak == 0        →  the mixer produced nothing:
                       check channel enable, SOUNDCNT, sample fetch
   peak >> 0 but
   nothing audible  →  the host path is at fault:
                       device, volume, ring buffer, format
```

---

## 13.8 Divergences

- **Sample rate** is host-derived rather than the hardware's 32728 Hz (§13.2).
- **No SPU FIFO.** Real hardware prefetches samples into a small per-channel
  FIFO; Lunaris reads one sample per step event
  (`// TODO: Use SPU FIFO`, [spu.rs:403](core/src/hw/spu.rs#L403)).
- **SOUNDBIAS** is stored but not applied to the output.
- **No channel hold / volume-divider edge cases** beyond what `generate_sample`
  implements.
- **No sound-hardware access timing**: sample fetches do not steal bus cycles.

---

[← 12. VRAM Banking and Display Output](12_vram_and_display.md) | [Next: 14. The Cartridge and Boot →](14_cartridge_and_boot.md)
