//! [`FpgaSlot`]: the FPGA as a Slot-2 device holding any number of blocks.
//!
//! ```text
//!  CPU ldr/str 0x0800_0000 + base + offset
//!    -> block whose [base, base + size) holds the address
//!    -> read:  eval(sel=1, we=0, addr=offset & !3) -> lane of bus_rdata
//!       write: posedge(sel=1, we=1, be from width/address, wdata on lanes)
//!  tick(cycles): every block gets cycles / clock_div idle clocks;
//!                a rising edge of any block's irq -> true (IF bit 13)
//! ```
//!
//! Each access is also recorded in a short log for debuggers.

use std::collections::VecDeque;

use ds_emu::slot2::{SLOT2_START, Slot2Device, Width};

use crate::{
    core::FpgaCore,
    signals::BusIn,
    spec::{BlockSpec, SpecError},
};

/// How many accesses [`FpgaSlot::log`] keeps.
pub const LOG_LEN: usize = 256;

/// One loaded block.
pub struct Block {
    pub spec: BlockSpec,
    pub core: Box<dyn FpgaCore>,
    /// Where the model came from ("spec model", "generated Rust", ...).
    pub backend: String,
    /// ARM7 cycles not yet turned into an FPGA clock.
    cycle_accum: u64,
    pub irq: bool,
}

impl Block {
    /// The block's address window on the CPU bus.
    pub const fn window(&self) -> core::ops::Range<u32> {
        let start = SLOT2_START + self.spec.base;
        start..start + self.spec.size
    }

    /// Current value of the register at `offset` (a combinational read).
    pub fn peek(&self, offset: u16) -> u32 {
        self.core.eval(&BusIn { sel: true, addr: offset, ..BusIn::IDLE }).rdata
    }
}

/// One CPU access that reached a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusEvent {
    pub block: usize,
    pub addr: u32,
    pub value: u32,
    pub write: bool,
    pub width: Width,
}

/// The FPGA in the Slot-2 slot.
#[derive(Default)]
pub struct FpgaSlot {
    blocks: Vec<Block>,
    log: VecDeque<BusEvent>,
    /// Accesses since creation (the log only keeps [`LOG_LEN`]).
    pub accesses: u64,
}

impl FpgaSlot {
    /// Adds a block driven by `core`.
    ///
    /// # Errors
    /// An invalid spec or a window overlapping another block's.
    pub fn add(
        &mut self,
        spec: BlockSpec,
        core: Box<dyn FpgaCore>,
        backend: impl Into<String>,
    ) -> Result<(), SpecError> {
        spec.validate()?;
        let (start, end) = (spec.base, spec.base + spec.size);
        if let Some(other) =
            self.blocks.iter().find(|b| start < b.spec.base + b.spec.size && b.spec.base < end)
        {
            return Err(SpecError(format!(
                "{} overlaps {} in the Slot-2 window",
                spec.name, other.spec.name
            )));
        }
        self.blocks.push(Block { spec, core, backend: backend.into(), cycle_accum: 0, irq: false });
        Ok(())
    }

    /// Adds `spec`: its RTL (`spec.rtl`) run by the built-in Verilog
    /// simulator, or else the register map run by the reference model.
    ///
    /// # Errors
    /// As [`Self::add`].
    pub fn add_spec(&mut self, spec: BlockSpec) -> Result<(), SpecError> {
        spec.validate()?;
        if let Some(src) = &spec.rtl {
            let core = crate::verilog::VerilogCore::new(src).map_err(|e| SpecError(e.0))?;
            return self.add(spec, Box::new(core), "Verilog (built-in simulator)");
        }
        let core = Box::new(crate::core::SpecCore::new(spec.clone()));
        self.add(spec, core, "spec model")
    }

