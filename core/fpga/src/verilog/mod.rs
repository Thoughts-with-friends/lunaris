//! The built-in Verilog simulator: hand-written (or generated) Verilog /
//! SystemVerilog runs inside the emulator as an [`FpgaCore`], with no
//! external tool.
//!
//! ```text
//!  .v / .sv --parse--> Module --elaborate--> Design --+--> VerilogCore (FpgaCore)
//!                                                    `--> BlockSpec (register map
//!                                                         from `localparam ADDR_*`)
//! ```
//!
//! Supported (the synthesizable core of the language): one module with the
//! bus pins of [`crate::signals`] (ANSI or body port declarations);
//! `wire` / `reg` / `logic` / `integer`, vectors up to 64 bits, memories
//! (`reg [31:0] mem [0:1023]`); `localparam` / `parameter`; `assign`;
//! `always @(posedge clk)` / `always_ff` (a `negedge` / async-reset term
//! is treated as part of the same clock: resets are synchronous);
//! `always @(*)` / `always_comb`; `initial`; `begin`/`end`, `if`/`else`,
//! `case` (also `unique`/`priority`), `=` and `<=`; all operators,
//! concatenation, replication, bit/part/indexed-part selects, `$signed`,
//! `$unsigned`. Not supported: module instances, `generate`, `for`,
//! functions/tasks, `casez` wildcards, `x`/`z` (read as 0), widths > 64.
//!
//! Pragma, anywhere in the source: `// ds_fpga: size=0x4000 clock_div=64
//! base=0x10000` sets the block's window and clock (defaults: the smallest
//! power of two >= 256 covering every `ADDR_*`, divider 1, base 0).

mod parse;
mod sim;

use core::{cell::RefCell, fmt};

use crate::{
    core::FpgaCore,
    signals::{BusIn, BusOut},
    spec::{Access, BlockSpec, Register},
};

/// Why a source could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerilogError(pub String);

impl fmt::Display for VerilogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VerilogError {}

/// A Verilog module run as a block.
pub struct VerilogCore {
    design: sim::Design,
    /// Interior: `eval` (`&self`) drives inputs and settles logic.
    state: RefCell<sim::State>,
    /// The last edge had an idle bus and changed nothing, so further idle
    /// edges are no-ops (same state + same inputs = same next state).
    idle_stable: bool,
    trace: Option<crate::trace::Trace>,
}

impl fmt::Debug for VerilogCore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerilogCore").field("module", &self.design.name).finish_non_exhaustive()
    }
}

impl VerilogCore {
    /// Parses and elaborates `src`, then clocks one power-on reset.
    ///
    /// # Errors
    /// Syntax outside the supported subset, unknown names, missing bus pins.
    pub fn new(src: &str) -> Result<Self, VerilogError> {
        let design = sim::Design::elaborate(&parse::parse(src)?)?;
        let state = RefCell::new(design.new_state());
        let mut core = Self { design, state, idle_stable: false, trace: None };
        // Power-on reset: one clock with `rst` high, as the slot would give.
        core.posedge(&BusIn { rst: true, ..BusIn::IDLE });
        Ok(core)
    }

    pub fn module_name(&self) -> &str {
        &self.design.name
    }

    fn drive(&self, st: &mut sim::State, bus: &BusIn) {
        let p = self.design.pins;
        self.design.set(st, p.rst, u64::from(bus.rst));
        self.design.set(st, p.sel, u64::from(bus.sel));
        self.design.set(st, p.we, u64::from(bus.we));
        self.design.set(st, p.addr, u64::from(bus.addr));
        self.design.set(st, p.be, u64::from(bus.be));
        self.design.set(st, p.wdata, u64::from(bus.wdata));
        self.design.settle(st);
    }

    /// A signal's current value by name (for debuggers and tests).
    pub fn signal(&self, name: &str) -> Option<u64> {
        let id = self.design.sigs.iter().position(|s| s.name == name)?;
        Some(self.state.borrow().vals[id])
    }

    /// Word `index` of memory `name`.
    pub fn memory(&self, name: &str, index: usize) -> Option<u64> {
        let id = self.design.mems.iter().position(|m| m.name == name)?;
        self.state.borrow().mems[id].get(index).copied()
    }
}

impl FpgaCore for VerilogCore {
    fn eval(&self, bus: &BusIn) -> BusOut {
        let mut st = self.state.borrow_mut();
        self.drive(&mut st, bus);
        let p = self.design.pins;
        BusOut { rdata: st.vals[p.rdata] as u32, irq: st.vals[p.irq] & 1 != 0 }
    }

    fn posedge(&mut self, bus: &BusIn) {
        let st = self.state.get_mut();
        // `drive` borrows `self`; work on the state through a local swap.
        let mut local = core::mem::replace(st, sim::State { vals: Vec::new(), mems: Vec::new() });
        self.drive(&mut local, bus);
        let changed = self.design.clock(&mut local);
        if let Some(trace) = &mut self.trace {
            trace.advance(1);
            trace.record(local.vals.iter().copied());
        }
        *self.state.get_mut() = local;
        self.idle_stable = !changed && *bus == BusIn::IDLE;
    }

