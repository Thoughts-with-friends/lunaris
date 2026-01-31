/// Graphics Processing Unit (GPU) implementation for Nintendo DS
/// Handles 2D and 3D rendering, VRAM management, and display output
pub(crate) mod arm_rw;
pub(crate) mod draw_scanline;
pub(crate) mod draw_sprite;
pub mod gpu_reg;
pub(crate) mod render_2d;
pub(crate) mod vram_reader;

use crate::gpu_2d::Gpu2DEngine;
use crate::gpu_3d::structs::Gpu3D;
use crate::gpu_root::gpu_reg::{DispStatReg, PowerCtrlReg, SchedulerEvent, VramBankCfg};

/// VRAM memory constants
pub const VRAM_A_SIZE: usize = 128 * 1024; // 128KB
pub const VRAM_B_SIZE: usize = 128 * 1024; // 128KB
pub const VRAM_C_SIZE: usize = 128 * 1024; // 128KB
pub const VRAM_D_SIZE: usize = 128 * 1024; // 128KB
pub const VRAM_E_SIZE: usize = 64 * 1024; // 64KB
pub const VRAM_F_SIZE: usize = 16 * 1024; // 16KB
pub const VRAM_G_SIZE: usize = 16 * 1024; // 16KB
pub const VRAM_H_SIZE: usize = 32 * 1024; // 32KB
pub const VRAM_I_SIZE: usize = 16 * 1024; // 16KB

/// Graphics Processing Unit
/// Manages 2D and 3D rendering for both screens
#[derive(Debug)]
pub struct Gpu {
    /// 2D Engine upper screen
    engine_upper: Gpu2DEngine,
    /// 2D Engine lower screen
    engine_lower: Gpu2DEngine,

    engine_3d: Gpu3D,

    /// Frame rendering complete flag
    frame_complete: bool,
    /// Number of frames skipped for frame skip
    frames_skipped: u32,

    /// Cycle counter
    cycles: u64,

    /// VRAM Banks A-I (9 separate memory banks)
    vram_a: Vec<u8>,
    vram_b: Vec<u8>,
    vram_c: Vec<u8>,
    vram_d: Vec<u8>,
    vram_e: Vec<u8>,
    vram_f: Vec<u8>,
    vram_g: Vec<u8>,
    vram_h: Vec<u8>,
    vram_i: Vec<u8>,

    /// Palette memory for engine A (1024 bytes)
    palette_upper: Vec<u8>,
    /// Palette memory for engine B (1024 bytes)
    palette_lower: Vec<u8>,

    /// OAM (Object Attribute Memory) for sprites (2KB)
    oam: Vec<u8>,

    /// Display status register for ARM7
    display_status_arm7: DispStatReg,
    /// Display status register for ARM9
    display_status_arm9: DispStatReg,

    /// Current vertical line counter
    vertical_count: u16,

    /// VRAM bank configuration A-I
    vramcnt_a: VramBankCfg,
    vramcnt_b: VramBankCfg,
    vramcnt_c: VramBankCfg,
    vramcnt_d: VramBankCfg,
    vramcnt_e: VramBankCfg,
    vramcnt_f: VramBankCfg,
    vramcnt_g: VramBankCfg,
    vramcnt_h: VramBankCfg,
    vramcnt_i: VramBankCfg,

    /// Power control register
    power_control_reg: PowerCtrlReg,
}

impl Default for Gpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Gpu {
    /// Create new GPU instance
    pub fn new() -> Self {
        Gpu {
            engine_upper: Gpu2DEngine::new(),
            engine_lower: Gpu2DEngine::new(),
            engine_3d: Gpu3D::new(),

            frame_complete: false,
            frames_skipped: 0,

            cycles: 0,

            vram_a: vec![0u8; VRAM_A_SIZE],
            vram_b: vec![0u8; VRAM_B_SIZE],
            vram_c: vec![0u8; VRAM_C_SIZE],
            vram_d: vec![0u8; VRAM_D_SIZE],
            vram_e: vec![0u8; VRAM_E_SIZE],
            vram_f: vec![0u8; VRAM_F_SIZE],
            vram_g: vec![0u8; VRAM_G_SIZE],
            vram_h: vec![0u8; VRAM_H_SIZE],
            vram_i: vec![0u8; VRAM_I_SIZE],

            palette_upper: vec![0u8; 1024],
            palette_lower: vec![0u8; 1024],

            oam: vec![0u8; 2048],

            display_status_arm7: DispStatReg::new(),
            display_status_arm9: DispStatReg::new(),

            vertical_count: 0,

            vramcnt_a: VramBankCfg::new(),
            vramcnt_b: VramBankCfg::new(),
            vramcnt_c: VramBankCfg::new(),
            vramcnt_d: VramBankCfg::new(),
            vramcnt_e: VramBankCfg::new(),
            vramcnt_f: VramBankCfg::new(),
            vramcnt_g: VramBankCfg::new(),
            vramcnt_h: VramBankCfg::new(),
            vramcnt_i: VramBankCfg::new(),

            power_control_reg: PowerCtrlReg::new(),
        }
    }
}

