//! [`FpgaCore`]: a block as the emulator clocks it, and [`SpecCore`], the
//! reference model that runs a [`BlockSpec`] directly.
//!
//! Clock-edge semantics (the generated Verilog / SystemVerilog / Rust follow
//! them line by line; every next value is computed from values *before* the
//! edge, like non-blocking assignments):
//!
//! ```text
//!  rst            every register <- reset
//!  rw             write ? (old & ~wmask) | (wdata & wmask) : old
//!  const          reset
//!  counter        old + (enable bit of old ? 1 : 0)
//!  w1c            (old & ~(write ? wdata & wmask : 0)) | events
//!                 events: bit set where old[a] == old[b]
//!  bus_rdata      register at bus_addr (0 for a hole), combinational
//!  irq            |(status & enable)
//! ```

use crate::{
    signals::{BusIn, BusOut},
    spec::{Access, BlockSpec},
    trace::Trace,
};

/// One block, clocked by [`crate::FpgaSlot`]. Implemented by
/// [`SpecCore`], by generated Rust ([`crate::codegen::rust`]) and — the
/// intended RTL backend — by a Verilator model of the generated HDL.
pub trait FpgaCore: Send {
    /// Combinational outputs for `bus` with the clock low (no state change).
    fn eval(&self, bus: &BusIn) -> BusOut;
    /// One rising clock edge sampling `bus`.
    fn posedge(&mut self, bus: &BusIn);
    /// `clocks` edges with an idle bus. Override when it can be computed
    /// faster than edge by edge.
    fn idle(&mut self, clocks: u64) {
        for _ in 0..clocks {
            self.posedge(&BusIn::IDLE);
        }
    }
    /// Starts (fresh, at clock 0) or stops recording a waveform.
    fn set_tracing(&mut self, _on: bool) {}
    /// The waveform recorded so far, while tracing.
    fn trace(&self) -> Option<&Trace> {
        None
    }
}

/// Resolved [`Access`]: register names turned into indices.
#[derive(Debug, Clone)]
enum Kind {
    Rw,
    Const,
    Counter { enable: Option<(usize, u8)> },
    W1c { events: Vec<(u8, usize, usize)> },
}

/// The reference model: interprets a [`BlockSpec`].
#[derive(Debug, Clone)]
pub struct SpecCore {
    spec: BlockSpec,
    kinds: Vec<Kind>,
    values: Vec<u32>,
    next: Vec<u32>,
    irq: Option<(usize, usize)>,
    trace: Option<Trace>,
}

impl SpecCore {
    /// A model of `spec` (which must be valid, see [`BlockSpec::validate`]),
    /// in reset state.
    ///
    /// # Panics
    /// If `spec` references unknown registers.
    #[expect(clippy::unwrap_used, reason = "validated spec")]
    pub fn new(spec: BlockSpec) -> Self {
        let idx = |name: &str| spec.index_of(name).unwrap();
        let kinds = spec
            .registers
            .iter()
            .map(|r| match &r.access {
                Access::Rw => Kind::Rw,
                Access::Const => Kind::Const,
                Access::Counter { enable } => {
                    Kind::Counter { enable: enable.as_ref().map(|e| (idx(&e.reg), e.bit)) }
                }
                Access::W1c { events } => Kind::W1c {
                    events: events
                        .iter()
                        .map(|e| (e.bit, idx(&e.equal[0]), idx(&e.equal[1])))
                        .collect(),
                },
            })
            .collect();
        let irq = spec.irq.as_ref().map(|i| (idx(&i.status), idx(&i.enable)));
        let values: Vec<u32> = spec.registers.iter().map(|r| r.reset).collect();
        let next = values.clone();
        Self { spec, kinds, values, next, irq, trace: None }
    }

    pub const fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    /// Current register values, in [`BlockSpec::registers`] order.
    pub fn values(&self) -> &[u32] {
        &self.values
    }

    /// After `clocks` edges: advance the trace and sample the registers.
    fn traced(&mut self, clocks: u64) {
        if let Some(trace) = &mut self.trace {
            trace.advance(clocks);
            trace.record(self.values.iter().map(|&v| u64::from(v)));
        }
    }

    fn counter_enabled(&self, enable: Option<(usize, u8)>) -> bool {
        enable.is_none_or(|(reg, bit)| self.values[reg] & (1 << bit) != 0)
    }

    fn events(&self, events: &[(u8, usize, usize)]) -> u32 {
        events
            .iter()
            .filter(|(_, a, b)| self.values[*a] == self.values[*b])
            .fold(0, |acc, (bit, _, _)| acc | (1 << bit))
    }
}

impl FpgaCore for SpecCore {
    fn eval(&self, bus: &BusIn) -> BusOut {
        let rdata = self
            .spec
            .registers
            .iter()
            .position(|r| r.offset == bus.addr)
            .map_or(0, |i| self.values[i]);
        let irq = self.irq.is_some_and(|(s, e)| self.values[s] & self.values[e] != 0);
        BusOut { rdata, irq }
    }

