//! Verilog-2001 and SystemVerilog emitters. Both print the same netlist;
//! SystemVerilog only swaps in `logic`, `always_ff` and `always_comb`.

use super::{header, register_map, stateful};
use crate::spec::{Access, BlockSpec};

fn hex32(v: u32) -> String {
    format!("32'h{v:08X}")
}

/// The signal holding register `name`: `r_<name>` or the `C_<NAME>`
/// constant.
fn signal(spec: &BlockSpec, name: &str) -> String {
    match spec.register(name).map(|r| &r.access) {
        Some(Access::Const) => format!("C_{name}"),
        _ => format!("r_{}", name.to_ascii_lowercase()),
    }
}

/// Emits `spec` as Verilog (`sv == false`) or SystemVerilog.
pub fn generate(spec: &BlockSpec, sv: bool) -> String {
    let (wire, reg, ff, comb) = if sv {
        ("logic", "logic", "always_ff @(posedge clk)", "always_comb")
    } else {
        ("wire ", "reg  ", "always @(posedge clk)", "always @(*)")
    };
    let mut o = header(spec, "//");
    o += "//\n// Register map:\n";
    o += &register_map(spec, "//");
    o += "\n`default_nettype none\n\n";
    o += &format!("module {} (\n", spec.name);
    for (dir, ty, range, name) in [
        ("input ", wire, "      ", "clk"),
        ("input ", wire, "      ", "rst"),
        ("input ", wire, "      ", "bus_sel"),
        ("input ", wire, "      ", "bus_we"),
        ("input ", wire, "[15:0]", "bus_addr"),
        ("input ", wire, "[3:0] ", "bus_be"),
        ("input ", wire, "[31:0]", "bus_wdata"),
        ("output", reg, "[31:0]", "bus_rdata"),
    ] {
        o += &format!("    {dir} {ty} {range} {name},\n");
    }
    o += &format!("    output {wire}        irq\n);\n\n");

    for r in &spec.registers {
        o += &format!("    localparam [15:0] ADDR_{} = 16'h{:04X};\n", r.name, r.offset);
    }
    for r in spec.registers.iter().filter(|r| matches!(r.access, Access::Const)) {
        o += &format!("    localparam [31:0] C_{} = {};\n", r.name, hex32(r.reset));
    }
    o += "\n";
    // A continuous net. In SystemVerilog `logic x = e;` at module scope is a
    // one-time variable initializer, so it is declared and assigned apart.
    let net = |range: &str, name: &str, expr: &str| {
        if sv {
            format!("    logic {range} {name};\n    assign {name} = {expr};\n")
        } else {
            format!("    wire  {range} {name} = {expr};\n")
        }
    };
    o +=
        &net("[31:0]", "wmask", "{{8{bus_be[3]}}, {8{bus_be[2]}}, {8{bus_be[1]}}, {8{bus_be[0]}}}");
    for r in stateful(spec) {
        o += &format!("    {reg} [31:0] r_{};\n", r.name.to_ascii_lowercase());
    }
    for r in stateful(spec) {
        if matches!(r.access, Access::Rw | Access::W1c { .. }) {
            o += &net(
                "      ",
                &format!("wr_{}", r.name.to_ascii_lowercase()),
                &format!("bus_sel & bus_we & (bus_addr == ADDR_{})", r.name),
            );
        }
    }
    for r in &spec.registers {
        if let Access::W1c { events } = &r.access {
            let terms: Vec<String> = events
                .iter()
                .map(|e| {
                    format!(
                        "(({} == {}) ? {} : 32'h0)",
                        signal(spec, &e.equal[0]),
                        signal(spec, &e.equal[1]),
                        hex32(1 << e.bit)
                    )
                })
                .collect();
            let expr = if terms.is_empty() {
                "32'h0".to_owned()
            } else {
                terms.join("\n                         | ")
            };
            o += &net("[31:0]", &format!("ev_{}", r.name.to_ascii_lowercase()), &expr);
        }
    }

    o += &format!("\n    {ff} begin\n        if (rst) begin\n");
    for r in stateful(spec) {
        o += &format!("            r_{} <= {};\n", r.name.to_ascii_lowercase(), hex32(r.reset));
    }
    o += "        end else begin\n";
    for r in stateful(spec) {
        let s = format!("r_{}", r.name.to_ascii_lowercase());
        let next = match &r.access {
            Access::Rw => format!(
                "wr_{0} ? (({s} & ~wmask) | (bus_wdata & wmask)) : {s}",
                r.name.to_ascii_lowercase()
            ),
            Access::Counter { enable: Some(en) } => {
                format!("{s} + {{31'd0, {}[{}]}}", signal(spec, &en.reg), en.bit)
            }
            Access::Counter { enable: None } => format!("{s} + 32'd1"),
            Access::W1c { .. } => {
                let n = r.name.to_ascii_lowercase();
                format!("({s} & ~(wr_{n} ? (bus_wdata & wmask) : 32'h0)) | ev_{n}")
            }
            Access::Const => continue,
        };
        o += &format!("            {s} <= {next};\n");
    }
    o += "        end\n    end\n\n";

    o += &format!("    {comb} begin\n        case (bus_addr)\n");
    for r in &spec.registers {
        o += &format!("            ADDR_{}: bus_rdata = {};\n", r.name, signal(spec, &r.name));
    }
    o += "            default: bus_rdata = 32'h0;\n        endcase\n    end\n\n";

    match &spec.irq {
        Some(irq) => {
            o += &format!(
                "    assign irq = |({} & {});\n",
                signal(spec, &irq.status),
                signal(spec, &irq.enable)
            );
        }
        None => o += "    assign irq = 1'b0;\n",
    }
    o += "endmodule\n\n`default_nettype wire\n";
    o
}
