//! NAND save memory (WarioWare D.I.Y., Jam with the Band): 8-64 MiB on
//! the ROM bus, reached with KEY2 commands instead of AUXSPI.
//!
//! ```text
//!  ROM address space:  [ ROM ............ | NAND save (base = header 96h << 17) ]
//!
//!  B2 addr      open a 128 KiB window at `addr` (0 = back to ROM reads: 8B)
//!  B7 addr      read words from the window (NAND mode) or from the ROM
//!  85           write enable (status bit 4)
//!  81 addr      x4 commands of 512 bytes fill the 2 KiB write buffer
//!  82           commit the buffer to `addr`        84  discard it
//!  94           NAND ID (30h bytes)                D6  status (bit 5 ready)
//!
//!  the last 128 KiB of the save area are read-only: FFh plus the ID
//! ```
//!
//! A port of melonDS `CartRetailNAND`.

use super::{BackupMemory, SaveChip};

/// Window size opened by command B2.
const WINDOW: u32 = 0x2_0000;
/// Size of the write buffer that command 82 commits.
const PAGE: usize = 0x800;
/// NAND ID found in the read-only area (from a Jam with the Band card).
const ID: [u8; 16] = [
    0xEC, 0x00, 0x9E, 0xA1, 0x51, 0x65, 0x34, 0x35, 0x30, 0x35, 0x30, 0x31, 0x19, 0x19, 0x02, 0x0A,
];

/// NAND controller state.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct Nand {
    /// ROM address of the save area.
    base: u32,
    /// Open window (0 = ROM mode).
    window: u32,
    /// Current read / write address (0 = none).
    addr: u32,
    /// Bit 5 ready, bit 4 write enabled.
    status: u8,
    write_buf: Vec<u8>,
    write_pos: usize,
    write_len: usize,
    /// Command being transferred.
    cmd: u8,
    /// Read position within the ID (command 94).
    id_pos: usize,
}

impl Nand {
    pub(super) fn new() -> Self {
        Self {
            base: 0,
            window: 0,
            addr: 0,
            status: 0x20,
            write_buf: vec![0; PAGE],
            write_pos: 0,
            write_len: 0,
            cmd: 0,
            id_pos: 0,
        }
    }
}

/// What a ROM command means to a NAND card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NandCommand {
    /// Not a NAND command: the normal ROM protocol handles it.
    Rom,
    /// Handled; data words (if any) come from [`BackupMemory::nand_receive`]
    /// or go to [`BackupMemory::nand_transmit`].
    Nand,
}

const fn be32(cmd: &[u8; 8]) -> u32 {
    u32::from_be_bytes([cmd[1], cmd[2], cmd[3], cmd[4]])
}

impl BackupMemory {
    /// Whether this is a NAND card.
    pub fn is_nand(&self) -> bool {
        self.chip == SaveChip::Nand
    }

    /// Sets the save area start from the ROM header (u16 at 96h, in
    /// 128 KiB units) and rebuilds the read-only ID area.
    pub fn nand_setup(&mut self, rom_header_96: u16) {
        if let Some(n) = &mut self.nand {
            n.base = u32::from(rom_header_96) << 17;
        }
        let len = self.data.len();
        if len > WINDOW as usize {
            self.data[len - WINDOW as usize..].fill(0xFF);
            self.data[len - PAGE..len - PAGE + 16].copy_from_slice(&ID);
        }
    }

    /// Whether a ROM-mode B7 read at `addr` falls in the save area (it
    /// reads FFh there).
    pub fn nand_hides(&self, addr: u32) -> bool {
        self.nand.as_ref().is_some_and(|n| {
            // base 0 would hide the whole ROM: no real NAND card has it
            n.base != 0 && n.window == 0 && addr.wrapping_sub(n.base) < self.data.len() as u32
        })
    }