    fn posedge(&mut self, bus: &BusIn) {
        if bus.rst {
            for (v, r) in self.values.iter_mut().zip(&self.spec.registers) {
                *v = r.reset;
            }
            self.traced(1);
            return;
        }
        let wmask = bus.wmask();
        for (i, kind) in self.kinds.iter().enumerate() {
            let reg = &self.spec.registers[i];
            let old = self.values[i];
            let written = bus.writes(reg.offset);
            self.next[i] = match kind {
                Kind::Rw if written => (old & !wmask) | (bus.wdata & wmask),
                Kind::Rw => old,
                Kind::Const => reg.reset,
                Kind::Counter { enable } => {
                    old.wrapping_add(u32::from(self.counter_enabled(*enable)))
                }
                Kind::W1c { events } => {
                    let clear = if written { bus.wdata & wmask } else { 0 };
                    (old & !clear) | self.events(events)
                }
            };
        }
        core::mem::swap(&mut self.values, &mut self.next);
        self.traced(1);
    }

    fn set_tracing(&mut self, on: bool) {
        self.trace =
            on.then(|| Trace::new(self.spec.registers.iter().map(|r| (r.name.clone(), 32))));
        self.traced(0);
    }

    fn trace(&self) -> Option<&Trace> {
        self.trace.as_ref()
    }

    /// Closed form: an idle bus changes only counters and w1c events.
    fn idle(&mut self, clocks: u64) {
        if clocks == 0 {
            return;
        }
        let mut next = self.values.clone();
        for (i, kind) in self.kinds.iter().enumerate() {
            match kind {
                Kind::Counter { enable } if self.counter_enabled(*enable) => {
                    next[i] = self.values[i].wrapping_add(clocks as u32);
                }
                Kind::W1c { events } => {
                    for &(bit, a, b) in events {
                        let target = self.values[b];
                        let hit = match self.kinds[a] {
                            // Counter visits values[a] .. values[a] + clocks - 1.
                            Kind::Counter { enable } if self.counter_enabled(enable) => {
                                u64::from(target.wrapping_sub(self.values[a])) < clocks
                            }
                            _ => self.values[a] == target,
                        };
                        if hit {
                            next[i] |= 1 << bit;
                        }
                    }
                }
                _ => {}
            }
        }
        self.values = next;
        // The closed form has no intermediate edges: one sample at the end.
        self.traced(clocks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: u16 = 4;
    const COUNT: u16 = 8;
    const COMPARE: u16 = 12;
    const STATUS: u16 = 16;
    const IRQ_EN: u16 = 20;

    fn write(core: &mut dyn FpgaCore, addr: u16, wdata: u32) {
        core.posedge(&BusIn { sel: true, we: true, addr, be: 0xF, wdata, rst: false });
    }

    fn read(core: &dyn FpgaCore, addr: u16) -> u32 {
        core.eval(&BusIn { sel: true, addr, ..BusIn::IDLE }).rdata
    }

    #[test]
    fn timer_counts_matches_and_interrupts() {
        let mut core = SpecCore::new(crate::example_timer());
        assert_eq!(read(&core, 0), 0x4650_4741);
        write(&mut core, COMPARE, 10);
        write(&mut core, IRQ_EN, 1);
        write(&mut core, CTRL, 1);
        for _ in 0..10 {
            core.posedge(&BusIn::IDLE);
        }
        assert_eq!(read(&core, COUNT), 10);
        assert!(!core.eval(&BusIn::IDLE).irq, "match is seen at the next edge");
        core.posedge(&BusIn::IDLE);
        assert!(core.eval(&BusIn::IDLE).irq);
        write(&mut core, STATUS, 1);
        assert_eq!(read(&core, STATUS), 0);
    }

    #[test]
    fn idle_closed_form_equals_edge_by_edge() {
        for (compare, clocks) in [(5, 3), (5, 6), (5, 7), (0, 1), (100, 1000), (u32::MAX, 2)] {
            let mut fast = SpecCore::new(crate::example_timer());
            write(&mut fast, COMPARE, compare);
            write(&mut fast, CTRL, 1);
            let mut slow = fast.clone();
            fast.idle(clocks);
            for _ in 0..clocks {
                slow.posedge(&BusIn::IDLE);
            }
            assert_eq!(fast.values(), slow.values(), "compare {compare}, {clocks} clocks");
        }
    }

    #[test]
    fn byte_enables_merge() {
        let mut core = SpecCore::new(crate::example_timer());
        write(&mut core, 24, 0x1122_3344);
        core.posedge(&BusIn {
            sel: true,
            we: true,
            addr: 24,
            be: 0b0010,
            wdata: 0xAAAA_AAAA,
            rst: false,
        });
        assert_eq!(read(&core, 24), 0x1122_AA44);
    }
}
