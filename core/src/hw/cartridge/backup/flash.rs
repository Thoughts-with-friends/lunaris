use std::path::PathBuf;

use super::{Backup, SaveMem};

pub struct Flash {
    mem: SaveMem,

    mode: Mode,
    value: u8,
    // Status Reg
    write_enable: bool,
}

impl Flash {
    /// SPI flash page size in bytes. A Page Write/Program must not cross
    /// this boundary: the address wraps back to the start of the *same*
    /// page instead of advancing into the next one.
    ///
    /// GBATEK "0Ah PW Page Write (Write 3-Byte-Address, write 1..256 data
    /// bytes) ... Write/Program may not cross page-boundaries":
    /// <https://problemkaputt.de/gbatek.htm#dsfirmwareserialflashmemory>
    const PAGE_SIZE: usize = 256;
    /// Sector erase (D8h) granularity.
    const SECTOR_SIZE: usize = 0x10000;

    pub fn new_backup(save_path: PathBuf, size: usize) -> Self {
        Flash {
            mem: SaveMem::new(save_path, 0xFF, size),

            mode: Mode::ReadInstr,
            value: 0,
            // Status Reg
            write_enable: false,
        }
    }

    pub fn new_firmware(mem: SaveMem) -> Self {
        Flash {
            mem,

            mode: Mode::ReadInstr,
            value: 0,
            // Status Reg
            write_enable: false,
        }
    }

    fn set_instr(&mut self, instr: Instr) -> Mode {
        match instr {
            Instr::WREN => {
                self.write_enable = true;
                Mode::ReadInstr
            }
            Instr::WRDI => {
                self.write_enable = false;
                Mode::ReadInstr
            }
            _ => Mode::HandleInstr(instr),
        }
    }

    /// Erases `size` bytes (FFh-fill) starting at the `align`-aligned block
    /// containing `addr`. Used by both sector erase (D8h) and page erase
    /// (DBh).
    fn erase_block(&mut self, addr: usize, align: usize, size: usize) {
        let base = addr - (addr % align);
        for offset in 0..size {
            self.mem.write(base + offset, 0xFF);
        }
    }

    fn handle_instr(&mut self, instr: Instr, value: u8) -> Mode {
        match instr {
            Instr::READ(0, addr) => {
                self.value = self.mem.read(addr);
                Mode::HandleInstr(Instr::READ(0, addr + 1))
            }
            Instr::READ(addr_bytes_left, addr) => {
                Mode::HandleInstr(Instr::READ(addr_bytes_left - 1, addr << 8 | value as usize))
            }

            // Fast Read (0Bh): identical to READ but with one dummy byte
            // clocked out after the 3 address bytes before data starts.
            // GBATEK "0Bh FAST Read (Read 3-Byte-Address, 1 Dummy-Byte,
            // read 1..N data bytes)":
            // <https://problemkaputt.de/gbatek.htm#dsfirmwareserialflashmemory>
            Instr::FastRead(FastReadPhase::Dummy, addr) => {
                Mode::HandleInstr(Instr::FastRead(FastReadPhase::Data, addr))
            }
            Instr::FastRead(FastReadPhase::Data, addr) => {
                self.value = self.mem.read(addr);
                Mode::HandleInstr(Instr::FastRead(FastReadPhase::Data, addr + 1))
            }
            Instr::FastReadAddr(0, addr) => {
                Mode::HandleInstr(Instr::FastRead(FastReadPhase::Dummy, addr))
            }
            Instr::FastReadAddr(addr_bytes_left, addr) => Mode::HandleInstr(Instr::FastReadAddr(
                addr_bytes_left - 1,
                addr << 8 | value as usize,
            )),

            Instr::RDSR => {
                // TODO: Figure out if in Progress needs to be emulated
                self.value = (self.write_enable as u8) << 1;
                Mode::ReadInstr
            }
            Instr::RDID => {
                // melonDS returns all-zero JEDEC ID bytes for its emulated
                // retail flash chips (no real manufacturer is modeled).
                self.value = 0x00;
                Mode::HandleInstr(Instr::RDID)
            }

            Instr::WREN | Instr::WRDI => unreachable!(),

            Instr::PW(0, addr) => {
                self.value = self.mem.read(addr);
                if self.write_enable {
                    self.mem.write(addr, value);
                }
                // Wrap within the current page instead of spilling into the
                // next one (see `PAGE_SIZE` doc comment). Without this, a
                // write session that (intentionally or not) transfers more
                // than 256 bytes silently corrupts the *next* page instead
                // of wrapping, which can clobber unrelated save data.
                let next_addr =
                    (addr & !(Self::PAGE_SIZE - 1)) | ((addr + 1) & (Self::PAGE_SIZE - 1));
                Mode::HandleInstr(Instr::PW(0, next_addr))
            }
            Instr::PW(addr_bytes_left, addr) => {
                Mode::HandleInstr(Instr::PW(addr_bytes_left - 1, addr << 8 | value as usize))
            }

            // Page Program (02h): like Page Write, but a program can only
            // clear bits (AND them into the array), never set them — only
            // an erase can bring a bit back to 1. GBATEK "02h PP Page
            // Program": <https://problemkaputt.de/gbatek.htm#dsfirmwareserialflashmemory>
            Instr::PP(0, addr) => {
                if self.write_enable {
                    let programmed = self.mem.read(addr) & value;
                    self.mem.write(addr, programmed);
                }
                let next_addr =
                    (addr & !(Self::PAGE_SIZE - 1)) | ((addr + 1) & (Self::PAGE_SIZE - 1));
                Mode::HandleInstr(Instr::PP(0, next_addr))
            }
            Instr::PP(addr_bytes_left, addr) => {
                Mode::HandleInstr(Instr::PP(addr_bytes_left - 1, addr << 8 | value as usize))
            }

            Instr::SE(0, addr) => {
                if self.write_enable {
                    self.erase_block(addr, Self::SECTOR_SIZE, Self::SECTOR_SIZE);
                }
                Mode::ReadInstr
            }
            Instr::SE(addr_bytes_left, addr) => {
                Mode::HandleInstr(Instr::SE(addr_bytes_left - 1, addr << 8 | value as usize))
            }

            Instr::PE(0, addr) => {
                if self.write_enable {
                    self.erase_block(addr, Self::PAGE_SIZE, Self::PAGE_SIZE);
                }
                Mode::ReadInstr
            }
            Instr::PE(addr_bytes_left, addr) => {
                Mode::HandleInstr(Instr::PE(addr_bytes_left - 1, addr << 8 | value as usize))
            }

            Instr::Unknown(opcode) => {
                warn!(target: "nds_core::savedata", "Flash: ignoring unimplemented instruction 0x{opcode:X}");
                Mode::ReadInstr
            }
        }
    }

