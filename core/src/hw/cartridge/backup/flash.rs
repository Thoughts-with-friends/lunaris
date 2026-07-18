use memmap::MmapMut;
use std::fs::File;

use super::Backup;

pub struct Flash {
    mem: MmapMut,

    mode: Mode,
    value: u8,
    // Status Reg
    write_enable: bool,
}

impl Flash {
    /// SPI flash page size in bytes. A Page Write must not cross this
    /// boundary: the address wraps back to the start of the *same* page
    /// instead of advancing into the next one.
    ///
    /// GBATEK "0Ah PW Page Write (Write 3-Byte-Address, write 1..256 data
    /// bytes) ... Write/Program may not cross page-boundaries":
    /// <https://problemkaputt.de/gbatek.htm#dsfirmwareserialflashmemory>
    const PAGE_SIZE: usize = 256;

    pub fn new_backup(save_file: File, size: usize) -> Self {
        Flash {
            mem: <dyn Backup>::mmap(save_file, 0xFF, size),

            mode: Mode::ReadInstr,
            value: 0,
            // Status Reg
            write_enable: false,
        }
    }

    pub fn new_firmware(mem: MmapMut) -> Self {
        Flash {
            mem,

            mode: Mode::ReadInstr,
            value: 0,
            // Status Reg
            write_enable: false,
        }
    }

    fn set_instr(&mut self, instr: Instr) -> Mode {
        eprintln!("[flash] instr: {instr:?}");
        match instr {
            Instr::IR => Mode::ReadInstr, // TODO: Actually implement IR
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

    fn handle_instr(&mut self, instr: Instr, value: u8) -> Mode {
        match instr {
            Instr::IR => unreachable!(),

            Instr::READ(0, addr) => {
                assert_eq!(value, 0);
                self.value = self.mem[addr];
                Mode::HandleInstr(Instr::READ(0, addr + 1))
            }
            Instr::READ(addr_bytes_left, addr) => {
                let new_addr = addr << 8 | value as usize;
                if addr_bytes_left == 1 {
                    eprintln!("[flash] READ session start addr=0x{new_addr:X}");
                }
                Mode::HandleInstr(Instr::READ(addr_bytes_left - 1, new_addr))
            }

            Instr::RDSR => {
                assert_eq!(value, 0);
                // TODO: Figure out if in Progress needs to be emulated
                self.value = (self.write_enable as u8) << 1;
                Mode::ReadInstr
            }

            Instr::WREN => unreachable!(),
            Instr::WRDI => unreachable!(),

            Instr::PW(0, addr) => {
                self.value = self.mem[addr];
                self.mem[addr] = value;
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
            self.mode = Mode::ReadInstr
        }
    }

    /// Captures the in-flight SPI instruction state (not the memory
    /// contents, which live in the `.sav` mmap / firmware file and are not
    /// part of the savestate).
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
pub(crate) enum Instr {
    IR,
    READ(usize, usize),
    RDSR,             // Read Status Register
    WREN,             // Write Enable
    WRDI,             // Write Disable
    PW(usize, usize), // Page Write
}

impl Instr {
    fn get(value: u8) -> Self {
        match value {
            0x00 => Instr::IR,
            0x08 => Instr::IR,
            0x03 => Instr::READ(3, 0),
            0x05 => Instr::RDSR,
            0x06 => Instr::WREN,
            0x04 => Instr::WRDI,
            0x0A => Instr::PW(3, 0),
            _ => unimplemented!("Flash Instr: 0x{:X}", value),
        }
    }
}