// TODO: Rename bytes_to_u16_slice()
pub fn bytes_to_palette(bytes: &[u8]) -> &[u16] {
    assert_eq!(bytes.len() % 2, 0, "u8 vector length must be even");
    unsafe { core::slice::from_raw_parts(bytes.as_ptr() as *const u16, bytes.len() / 2) }
}

/// # Panics
/// Panics if address is out of bounds(<= 0x3ff == 1023) for palette memory
pub fn read_palette_value(bytes: &[u8], address: u32) -> u16 {
    let off = (address & 0x3ff) as usize;
    u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap())
}

impl Gpu {
    pub fn draw_bg_txt_line(&self, index: i32, engine_a: bool) {}
    pub fn draw_bg_extended_line(&self, index: i32, engine_a: bool) {}
    pub fn draw_sprite_line(&self, engine_a: bool) {}

    // moved draw_scanline.rs
    // pub fn draw_scanline(&self) {}

    /// Get current cycle count
    pub fn get_cycles(&self) -> u64 {
        self.cycles
    }

    pub fn draw_3D_scanline(
        &self,
        framebuffer: &[u32],
        bg_priorities: [u8; 256],
        bg0_priority: u8,
    ) {
    }

    /// Power on GPU
    pub fn power_on(&mut self) -> Result<(), String> {
        self.frame_complete = false;
        self.vertical_count = 0;
        Ok(())
    }

    /// Run 3D rendering for specified cycles
    pub fn run_3D(&self, cycles: u64) {}
    pub fn handle_event(&self, event: &SchedulerEvent) {}

    /// Get upper screen framebuffer data
    pub fn get_upper_frame(&self, buffer: &mut [u32]) {
        let engine = match self.power_control_reg.swap_display {
            true => &self.engine_upper,
            false => &self.engine_lower,
        };
        engine.get_framebuffer(buffer);
    }

    /// Get lower screen framebuffer data
    pub fn get_lower_frame(&self, buffer: &mut [u32]) {
        let engine = match self.power_control_reg.swap_display {
            true => &self.engine_lower,
            false => &self.engine_upper,
        };
        engine.get_framebuffer(buffer);
    }

    /// Mark frame start
    pub fn start_frame(&mut self) {
        self.frame_complete = false;
    }

    /// Mark frame completion
    pub fn end_frame(&mut self) {
        self.frame_complete = true;
    }
    pub fn check_gxfifo_dma(&self) {}
    pub fn check_gxfifo_irq(&self) {}

    /// Check if current frame is complete
    pub fn is_frame_complete(&self) -> bool {
        self.frame_complete
    }

    /// Check if display screens are swapped
    pub fn display_swapped(&self) -> bool {
        self.power_control_reg.swap_display
    }

    /// Read from palette A
    pub fn read_palette_a(&self, address: u32) -> u16 {
        let address = address as usize;
        if address + 1 < self.palette_upper.len() {
            let lo = self.palette_upper[address] as u16;
            let hi = self.palette_upper[address + 1] as u16;
            lo | (hi << 8)
        } else {
            0
        }
    }

    pub fn read_extpal_bga(&self, address: u32) -> u16 {
        todo!()
    }
    // moved vram_reader.rs
    // pub fn read_extpal_obja(&self, address: u32) -> u16 {
    //     todo!()
    // }