    fn idle(&mut self, clocks: u64) {
        for done in 0..clocks {
            if self.idle_stable {
                if let Some(trace) = &mut self.trace {
                    trace.advance(clocks - done);
                }
                return;
            }
            self.posedge(&BusIn::IDLE);
        }
    }

    /// Records every signal (memories excluded), one sample per clock.
    fn set_tracing(&mut self, on: bool) {
        self.trace = on.then(|| {
            let mut trace = crate::trace::Trace::new(
                self.design.sigs.iter().map(|s| (s.name.clone(), s.width)),
            );
            trace.record(self.state.borrow().vals.iter().copied());
            trace
        });
    }

    fn trace(&self) -> Option<&crate::trace::Trace> {
        self.trace.as_ref()
    }
}

/// `key=value` pairs of a `// ds_fpga:` pragma line.
fn pragma(src: &str, key: &str) -> Option<u32> {
    let line = src.lines().find_map(|l| l.split_once("ds_fpga:").map(|(_, rest)| rest))?;
    line.split_whitespace().find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| match v.strip_prefix("0x") {
            Some(hex) => u32::from_str_radix(&hex.replace('_', ""), 16).ok(),
            None => v.replace('_', "").parse().ok(),
        })?
    })
}

/// The `// comment` on the line defining `param`.
fn description(src: &str, param: &str) -> String {
    src.lines()
        .find(|l| {
            l.contains('=')
                && l.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| w == param)
        })
        .and_then(|l| l.split_once("//"))
        .map(|(_, c)| c.trim().to_owned())
        .unwrap_or_default()
}

impl BlockSpec {
    /// The register map of a Verilog block: its `localparam ADDR_<NAME>`
    /// constants (documentation only: the RTL defines the behaviour), plus
    /// the window and clock of its `ds_fpga:` pragma.
    ///
    /// # Errors
    /// What [`VerilogCore::new`] or [`BlockSpec::validate`] rejects.
    pub fn from_verilog(src: &str) -> Result<Self, VerilogError> {
        let design = sim::Design::elaborate(&parse::parse(src)?)?;
        let mut registers: Vec<Register> = design
            .params
            .iter()
            .filter_map(|(name, v)| {
                let reg = name.strip_prefix("ADDR_")?;
                Some(Register {
                    name: reg.to_owned(),
                    offset: u16::try_from(*v).ok()?,
                    access: Access::Rw,
                    reset: 0,
                    description: description(src, name),
                })
            })
            .collect();
        registers.sort_by_key(|r| r.offset);
        let top = registers.iter().map(|r| u32::from(r.offset) + 4).max().unwrap_or(4);
        let spec = Self {
            name: design.name.to_ascii_lowercase(),
            description: format!("Verilog module `{}`", design.name),
            base: pragma(src, "base").unwrap_or(0),
            size: pragma(src, "size").unwrap_or_else(|| top.next_power_of_two().max(256)),
            clock_div: pragma(src, "clock_div").unwrap_or(1),
            registers,
            irq: None,
            rtl: Some(src.to_owned()),
        };
        spec.validate().map_err(|e| VerilogError(e.0))?;
        Ok(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{codegen, core::SpecCore};

    fn rng(state: &mut u32) -> u32 {
        *state ^= *state << 13;
        *state ^= *state >> 17;
        *state ^= *state << 5;
        *state
    }

    /// The generator's HDL, run by this simulator, against the reference
    /// model: all three languages now execute and agree.
    fn lockstep(lang: codegen::Language) {
        let spec = crate::example_timer();
        let mut reference = SpecCore::new(spec.clone());
        let src = codegen::generate(&spec, lang);
        let mut rtl = VerilogCore::new(&src).unwrap_or_else(|e| panic!("{e}\n{src}"));
        let offsets: Vec<u16> = spec.registers.iter().map(|r| r.offset).collect();
        let mut seed = 0x9E37_79B9;
        for cycle in 0..5_000 {
            let r = rng(&mut seed);
            let bus = BusIn {
                rst: r.is_multiple_of(613),
                sel: !r.is_multiple_of(3),
                we: r % 5 < 2,
                addr: offsets[(r >> 8) as usize % offsets.len()],
                be: (r >> 4) as u8 & 0xF,
                wdata: rng(&mut seed) % 48,
            };
            for &addr in &offsets {
                let probe = BusIn { sel: true, addr, ..BusIn::IDLE };
                assert_eq!(
                    reference.eval(&probe),
                    rtl.eval(&probe),
                    "{lang:?} cycle {cycle} +{addr:X}"
                );
            }
            reference.posedge(&bus);
            rtl.posedge(&bus);
        }
    }

    #[test]
    fn generated_verilog_runs_like_the_reference_model() {
        lockstep(codegen::Language::Verilog);
    }

    #[test]
    fn generated_systemverilog_runs_like_the_reference_model() {
        lockstep(codegen::Language::SystemVerilog);
    }

    #[test]
    fn errors_name_the_line() {
        let e = VerilogCore::new("module m(input wire clk);\n  wire a = ;\nendmodule").err();
        assert!(e.is_some_and(|e| e.0.contains("line 2")));
    }

    #[test]
    fn missing_bus_pins_are_reported() {
        let e = VerilogCore::new("module m(input wire clk); endmodule").err();
        assert!(e.is_some_and(|e| e.0.contains("rst")));
    }
}
