/// Sound Processing Unit (SPU) implementation for Nintendo DS
/// Manages 16 audio channels with mixing and capture capabilities

/// Audio channel control register
#[derive(Debug, Clone, Copy)]
pub struct ChannelCntReg {
    /// Volume level (0-127)
    pub volume: u32,
    /// Frequency divider
    pub divider: u32,
    /// Hold last sample when stopped
    pub hold_sample: bool,
    /// Panning (0=left, 64=center, 127=right)
    pub panning: u32,
    /// Wave duty cycle for generators
    pub wave_duty: u32,
    /// Repeat mode (0=manual, 1=loop, 3=one-shot)
    pub repeat_mode: u32,
    /// Audio format (0=PCM8, 1=PCM16, 2=ADPCM, 3=PSG)
    pub format: u32,
    /// Channel is currently playing
    pub busy: bool,
}

impl ChannelCntReg {
    /// Create new channel control register
    pub fn new() -> Self {
        ChannelCntReg {
            volume: 0,
            divider: 0,
            hold_sample: false,
            panning: 64,
            wave_duty: 0,
            repeat_mode: 0,
            format: 0,
            busy: false,
        }
    }
}

impl Default for ChannelCntReg {
    fn default() -> Self {
        Self::new()
    }
}

/// Individual sound channel
#[derive(Debug, Clone, Copy)]
pub struct SoundChannel {
    /// Channel control register
    pub channel_cnt: ChannelCntReg,
    /// Source address in memory
    pub sound_source: u32,
    /// Playback timer/frequency
    pub sound_timer: u16,
    /// Current playback position
    pub sound_pnt: u16,
    /// Length of audio data
    pub sound_len: u16,
}

impl SoundChannel {
    /// Create new sound channel
    pub fn new() -> Self {
        SoundChannel {
            channel_cnt: ChannelCntReg::new(),
            sound_source: 0,
            sound_timer: 0,
            sound_pnt: 0,
            sound_len: 0,
        }
    }
}

impl Default for SoundChannel {
    fn default() -> Self {
        Self::new()
    }
}

/// Main sound control register
#[derive(Debug, Clone, Copy)]
pub struct SoundCntReg {
    /// Master volume (0-127)
    pub master_volume: u32,
    /// Left output volume
    pub left_output: u32,
    /// Right output volume
    pub right_output: u32,
    /// Mix channel 1 to output
    pub output_ch1_mixer: bool,
    /// Mix channel 3 to output
    pub output_ch3_mixer: bool,
    /// Master audio enable
    pub master_enable: bool,
}

impl SoundCntReg {
    /// Create new sound control register
    pub fn new() -> Self {
        SoundCntReg {
            master_volume: 0,
            left_output: 0,
            right_output: 0,
            output_ch1_mixer: false,
            output_ch3_mixer: false,
            master_enable: false,
        }
    }

    /// Get register value as 16-bit halfword
    pub fn get(&self) -> u16 {
        let mut value = 0u16;
        value |= ((self.master_volume & 0x7F) as u16);
        value |= ((self.left_output & 0x3) as u16) << 8;
        value |= ((self.right_output & 0x3) as u16) << 10;
        if self.output_ch1_mixer {
            value |= 1 << 12;
        }
        if self.output_ch3_mixer {
            value |= 1 << 13;
        }
        if self.master_enable {
            value |= 1 << 15;
        }
        value
    }

    /// Set register value from 16-bit halfword
    pub fn set(&mut self, value: u16) {
        self.master_volume = (value & 0x7F) as u32;
        self.left_output = ((value >> 8) & 0x3) as u32;
        self.right_output = ((value >> 10) & 0x3) as u32;
        self.output_ch1_mixer = (value & (1 << 12)) != 0;
        self.output_ch3_mixer = (value & (1 << 13)) != 0;
        self.master_enable = (value & (1 << 15)) != 0;
    }
}

impl Default for SoundCntReg {
    fn default() -> Self {
        Self::new()
    }
}

