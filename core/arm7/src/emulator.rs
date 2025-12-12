/// Core emulator system that manages CPU, memory, and all peripheral devices
/// Handles the dual-CPU architecture of the Nintendo DS and system timing
use crate::bios::BIOS;
use crate::cartridge::NDSCart;
use crate::cp15::CP15;
use crate::cpu::ARMCPU;
use crate::dma::NDS_DMA;
use crate::gpu::GPU;
use crate::ipc::{IpcFifo, IpcSync};
use crate::rtc::RealTimeClock;
use crate::spi::SPIBus;
use crate::spu::SPU;
use crate::timers::NDSTiming;
use std::collections::VecDeque;

/// Button input register for standard DS buttons
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyInputReg {
    pub button_a: bool,
    pub button_b: bool,
    pub select: bool,
    pub start: bool,
    pub right: bool,
    pub left: bool,
    pub up: bool,
    pub down: bool,
    pub button_r: bool,
    pub button_l: bool,
}

impl KeyInputReg {
    /// Get the current key input register value (bit-packed format)
    pub fn get_value(&self) -> u16 {
        let mut value = 0u16;
        if self.button_a {
            value |= 0x0001;
        }
        if self.button_b {
            value |= 0x0002;
        }
        if self.select {
            value |= 0x0004;
        }
        if self.start {
            value |= 0x0008;
        }
        if self.right {
            value |= 0x0010;
        }
        if self.left {
            value |= 0x0020;
        }
        if self.up {
            value |= 0x0040;
        }
        if self.down {
            value |= 0x0080;
        }
        if self.button_r {
            value |= 0x0100;
        }
        if self.button_l {
            value |= 0x0200;
        }
        value
    }

    /// Set value from bit-packed register format
    pub fn set_value(&mut self, value: u16) {
        self.button_a = (value & 0x0001) != 0;
        self.button_b = (value & 0x0002) != 0;
        self.select = (value & 0x0004) != 0;
        self.start = (value & 0x0008) != 0;
        self.right = (value & 0x0010) != 0;
        self.left = (value & 0x0020) != 0;
        self.up = (value & 0x0040) != 0;
        self.down = (value & 0x0080) != 0;
        self.button_r = (value & 0x0100) != 0;
        self.button_l = (value & 0x0200) != 0;
    }
}

/// Extended key input register for additional buttons (X, Y, pen, hinge)
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtKeyInReg {
    pub button_x: bool,
    pub button_y: bool,
    pub pen: bool,
    pub hinge: bool,
}

impl ExtKeyInReg {
    /// Get the extended key input register value
    pub fn get_value(&self) -> u16 {
        let mut value = 0u16;
        if self.button_x {
            value |= 0x0001;
        }
        if self.button_y {
            value |= 0x0002;
        }
        if self.pen {
            value |= 0x0004;
        }
        if self.hinge {
            value |= 0x0008;
        }
        value
    }

    /// Set value from bit-packed register format
    pub fn set_value(&mut self, value: u16) {
        self.button_x = (value & 0x0001) != 0;
        self.button_y = (value & 0x0002) != 0;
        self.pen = (value & 0x0004) != 0;
        self.hinge = (value & 0x0008) != 0;
    }
}

/// Power control register for power management
#[derive(Debug, Clone, Copy, Default)]
pub struct PowCnt2Reg {
    pub speakers: bool,
    pub wifi: bool,
    pub led: bool,
    pub cartridge: bool,
}

impl PowCnt2Reg {
    /// Get the power control register value
    pub fn get_value(&self) -> u16 {
        let mut value = 0u16;
        if self.speakers {
            value |= 0x0001;
        }
        if self.wifi {
            value |= 0x0002;
        }
        if self.led {
            value |= 0x0004;
        }
        if self.cartridge {
            value |= 0x0008;
        }
        value
    }

