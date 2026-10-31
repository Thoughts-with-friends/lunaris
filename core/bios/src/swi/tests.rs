use super::{SwiContext, arm7, arm9};

/// A THUMB CPU with 64 KiB of flat memory; the SWI opcode sits at 0xF000.
struct Mock {
    mem: Vec<u8>,
    regs: [u32; 16],
    halted: bool,
    restarted: bool,
}

const SWI_AT: u32 = 0xF000;
const FLAGS_AT: u32 = 0x3FF8;

impl Mock {
    fn new() -> Self {
        Self { mem: vec![0; 0x1_0000], regs: [0; 16], halted: false, restarted: false }
    }

    fn put(&mut self, addr: u32, bytes: &[u8]) {
        self.mem[addr as usize..addr as usize + bytes.len()].copy_from_slice(bytes);
    }

    fn get(&self, addr: u32, len: usize) -> &[u8] {
        &self.mem[addr as usize..addr as usize + len]
    }

    /// Runs `SWI n` on the ARM9 (or ARM7) with r0-r3.
    fn swi(&mut self, arm9_cpu: bool, n: u8, r: [u32; 4]) {
        self.regs[..4].copy_from_slice(&r);
        self.put(SWI_AT, &(0xDF00_u16 | u16::from(n)).to_le_bytes());
        self.regs[15] = SWI_AT + 4; // THUMB: two instructions ahead
        self.halted = false;
        self.restarted = false;
        if arm9_cpu {
            arm9(self);
        } else {
            arm7(self);
        }
    }
}

impl SwiContext for Mock {
    fn reg(&self, index: usize) -> u32 {
        self.regs[index]
    }
    fn set_reg(&mut self, index: usize, value: u32) {
        self.regs[index] = value;
    }
    fn pc(&self) -> u32 {
        self.regs[15]
    }
    fn is_thumb(&self) -> bool {
        true
    }
    fn read8(&mut self, addr: u32) -> u8 {
        self.mem.get(addr as usize).copied().unwrap_or(0)
    }
    fn read16(&mut self, addr: u32) -> u16 {
        u16::from_le_bytes([self.read8(addr), self.read8(addr + 1)])
    }
    fn read32(&mut self, addr: u32) -> u32 {
        u32::from(self.read16(addr)) | u32::from(self.read16(addr + 2)) << 16
    }
    fn write8(&mut self, addr: u32, value: u8) {
        if let Some(b) = self.mem.get_mut(addr as usize) {
            *b = value;
        }
    }
    fn write16(&mut self, addr: u32, value: u16) {
        let [a, b] = value.to_le_bytes();
        self.write8(addr, a);
        self.write8(addr + 1, b);
    }
    fn write32(&mut self, addr: u32, value: u32) {
        self.write16(addr, value as u16);
        self.write16(addr + 2, (value >> 16) as u16);
    }
    fn add_cycles(&mut self, _cycles: i32) {}
    fn halt(&mut self) {
        self.halted = true;
    }
    fn irq_check_flags_addr(&self) -> u32 {
        FLAGS_AT
    }
    fn restart_swi(&mut self) {
        self.restarted = true;
    }
}

#[test]
fn lz77_copies_back_references() {
    let mut m = Mock::new();
    // "ABC" literals, then 6 bytes from 3 back.
    m.put(0x100, &[0x10, 9, 0, 0, 0x10, b'A', b'B', b'C', 0x30, 0x02]);
    m.swi(true, 0x11, [0x100, 0x200, 0, 0]);
    assert_eq!(m.get(0x200, 9), b"ABCABCABC");
    m.swi(true, 0x12, [0x100, 0x300, 0, 0]);
    assert_eq!(m.get(0x300, 9), b"ABCABCABC");
}

#[test]
fn run_length_mixes_literals_and_runs() {
    let mut m = Mock::new();
    m.put(0x100, &[0x30, 7, 0, 0, 0x01, b'a', b'b', 0x82, b'X']);
    m.swi(false, 0x14, [0x100, 0x200, 0, 0]);
    assert_eq!(m.get(0x200, 7), b"abXXXXX");
}

#[test]
fn huffman_walks_the_tree() {
    let mut m = Mock::new();
    // Root 0xC0: both children are data, 'a' for bit 0 and 'b' for bit 1.
    m.put(0x100, &[0x28, 4, 0, 0, 1, 0xC0, b'a', b'b']);
    m.put(0x108, &0x6000_0000_u32.to_le_bytes()); // bits 0 1 1 0
    m.swi(true, 0x13, [0x100, 0x200, 0, 0]);
    assert_eq!(m.get(0x200, 4), b"abba");
}

#[test]
fn bit_unpack_widens_units() {
    let mut m = Mock::new();
    m.put(0x100, &[0b1101_0010]);
    m.put(0x110, &[1, 0, 1, 4, 0, 0, 0, 0]); // 1 byte, 1 -> 4 bits, offset 0
    m.swi(true, 0x10, [0x100, 0x200, 0x110, 0]);
    assert_eq!(m.get(0x200, 4), &0x1101_0010_u32.to_le_bytes());
}

#[test]
fn diff_unfilter_sums_units() {
    let mut m = Mock::new();
    m.put(0x100, &[0x81, 4, 0, 0, 1, 1, 2, 0xFF]);
    m.swi(true, 0x16, [0x100, 0x200, 0, 0]);
    assert_eq!(m.get(0x200, 4), &[1, 2, 4, 3]);
}

#[test]
fn div_and_sqrt() {
    let mut m = Mock::new();
    m.swi(true, 0x09, [(-7_i32) as u32, 2, 0, 0]);
    assert_eq!((m.regs[0] as i32, m.regs[1] as i32, m.regs[3]), (-3, -1, 3));
    m.swi(false, 0x0D, [1_000_000, 0, 0, 0]);
    assert_eq!(m.regs[0], 1000);
}

#[test]
fn cpu_fast_set_rounds_up_to_eight_words() {
    let mut m = Mock::new();
    m.put(0x100, &0xAABB_CCDD_u32.to_le_bytes());
    m.swi(true, 0x0C, [0x100, 0x200, (1 << 24) | 1, 0]);
    for i in 0..8 {
        assert_eq!(m.get(0x200 + i * 4, 4), &0xAABB_CCDD_u32.to_le_bytes());
    }
    assert_eq!(m.get(0x220, 4), &[0; 4]);
}

#[test]
fn intr_wait_waits_for_a_new_flag() {
    let mut m = Mock::new();
    m.put(FLAGS_AT, &1_u32.to_le_bytes()); // old VBlank flag
    m.swi(true, 0x05, [0; 4]);
    assert!(m.halted && m.restarted, "discards the old flag and waits");
    assert_eq!(m.regs[0], 0, "the restarted SWI does not discard again");

    m.put(FLAGS_AT, &3_u32.to_le_bytes()); // handler raised VBlank + HBlank
    m.swi(true, 0x04, [0, 1, 0, 0]);
    assert!(!m.halted && !m.restarted);
    assert_eq!(m.get(FLAGS_AT, 4), &2_u32.to_le_bytes(), "only the waited flag is cleared");
}