    pub fn deselect(&mut self) {
        self.mode = Mode::ReadInstr;
    }
}

impl Backup for Flash {
    fn read(&self) -> u8 {
        self.value
    }

    fn write(&mut self, hold: bool, value: u8) {
        self.mode = match self.mode {
            Mode::ReadInstr => self.set_instr(Instr::get(value)),
            Mode::HandleInstr(instr) => self.handle_instr(instr, value),
        };
        if !hold {
            // Chip-select release: hardware self-clears the write-enable
            // latch and commits any buffered write/program/erase. GBATEK
            // "DS Firmware Serial Flash Memory":
            // <https://problemkaputt.de/gbatek.htm#dsfirmwareserialflashmemory>
            if matches!(
                self.mode,
                Mode::HandleInstr(Instr::PW(..) | Instr::PP(..) | Instr::SE(..) | Instr::PE(..))
            ) {
                self.write_enable = false;
            }
            self.mode = Mode::ReadInstr;
            self.mem.flush();
        }
    }

    /// Captures the in-flight SPI instruction state (not the memory
    /// contents, which are captured separately via [`Backup::save_bytes`]).
    fn protocol_snapshot(&self) -> super::BackupProtocolState {
        super::BackupProtocolState::Flash {
            mode: self.mode,
            write_enable: self.write_enable,
            value: self.value,
        }
    }

    fn restore_protocol_state(&mut self, state: super::BackupProtocolState) {
        if let super::BackupProtocolState::Flash { mode, write_enable, value } = state {
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
    ReadInstr,
    HandleInstr(Instr),
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum FastReadPhase {
    Dummy,
    Data,
}

#[derive(emu_utils::Savestate)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum Instr {
    READ(usize, usize),
    /// Address bytes still expected before the dummy byte (0Bh Fast Read).
    FastReadAddr(usize, usize),
    FastRead(FastReadPhase, usize),
    RDSR,             // Read Status Register
    RDID,             // Read JEDEC ID (9Fh)
    WREN,             // Write Enable
    WRDI,             // Write Disable
    PW(usize, usize), // Page Write (0Ah)
    PP(usize, usize), // Page Program (02h): AND-only write
    SE(usize, usize), // Sector Erase (D8h): 64 KiB -> FFh
    PE(usize, usize), // Page Erase (DBh): 256 B -> FFh
    /// Unrecognized opcode: consumes the rest of the transaction as a no-op
    /// instead of panicking, matching real hardware's tolerance of unknown
    /// commands better than the previous `unimplemented!()`.
    Unknown(u8),
}

impl Instr {
    fn get(value: u8) -> Self {
        match value {
            // 00h/08h are not ordinary flash opcodes on this hardware: on
            // IR carts they select the flash chip vs. the IR MCU (see
            // `backup/ir.rs`), and on plain Flash chips they simply aren't
            // valid instructions. Falling through to `Unknown` here (warn
            // + ignore) is correct for both cases.
            0x03 => Instr::READ(3, 0),
            0x0B => Instr::FastReadAddr(3, 0),
            0x05 => Instr::RDSR,
            0x9F => Instr::RDID,
            0x06 => Instr::WREN,
            0x04 => Instr::WRDI,
            0x0A => Instr::PW(3, 0),
            0x02 => Instr::PP(3, 0),
            0xD8 => Instr::SE(3, 0),
            0xDB => Instr::PE(3, 0),
            other => Instr::Unknown(other),
        }
    }
}