    /// Set value from bit-packed register format
    pub fn set_value(&mut self, value: u16) {
        self.speakers = (value & 0x0001) != 0;
        self.wifi = (value & 0x0002) != 0;
        self.led = (value & 0x0004) != 0;
        self.cartridge = (value & 0x0008) != 0;
    }
}

/// Scheduler event entry for timing-based events
#[derive(Debug, Clone)]
pub struct SchedulerEvent {
    /// Event identifier (GPU event, DMA event, etc.)
    pub event_id: u32,
    /// Timestamp when event should fire
    pub timestamp: u64,
}

/// Interrupt identifier for different interrupt sources
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupt {
    VBlank = 0,
    HBlank = 1,
    VCounter = 2,
    Timer0 = 3,
    Timer1 = 4,
    Timer2 = 5,
    Timer3 = 6,
    SerialComm = 7,
    DMA0 = 8,
    DMA1 = 9,
    DMA2 = 10,
    DMA3 = 11,
    Keypad = 12,
    GameCart = 13,
    IPC_Sync = 16,
    IPC_SendEmpty = 17,
    IPC_RecvNotEmpty = 18,
    NdsMeCardData = 19,
    Geometry = 20,
    Screens = 21,
    Mem = 22,
    RTCAlarm = 23,
}

/// Core Nintendo DS emulator system
/// Manages dual ARM CPUs, memory, and all peripheral devices
pub struct Emulator {
    pub cycle_count: u64,
    pub arm7: ARMCPU,
    pub arm9: ARMCPU,
    pub bios: BIOS,
    pub arm9_cp15: CP15,
    pub cart: NDSCart,
    pub dma: NDS_DMA,
    pub gpu: GPU,
    pub rtc: RealTimeClock,
    pub spi: SPIBus,
    pub spu: SPU,
    pub timers: NDSTiming,
    pub wifi: NDSTiming,

    /// Main system RAM (4MB)
    pub main_ram: Vec<u8>,
    /// Shared CPU WRAM (32KB)
    pub shared_wram: Vec<u8>,
    /// ARM7-only WRAM (64KB)
    pub arm7_wram: Vec<u8>,
    /// VRAM for graphics (used by GPU)
    pub vram: Vec<u8>,
    /// VRAM palette memory
    pub palette_ram: Vec<u8>,
    /// OAM (Object Attribute Memory)
    pub oam: Vec<u8>,

    /// Input state - standard buttons
    pub key_input: KeyInputReg,
    /// Input state - extended buttons
    pub ext_key_in: ExtKeyInReg,
    /// Power control register
    pub pow_cnt2: PowCnt2Reg,

    /// DMA fill values for each of 4 DMA units
    pub dma_fill: [u32; 4],
    /// Serial I/O control register
    pub siocnt: u16,
    /// External memory control
    pub rcnt: u16,
    pub exmemcnt: u16,
    pub wramcnt: u8,

    /// Division engine registers
    pub divcnt: u16,
    pub div_numer: u64,
    pub div_denom: u64,
    pub div_result: u64,
    pub div_remresult: u64,

    /// Square root engine registers
    pub sqrtcnt: u16,
    pub sqrt_result: u32,
    pub sqrt_param: u64,

    /// Power-on flags for debugging/BIOS
    pub postflg7: u8,
    pub postflg9: u8,

    /// BIOS protection register
    pub biosprot: u32,

    /// Debug flag for half-step execution
    pub hstep_even: bool,

    /// Current instruction cycle counter
    pub cycles: i32,

    /// Total system timestamp in cycles
    pub total_timestamp: u64,
    /// Last ARM9 execution timestamp
    pub last_arm9_timestamp: u64,
    /// Last ARM7 execution timestamp
    pub last_arm7_timestamp: u64,

    /// ARM9 interrupt enable register
    pub int9_reg_ie: u32,
    /// ARM9 interrupt request/flag register
    pub int9_reg_if: u32,
    /// ARM9 interrupt master enable
    pub int9_reg_ime: u8,

