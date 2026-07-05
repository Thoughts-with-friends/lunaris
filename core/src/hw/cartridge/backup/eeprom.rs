use memmap::MmapMut;
use std::fs::File;
use std::marker::PhantomData;

use super::Backup;

pub struct EEPROM<T: EEPROMType> {
    eeprom_type: PhantomData<T>,
    mem: MmapMut,

    mode: Mode,
    value: u8,
    // Status Reg
    write_enable: bool,
    write_protect: WriteProtect,
}

impl<T: EEPROMType> EEPROM<T> {
    pub fn new(save_file: File, size: usize) -> EEPROM<T> {
        EEPROM {
            eeprom_type: PhantomData,
            // Real EEPROM/Flash chips are 0xFF-filled ("erased") from the
            // factory; a fresh save must match that so its contents look
            // identical to a fresh save from other emulators/flashcarts.
            // GBATEK "erased memory is FFh-filled":
            // <https://problemkaputt.de/gbatek.htm#gbacartbackupeeprom>
            mem: <dyn Backup>::mmap(save_file, 0xFF, size),

            mode: Mode::ReadCommand,
            value: 0,
            // Status Reg
            write_enable: false,
            write_protect: WriteProtect::None,
        }
    }

    fn set_command(&mut self, command: Command) -> Mode {
        match command {
            Command::WREN => {
                self.write_enable = true;
                Mode::ReadCommand
            }
            _ => Mode::HandleCommand(command),
        }
    }

    fn handle_command(&mut self, command: Command, value: u8) -> Mode {
        match command {
            Command::RD(0, addr) => {
                assert_eq!(value, 0);
                self.value = self.mem[addr];
                Mode::HandleCommand(Command::RD(0, addr + 1))
            }
            Command::RD(addr_bytes_left, addr) => {
                Mode::HandleCommand(Command::RD(addr_bytes_left - 1, addr << 8 | value as usize))
            }

            Command::WR(0, addr) => {
                if self.write_enable {
                    self.mem[addr] = value
                }
                Mode::HandleCommand(Command::WR(0, addr + 1))
            }
            Command::WR(addr_bytes_left, addr) => {
                Mode::HandleCommand(Command::WR(addr_bytes_left - 1, addr << 8 | value as usize))
            }

            Command::RDSR => {
                assert_eq!(value, 0);
                // TODO: Figure out Write in Progress needs to be emulated
                let low_nibble = (self.write_protect as u8) << 2 | (self.write_enable as u8) << 1;
                // TODO: Figure out what SWRD Status Register is
                let high_nibble = if T::is_small() { 0xF } else { 0 };
                self.value = high_nibble << 4 | low_nibble;
                Mode::ReadCommand
            }

            Command::WREN => unreachable!(),
        }
    }
}

impl<T: EEPROMType> Backup for EEPROM<T> {
    fn read(&self) -> u8 {
        self.value
    }

    fn write(&mut self, hold: bool, value: u8) {
        self.mode = match self.mode {
            Mode::ReadCommand if value == 0 => return,
            Mode::ReadCommand => self.set_command(Command::get::<T>(value)),
            Mode::HandleCommand(command) => self.handle_command(command, value),
        };
        if !hold {
            self.mode = Mode::ReadCommand
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    ReadCommand,
    HandleCommand(Command),
}

#[derive(Clone, Copy, Debug)]
enum Command {
    WR(usize, usize), // Write
    RD(usize, usize), // Read
    RDSR,             // Read Status Register
    WREN,             // Write Enable
}

impl Command {
    fn get<T: EEPROMType>(value: u8) -> Self {
        match value {
            0x02 if T::is_small() => Command::WR(1, 0), // WRLO
            0x03 if T::is_small() => Command::RD(1, 0), // RDLO
            0x02 => Command::WR(T::addr_bytes(), 0),
            0x03 => Command::RD(T::addr_bytes(), 0),
            0x05 => Command::RDSR,
            0x06 => Command::WREN,
            0x0A if T::is_small() => Command::WR(1, 1), // WRHI
            0x0B if T::is_small() => Command::RD(1, 1), // RDHI
            _ => unimplemented!("{} EEPROM Command: 0x{:X}", T::debug_str(), value),
        }
    }
}

#[derive(Clone, Copy)]
enum WriteProtect {
    None = 0,
    _UpperQuarter = 1,
    _UpperHalf = 2,
    _All = 3,
}

/// GBATEK "DS Cartridge Backup" distinguishes EEPROM chips not just by
/// size but by address-bus width:
/// - 0.5K: 8+1 bit address, split RDLO/RDHI/WRLO/WRHI commands.
/// - 8K/64K: plain 16-bit (2-byte) address.
/// - 128K: 24-bit (3-byte) address.
///
/// Using the wrong `addr_bytes` silently mis-addresses every read/write on
/// 128K carts (e.g. *Pokémon Mystery Dungeon: Explorers of Sky*): the top
/// address byte never gets sent, so all accesses alias into the first 64
/// KiB and corrupt/lose the rest of the save.
///
/// GBATEK: <https://problemkaputt.de/gbatek.htm#dscartridgebackup>
pub trait EEPROMType {
    fn is_small() -> bool;
    fn addr_bytes() -> usize;
    fn debug_str() -> &'static str;
}

pub struct EEPROMSmall {}
pub struct EEPROMNormal {}
pub struct EEPROMLarge {}

impl EEPROMType for EEPROMSmall {
    fn is_small() -> bool {
        true
    }
    fn addr_bytes() -> usize {
        1
    }
    fn debug_str() -> &'static str {
        "Small"
    }
}
impl EEPROMType for EEPROMNormal {
    fn is_small() -> bool {
        false
    }
    fn addr_bytes() -> usize {
        2
    }
    fn debug_str() -> &'static str {
        "Normal"
    }
}
impl EEPROMType for EEPROMLarge {
    fn is_small() -> bool {
        false
    }
    fn addr_bytes() -> usize {
        3
    }
    fn debug_str() -> &'static str {
        "Large"
    }
}
