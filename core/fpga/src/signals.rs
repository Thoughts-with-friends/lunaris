//! The pin-level contract of every FPGA block, identical in all three
//! languages.
//!
//! ```text
//!            +---------------------------+
//!  clk  ---->|                           |
//!  rst  ---->| sync, active high         |
//!  bus_sel ->| access to this block      |
//!  bus_we -->| 1 = write                 |---> bus_rdata[31:0]  combinational
//!  bus_addr->| [15:0] byte offset, w.a.  |      (valid while clk is low)
//!  bus_be -->| [3:0] byte enables        |---> irq              level
//!  bus_wdata>| [31:0]                    |
//!            +---------------------------+
//! ```
//!
//! A CPU read is `eval()` with `sel=1, we=0` (no edge); a write is one
//! rising edge with `sel=1, we=1`. Idle clocks have `sel=0`.

/// Inputs of one block, as sampled at a rising edge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BusIn {
    pub rst: bool,
    pub sel: bool,
    pub we: bool,
    /// Byte offset inside the block, word aligned.
    pub addr: u16,
    pub be: u8,
    pub wdata: u32,
}

impl BusIn {
    /// No access, no reset.
    pub const IDLE: Self = Self { rst: false, sel: false, we: false, addr: 0, be: 0, wdata: 0 };

    /// `bus_be` expanded to a bit mask (`{{8{be[3]}}, .., {8{be[0]}}}`).
    pub const fn wmask(&self) -> u32 {
        let mut mask = 0;
        let mut i = 0;
        while i < 4 {
            if self.be & (1 << i) != 0 {
                mask |= 0xFF << (i * 8);
            }
            i += 1;
        }
        mask
    }

    /// Whether this cycle writes the register at `offset`.
    pub const fn writes(&self, offset: u16) -> bool {
        self.sel && self.we && self.addr == offset
    }
}

/// Outputs of one block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BusOut {
    pub rdata: u32,
    pub irq: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wmask_expands_byte_enables() {
        let bus = BusIn { be: 0b0101, ..BusIn::IDLE };
        assert_eq!(bus.wmask(), 0x00FF_00FF);
        assert_eq!(BusIn { be: 0xF, ..BusIn::IDLE }.wmask(), u32::MAX);
    }
}
