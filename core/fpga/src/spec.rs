//! `BlockSpec`: the register map and behaviour of one FPGA block.
//!
//! Read from JSON (`hdl/<name>/<name>.json`). Everything else — the reference model and the
//! Verilog / SystemVerilog / Rust sources — is derived from it.
//!
//! ```json
//! { "name": "fpga_timer", "base": 0, "size": 256, "clock_div": 1,
//!   "registers": [
//!     { "name": "CTRL",   "offset": 4,  "access": "rw" },
//!     { "name": "COUNT",  "offset": 8,  "access": { "counter": { "enable": { "reg": "CTRL", "bit": 0 } } } },
//!     { "name": "STATUS", "offset": 16, "access": { "w1c": { "events": [ { "bit": 0, "equal": ["COUNT", "COMPARE"] } ] } } }
//!   ],
//!   "irq": { "status": "STATUS", "enable": "IRQ_EN" } }
//! ```

use core::fmt;

use serde::{Deserialize, Serialize};

/// One FPGA block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockSpec {
    /// Module / struct name: a lower-case identifier.
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Offset of the block inside the Slot-2 window (0x0800_0000 + base).
    #[serde(default)]
    pub base: u32,
    /// Bytes of address space; a power of two, at most 64 KiB.
    pub size: u32,
    /// FPGA clock = 33.51 MHz / `clock_div`.
    #[serde(default = "one")]
    pub clock_div: u32,
    pub registers: Vec<Register>,
    #[serde(default)]
    pub irq: Option<IrqSpec>,
    /// Verilog / SystemVerilog source defining the behaviour (see
    /// [`crate::verilog`]); `registers` then only document the map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtl: Option<String>,
}

const fn one() -> u32 {
    1
}

/// One 32-bit register.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Register {
    /// Upper-case identifier.
    pub name: String,
    /// Byte offset inside the block, word aligned.
    pub offset: u16,
    pub access: Access,
    #[serde(default)]
    pub reset: u32,
    #[serde(default)]
    pub description: String,
}

/// What drives a register.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// CPU read/write (byte enables honoured).
    Rw,
    /// Read-only constant (`reset`), e.g. an ID.
    Const,
    /// Read-only; +1 every FPGA clock while `enable` (an `rw` bit) is set.
    Counter {
        #[serde(default)]
        enable: Option<BitRef>,
    },
    /// Status bits set by hardware `events`, cleared by writing 1.
    /// A bit set and cleared in the same clock stays set.
    W1c { events: Vec<Event> },
}

/// A bit of another register.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitRef {
    pub reg: String,
    pub bit: u8,
}

/// Sets `bit` in the clock where `equal[0] == equal[1]` (values before the
/// edge). `equal[0]` may be a counter; `equal[1]` must be `rw` or `const`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub bit: u8,
    pub equal: [String; 2],
}

/// `irq = |(status & enable)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrqSpec {
    pub status: String,
    pub enable: String,
}

/// Why a spec was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecError(pub String);

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SpecError {}

fn err<T>(msg: impl Into<String>) -> Result<T, SpecError> {
    Err(SpecError(msg.into()))
}

fn is_ident(s: &str, upper: bool) -> bool {
    let mut chars = s.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    first_ok
        && s.chars().all(|c| {
            c == '_'
                || c.is_ascii_digit()
                || if upper { c.is_ascii_uppercase() } else { c.is_ascii_lowercase() }
        })
}

impl BlockSpec {
    /// Parses and validates a JSON spec.
    ///
    /// # Errors
    /// Malformed JSON or a spec [`Self::validate`] rejects.
    pub fn from_json(json: &str) -> Result<Self, SpecError> {
        let spec: Self = serde_json::from_str(json).map_err(|e| SpecError(e.to_string()))?;
        spec.validate()?;
        Ok(spec)
    }

    /// Pretty JSON.
    ///
    /// # Errors
    /// Never in practice (plain data).
    pub fn to_json(&self) -> Result<String, SpecError> {
        serde_json::to_string_pretty(self).map_err(|e| SpecError(e.to_string()))
    }

