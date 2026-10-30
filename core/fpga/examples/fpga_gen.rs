//! Generates Verilog, SystemVerilog and Rust from a block spec.
//!
//! ```text
//! cargo run -p ds_fpga --example fpga_gen -- <spec.json> <out dir>
//! cargo run -p ds_fpga --example fpga_gen              # the bundled example -> core/fpga/hdl
//! ```
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (spec, out) = if let (Some(spec), Some(out)) = (args.next(), args.next()) {
        (ds_fpga::BlockSpec::from_json(&std::fs::read_to_string(spec)?)?, PathBuf::from(out))
    } else {
        let spec = ds_fpga::example_timer();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hdl").join(&spec.name);
        (spec, dir)
    };
    for path in ds_fpga::codegen::write_all(&spec, &out)? {
        println!("wrote {}", path.display());
    }
    Ok(())
}
