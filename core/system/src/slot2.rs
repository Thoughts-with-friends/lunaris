//! Slot-2 (GBA slot) expansion bus: where add-on hardware plugs in.
//!
//! ```text
//!  ARM9 read/write 0x0800_0000..0x0A01_0000
//!    |  (no device: open bus, 0xFF.. on reads, writes ignored)
//!    v
//!  Slot2Bus ---> Box<dyn Slot2Device>   e.g. ds_fpga::FpgaSlot (an FPGA
//!    ^                                   block written in Rust / Verilog /
//!    |                                   SystemVerilog, see core/fpga)
//!  run(): tick(slice cycles) after every scheduler slice
//!         -> `true` raises IF bit 13 (GBA-slot IREQ) on the ARM9
//! ```
//!
//! GBATEK "DS Memory Maps" / "GBA Cart Expansion" (`08000000h..09FFFFFFh`
//! ROM, `0A000000h..0A00FFFFh` SRAM) and "DS Interrupts" (bit 13: GBA-slot).
//! The device is host data like the BIOS: it is not part of a savestate and
//! survives [`crate::Emulator::load_state`].

use core::fmt;

/// First address decoded to the Slot-2 device.
pub const SLOT2_START: u32 = 0x0800_0000;
/// One past the last decoded address (end of the 64 KiB GBA SRAM window).
pub const SLOT2_END: u32 = 0x0A01_0000;

/// IE/IF bit of the GBA-slot interrupt (GBATEK "DS Interrupts").
pub const SLOT2_IRQ_BIT: u32 = 13;

/// Access width of one bus transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Byte,
    Half,
    Word,
}

impl Width {
    /// Byte enables (`bus_be`) of an access at `addr`, as the HDL side sees
    /// them on its 32-bit data bus.
    pub const fn byte_enable(self, addr: u32) -> u8 {
        match self {
            Self::Byte => 1 << (addr & 3),
            Self::Half => 0b11 << (addr & 2),
            Self::Word => 0b1111,
        }
    }
}

/// Hardware attached to the Slot-2 bus.
///
/// `addr` is the full CPU address; `value` is right-aligned (a halfword
/// read returns its 16 bits in the low half).
pub trait Slot2Device: Send {
    fn read(&mut self, addr: u32, width: Width) -> u32;
    fn write(&mut self, addr: u32, value: u32, width: Width);
    /// Advances the device by `cycles` ARM7-clock (33.51 MHz) cycles.
    /// Returns whether its interrupt line is asserted.
    fn tick(&mut self, cycles: u32) -> bool;
    /// Name shown by front ends.
    fn name(&self) -> &str;
    /// For front ends that know the concrete type (debuggers).
    fn as_any(&self) -> &dyn core::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn core::any::Any;
}

/// The (optional) device in the slot.
#[derive(Default)]
pub struct Slot2Bus {
    device: Option<Box<dyn Slot2Device>>,
}

impl fmt::Debug for Slot2Bus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slot2Bus").field("device", &self.device.as_ref().map(|d| d.name())).finish()
    }
}

impl Slot2Bus {
    /// Whether `addr` belongs to the slot.
    pub const fn decodes(addr: u32) -> bool {
        addr >= SLOT2_START && addr < SLOT2_END
    }

    pub fn device(&self) -> Option<&dyn Slot2Device> {
        self.device.as_deref()
    }

    /// Mutable access, e.g. for a debugger that inspects the device.
    pub fn device_mut(&mut self) -> Option<&mut (dyn Slot2Device + 'static)> {
        self.device.as_deref_mut()
    }

    /// Inserts `device` (or empties the slot), returning the previous one.
    pub fn insert(&mut self, device: Option<Box<dyn Slot2Device>>) -> Option<Box<dyn Slot2Device>> {
        core::mem::replace(&mut self.device, device)
    }

    /// A read through the slot: `None` means open bus.
    pub fn read(&mut self, addr: u32, width: Width) -> Option<u32> {
        let device = self.device.as_mut()?;
        Self::decodes(addr).then(|| device.read(addr, width))
    }

    /// A write through the slot; returns whether a device took it.
    pub fn write(&mut self, addr: u32, value: u32, width: Width) -> bool {
        match self.device.as_mut() {
            Some(device) if Self::decodes(addr) => {
                device.write(addr, value, width);
                true
            }
            _ => false,
        }
    }

    pub fn tick(&mut self, cycles: u32) -> bool {
        self.device.as_mut().is_some_and(|d| d.tick(cycles))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo(u32);

    impl Slot2Device for Echo {
        fn read(&mut self, _addr: u32, _width: Width) -> u32 {
            self.0
        }
        fn write(&mut self, _addr: u32, value: u32, _width: Width) {
            self.0 = value;
        }
        fn tick(&mut self, _cycles: u32) -> bool {
            self.0 == 7
        }
        fn name(&self) -> &str {
            "echo"
        }
        fn as_any(&self) -> &dyn core::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
            self
        }
    }

    #[test]
    fn empty_slot_is_open_bus() {
        let mut bus = Slot2Bus::default();
        assert_eq!(bus.read(SLOT2_START, Width::Word), None);
        assert!(!bus.write(SLOT2_START, 1, Width::Word));
        assert!(!bus.tick(100));
    }

    #[test]
    fn device_sees_only_its_window() {
        let mut bus = Slot2Bus::default();
        bus.insert(Some(Box::new(Echo(0))));
        assert!(bus.write(SLOT2_START + 4, 7, Width::Word));
        assert_eq!(bus.read(SLOT2_START, Width::Word), Some(7));
        assert_eq!(bus.read(SLOT2_END, Width::Word), None);
        assert!(bus.tick(1));
    }

    #[test]
    fn byte_enables_follow_the_address() {
        assert_eq!(Width::Byte.byte_enable(0x0800_0003), 0b1000);
        assert_eq!(Width::Half.byte_enable(0x0800_0002), 0b1100);
        assert_eq!(Width::Word.byte_enable(0x0800_0002), 0b1111);
    }
}