    pub fn register(&self, name: &str) -> Option<&Register> {
        self.registers.iter().find(|r| r.name == name)
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.registers.iter().position(|r| r.name == name)
    }

    /// Checks names, offsets and references so every backend can emit the
    /// spec without further checks.
    ///
    /// # Errors
    /// The first problem found.
    pub fn validate(&self) -> Result<(), SpecError> {
        if !is_ident(&self.name, false) {
            return err(format!("block name `{}` must be a lower-case identifier", self.name));
        }
        if !self.size.is_power_of_two() || !(4..=0x1_0000).contains(&self.size) {
            return err("size must be a power of two in 4..=65536");
        }
        if !self.base.is_multiple_of(self.size)
            || self.base + self.size > ds_emu::slot2::SLOT2_END - ds_emu::slot2::SLOT2_START
        {
            return err("base must be size-aligned and inside the Slot-2 window");
        }
        if self.clock_div == 0 {
            return err("clock_div must be at least 1");
        }
        for (i, reg) in self.registers.iter().enumerate() {
            if !is_ident(&reg.name, true) {
                return err(format!("register `{}` must be an upper-case identifier", reg.name));
            }
            if reg.offset % 4 != 0 || u32::from(reg.offset) >= self.size {
                return err(format!(
                    "{}: offset must be word aligned and inside the block",
                    reg.name
                ));
            }
            if self.registers[..i].iter().any(|r| r.name == reg.name || r.offset == reg.offset) {
                return err(format!("{}: duplicate name or offset", reg.name));
            }
        }
        for reg in &self.registers {
            self.validate_access(reg)?;
        }
        if let Some(irq) = &self.irq {
            self.expect_kind(&irq.status, "irq.status", |a| matches!(a, Access::W1c { .. }))?;
            self.expect_kind(&irq.enable, "irq.enable", |a| matches!(a, Access::Rw))?;
        }
        Ok(())
    }

    fn validate_access(&self, reg: &Register) -> Result<(), SpecError> {
        match &reg.access {
            Access::Rw | Access::Const | Access::Counter { enable: None } => Ok(()),
            Access::Counter { enable: Some(en) } => {
                if en.bit >= 32 {
                    return err(format!("{}: enable bit out of range", reg.name));
                }
                self.expect_kind(&en.reg, &reg.name, |a| matches!(a, Access::Rw))
            }
            Access::W1c { events } => {
                for ev in events {
                    if ev.bit >= 32 {
                        return err(format!("{}: event bit out of range", reg.name));
                    }
                    self.expect_kind(&ev.equal[0], &reg.name, |a| {
                        matches!(a, Access::Rw | Access::Const | Access::Counter { .. })
                    })?;
                    self.expect_kind(&ev.equal[1], &reg.name, |a| {
                        matches!(a, Access::Rw | Access::Const)
                    })?;
                }
                Ok(())
            }
        }
    }

    fn expect_kind(
        &self,
        name: &str,
        user: &str,
        ok: impl Fn(&Access) -> bool,
    ) -> Result<(), SpecError> {
        match self.register(name) {
            Some(r) if ok(&r.access) => Ok(()),
            Some(_) => err(format!("{user}: `{name}` has the wrong access kind")),
            None => err(format!("{user}: unknown register `{name}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_example_is_valid() {
        let spec = crate::example_timer();
        assert_eq!(spec.name, "fpga_timer");
        assert!(spec.irq.is_some());
    }

    #[test]
    fn bad_references_are_rejected() {
        let mut spec = crate::example_timer();
        spec.irq = Some(IrqSpec { status: "CTRL".into(), enable: "CTRL".into() });
        assert!(spec.validate().is_err());

        let mut spec = crate::example_timer();
        spec.registers[1].offset = 2;
        assert!(spec.validate().is_err());
    }

    #[test]
    fn json_round_trip() {
        let spec = crate::example_timer();
        let again = BlockSpec::from_json(&spec.to_json().unwrap_or_default());
        assert_eq!(again, Ok(spec));
    }
}
