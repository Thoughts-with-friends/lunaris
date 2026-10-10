//! ARM9 Coprocessor 15 (CP15) - System Control Coprocessor
//! Manages ARM9 memory protection unit, caches, TCM, and control registers.
//!
//! Register numbering follows melonDS `CP15.cpp`: a register is addressed by
//! `id = (CRn << 8) | (CRm << 4) | opcode2`, e.g. `0x910` = DTCM region
//! (c9,c1,0) and `0x911` = ITCM region (c9,c1,1). See GBATEK "ARM CP15 System
//! Control Coprocessor".

/// Physical ITCM size (mirrored inside the configured virtual size).
const ITCM_PHYS_SIZE: usize = 32 * 1024;
/// Physical DTCM size (mirrored inside the configured virtual size).
const DTCM_PHYS_SIZE: usize = 16 * 1024;

/// Control register bits that are writable (others are fixed).
const CONTROL_WRITABLE: u32 = 0x000F_F085;
/// Control register bits that always read as 1 (write buffer, 32-bit, ...).
const CONTROL_FIXED: u32 = 0x0000_0078;

const CONTROL_HIGH_VECTORS: u32 = 1 << 13;
const CONTROL_DTCM_ENABLE: u32 = 1 << 16;
const CONTROL_ITCM_ENABLE: u32 = 1 << 18;

/// Side effect of an MCR write that the CPU core must apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cp15Effect {
    None,
    /// "Wait for interrupt" (c7,c0,4 / c7,c8,2): halt the ARM9.
    Halt,
    /// Control register changed; the exception vector base may have moved.
    ControlChanged,
}

/// ARM9 Coprocessor 15 System Control
/// Manages instruction/data TCM and the (unemulated) protection unit/caches.
#[derive(Debug)]
pub struct Cp15 {
    /// Control register (c1,c0,0)
    control: u32,
    /// DTCM region register (c9,c1,0)
    dtcm_setting: u32,
    /// ITCM region register (c9,c1,1)
    itcm_setting: u32,

    /// PU / cache configuration registers, stored only for read-back.
    pu_data_cacheable: u32,
    pu_code_cacheable: u32,
    pu_data_cache_write: u32,
    pu_data_rw: u32,
    pu_code_rw: u32,
    pu_region: [u32; 8],
    trace_process_id: u32,

    /// Virtual ITCM size in bytes (0 when disabled). ITCM is always at 0.
    pub itcm_size: u32,
    /// DTCM base address (`0xFFFF_FFFF` when disabled).
    pub dtcm_base: u32,
    /// Mask applied to an address before comparing it with `dtcm_base`
    /// (0 when disabled, so nothing matches `0xFFFF_FFFF`).
    pub dtcm_mask: u32,

    /// Instruction TCM memory (32 KB)
    itcm: Vec<u8>,
    /// Data TCM memory (16 KB)
    dtcm: Vec<u8>,
}

impl Default for Cp15 {
    fn default() -> Self {
        Self::new()
    }
}

impl Cp15 {
    /// Create new CP15 controller
    pub fn new() -> Self {
        let mut cp15 = Self {
            control: CONTROL_FIXED,
            dtcm_setting: 0,
            itcm_setting: 0,
            pu_data_cacheable: 0,
            pu_code_cacheable: 0,
            pu_data_cache_write: 0,
            pu_data_rw: 0,
            pu_code_rw: 0,
            pu_region: [0; 8],
            trace_process_id: 0,
            itcm_size: 0,
            dtcm_base: 0xFFFF_FFFF,
            dtcm_mask: 0,
            itcm: vec![0; ITCM_PHYS_SIZE],
            dtcm: vec![0; DTCM_PHYS_SIZE],
        };
        cp15.update_tcm();
        cp15
    }

    /// Power on CP15 (BIOS reset state: high vectors, TCMs disabled).
    pub fn power_on(&mut self) {
        self.control = CONTROL_FIXED | CONTROL_HIGH_VECTORS;
        self.dtcm_setting = 0;
        self.itcm_setting = 0;
        self.itcm.fill(0);
        self.dtcm.fill(0);
        self.update_tcm();
    }