/// Audio capture settings
#[derive(Debug, Clone, Copy)]
pub struct SndCapture {
    /// Add channel to capture
    pub add_to_channel: bool,
    /// Capture source (0=mixer, 1=channel)
    pub capture_source: bool,
    /// One-shot mode (not repeating)
    pub one_shot: bool,
    /// Capture format is PCM8 (vs PCM16)
    pub capture_pcm8: bool,
    /// Capture is active
    pub busy: bool,
    /// Destination address in memory
    pub destination: u32,
    /// Capture length
    pub len: u16,
}

impl SndCapture {
    /// Create new capture settings
    pub fn new() -> Self {
        SndCapture {
            add_to_channel: false,
            capture_source: false,
            one_shot: false,
            capture_pcm8: false,
            busy: false,
            destination: 0,
            len: 0,
        }
    }

    /// Get capture register value
    pub fn get(&self) -> u8 {
        let mut value = 0u8;
        if self.add_to_channel {
            value |= 1;
        }
        if self.capture_source {
            value |= 2;
        }
        if self.one_shot {
            value |= 4;
        }
        if self.capture_pcm8 {
            value |= 8;
        }
        if self.busy {
            value |= 0x80;
        }
        value
    }

    /// Set capture register value
    pub fn set(&mut self, value: u8) {
        self.add_to_channel = (value & 1) != 0;
        self.capture_source = (value & 2) != 0;
        self.one_shot = (value & 4) != 0;
        self.capture_pcm8 = (value & 8) != 0;
        self.busy = (value & 0x80) != 0;
    }
}

impl Default for SndCapture {
    fn default() -> Self {
        Self::new()
    }
}

/// Sound Processing Unit
/// Manages 16 audio channels with mixing and PCM/ADPCM playback
pub struct SPU {
    /// 16 audio channels
    channels: [SoundChannel; 16],

    /// Main sound control register
    soundcnt: SoundCntReg,

    /// Capture 0 settings
    sndcap0: SndCapture,
    /// Capture 1 settings
    sndcap1: SndCapture,

    /// Sound bias (sample offset)
    soundbias: u16,
}

impl SPU {
    /// Create new SPU instance
    pub fn new() -> Self {
        let mut channels = [SoundChannel::new(); 16];
        // Initialize all channels
        for ch in &mut channels {
            *ch = SoundChannel::new();
        }

        SPU {
            channels,
            soundcnt: SoundCntReg::new(),
            sndcap0: SndCapture::new(),
            sndcap1: SndCapture::new(),
            soundbias: 0,
        }
    }

    /// Power on SPU and initialize
    pub fn power_on(&mut self) -> Result<(), String> {
        self.soundcnt.master_enable = true;
        self.soundbias = 0x200; // Default bias value
        Ok(())
    }

    /// Read from channel at address
    pub fn read_channel_byte(&self, address: u32) -> u8 {
        let channel_idx = ((address >> 4) & 0xF) as usize;
        let offset = (address & 0xF) as usize;

        if channel_idx >= 16 {
            return 0;
        }

        match offset {
            0..=3 => ((self.channels[channel_idx].channel_cnt.volume >> (offset * 8)) & 0xFF) as u8,
            4..=7 => ((self.channels[channel_idx].sound_source >> ((offset - 4) * 8)) & 0xFF) as u8,
            8..=9 => ((self.channels[channel_idx].sound_timer >> ((offset - 8) * 8)) & 0xFF) as u8,
            10..=11 => ((self.channels[channel_idx].sound_pnt >> ((offset - 10) * 8)) & 0xFF) as u8,
            12..=13 => ((self.channels[channel_idx].sound_len >> ((offset - 12) * 8)) & 0xFF) as u8,
            _ => 0,
        }
    }

    /// Write byte to channel at address
    pub fn write_channel_byte(&mut self, address: u32, byte: u8) {
        let channel_idx = ((address >> 4) & 0xF) as usize;
        let offset = (address & 0xF) as usize;

        if channel_idx >= 16 {
            return;
        }

        match offset {
            0..=3 => {
                let shift = (offset * 8) as u32;
                self.channels[channel_idx].channel_cnt.volume &= !(0xFF << shift);
                self.channels[channel_idx].channel_cnt.volume |= ((byte as u32) << shift);
            }
            _ => {}
        }
    }

