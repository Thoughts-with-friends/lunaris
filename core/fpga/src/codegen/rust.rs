//! Rust emitter: a struct with one field per stateful register and an
//! `impl FpgaCore` that mirrors the HDL statement for statement.

use super::{camel_case, header, register_map, stateful};
use crate::spec::{Access, BlockSpec};

fn hex(v: u32) -> String {
    format!("0x{:04X}_{:04X}", v >> 16, v & 0xFFFF)
}

fn field(name: &str) -> String {
    format!("r_{}", name.to_ascii_lowercase())
}

/// The expression reading register `name` inside `impl`.
fn value(spec: &BlockSpec, name: &str) -> String {
    match spec.register(name).map(|r| &r.access) {
        Some(Access::Const) => format!("Self::C_{name}"),
        _ => format!("self.{}", field(name)),
    }
}

pub fn generate(spec: &BlockSpec) -> String {
    let ty = camel_case(&spec.name);
    let mut o = header(spec, "//");
    o += "//\n// Register map:\n";
    o += &register_map(spec, "//");
    o += "\nuse ds_fpga::{BusIn, BusOut, FpgaCore};\n\n";

    o += "#[derive(Debug, Clone, PartialEq, Eq)]\n";
    o += &format!("pub struct {ty} {{\n");
    for r in stateful(spec) {
        o += &format!("    pub {}: u32,\n", field(&r.name));
    }
    o += "}\n\n";

    o += &format!("impl {ty} {{\n");
    for r in &spec.registers {
        o += &format!("    pub const ADDR_{}: u16 = 0x{:04X};\n", r.name, r.offset);
    }
    for r in spec.registers.iter().filter(|r| matches!(r.access, Access::Const)) {
        o += &format!("    pub const C_{}: u32 = {};\n", r.name, hex(r.reset));
    }
    o += "    pub const RESET: Self = Self {\n";
    for r in stateful(spec) {
        o += &format!("        {}: {},\n", field(&r.name), hex(r.reset));
    }
    o += "    };\n}\n\n";

    o += &format!(
        "impl Default for {ty} {{\n    fn default() -> Self {{\n        Self::RESET\n    }}\n}}\n\n"
    );

    o += &format!("impl FpgaCore for {ty} {{\n");
    o += "    fn eval(&self, bus: &BusIn) -> BusOut {\n        let rdata = match bus.addr {\n";
    for r in &spec.registers {
        o += &format!("            Self::ADDR_{} => {},\n", r.name, value(spec, &r.name));
    }
    o += "            _ => 0,\n        };\n";
    let irq = spec.irq.as_ref().map_or_else(
        || "false".to_owned(),
        |i| format!("{} & {} != 0", value(spec, &i.status), value(spec, &i.enable)),
    );
    o += &format!("        BusOut {{ rdata, irq: {irq} }}\n    }}\n\n");

    o += "    fn posedge(&mut self, bus: &BusIn) {\n";
    o += "        if bus.rst {\n            *self = Self::RESET;\n            return;\n        }\n";
    o += "        let wmask = bus.wmask();\n";
    for r in &spec.registers {
        if let Access::W1c { events } = &r.access {
            let mut expr = "0".to_owned();
            for e in events {
                expr += &format!(
                    "\n            | if {} == {} {{ {} }} else {{ 0 }}",
                    value(spec, &e.equal[0]),
                    value(spec, &e.equal[1]),
                    hex(1 << e.bit)
                );
            }
            o += &format!("        let ev_{} = {expr};\n", r.name.to_ascii_lowercase());
        }
    }
    o += "        *self = Self {\n";
    for r in stateful(spec) {
        let s = format!("self.{}", field(&r.name));
        let next = match &r.access {
            Access::Rw => format!(
                "if bus.writes(Self::ADDR_{}) {{ ({s} & !wmask) | (bus.wdata & wmask) }} else {{ {s} }}",
                r.name
            ),
            Access::Counter { enable: Some(en) } => format!(
                "{s}.wrapping_add(u32::from({} & (1 << {}) != 0))",
                value(spec, &en.reg),
                en.bit
            ),
            Access::Counter { enable: None } => format!("{s}.wrapping_add(1)"),
            Access::W1c { .. } => format!(
                "({s} & !(if bus.writes(Self::ADDR_{}) {{ bus.wdata & wmask }} else {{ 0 }})) | ev_{}",
                r.name,
                r.name.to_ascii_lowercase()
            ),
            Access::Const => continue,
        };
        o += &format!("            {}: {next},\n", field(&r.name));
    }
    o += "        };\n    }\n}\n";
    o
}

#[cfg(test)]
mod tests {
    use crate::{
        core::{FpgaCore, SpecCore},
        signals::BusIn,
    };

    #[allow(clippy::all, clippy::nursery, clippy::pedantic, unused)]
    mod generated {
        include!("../../hdl/fpga_timer/fpga_timer.rs");
    }

    /// xorshift32: deterministic bus traffic.
    fn rng(state: &mut u32) -> u32 {
        *state ^= *state << 13;
        *state ^= *state >> 17;
        *state ^= *state << 5;
        *state
    }

    #[test]
    fn generated_rust_matches_the_reference_model() {
        let mut reference = SpecCore::new(crate::example_timer());
        let mut generated = generated::FpgaTimer::default();
        let offsets: Vec<u16> = reference.spec().registers.iter().map(|r| r.offset).collect();
        let mut seed = 0x1234_5678;
        for cycle in 0..20_000 {
            let r = rng(&mut seed);
            let bus = BusIn {
                rst: r.is_multiple_of(997),
                sel: !r.is_multiple_of(3),
                we: r % 5 < 2,
                // Mostly small COMPARE values so matches actually happen.
                addr: offsets[(r >> 8) as usize % offsets.len()]
                    + if r.is_multiple_of(41) { 2 } else { 0 },
                be: (r >> 4) as u8 & 0xF,
                wdata: if r.is_multiple_of(7) { rng(&mut seed) } else { rng(&mut seed) % 64 },
            };
            for addr in offsets.iter().copied().chain([0xFC]) {
                let probe = BusIn { sel: true, addr, ..BusIn::IDLE };
                assert_eq!(
                    reference.eval(&probe),
                    generated.eval(&probe),
                    "cycle {cycle} +{addr:X}"
                );
            }
            reference.posedge(&bus);
            generated.posedge(&bus);
        }
    }
}