    /// ARM7 interrupt enable register
    pub int7_reg_ie: u32,
    /// ARM7 interrupt request/flag register
    pub int7_reg_if: u32,
    /// ARM7 interrupt master enable
    pub int7_reg_ime: u8,

    /// Scheduled events in priority order
    pub scheduled_events: VecDeque<SchedulerEvent>,

    /// IPC synchronization objects
    pub ipcsync_nds9: IpcSync,
    pub ipcsync_nds7: IpcSync,

    /// FIFO
    pub fifo7: IpcFifo,
    pub fifo9: IpcFifo,
}

impl Emulator {
    /// Create a new emulator instance with default values
    pub fn new() -> Self {
        Emulator {
            arm9: ARMCPU::new(0), // FIXME?
            arm7: ARMCPU::new(1), // FIXME?
            bios: BIOS::new(),
            arm9_cp15: CP15::new(),
            cart: NDSCart::new(),
            dma: NDS_DMA::new(),
            gpu: GPU::new(),
            rtc: RealTimeClock::new(),
            spi: SPIBus::new(),
            spu: SPU::new(),
            timers: NDSTiming::new(),
            wifi: NDSTiming::new(),

            main_ram: vec![0u8; 4 * 1024 * 1024], // 4MB
            shared_wram: vec![0u8; 32 * 1024],    // 32KB
            arm7_wram: vec![0u8; 64 * 1024],      // 64KB
            vram: vec![0u8; 656 * 1024],          // 656KB VRAM
            palette_ram: vec![0u8; 2 * 1024],     // 2KB palette
            oam: vec![0u8; 2 * 1024],             // 2KB OAM

            key_input: KeyInputReg::default(),
            ext_key_in: ExtKeyInReg::default(),
            pow_cnt2: PowCnt2Reg::default(),

            cycle_count: 0,
            ipcsync_nds7: IpcSync::new(),
            ipcsync_nds9: IpcSync::new(),
            fifo7: IpcFifo::new(),
            fifo9: IpcFifo::new(),

            dma_fill: [0u32; 4],
            siocnt: 0,
            rcnt: 0,
            exmemcnt: 0,
            wramcnt: 0,

            divcnt: 0,
            div_numer: 0,
            div_denom: 0,
            div_result: 0,
            div_remresult: 0,

            sqrtcnt: 0,
            sqrt_result: 0,
            sqrt_param: 0,

            postflg7: 0,
            postflg9: 0,

            biosprot: 0,

            hstep_even: false,
            cycles: 0,

            total_timestamp: 0,
            last_arm9_timestamp: 0,
            last_arm7_timestamp: 0,

            int9_reg_ie: 0,
            int9_reg_if: 0,
            int9_reg_ime: 0,

            int7_reg_ie: 0,
            int7_reg_if: 0,
            int7_reg_ime: 0,

            scheduled_events: VecDeque::new(),
        }
    }

    /// Initialize the emulator system
    pub fn init(&mut self) -> Result<(), String> {
        // Initialize components - add CPU, GPU, DMA, etc. as they are converted
        Ok(())
    }

    /// Load system firmware
    pub fn load_firmware(&mut self) -> Result<(), String> {
        // Load firmware from file
        Ok(())
    }

    /// Load ARM7 BIOS
    pub fn load_bios7(&mut self, bios: &[u8]) -> Result<(), String> {
        // Validate and load ARM7 BIOS
        Ok(())
    }

    /// Load ARM9 BIOS
    pub fn load_bios9(&mut self, bios: &[u8]) -> Result<(), String> {
        // Validate and load ARM9 BIOS
        Ok(())
    }

    /// Load save game database
    pub fn load_save_database(&mut self, _name: &str) -> Result<(), String> {
        Ok(())
    }

    /// Load game cartridge ROM
    pub fn load_rom(&mut self, rom_name: &str) -> Result<(), String> {
        // Load ROM from file and verify
        Ok(())
    }