    /// Write halfword to channel
    pub fn write_channel_halfword(&mut self, address: u32, halfword: u16) {
        let channel_idx = ((address >> 4) & 0xF) as usize;
        let offset = (address & 0xF) as usize;

        if channel_idx >= 16 {
            return;
        }

        match offset {
            0..=1 => {
                self.channels[channel_idx].channel_cnt.volume = (halfword & 0x7F) as u32;
            }
            4..=5 => {
                self.channels[channel_idx].sound_source &= 0xFFFF0000;
                self.channels[channel_idx].sound_source |= halfword as u32;
            }
            8..=9 => {
                self.channels[channel_idx].sound_timer = halfword;
            }
            10..=11 => {
                self.channels[channel_idx].sound_pnt = halfword;
            }
            12..=13 => {
                self.channels[channel_idx].sound_len = halfword;
            }
            _ => {}
        }
    }

    /// Write word to channel
    pub fn write_channel_word(&mut self, address: u32, word: u32) {
        let channel_idx = ((address >> 4) & 0xF) as usize;

        if channel_idx >= 16 {
            return;
        }

        match address & 0xF {
            0 => {
                self.channels[channel_idx].channel_cnt.volume = (word & 0x7F) as u32;
            }
            4 => {
                self.channels[channel_idx].sound_source = word;
            }
            8 => {
                self.channels[channel_idx].sound_timer = (word & 0xFFFF) as u16;
            }
            12 => {
                self.channels[channel_idx].sound_len = (word & 0xFFFF) as u16;
            }
            _ => {}
        }
    }

    /// Get SOUNDCNT register value
    pub fn get_soundcnt(&self) -> u16 {
        self.soundcnt.get()
    }

    /// Get SOUNDBIAS register value
    pub fn get_soundbias(&self) -> u16 {
        self.soundbias
    }

    /// Get SNDCAP0 register value
    pub fn get_sndcap0(&self) -> u8 {
        self.sndcap0.get()
    }

    /// Get SNDCAP1 register value
    pub fn get_sndcap1(&self) -> u8 {
        self.sndcap1.get()
    }

    /// Set SOUNDCNT low byte
    pub fn set_soundcnt_lo(&mut self, byte: u8) {
        self.soundcnt.master_volume = (byte & 0x7F) as u32;
    }

    /// Set SOUNDCNT high byte
    pub fn set_soundcnt_hi(&mut self, byte: u8) {
        self.soundcnt.left_output = ((byte >> 0) & 0x3) as u32;
        self.soundcnt.right_output = ((byte >> 2) & 0x3) as u32;
        self.soundcnt.output_ch1_mixer = (byte & (1 << 4)) != 0;
        self.soundcnt.output_ch3_mixer = (byte & (1 << 5)) != 0;
        self.soundcnt.master_enable = (byte & (1 << 7)) != 0;
    }

    /// Set SOUNDCNT register (16-bit)
    pub fn set_soundcnt(&mut self, halfword: u16) {
        self.soundcnt.set(halfword);
    }

    /// Set SOUNDBIAS register
    pub fn set_soundbias(&mut self, value: u16) {
        self.soundbias = value;
    }

    /// Set SNDCAP0 register
    pub fn set_sndcap0(&mut self, value: u8) {
        self.sndcap0.set(value);
    }

    /// Set SNDCAP1 register
    pub fn set_sndcap1(&mut self, value: u8) {
        self.sndcap1.set(value);
    }

    /// Get channel reference
    pub fn get_channel(&self, index: usize) -> Option<&SoundChannel> {
        if index < 16 {
            Some(&self.channels[index])
        } else {
            None
        }
    }

    /// Get mutable channel reference
    pub fn get_channel_mut(&mut self, index: usize) -> Option<&mut SoundChannel> {
        if index < 16 {
            Some(&mut self.channels[index])
        } else {
            None
        }
    }
}

impl Default for SPU {
    fn default() -> Self {
        Self::new()
    }
}