    /// Applies the CP15 state the DS BIOS leaves behind before jumping to a
    /// cartridge (melonDS `NDS::SetupDirectBoot`): DTCM at 0x0300_0000
    /// (16 KiB), ITCM 32 MiB virtual, PU/cache settings as the BIOS sets them.
    pub fn direct_boot(&mut self) {
        self.write(0x100, 0x0005_2078);
        self.write(0x200, 0x0000_0042);
        self.write(0x201, 0x0000_0042);
        self.write(0x300, 0x0000_0002);
        self.write(0x502, 0x1511_1011);
        self.write(0x503, 0x0510_0011);
        self.write(0x600, 0x0400_0033);
        self.write(0x610, 0x0200_002B);
        self.write(0x620, 0x0000_0000);
        self.write(0x630, 0x0800_0035);
        self.write(0x640, 0x0300_001B);
        self.write(0x650, 0x0000_0000);
        self.write(0x660, 0xFFFF_001D);
        self.write(0x670, 0x027F_F017);
        self.write(0x910, 0x0300_000A);
        self.write(0x911, 0x0000_0020);
    }

    /// Whether exception vectors are at 0xFFFF0000.
    pub const fn high_vectors(&self) -> bool {
        self.control & CONTROL_HIGH_VECTORS != 0
    }

    /// Recomputes TCM windows (melonDS `UpdateDTCMSetting`/`UpdateITCMSetting`).
    fn update_tcm(&mut self) {
        if self.control & CONTROL_DTCM_ENABLE != 0 {
            let size = (0x200_u32 << ((self.dtcm_setting >> 1) & 0x1F)).max(0x1000);
            self.dtcm_mask = 0xFFFF_F000 & !(size - 1);
            self.dtcm_base = self.dtcm_setting & self.dtcm_mask;
        } else {
            self.dtcm_mask = 0;
            self.dtcm_base = 0xFFFF_FFFF;
        }

        self.itcm_size = if self.control & CONTROL_ITCM_ENABLE != 0 {
            0x200_u32 << ((self.itcm_setting >> 1) & 0x1F)
        } else {
            0
        };
    }

    /// Returns the TCM buffer and offset an ARM9 data access hits, if any.
    #[inline]
    const fn tcm_offset(&self, address: u32) -> Option<(bool, usize)> {
        if address < self.itcm_size {
            Some((true, address as usize & (ITCM_PHYS_SIZE - 1)))
        } else if (address & self.dtcm_mask) == self.dtcm_base {
            Some((false, address as usize & (DTCM_PHYS_SIZE - 1)))
        } else {
            None
        }
    }

    /// Reads `N` bytes from TCM, or `None` if `address` is outside both TCMs.
    #[inline]
    pub fn tcm_read<const N: usize>(&self, address: u32) -> Option<[u8; N]> {
        let (is_itcm, off) = self.tcm_offset(address)?;
        let mem = if is_itcm { &self.itcm } else { &self.dtcm };
        let mut out = [0; N];
        out.copy_from_slice(mem.get(off..off + N)?);
        Some(out)
    }

    /// Writes bytes to TCM; returns `false` if `address` is outside both TCMs.
    #[inline]
    pub fn tcm_write(&mut self, address: u32, bytes: &[u8]) -> bool {
        let Some((is_itcm, off)) = self.tcm_offset(address) else {
            return false;
        };
        let mem = if is_itcm {
            &mut self.itcm
        } else {
            &mut self.dtcm
        };
        if let Some(dst) = mem.get_mut(off..off + bytes.len()) {
            dst.copy_from_slice(bytes);
        }
        true
    }