    /// Power on the system
    pub fn power_on(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Direct boot (skip BIOS)
    pub fn direct_boot(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Debug mode
    pub fn debug(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Run one frame of emulation
    pub fn run(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Check if CPU has pending interrupt
    pub fn requesting_interrupt(&self, cpu_id: u32) -> bool {
        if cpu_id == 7 {
            (self.int7_reg_ie & self.int7_reg_if) != 0 && self.int7_reg_ime != 0
        } else {
            (self.int9_reg_ie & self.int9_reg_if) != 0 && self.int9_reg_ime != 0
        }
    }

    /// Get current system timestamp
    pub fn get_timestamp(&self) -> u64 {
        self.total_timestamp
    }

    /// Get upper screen frame buffer
    pub fn get_upper_frame(&self) -> Vec<u32> {
        // Return upper screen pixel data
        vec![0u32; 256 * 192]
    }

    /// Get lower screen frame buffer
    pub fn get_lower_frame(&self) -> Vec<u32> {
        // Return lower screen pixel data
        vec![0u32; 256 * 192]
    }

    /// Set upper screen buffer
    pub fn set_upper_screen(&mut self, _buffer: &[u32]) -> Result<(), String> {
        Ok(())
    }

    /// Set lower screen buffer
    pub fn set_lower_screen(&mut self, _buffer: &[u32]) -> Result<(), String> {
        Ok(())
    }

    /// Check if current frame is complete
    pub fn frame_complete(&self) -> bool {
        false // Will be implemented with GPU
    }

    /// Check if display has been swapped
    pub fn display_swapped(&self) -> bool {
        false // Will be implemented with GPU
    }

    /// Check if any DMA transfer is active
    pub fn dma_active(&self) -> bool {
        false // Will be implemented with DMA
    }

    /// Request HBLANK DMA transfer
    pub fn hblank_dma_request(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Request game cartridge DMA transfer
    pub fn gamecart_dma_request(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Request GXFIFO DMA transfer
    pub fn gxfifo_dma_request(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Check and process GXFIFO DMA
    pub fn check_gxfifo_dma(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Schedule a GPU event
    pub fn add_gpu_event(&mut self, event_id: u32, relative_time: u64) {
        let timestamp = self.total_timestamp + relative_time;
        self.scheduled_events.push_back(SchedulerEvent {
            event_id,
            timestamp,
        });
        // Sort events by timestamp
        self.scheduled_events
            .make_contiguous()
            .sort_by_key(|e| e.timestamp);
    }

    /// Schedule a DMA event
    pub fn add_dma_event(&mut self, event_id: u32, relative_time: u64) {
        let timestamp = self.total_timestamp + relative_time;
        self.scheduled_events.push_back(SchedulerEvent {
            event_id,
            timestamp,
        });
        // Sort events by timestamp
        self.scheduled_events
            .make_contiguous()
            .sort_by_key(|e| e.timestamp);
    }

    /// Recalculate system timestamp based on CPU execution
    pub fn calculate_system_timestamp(&mut self) {
        self.total_timestamp = std::cmp::min(self.last_arm9_timestamp, self.last_arm7_timestamp);
    }

    /// Handle touchscreen press
    pub fn touchscreen_press(&mut self, x: i32, y: i32) -> Result<(), String> {
        // Update touchscreen input registers
        Ok(())
    }

    /// High-level BIOS emulation hook
    pub fn hle_bios(&mut self, _cpu_id: u32) -> Result<(), String> {
        Ok(())
    }

    // ARM9 Memory Access

    /// ARM9 read 32-bit word
    pub fn arm9_read_word(&self, address: u32) -> u32 {
        self.read_word_internal(address)
    }

    /// ARM9 read 16-bit halfword
    pub fn arm9_read_halfword(&self, address: u32) -> u16 {
        self.read_halfword_internal(address)
    }

    /// ARM9 read 8-bit byte
    pub fn arm9_read_byte(&self, address: u32) -> u8 {
        self.read_byte_internal(address)
    }

    /// ARM9 write 32-bit word
    pub fn arm9_write_word(&mut self, address: u32, word: u32) {
        self.write_word_internal(address, word);
    }

    /// ARM9 write 16-bit halfword
    pub fn arm9_write_halfword(&mut self, address: u32, halfword: u16) {
        self.write_halfword_internal(address, halfword);
    }

    /// ARM9 write 8-bit byte
    pub fn arm9_write_byte(&mut self, address: u32, byte: u8) {
        self.write_byte_internal(address, byte);
    }

    // ARM7 Memory Access

    /// ARM7 read 32-bit word
    // pub fn arm7_read_word(&self, address: u32) -> u32 {
    //     self.read_word_internal(address)
    // }

    /// ARM7 read 16-bit halfword
    pub fn arm7_read_halfword(&self, address: u32) -> u16 {
        self.read_halfword_internal(address)
    }

    /// ARM7 read 8-bit byte
    pub fn arm7_read_byte(&self, address: u32) -> u8 {
        self.read_byte_internal(address)
    }

    /// ARM7 write 32-bit word
    pub fn arm7_write_word(&mut self, address: u32, word: u32) {
        self.write_word_internal(address, word);
    }

    /// ARM7 write 16-bit halfword
    pub fn arm7_write_halfword(&mut self, address: u32, halfword: u16) {
        self.write_halfword_internal(address, halfword);
    }

    /// ARM7 write 8-bit byte
    pub fn arm7_write_byte(&mut self, address: u32, byte: u8) {
        self.write_byte_internal(address, byte);
    }

    // Cartridge operations

    /// Copy key buffer from cartridge
    pub fn cart_copy_keybuffer(&self, _buffer: &mut [u8]) -> Result<(), String> {
        Ok(())
    }

    /// Write to cartridge header
    pub fn cart_write_header(&mut self, _address: u32, _halfword: u16) -> Result<(), String> {
        Ok(())
    }

    // Interrupt control

    /// Request interrupt on ARM7
    pub fn request_interrupt7(&mut self, _id: Interrupt) {
        // Set interrupt flag
    }

    /// Request interrupt on ARM9
    pub fn request_interrupt9(&mut self, _id: Interrupt) {
        // Set interrupt flag
    }

    // CPU access

    /// Check if ARM7 has cartridge access rights
    pub fn arm7_has_cart_rights(&self) -> bool {
        false
    }

    // Placeholder for full CPU implementations to be added later

    // Button input methods

    /// Handle up button press
    pub fn button_up_pressed(&mut self) {
        self.key_input.up = true;
    }

    /// Handle down button press
    pub fn button_down_pressed(&mut self) {
        self.key_input.down = true;
    }

    /// Handle left button press
    pub fn button_left_pressed(&mut self) {
        self.key_input.left = true;
    }

    /// Handle right button press
    pub fn button_right_pressed(&mut self) {
        self.key_input.right = true;
    }

    /// Handle start button press
    pub fn button_start_pressed(&mut self) {
        self.key_input.start = true;
    }

    /// Handle select button press
    pub fn button_select_pressed(&mut self) {
        self.key_input.select = true;
    }

    /// Handle A button press
    pub fn button_a_pressed(&mut self) {
        self.key_input.button_a = true;
    }

    /// Handle B button press
    pub fn button_b_pressed(&mut self) {
        self.key_input.button_b = true;
    }

    /// Handle X button press
    pub fn button_x_pressed(&mut self) {
        self.ext_key_in.button_x = true;
    }

    /// Handle Y button press
    pub fn button_y_pressed(&mut self) {
        self.ext_key_in.button_y = true;
    }

    /// Handle L button press
    pub fn button_l_pressed(&mut self) {
        self.key_input.button_l = true;
    }

    /// Handle R button press
    pub fn button_r_pressed(&mut self) {
        self.key_input.button_r = true;
    }

    /// Handle up button release
    pub fn button_up_released(&mut self) {
        self.key_input.up = false;
    }

    /// Handle down button release
    pub fn button_down_released(&mut self) {
        self.key_input.down = false;
    }

    /// Handle left button release
    pub fn button_left_released(&mut self) {
        self.key_input.left = false;
    }

    /// Handle right button release
    pub fn button_right_released(&mut self) {
        self.key_input.right = false;
    }

    /// Handle start button release
    pub fn button_start_released(&mut self) {
        self.key_input.start = false;
    }

    /// Handle select button release
    pub fn button_select_released(&mut self) {
        self.key_input.select = false;
    }

    /// Handle A button release
    pub fn button_a_released(&mut self) {
        self.key_input.button_a = false;
    }

    /// Handle B button release
    pub fn button_b_released(&mut self) {
        self.key_input.button_b = false;
    }

    /// Handle X button release
    pub fn button_x_released(&mut self) {
        self.ext_key_in.button_x = false;
    }

    /// Handle Y button release
    pub fn button_y_released(&mut self) {
        self.ext_key_in.button_y = false;
    }

    /// Handle L button release
    pub fn button_l_released(&mut self) {
        self.key_input.button_l = false;
    }

    /// Handle R button release
    pub fn button_r_released(&mut self) {
        self.key_input.button_r = false;
    }

    // Internal memory access helpers

    fn read_byte_internal(&self, address: u32) -> u8 {
        match address {
            0x0000_0000..=0x00FF_FFFF => self.main_ram[address as usize],
            0x0200_0000..=0x0207_FFFF => self.shared_wram[(address - 0x0200_0000) as usize],
            0x0300_0000..=0x0300_FFFF => self.arm7_wram[(address - 0x0300_0000) as usize],
            _ => 0u8, // Default for unmapped regions
        }
    }

    fn read_halfword_internal(&self, address: u32) -> u16 {
        let lo = self.read_byte_internal(address) as u16;
        let hi = self.read_byte_internal(address + 1) as u16;
        lo | (hi << 8)
    }

    fn read_word_internal(&self, address: u32) -> u32 {
        let lo = self.read_halfword_internal(address) as u32;
        let hi = self.read_halfword_internal(address + 2) as u32;
        lo | (hi << 16)
    }

    fn write_byte_internal(&mut self, address: u32, byte: u8) {
        match address {
            0x0000_0000..=0x00FF_FFFF => {
                self.main_ram[address as usize] = byte;
            }
            0x0200_0000..=0x0207_FFFF => {
                self.shared_wram[(address - 0x0200_0000) as usize] = byte;
            }
            0x0300_0000..=0x0300_FFFF => {
                self.arm7_wram[(address - 0x0300_0000) as usize] = byte;
            }
            _ => {} // Ignore writes to unmapped regions
        }
    }

    fn write_halfword_internal(&mut self, address: u32, halfword: u16) {
        self.write_byte_internal(address, (halfword & 0xFF) as u8);
        self.write_byte_internal(address + 1, ((halfword >> 8) & 0xFF) as u8);
    }

    fn write_word_internal(&mut self, address: u32, word: u32) {
        self.write_halfword_internal(address, (word & 0xFFFF) as u16);
        self.write_halfword_internal(address + 2, ((word >> 16) & 0xFFFF) as u16);
    }

    /// Check ARM7 FIFO interrupt status
    fn check_fifo7_interrupt(&mut self) {
        // Check IPC FIFO receive status for ARM7
    }

    /// Check ARM9 FIFO interrupt status
    fn check_fifo9_interrupt(&mut self) {
        // Check IPC FIFO receive status for ARM9
    }

    /// Start division operation
    fn start_division(&mut self) {
        if self.div_denom != 0 {
            self.div_result = self.div_numer / self.div_denom;
            self.div_remresult = self.div_numer % self.div_denom;
        }
    }

    /// Start square root operation
    fn start_sqrt(&mut self) {
        self.sqrt_result = (self.sqrt_param as f64).sqrt() as u32;
    }
}

impl Default for Emulator {
    fn default() -> Self {
        Self::new()
    }
}