    /// A KEY2-mode ROM command was started.
    pub fn nand_command(&mut self, cmd: &[u8; 8]) -> NandCommand {
        let len = self.data.len() as u32;
        let Some(n) = &mut self.nand else { return NandCommand::Rom };
        let in_save = |w: u32, base: u32| w >= base && w < base + len;
        n.cmd = cmd[0];
        match cmd[0] {
            0x81 => {
                // write data (issued 4 times with the same address)
                let addr = be32(cmd);
                if n.status & 0x10 != 0 && in_save(n.window, n.base) {
                    if addr >= n.window && addr < n.window + WINDOW && n.addr == 0 {
                        n.addr = addr;
                    }
                } else {
                    n.addr = 0;
                }
            }
            0x82 => {
                // commit write (not into the read-only last 128 KiB)
                let commit = n.addr != 0 && n.write_len != 0 && n.addr < n.base + len - WINDOW;
                let offset = n.addr.wrapping_sub(n.base) as usize;
                let buf = std::mem::take(&mut n.write_buf);
                n.addr = 0;
                n.write_pos = 0;
                n.write_len = 0;
                n.status &= !0x10;
                if commit {
                    for (i, &b) in buf.iter().enumerate() {
                        self.data[(offset + i) % len as usize] = b;
                    }
                    self.mark_written();
                }
                if let Some(n) = &mut self.nand {
                    n.write_buf = buf;
                }
            }
            0x84 => {
                // discard write buffer
                n.addr = 0;
                n.write_pos = 0;
            }
            0x85 => {
                // write enable
                if n.window != 0 {
                    n.status |= 0x10;
                    n.write_pos = 0;
                }
            }
            0x8B => n.window = 0, // back to ROM reads
            0x94 => n.id_pos = 0,
            0xB2 => n.window = u32::from_be_bytes([cmd[1], cmd[2] & 0xFE, 0, 0]),
            0xB7 if n.window != 0 => {
                let addr = be32(cmd);
                n.addr =
                    if in_save(n.window, n.base) && addr >= n.window && addr < n.window + WINDOW {
                        addr
                    } else {
                        0
                    };
            }
            0xD6 => {}
            _ => return NandCommand::Rom,
        }
        NandCommand::Nand
    }

    /// Next data word of the current NAND command.
    pub fn nand_receive(&mut self) -> u32 {
        let len = self.data.len() as u32;
        let Some(n) = &mut self.nand else { return u32::MAX };
        match n.cmd {
            0x94 => {
                // ID: 0..17h zero-ish header, then the ID at 18h
                let mut id = [0_u8; 0x30];
                id[..5].copy_from_slice(&[0xEC, 0xF1, 0x00, 0x95, 0x40]);
                id[0x18..0x28].copy_from_slice(&ID);
                let w = id
                    .get(n.id_pos..n.id_pos + 4)
                    .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                n.id_pos += 4;
                w
            }
            0xB7 if n.addr != 0 => {
                // reads wrap within a 4 KiB page
                let off = (n.addr - n.base) & (len - 1);
                let (hi, lo) = (off & !0xFFF, off & 0xFFF);
                let byte = |i: u32| self.data[(hi | ((lo + i) & 0xFFF)) as usize];
                let w = u32::from_le_bytes([byte(0), byte(1), byte(2), byte(3)]);
                n.addr = n.base + (hi | ((lo + 4) & 0xFFF));
                w
            }
            0xD6 => u32::from(n.status) * 0x0101_0101,
            _ => u32::MAX,
        }
    }

    /// A data word written by the CPU during a NAND command (81).
    pub fn nand_transmit(&mut self, value: u32) {
        let Some(n) = &mut self.nand else { return };
        if n.cmd == 0x81 && n.addr != 0 {
            n.write_buf[n.write_pos..n.write_pos + 4].copy_from_slice(&value.to_le_bytes());
            n.write_pos = (n.write_pos + 4) % PAGE;
            n.write_len = (n.write_len + 4).min(PAGE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(op: u8, addr: u32) -> [u8; 8] {
        let a = addr.to_be_bytes();
        [op, a[0], a[1], a[2], a[3], 0, 0, 0]
    }

    #[test]
    fn write_then_read_back() {
        let mut m = BackupMemory::new(8); // 8 MiB
        m.nand_setup(0x80); // save at 16 MiB
        let base = 0x80 << 17;
        assert!(m.nand_hides(base + 4));
        m.nand_command(&cmd(0xB2, base));
        m.nand_command(&cmd(0x85, 0));
        m.nand_command(&cmd(0x81, base + 0x800));
        for i in 0..512 {
            m.nand_transmit(i);
        }
        m.nand_command(&cmd(0x82, 0));
        assert!(m.is_dirty());

        m.nand_command(&cmd(0xB7, base + 0x800));
        assert_eq!(m.nand_receive(), 0);
        assert_eq!(m.nand_receive(), 1);
        m.nand_command(&cmd(0xD6, 0));
        assert_eq!(m.nand_receive() & 0x30, 0x20, "ready, write disabled");
    }
}