    /// Read from palette B
    pub fn read_palette_b(&self, address: u32) -> u16 {
        let address = address as usize;
        if address + 1 < self.palette_lower.len() {
            let lo = self.palette_lower[address] as u16;
            let hi = self.palette_lower[address + 1] as u16;
            lo | (hi << 8)
        } else {
            0
        }
    }
    pub fn read_extpal_bgb(&self, address: u32) -> u16 {
        todo!()
    }
    // moved vram_reader.rs
    // pub fn read_extpal_objb(&self, address: u32) -> u16 {
    //     todo!()
    // }

    // ===== Read functions =====

    pub fn read_bga<T>(&self, address: u32) -> T {
        todo!()
    }

    pub fn read_bgb<T>(&self, address: u32) -> T {
        todo!()
    }

    pub fn read_obja<T>(&self, address: u32) -> T {
        todo!()
    }

    pub fn read_objb<T>(&self, address: u32) -> T {
        todo!()
    }

    pub fn read_teximage<T>(&self, address: u32) -> T {
        todo!()
    }

    pub fn read_texpal<T>(&self, address: u32) -> T {
        todo!()
    }

    pub fn read_lcdc<T>(&self, address: u32) -> T {
        todo!()
    }

    // moved arm_rw.rs
    // pub fn read_oam<T>(&self, address: u32) -> T {
    //     todo!()
    // }

    // ===== Write functions =====

    /// Write to palette A
    pub fn write_palette_a(&mut self, address: u32, value: u16) {
        let address = address as usize;
        if address < self.palette_upper.len() {
            self.palette_upper[address] = (value & 0xFF) as u8;
        }
        if address + 1 < self.palette_upper.len() {
            self.palette_upper[address + 1] = ((value >> 8) & 0xFF) as u8;
        }
    }

    /// Write to palette B
    pub fn write_palette_b(&mut self, address: u32, value: u16) {
        let address = address as usize;
        if address < self.palette_lower.len() {
            self.palette_lower[address] = (value & 0xFF) as u8;
        }
        if address + 1 < self.palette_lower.len() {
            self.palette_lower[address + 1] = ((value >> 8) & 0xFF) as u8;
        }
    }

    pub fn write_bga(&mut self, address: u32, halfword: u16) {
        todo!()
    }

    pub fn write_bgb(&mut self, address: u32, halfword: u16) {
        todo!()
    }

    pub fn write_obja(&mut self, address: u32, halfword: u16) {
        todo!()
    }

    pub fn write_objb(&mut self, address: u32, halfword: u16) {
        todo!()
    }

    pub fn write_lcdc(&mut self, address: u32, halfword: u16) {
        todo!()
    }

    pub fn write_oam(&mut self, address: u32, halfword: u16) {
        todo!()
    }

    // ===== ARM7 access =====

    pub fn read_arm7<T>(&self, address: u32) -> T {
        todo!()
    }

    pub fn write_arm7<T>(&mut self, address: u32, value: T) {
        todo!()
    }

    // ===== Helpers =====

    pub fn get_palette(&mut self, engine_a: bool) -> &mut [u16] {
        todo!()
    }

