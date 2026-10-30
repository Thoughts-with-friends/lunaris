//! FPGA blocks for the DS: user hardware on the Slot-2 (GBA slot) bus.
//!
//! ```text
//!  hdl/<name>/*.json (BlockSpec: register map + behaviour)       one source of truth
//!     |                         |                    |
//!     | SpecCore (reference)    | codegen::verilog   | codegen::rust
//!     |                         | codegen::sv        |
//!     v                         v                    v
//!  FpgaCore (eval / posedge)   <name>.v / <name>.sv  <name>.rs (impl FpgaCore)
//!     |                         (synthesis, or a      (hand-extendable, same
//!     |                          Verilator backend     pins and semantics)
//!     |                          behind FpgaCore)
//!     v
//!  FpgaSlot: ds_emu::slot2::Slot2Device  -> Emulator::slot2 (0x0800_0000..)
//!     CPU load/store -> one bus cycle on BusIn/BusOut, IRQ -> IF bit 13
//! ```
//!
//! Every language sees the same pins ([`signals`]) and the same clock-edge
//! semantics ([`core::SpecCore`] documents them), so a block can move from
//! the Rust model to RTL without the ROM driving it noticing.

// Lets generated Rust (`use ds_fpga::...`) compile inside this crate's tests.
extern crate self as ds_fpga;

pub mod codegen;
pub mod core;
pub mod samples;
pub mod signals;
pub mod slot;
pub mod spec;
pub mod trace;
pub mod verilog;

pub use crate::{
    core::{FpgaCore, SpecCore},
    signals::{BusIn, BusOut},
    slot::{BusEvent, FpgaSlot},
    spec::{Access, BlockSpec, Event, Register, SpecError},
    verilog::{VerilogCore, VerilogError},
};

/// The bundled example block (`hdl/fpga_timer/fpga_timer.json`).
pub const EXAMPLE_TIMER_JSON: &str = include_str!("../hdl/fpga_timer/fpga_timer.json");

/// Parses [`EXAMPLE_TIMER_JSON`].
///
/// # Panics
/// Never: the bundled file is checked by the tests.
#[expect(clippy::expect_used, reason = "bundled, test-checked spec")]
pub fn example_timer() -> BlockSpec {
    BlockSpec::from_json(EXAMPLE_TIMER_JSON).expect("bundled spec is valid")
}