    pub fn remove(&mut self, index: usize) -> Option<Block> {
        (index < self.blocks.len()).then(|| self.blocks.remove(index))
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Starts recording in blocks that are not recording yet (blocks added
    /// since tracing was switched on).
    pub fn ensure_tracing(&mut self) {
        for block in &mut self.blocks {
            if block.core.trace().is_none() {
                block.core.set_tracing(true);
            }
        }
    }

    /// Starts or stops waveform recording in every block.
    pub fn set_tracing(&mut self, on: bool) {
        for block in &mut self.blocks {
            block.core.set_tracing(on);
        }
    }

    /// Most recent last.
    pub const fn log(&self) -> &VecDeque<BusEvent> {
        &self.log
    }

    fn find(&self, addr: u32) -> Option<usize> {
        self.blocks.iter().position(|b| b.window().contains(&addr))
    }

    fn record(&mut self, event: BusEvent) {
        if self.log.len() == LOG_LEN {
            self.log.pop_front();
        }
        self.log.push_back(event);
        self.accesses += 1;
    }
}

const fn lane_mask(width: Width) -> u32 {
    match width {
        Width::Byte => 0xFF,
        Width::Half => 0xFFFF,
        Width::Word => u32::MAX,
    }
}

const fn lane_shift(addr: u32, width: Width) -> u32 {
    match width {
        Width::Byte => (addr & 3) * 8,
        Width::Half => (addr & 2) * 8,
        Width::Word => 0,
    }
}

impl Slot2Device for FpgaSlot {
    fn read(&mut self, addr: u32, width: Width) -> u32 {
        let Some(index) = self.find(addr) else {
            return lane_mask(width); // open bus
        };
        let block = &self.blocks[index];
        let offset = ((addr - block.window().start) & !3) as u16;
        let word = block.peek(offset);
        let value = (word >> lane_shift(addr, width)) & lane_mask(width);
        self.record(BusEvent { block: index, addr, value, write: false, width });
        value
    }

    fn write(&mut self, addr: u32, value: u32, width: Width) {
        let Some(index) = self.find(addr) else { return };
        let block = &mut self.blocks[index];
        let offset = ((addr - block.window().start) & !3) as u16;
        let bus = BusIn {
            rst: false,
            sel: true,
            we: true,
            addr: offset,
            be: width.byte_enable(addr),
            wdata: (value & lane_mask(width)) << lane_shift(addr, width),
        };
        block.core.posedge(&bus);
        self.record(BusEvent { block: index, addr, value, write: true, width });
    }

    fn tick(&mut self, cycles: u32) -> bool {
        let mut rising = false;
        for block in &mut self.blocks {
            block.cycle_accum += u64::from(cycles);
            let div = u64::from(block.spec.clock_div);
            block.core.idle(block.cycle_accum / div);
            block.cycle_accum %= div;
            let irq = block.core.eval(&BusIn::IDLE).irq;
            rising |= irq && !block.irq;
            block.irq = irq;
        }
        rising
    }

    fn name(&self) -> &str {
        "FPGA"
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_accesses_reach_the_block() {
        let mut slot = FpgaSlot::default();
        slot.add_spec(crate::example_timer()).unwrap_or_default();
        assert_eq!(slot.read(0x0800_0000, Width::Word), 0x4650_4741);
        assert_eq!(slot.read(0x0800_0001, Width::Byte), 0x47);
        assert_eq!(slot.read(0x0800_0002, Width::Half), 0x4650);
        slot.write(0x0800_001A, 0xBEEF, Width::Half);
        assert_eq!(slot.read(0x0800_0018, Width::Word), 0xBEEF_0000);
        assert_eq!(slot.read(0x0900_0000, Width::Word), u32::MAX, "no block: open bus");
        assert_eq!(slot.log().len(), 5);
    }

    #[test]
    fn irq_fires_once_on_the_rising_edge() {
        let mut slot = FpgaSlot::default();
        slot.add_spec(crate::example_timer()).unwrap_or_default();
        slot.write(0x0800_000C, 100, Width::Word); // COMPARE
        slot.write(0x0800_0014, 1, Width::Word); // IRQ_EN
        slot.write(0x0800_0004, 1, Width::Word); // CTRL
        assert!(!slot.tick(50));
        assert!(slot.tick(100));
        assert!(!slot.tick(100), "still high, no new edge");
    }

    #[test]
    fn overlapping_blocks_are_rejected() {
        let mut slot = FpgaSlot::default();
        assert!(slot.add_spec(crate::example_timer()).is_ok());
        assert!(slot.add_spec(crate::example_timer()).is_err());
    }
}