    // TODO: id to enum?
    /// Get VRAM bank by ID.
    /// # Panics
    /// Returns empty slice if ID is 0..=3 outside valid range.
    pub fn get_vram_block(&self, id: i32) -> &[u16] {
        let bytes = match id {
            0 => &self.vram_a,
            1 => &self.vram_b,
            2 => &self.vram_c,
            3 => &self.vram_d,
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!("Invalid VRAM bank ID: {id}");
                panic!("Invalid VRAM bank ID: {id}");
            }
        };
        assert_eq!(bytes.len() % 2, 0, "u8 vector length must be even");
        unsafe { core::slice::from_raw_parts(bytes.as_ptr() as *mut u16, bytes.len() / 2) }
    }

    // TODO: id to enum?
    /// Get VRAM bank by ID.
    /// # Panics
    /// Returns empty slice if ID is 0..=3 outside valid range.
    pub fn get_vram_block_mut(&mut self, id: i32) -> &mut [u16] {
        let bytes = match id {
            0 => &mut self.vram_a,
            1 => &mut self.vram_b,
            2 => &mut self.vram_c,
            3 => &mut self.vram_d,
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!("Invalid VRAM bank ID: {id}");
                panic!("Invalid VRAM bank ID: {id}");
            }
        };
        assert_eq!(bytes.len() % 2, 0, "u8 vector length must be even");
        unsafe { core::slice::from_raw_parts_mut(bytes.as_mut_ptr() as *mut u16, bytes.len() / 2) }
    }

    // TODO: id to enum?
    /// Get VRAM bank by ID.
    /// # Panics
    /// Returns empty slice if ID is 0..=3 outside valid range.
    pub fn get_movable_vram_block(&mut self, src_id: i32, dest_id: i32) -> (&[u16], &mut [u16]) {
        let src_bytes = match src_id {
            0 => &self.vram_a,
            1 => &self.vram_b,
            2 => &self.vram_c,
            3 => &self.vram_d,
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!("Invalid VRAM bank ID: {src_id}");
                panic!("Invalid VRAM bank ID: {src_id}");
            }
        };
        let dest_bytes = match dest_id {
            0 => &self.vram_a,
            1 => &self.vram_b,
            2 => &self.vram_c,
            3 => &self.vram_d,
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!("Invalid VRAM bank ID: {dest_id}");
                panic!("Invalid id VRAM bank ID: {dest_id}");
            }
        };
        assert!(
            (src_bytes.len() % 2) == 0 && (dest_bytes.len() % 2) == 0,
            "u8 vector length must be even"
        );
        unsafe {
            (
                core::slice::from_raw_parts(src_bytes.as_ptr() as *mut u16, src_bytes.len() / 2),
                core::slice::from_raw_parts_mut(
                    dest_bytes.as_ptr() as *mut u16,
                    dest_bytes.len() / 2,
                ),
            )
        }
    }

    pub fn get_dispcnt_a(&self) -> u32 {
        self.engine_upper.get_dispcnt()
    }

    pub fn get_dispcnt_b(&self) -> u32 {
        self.engine_lower.get_dispcnt()
    }

    /// Get DISPSTAT7 register value
    pub fn get_dispstat7(&self) -> u16 {
        self.display_status_arm7.get()
    }

    /// Get DISPSTAT9 register value
    pub fn get_dispstat9(&self) -> u16 {
        self.display_status_arm9.get()
    }

    /// Get BGCNT
    pub fn get_bgcnt_a(&self, index: usize) -> u16 {
        self.engine_upper.get_bgcnt(index)
    }

    pub fn get_bgcnt_b(&self, index: usize) -> u16 {
        self.engine_lower.get_bgcnt(index)
    }

    /// Get VCOUNT register value
    pub fn get_vcount(&self) -> u16 {
        self.vertical_count
    }

    /// Get BGH
    pub fn get_bghofs_a(&self, index: usize) -> u16 {
        self.engine_upper.get_bgvofs(index)
    }

    pub fn get_bgvofs_a(&self, index: usize) -> u16 {
        self.engine_upper.get_bgvofs(index)
    }

    pub fn get_bghofs_b(&self, index: usize) -> u16 {
        self.engine_lower.get_bgcnt(index)
    }

    pub fn get_bgvofs_b(&self, index: usize) -> u16 {
        self.engine_lower.get_bgvofs(index)
    }

    pub fn get_win0v_a(&self) -> u16 {
        todo!()
    }

    pub fn get_win1v_a(&self) -> u16 {
        todo!()
    }

    pub fn get_win0v_b(&self) -> u16 {
        todo!()
    }

    pub fn get_win1v_b(&self) -> u16 {
        todo!()
    }

    pub fn get_winin_a(&self) -> u16 {
        todo!()
    }

    pub fn get_winin_b(&self) -> u16 {
        todo!()
    }

    pub fn get_winout_a(&self) -> u16 {
        todo!()
    }

    pub fn get_winout_b(&self) -> u16 {
        todo!()
    }

    pub fn get_bldcnt_a(&self) -> u16 {
        todo!()
    }

    pub fn get_bldcnt_b(&self) -> u16 {
        todo!()
    }

    pub fn get_bldalpha_a(&self) -> u16 {
        todo!()
    }

    pub fn get_bldalpha_b(&self) -> u16 {
        todo!()
    }

    pub fn get_disp3dcnt(&self) -> u16 {
        todo!()
    }

    pub fn get_master_bright_a(&self) -> u16 {
        todo!()
    }

    pub fn get_master_bright_b(&self) -> u16 {
        todo!()
    }

    /// Replace uint32_t get_DISPCAPCNT();
    pub fn get_dispcapcnt_a(&self) -> u32 {
        let is_engine_a = true;
        self.engine_upper.get_dispcapcnt(is_engine_a)
    }

    pub fn get_vramstat(&self) -> u8 {
        let mut reg: u8 = 0;
        reg = reg | (self.vramcnt_c.enabled && self.vramcnt_c.mst == 2) as u8;
        reg = reg | ((self.vramcnt_d.enabled && self.vramcnt_d.mst == 2) as u8) << 1;
        reg
    }

    /// Get VRAM bank configuration A
    pub fn get_vramcnt_a(&self) -> u8 {
        ((self.vramcnt_a.mst & 0x7) as u8) | (if self.vramcnt_a.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration B
    pub fn get_vramcnt_b(&self) -> u8 {
        ((self.vramcnt_b.mst & 0x7) as u8) | (if self.vramcnt_b.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration C
    pub fn get_vramcnt_c(&self) -> u8 {
        ((self.vramcnt_c.mst & 0x7) as u8) | (if self.vramcnt_c.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration D
    pub fn get_vramcnt_d(&self) -> u8 {
        ((self.vramcnt_d.mst & 0x7) as u8) | (if self.vramcnt_d.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration E
    pub fn get_vramcnt_e(&self) -> u8 {
        ((self.vramcnt_e.mst & 0x7) as u8) | (if self.vramcnt_e.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration F
    pub fn get_vramcnt_f(&self) -> u8 {
        ((self.vramcnt_f.mst & 0x7) as u8) | (if self.vramcnt_f.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration G
    pub fn get_vramcnt_g(&self) -> u8 {
        ((self.vramcnt_g.mst & 0x7) as u8) | (if self.vramcnt_g.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration H
    pub fn get_vramcnt_h(&self) -> u8 {
        ((self.vramcnt_h.mst & 0x7) as u8) | (if self.vramcnt_h.enabled { 0x80 } else { 0 })
    }

    /// Get VRAM bank configuration I
    pub fn get_vramcnt_i(&self) -> u8 {
        ((self.vramcnt_i.mst & 0x7) as u8) | (if self.vramcnt_i.enabled { 0x80 } else { 0 })
    }

    /// Get POWCNT1 register value
    pub fn get_powcnt1(&self) -> u16 {
        self.power_control_reg.get()
    }

    pub fn get_gxstat(&self) -> u32 {
        todo!()
    }

    pub fn get_vert_count(&self) -> u16 {
        todo!()
    }

    pub fn get_poly_count(&self) -> u16 {
        todo!()
    }

    pub fn read_vec_test(&self, address: u32) -> u16 {
        todo!()
    }

    pub fn read_clip_mtx(&self, address: u32) -> u32 {
        todo!()
    }

    pub fn read_vec_mtx(&self, address: u32) -> u32 {
        todo!()
    }

    /// Set upper screen framebuffer
    pub fn set_upper_buffer(&mut self, buffer: Vec<u32>) {
        self.engine_upper.set_framebuffer(buffer);
    }

    /// Set lower screen framebuffer
    pub fn set_lower_buffer(&mut self, buffer: Vec<u32>) {
        self.engine_lower.set_framebuffer(buffer);
    }

    // ===== Setters / Commands =====

    pub fn set_dispcnt_a_lo(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_dispcnt_a(&mut self, word: u32) {
        todo!()
    }

    pub fn set_dispcnt_b_lo(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_dispcnt_b(&mut self, word: u32) {
        todo!()
    }

    /// Set DISPSTAT7 register value
    pub fn set_dispstat7(&mut self, value: u16) {
        self.display_status_arm7.set(value);
    }

    /// Set DISPSTAT9 register value
    pub fn set_dispstat9(&mut self, value: u16) {
        self.display_status_arm9.set(value);
    }

    pub fn set_bgcnt_a(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bgcnt_b(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bghofs_a(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bgvofs_a(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bghofs_b(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bgvofs_b(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bg2p_a(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bg2p_b(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bg3p_a(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bg3p_b(&mut self, halfword: u16, index: i32) {
        todo!()
    }

    pub fn set_bg2x_a(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg2y_a(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg3x_a(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg3y_a(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg2x_b(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg2y_b(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg3x_b(&mut self, word: u32) {
        todo!()
    }

    pub fn set_bg3y_b(&mut self, word: u32) {
        todo!()
    }

    pub fn set_win0h_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win1h_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win0v_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win1v_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win0h_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win1h_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win0v_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_win1v_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_winin_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_winin_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_winout_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_winout_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_mosaic_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_mosaic_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_bldcnt_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_bldcnt_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_bldalpha_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_bldalpha_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_bldy_a(&mut self, byte: u8) {
        todo!()
    }

    pub fn set_bldy_b(&mut self, byte: u8) {
        todo!()
    }

    pub fn set_disp3dcnt(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_master_bright_a(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_master_bright_b(&mut self, halfword: u16) {
        todo!()
    }

    pub fn set_dispcapcnt(&mut self, word: u32) {
        todo!()
    }

    /// Set VRAM bank configuration A
    pub fn set_vramcnt_a(&mut self, value: u8) {
        self.vramcnt_a.mst = (value & 0x7) as u32;
        self.vramcnt_a.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration B
    pub fn set_vramcnt_b(&mut self, value: u8) {
        self.vramcnt_b.mst = (value & 0x7) as u32;
        self.vramcnt_b.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration C
    pub fn set_vramcnt_c(&mut self, value: u8) {
        self.vramcnt_c.mst = (value & 0x7) as u32;
        self.vramcnt_c.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration D
    pub fn set_vramcnt_d(&mut self, value: u8) {
        self.vramcnt_d.mst = (value & 0x7) as u32;
        self.vramcnt_d.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration E
    pub fn set_vramcnt_e(&mut self, value: u8) {
        self.vramcnt_e.mst = (value & 0x7) as u32;
        self.vramcnt_e.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration F
    pub fn set_vramcnt_f(&mut self, value: u8) {
        self.vramcnt_f.mst = (value & 0x7) as u32;
        self.vramcnt_f.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration G
    pub fn set_vramcnt_g(&mut self, value: u8) {
        self.vramcnt_g.mst = (value & 0x7) as u32;
        self.vramcnt_g.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration H
    pub fn set_vramcnt_h(&mut self, value: u8) {
        self.vramcnt_h.mst = (value & 0x7) as u32;
        self.vramcnt_h.enabled = (value & 0x80) != 0;
    }

    /// Set VRAM bank configuration I
    pub fn set_vramcnt_i(&mut self, value: u8) {
        self.vramcnt_i.mst = (value & 0x7) as u32;
        self.vramcnt_i.enabled = (value & 0x80) != 0;
    }

    /// Set POWCNT1 register value
    pub fn set_powcnt1(&mut self, value: u16) {
        self.power_control_reg.set(value);
    }

    /// Write to GXFIFO command queue
    pub fn write_gxfifo(&mut self, _word: u32) {
        todo!()
    }

    pub fn write_fifo_direct(&mut self, address: u32, word: u32) {
        todo!()
    }

    pub fn set_clear_color(&mut self, word: u32) {
        todo!()
    }

    pub fn set_clear_depth(&mut self, word: u32) {
        todo!()
    }

    pub fn set_mtx_mode(&mut self, word: u32) {
        todo!()
    }

    pub fn mtx_push(&mut self) {
        todo!()
    }

    pub fn mtx_pop(&mut self, word: u32) {
        todo!()
    }

    pub fn mtx_identity(&mut self) {
        todo!()
    }

    pub fn mtx_mult_4x4(&mut self, word: u32) {
        todo!()
    }

    pub fn mtx_mult_4x3(&mut self, word: u32) {
        todo!()
    }

    pub fn mtx_mult_3x3(&mut self, word: u32) {
        todo!()
    }

    pub fn mtx_trans(&mut self, word: u32) {
        todo!()
    }

    pub fn color(&mut self, word: u32) {
        todo!()
    }

    pub fn vtx_16(&mut self, word: u32) {
        todo!()
    }

    pub fn set_polygon_attr(&mut self, word: u32) {
        todo!()
    }

    pub fn set_teximage_param(&mut self, word: u32) {
        todo!()
    }

    pub fn set_toon_table(&mut self, address: u32, color: u16) {
        todo!()
    }

    pub fn begin_vtxs(&mut self, word: u32) {
        todo!()
    }

    pub fn swap_buffers(&mut self, word: u32) {
        todo!()
    }

    pub fn viewport(&mut self, word: u32) {
        todo!()
    }

    pub fn set_gxstat(&mut self, word: u32) {
        todo!()
    }
}
