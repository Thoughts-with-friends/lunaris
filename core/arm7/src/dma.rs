/// Direct Memory Access (DMA) controller for Nintendo DS
/// Manages high-speed memory transfers between memory regions

/// DMA control register
#[derive(Debug, Clone, Copy)]
pub struct DmaCnt {
    /// Destination address control (0=increment, 1=decrement, 2=fixed, 3=reload)
    pub dest_control: u32,
    /// Source address control (0=increment, 1=decrement, 2=fixed)
    pub source_control: u32,
    /// Repeat the transfer when complete
    pub repeat: bool,
    /// Use 32-bit transfers instead of 16-bit
    pub word_transfer: bool,
    /// Start timing (0=immediate, 1=VBLANK, 2=HBLANK, 3=sync/special)
    pub timing: u32,
    /// Generate interrupt when transfer completes
    pub irq_after_transfer: bool,
    /// Enable this DMA channel
    pub enabled: bool,
}

impl DmaCnt {
    /// Create new DMA control register
    pub fn new() -> Self {
        DmaCnt {
            dest_control: 0,
            source_control: 0,
            repeat: false,
            word_transfer: false,
            timing: 0,
            irq_after_transfer: false,
            enabled: false,
        }
    }

    /// Get register value as 16-bit halfword
    pub fn get(&self) -> u16 {
        let mut value = 0u16;
        value |= ((self.dest_control & 0x3) as u16) << 5;
        value |= ((self.source_control & 0x3) as u16) << 7;
        if self.repeat {
            value |= 1 << 9;
        }
        if self.word_transfer {
            value |= 1 << 10;
        }
        value |= ((self.timing & 0x3) as u16) << 11;
        if self.irq_after_transfer {
            value |= 1 << 14;
        }
        if self.enabled {
            value |= 1 << 15;
        }
        value
    }

    /// Set register value from 16-bit halfword
    pub fn set(&mut self, value: u16) {
        self.dest_control = ((value >> 5) & 0x3) as u32;
        self.source_control = ((value >> 7) & 0x3) as u32;
        self.repeat = (value & (1 << 9)) != 0;
        self.word_transfer = (value & (1 << 10)) != 0;
        self.timing = ((value >> 11) & 0x3) as u32;
        self.irq_after_transfer = (value & (1 << 14)) != 0;
        self.enabled = (value & (1 << 15)) != 0;
    }
}

impl Default for DmaCnt {
    fn default() -> Self {
        Self::new()
    }
}

/// Individual DMA channel state
#[derive(Debug, Clone, Copy)]
pub struct DmaChannel {
    /// Source address (user-controlled)
    pub source: u32,
    /// Source address (internal working copy)
    pub internal_source: u32,

    /// Destination address (user-controlled)
    pub destination: u32,
    /// Destination address (internal working copy)
    pub internal_dest: u32,

    /// Transfer length in units (user-controlled)
    pub length: u32,
    /// Transfer length (internal working copy)
    pub internal_len: u32,

    /// DMA control register
    pub cnt: DmaCnt,

    /// Channel index (0-7)
    pub index: u32,

    /// Is this ARM9 DMA (vs ARM7)
    pub is_arm9: bool,
}

impl DmaChannel {
    /// Create new DMA channel
    pub fn new(index: u32, is_arm9: bool) -> Self {
        DmaChannel {
            source: 0,
            internal_source: 0,
            destination: 0,
            internal_dest: 0,
            length: 0,
            internal_len: 0,
            cnt: DmaCnt::new(),
            index,
            is_arm9,
        }
    }
}

impl Default for DmaChannel {
    fn default() -> Self {
        Self::new(0, false)
    }
}

/// DMA Controller
/// Manages up to 8 DMA channels (4 ARM9, 4 ARM7) for fast memory transfers
pub struct NDS_DMA {
    /// Emulator reference
    emulator_ptr: *mut crate::emulator::Emulator,

    /// DMA channels (0-7: 0-3 are ARM9, 4-7 are ARM7)
    dmas: [DmaChannel; 8],

    /// Currently active ARM7 DMA channel
    active_dma7: Option<u32>,
    /// Currently active ARM9 DMA channel
    active_dma9: Option<u32>,

    /// Bitmask of which DMA channels are active
    active_dmas: u8,
}

impl NDS_DMA {
    /// Create new DMA controller
    pub fn new() -> Self {
        let mut dmas = [DmaChannel::new(0, false); 8];

        // Initialize channels
        for i in 0..8 {
            let is_arm9 = i < 4;
            dmas[i] = DmaChannel::new(i as u32, is_arm9);
        }

        NDS_DMA {
            emulator_ptr: std::ptr::null_mut(),
            dmas,
            active_dma7: None,
            active_dma9: None,
            active_dmas: 0,
        }
    }

    /// Power on DMA controller
    pub fn power_on(&mut self) -> Result<(), String> {
        self.active_dmas = 0;
        self.active_dma7 = None;
        self.active_dma9 = None;
        Ok(())
    }

    /// Process DMA event
    pub fn dma_event(&mut self, index: u32) -> Result<(), String> {
        if index >= 8 {
            return Err("Invalid DMA index".to_string());
        }

        // Check if this channel is active
        if self.dmas[index as usize].cnt.enabled {
            // Perform the transfer
            self.update_dma(index)?;
        }

        Ok(())
    }

