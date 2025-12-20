use crate::common::{Gpu, SchedulerEvent};

impl Gpu {
    /// Get current cycle count
    pub fn get_cycles(&self) -> u64 {
        self.cycles
    }

    /// Power on GPU
    pub fn power_on(&mut self) -> Result<(), String> {
        self.frame_complete = false;
        self.vertical_count = 0;
        Ok(())
    }

    /// Run 3D rendering for specified cycles
    pub fn run_3d(&mut self, _cycles: u64) -> Result<(), String> {
        // 3D geometry and rendering processing
        Ok(())
    }

    /// Handle scheduler event
    pub fn handle_event(&mut self, _event: &SchedulerEvent) -> Result<(), String> {
        // Process timing events (VBLANK, HBLANK, etc.)
        Ok(())
    }

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

    /// Check for GXFIFO DMA request
    pub fn check_gxfifo_dma(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Check for GXFIFO interrupt
    pub fn check_gxfifo_irq(&mut self) -> Result<(), String> {
        Ok(())
    }

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
}