    /// MRC: reads CP15 register `id` (melonDS `ARMv5::CP15Read`).
    pub fn read(&self, id: u32) -> u32 {
        match id {
            0x000 | 0x003..=0x007 => 0x4105_9461, // main ID
            0x001 => 0x0F0D_2112,                 // cache type
            0x002 => (6 << 6) | (5 << 18),        // TCM size
            0x100 => self.control,
            0x200 => self.pu_data_cacheable,
            0x201 => self.pu_code_cacheable,
            0x300 => self.pu_data_cache_write,
            0x500 => compress_rw(self.pu_data_rw),
            0x501 => compress_rw(self.pu_code_rw),
            0x502 => self.pu_data_rw,
            0x503 => self.pu_code_rw,
            0x600..=0x671 if id & 0xE == 0 => self.pu_region[((id >> 4) & 0x7) as usize],
            0x910 => self.dtcm_setting,
            0x911 => self.itcm_setting,
            0xD01 | 0xD11 => self.trace_process_id,
            _ => 0,
        }
    }

    /// MCR: writes CP15 register `id` (melonDS `ARMv5::CP15Write`).
    pub fn write(&mut self, id: u32, value: u32) -> Cp15Effect {
        match id {
            0x100 => {
                self.control = (self.control & !CONTROL_WRITABLE) | (value & CONTROL_WRITABLE);
                self.update_tcm();
                return Cp15Effect::ControlChanged;
            }
            0x200 => self.pu_data_cacheable = value,
            0x201 => self.pu_code_cacheable = value,
            0x300 => self.pu_data_cache_write = value,
            0x500 => self.pu_data_rw = expand_rw(value),
            0x501 => self.pu_code_rw = expand_rw(value),
            0x502 => self.pu_data_rw = value,
            0x503 => self.pu_code_rw = value,
            0x600..=0x671 if id & 0xE == 0 => self.pu_region[((id >> 4) & 0x7) as usize] = value,
            0x704 | 0x782 => return Cp15Effect::Halt,
            0x910 => {
                self.dtcm_setting = value & 0xFFFF_F03E;
                self.update_tcm();
            }
            0x911 => {
                self.itcm_setting = value & 0x0000_003E;
                self.update_tcm();
            }
            0xD01 | 0xD11 => self.trace_process_id = value,
            // Cache maintenance, drain write buffer, etc.: no cache emulated.
            _ => {}
        }
        Cp15Effect::None
    }
}

/// Packs 8 four-bit PU access fields into the legacy 2-bit format.
const fn compress_rw(rw: u32) -> u32 {
    let mut ret = 0;
    let mut i = 0;
    while i < 8 {
        ret |= ((rw >> (i * 4)) & 0x3) << (i * 2);
        i += 1;
    }
    ret
}

/// Expands the legacy 2-bit PU access format to 4-bit fields.
const fn expand_rw(value: u32) -> u32 {
    let mut ret = 0;
    let mut i = 0;
    while i < 8 {
        ret |= ((value >> (i * 2)) & 0x3) << (i * 4);
        i += 1;
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dtcm_window_and_readback() {
        let mut cp15 = Cp15::new();
        cp15.power_on();
        cp15.write(0x910, 0x02FF_000A); // base 0x02FF0000, 16 KiB
        assert_eq!(cp15.tcm_read::<4>(0x02FF_0000), None); // still disabled
        cp15.write(0x100, CONTROL_DTCM_ENABLE | CONTROL_ITCM_ENABLE);
        assert_eq!(cp15.read(0x910), 0x02FF_000A);
        assert!(cp15.tcm_write(0x02FF_3FFC, &0x1234_5678_u32.to_le_bytes()));
        assert_eq!(
            cp15.tcm_read::<4>(0x02FF_3FFC),
            Some(0x1234_5678_u32.to_le_bytes())
        );
        assert_eq!(cp15.tcm_read::<4>(0x02FF_4000), None);
        // ITCM writes must not alias DTCM
        cp15.write(0x911, 0x20);
        assert!(cp15.tcm_write(0x3FFC, &[0xAA; 4]));
        assert_eq!(
            cp15.tcm_read::<4>(0x02FF_3FFC),
            Some(0x1234_5678_u32.to_le_bytes())
        );
    }

    #[test]
    fn halt_and_control() {
        let mut cp15 = Cp15::new();
        assert_eq!(cp15.write(0x704, 0), Cp15Effect::Halt);
        assert_eq!(cp15.write(0x100, 0), Cp15Effect::ControlChanged);
        assert!(!cp15.high_vectors());
        assert_eq!(cp15.read(0x100), CONTROL_FIXED);
    }
}
