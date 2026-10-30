//! Replacement BIOS binaries run by the emulated CPUs when the high-level
//! BIOS ([`crate::swi`]) is disabled (DraStic free BIOS, BSD-2-Clause).
pub mod arm7;
pub mod arm9;