    /// Update and process DMA transfer
    pub fn update_dma(&mut self, index: u32) -> Result<(), String> {
        if index >= 8 {
            return Err("Invalid DMA index".to_string());
        }

        let channel = &mut self.dmas[index as usize];

        // Check if transfer should start
        if !channel.cnt.enabled {
            return Ok(());
        }

        // Set up internal working copies if starting fresh
        if channel.internal_len == 0 {
            channel.internal_source = channel.source;
            channel.internal_dest = channel.destination;
            channel.internal_len = channel.length;
        }

        // Process transfer
        while channel.internal_len > 0 {
            // Perform actual data transfer here
            // For now, just decrement the counter
            channel.internal_len -= 1;

            // Update addresses based on control settings
            match channel.cnt.source_control {
                0 => {
                    channel.internal_source = channel
                        .internal_source
                        .wrapping_add(if channel.cnt.word_transfer { 4 } else { 2 })
                }
                1 => {
                    channel.internal_source = channel
                        .internal_source
                        .wrapping_sub(if channel.cnt.word_transfer { 4 } else { 2 })
                }
                2 => {} // Fixed
                _ => {}
            }

            match channel.cnt.dest_control {
                0 => {
                    channel.internal_dest = channel
                        .internal_dest
                        .wrapping_add(if channel.cnt.word_transfer { 4 } else { 2 })
                }
                1 => {
                    channel.internal_dest = channel
                        .internal_dest
                        .wrapping_sub(if channel.cnt.word_transfer { 4 } else { 2 })
                }
                2 => {} // Fixed
                3 => {} // Reload (reloads on repeat)
                _ => {}
            }
        }

        // Check for repeat or completion
        if !channel.cnt.repeat {
            channel.cnt.enabled = false;
            self.active_dmas &= !(1 << index);

            // Trigger interrupt if requested
            if channel.cnt.irq_after_transfer {
                // Request interrupt
            }
        } else {
            // Reload for next transfer
            if channel.cnt.dest_control == 3 {
                channel.internal_dest = channel.destination;
            }
            channel.internal_len = channel.length;
        }

        Ok(())
    }

    /// Handle scheduler event
    pub fn handle_event(&mut self, _event: &crate::emulator::SchedulerEvent) -> Result<(), String> {
        // Process timing-based DMA triggers
        Ok(())
    }

    /// Check if any DMA channel is active
    pub fn is_active(&self) -> bool {
        self.active_dmas != 0
    }

    /// Read source address of DMA channel
    pub fn read_source(&self, index: usize) -> u32 {
        if index < 8 {
            self.dmas[index].source
        } else {
            0
        }
    }

    /// Read transfer length of DMA channel
    pub fn read_len(&self, index: usize) -> u16 {
        if index < 8 {
            (self.dmas[index].length & 0xFFFF) as u16
        } else {
            0
        }
    }

    /// Read control register of DMA channel
    pub fn read_cnt(&self, index: usize) -> u16 {
        if index < 8 {
            self.dmas[index].cnt.get()
        } else {
            0
        }
    }

    /// Write source address to DMA channel
    pub fn write_source(&mut self, index: usize, source: u32) {
        if index < 8 {
            self.dmas[index].source = source;
        }
    }

    /// Write destination address to DMA channel
    pub fn write_dest(&mut self, index: usize, dest: u32) {
        if index < 8 {
            self.dmas[index].destination = dest;
        }
    }

    /// Write transfer length to DMA channel
    pub fn write_len(&mut self, index: usize, len: u16) {
        if index < 8 {
            self.dmas[index].length = len as u32;
            self.dmas[index].internal_len = 0; // Reset internal counter
        }
    }

    /// Write control register to DMA channel
    pub fn write_cnt(&mut self, index: usize, cnt: u16) {
        if index < 8 {
            let was_enabled = self.dmas[index].cnt.enabled;
            self.dmas[index].cnt.set(cnt);

            // Check if DMA was just enabled
            if !was_enabled && self.dmas[index].cnt.enabled {
                // Check timing - if immediate (timing=0), start transfer now
                if self.dmas[index].cnt.timing == 0 {
                    self.active_dmas |= 1 << index;
                    self.dmas[index].internal_len = 0; // Reset to trigger setup on update
                }
            }
        }
    }

    /// Write length and control register as single word
    pub fn write_len_cnt(&mut self, index: usize, word: u32) {
        self.write_len(index, (word & 0xFFFF) as u16);
        self.write_cnt(index, ((word >> 16) & 0xFFFF) as u16);
    }

    /// Request HBLANK-triggered DMA transfers
    pub fn hblank_request(&mut self) -> Result<(), String> {
        // Start DMA channels with HBLANK timing
        for i in 0..8 {
            if self.dmas[i].cnt.enabled && self.dmas[i].cnt.timing == 2 {
                self.active_dmas |= 1 << i;
                self.dmas[i].internal_len = 0;
            }
        }
        Ok(())
    }

    /// Request game cartridge DMA transfer
    pub fn gamecart_request(&mut self) -> Result<(), String> {
        // Start DMA channels with game cartridge timing
        for i in 0..8 {
            if self.dmas[i].cnt.enabled && self.dmas[i].cnt.timing == 3 {
                self.active_dmas |= 1 << i;
                self.dmas[i].internal_len = 0;
            }
        }
        Ok(())
    }

    /// Request GXFIFO DMA transfer
    pub fn gxfifo_request(&mut self) -> Result<(), String> {
        // Start DMA channels with GXFIFO timing (timing=3 for ARM9)
        for i in 0..4 {
            // Only ARM9 DMA (channels 0-3)
            if self.dmas[i].cnt.enabled && self.dmas[i].cnt.timing == 3 {
                self.active_dmas |= 1 << i;
                self.dmas[i].internal_len = 0;
            }
        }
        Ok(())
    }
}

impl Default for NDS_DMA {
    fn default() -> Self {
        Self::new()
    }
}
