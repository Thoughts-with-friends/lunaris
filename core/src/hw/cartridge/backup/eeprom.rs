use std::marker::PhantomData;
use std::path::PathBuf;

use super::{Backup, SaveMem};

pub struct EEPROM<T: EEPROMType> {
    eeprom_type: PhantomData<T>,
    mem: SaveMem,

    mode: Mode,
    value: u8,
    // Status Reg
    write_enable: bool,
    write_protect: WriteProtect,
}

impl<T: EEPROMType> EEPROM<T> {
    pub fn new(save_path: PathBuf, size: usize) -> EEPROM<T> {
        EEPROM {
            eeprom_type: PhantomData,
            // Real EEPROM/Flash chips are 0xFF-filled ("erased") from the
            // factory; a fresh save must match that so its contents look
            // identical to a fresh save from other emulators/flashcarts.
            // GBATEK "erased memory is FFh-filled":
            // <https://problemkaputt.de/gbatek.htm#gbacartbackupeeprom>
            mem: SaveMem::new(save_path, 0xFF, size),

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
            Command::WRDI => {
                self.write_enable = false;
                Mode::ReadCommand
            }
            _ => Mode::HandleCommand(command),
        }
    }

    /// Returns whether `addr` (byte offset into the chip) falls inside the
    /// region currently protected by the status register's write-protect
    /// bits.
    ///
    /// GBATEK "DS Cartridge Backup" status register bits 2-3 (BP0/BP1):
    /// <https://problemkaputt.de/gbatek.htm#dscartridgebackup>
    fn is_protected(&self, addr: usize) -> bool {
        let len = self.mem.buf_len();
        match self.write_protect {
            WriteProtect::None => false,
            WriteProtect::UpperQuarter => addr >= len - len / 4,
            WriteProtect::UpperHalf => addr >= len / 2,
            WriteProtect::All => true,
        }
    }

    fn handle_command(&mut self, command: Command, value: u8) -> Mode {
        match command {
            Command::RD(0, addr) => {
                self.value = self.mem.read(addr);
                Mode::HandleCommand(Command::RD(0, addr + 1))
            }
            Command::RD(addr_bytes_left, addr) => {
                Mode::HandleCommand(Command::RD(addr_bytes_left - 1, addr << 8 | value as usize))
            }

            Command::WR(0, addr) => {
                if self.write_enable && !self.is_protected(addr) {
                    self.mem.write(addr, value);
                }
                Mode::HandleCommand(Command::WR(0, addr + 1))
            }
            Command::WR(addr_bytes_left, addr) => {
                Mode::HandleCommand(Command::WR(addr_bytes_left - 1, addr << 8 | value as usize))
            }

            Command::RDSR => {
                // TODO: Figure out Write in Progress needs to be emulated
                let low_nibble =
                    (self.write_protect as u8) << 2 | (self.write_enable as u8) << 1;
                // TODO: Figure out what SWRD Status Register is
                let high_nibble = if T::is_small() { 0xF } else { 0 };
                self.value = high_nibble << 4 | low_nibble;
                Mode::ReadCommand
            }
            Command::WRSR => {
                if self.write_enable {
                    self.write_protect = WriteProtect::from_bits(value);
                }
                Mode::ReadCommand
            }
            Command::RDID => {
                // EEPROM chips have no JEDEC ID register; real hardware
                // (and melonDS) returns all-FFh bytes for this command.
                self.value = 0xFF;
                Mode::HandleCommand(Command::RDID)
            }
            Command::Unknown(opcode) => {
                warn!(
                    target: "nds_core::savedata",
                    "{} EEPROM: ignoring unimplemented command 0x{opcode:X}",
                    T::debug_str()
                );
                Mode::ReadCommand
            }

            Command::WREN | Command::WRDI => unreachable!(),
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
            // Chip-select release: hardware self-clears the write-enable
            // latch and commits any buffered write. GBATEK "DS Cartridge
            // Backup": <https://problemkaputt.de/gbatek.htm#dscartridgebackup>
            if matches!(self.mode, Mode::HandleCommand(Command::WR(..))) {
                self.write_enable = false;
            }
            self.mode = Mode::ReadCommand;
            self.mem.flush();
        }
    }

    /// Captures the in-flight SPI command state (not the memory contents,
    /// which are captured separately via [`Backup::save_bytes`]).
    fn protocol_snapshot(&self) -> super::BackupProtocolState {
        super::BackupProtocolState::Eeprom {
            mode: self.mode,
            write_enable: self.write_enable,
            value: self.value,
        }
    }

    fn restore_protocol_state(&mut self, state: super::BackupProtocolState) {
        if let super::BackupProtocolState::Eeprom { mode, write_enable, value } = state {
            self.mode = mode;
            self.write_enable = write_enable;
            self.value = value;
        }
    }

    fn save_bytes(&self) -> Option<&[u8]> {
        Some(self.mem.bytes())
    }

    fn set_save_bytes(&mut self, bytes: &[u8]) {
        self.mem.set_bytes(bytes, 0xFF);
    }

    fn flush(&mut self) {
        self.mem.flush();
    }
}

/// Visible to [`super::BackupProtocolState`] so a savestate can capture and
/// restore an in-progress SPI transaction across save/load. See
/// `docs/design/savestate-and-video-design.md` §2.3.
#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum Mode {
    ReadCommand,
    HandleCommand(Command),
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum Command {
    WR(usize, usize), // Write
    RD(usize, usize), // Read
    RDSR,             // Read Status Register
    WRSR,             // Write Status Register
    WREN,             // Write Enable
    WRDI,             // Write Disable
    RDID,             // Read JEDEC ID (returns FFh on EEPROM)
    /// Unrecognized opcode: consumes the rest of the transaction as a no-op
    /// instead of panicking, matching real hardware's tolerance of unknown
    /// commands better than the previous `unimplemented!()`.
    Unknown(u8),
}

impl Command {
    fn get<T: EEPROMType>(value: u8) -> Self {
        match value {
            0x02 if T::is_small() => Command::WR(1, 0), // WRLO
            0x03 if T::is_small() => Command::RD(1, 0), // RDLO
            0x02 => Command::WR(T::addr_bytes(), 0),
            0x03 => Command::RD(T::addr_bytes(), 0),
            0x01 => Command::WRSR,
            0x05 => Command::RDSR,
            0x06 => Command::WREN,
            0x04 => Command::WRDI,
            0x9F => Command::RDID,
            0x0A if T::is_small() => Command::WR(1, 1), // WRHI
            0x0B if T::is_small() => Command::RD(1, 1), // RDHI
            other => Command::Unknown(other),
        }
    }
}

#[derive(Clone, Copy)]
enum WriteProtect {
    None = 0,
    UpperQuarter = 1,
    UpperHalf = 2,
    All = 3,
}

impl WriteProtect {
    fn from_bits(value: u8) -> Self {
        match (value >> 2) & 0x3 {
            0 => WriteProtect::None,
            1 => WriteProtect::UpperQuarter,
            2 => WriteProtect::UpperHalf,
            _ => WriteProtect::All,
        }
    }
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
