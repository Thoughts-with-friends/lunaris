// SPDX-FileCopyrightText: (C) 2016-2026 melonDS team
// SPDX-License-Identifier: GPL-3.0
// https://github.com/melonDS-emu/melonDS/blob/10a173b5536fc75cd93f8a3868349dad963542ef/src/AREngine.cpp#L42
use crate::hw::HW;

// DRAM: 02000000-023FFFFF
const MAIN_MEM_MASK: u32 = 0x003F_FFFF; // 4 MB

impl HW {
    pub fn apply_cheats(&mut self) {
        // Clone each code's instruction stream up front to avoid holding an
        // immutable borrow of `self.cheats` while `run_cheat` mutably
        // borrows `self.main_mem`. Cheat code lists are tiny, so the copy
        // is negligible.
        let enabled_codes: Vec<Vec<u32>> =
            self.cheat_map.iter().filter(|c| c.enabled).map(|c| c.code.clone()).collect();

        for code in &enabled_codes {
            self.run_cheat(code);
        }
    }

    /// Interprets a single AR code's instruction stream.
    ///
    /// This is a direct port of melonDS's `AREngine::RunCheat`, adapted to
    /// operate only on main RAM (see `read32`/`write32` etc. below) instead
    /// of the full ARM7 address space.
    fn run_cheat(&mut self, code: &[u32]) {
        let mut pc: usize = 0;
        let mut offset: u32 = 0;
        let mut datareg: u32 = 0;
        let mut cond: u32 = 1;
        let mut cond_stack: u32 = 0;

        let mut loop_start: usize = 0;
        let mut loop_count: u32 = 0;
        let mut loop_cond: u32 = 1;
        let mut loop_cond_stack: u32 = 0;

        // TODO: does anything reset this across cheat runs?
        let mut c5_count: u32 = 0;

        loop {
            if pc + 1 >= code.len() {
                break;
            }

            let a = code[pc];
            let b = code[pc + 1];
            pc += 2;

            let op = (a >> 24) as u8;

            if ((op < 0xD0 && op != 0xC5) || op > 0xD2) && cond == 0 {
                if (op & 0xF0) == 0xE0 {
                    let mut i = 0u32;
                    while i < b {
                        pc += 2;
                        i += 8;
                    }
                }
                continue;
            }

            match op {
                0x00..=0x0F => {
                    // 32-bit write
                    self.write32((a & 0x0FFF_FFFF) + offset, b);
                }
                0x10..=0x1F => {
                    // 16-bit write
                    self.write16((a & 0x0FFF_FFFF) + offset, b as u16);
                }
                0x20..=0x2F => {
                    // 8-bit write
                    self.write8((a & 0x0FFF_FFFF) + offset, b as u8);
                }
                0x30..=0x3F => {
                    // IF b > u32[a]
                    cond_stack = (cond_stack << 1) | cond;
                    let addr = if a & 0x0FFF_FFFF == 0 { offset } else { a & 0x0FFF_FFFF };
                    let chk = self.read32(addr);
                    cond = (b > chk) as u32;
                }
                0x40..=0x4F => {
                    // IF b < u32[a]
                    cond_stack = (cond_stack << 1) | cond;
                    let addr = if a & 0x0FFF_FFFF == 0 { offset } else { a & 0x0FFF_FFFF };
                    let chk = self.read32(addr);
                    cond = (b < chk) as u32;
                }
                0x50..=0x5F => {
                    // IF b == u32[a]
                    cond_stack = (cond_stack << 1) | cond;
                    let addr = if a & 0x0FFF_FFFF == 0 { offset } else { a & 0x0FFF_FFFF };
                    let chk = self.read32(addr);
                    cond = (b == chk) as u32;
                }
                0x60..=0x6F => {
                    // IF b != u32[a]
                    cond_stack = (cond_stack << 1) | cond;
                    let addr = if a & 0x0FFF_FFFF == 0 { offset } else { a & 0x0FFF_FFFF };
                    let chk = self.read32(addr);
                    cond = (b != chk) as u32;
                }
                0x70..=0x7F => {
                    // IF b.l > ((~b.h) & u16[a])
                    cond_stack = (cond_stack << 1) | cond;
                    let addr = if a & 0x0FFF_FFFF == 0 { offset } else { a & 0x0FFF_FFFF };
                    let val = self.read16(addr);
                    let chk = (!(b >> 16) as u16) & val;
                    cond = ((b as u16) > chk) as u32;
                }
                0x80..=0x8F => {
                    // IF b.l < ((~b.h) & u16[a])
                    cond_stack = (cond_stack << 1) | cond;
                    let addr = if a & 0x0FFF_FFFF == 0 { offset } else { a & 0x0FFF_FFFF };
                    let val = self.read16(addr);
                    let chk = (!(b >> 16) as u16) & val;
                    cond = ((b as u16) < chk) as u32;
                }
                0x90..=0x9F => {
                    // IF b.l == ((~b.h) & u16[a])
                    cond_stack = (cond_stack << 1) | cond;
                    let addr = if a & 0x0FFF_FFFF == 0 { offset } else { a & 0x0FFF_FFFF };
                    let val = self.read16(addr);
                    let chk = (!(b >> 16) as u16) & val;
                    cond = ((b as u16) == chk) as u32;
                }
                0xA0..=0xAF => {
                    // IF b.l != ((~b.h) & u16[a])
                    cond_stack = (cond_stack << 1) | cond;
                    let addr = if a & 0x0FFF_FFFF == 0 { offset } else { a & 0x0FFF_FFFF };
                    let val = self.read16(addr);
                    let chk = (!(b >> 16) as u16) & val;
                    cond = ((b as u16) != chk) as u32;
                }
                0xB0..=0xBF => {
                    // offset = u32[a + offset]
                    offset = self.read32((a & 0x0FFF_FFFF) + offset);
                }
                0xC0 => {
                    // FOR 0..b
                    loop_start = pc; // first opcode after FOR
                    loop_count = b;
                    loop_cond = cond;
                    loop_cond_stack = cond_stack;
                }
                0xC4 => {
                    // Self-modifying "pointer to this opcode" trick.
                    // Not supported upstream either; bail out like melonDS does.
                    // TODO: warn log — unsupported C4000000 opcode
                    return;
                }
                0xC5 => {
                    // count++ / IF (count & b.l) == b.h
                    c5_count += 1;
                    if cond != 0 {
                        cond_stack = (cond_stack << 1) | cond;
                        let mask = b as u16;
                        let chk = (b >> 16) as u16;
                        cond = ((c5_count as u16 & mask) == chk) as u32;
                    }
                }
                0xC6 => {
                    // u32[b] = offset
                    self.write32(b, offset);
                }
                0xD0 => {
                    // ENDIF
                    cond = cond_stack & 0x1;
                    cond_stack >>= 1;
                }
                0xD1 => {
                    // NEXT
                    if loop_count > 0 {
                        loop_count -= 1;
                        pc = loop_start;
                    } else {
                        cond = loop_cond;
                        cond_stack = loop_cond_stack;
                    }
                }
                0xD2 => {
                    // NEXT+FLUSH
                    if loop_count > 0 {
                        loop_count -= 1;
                        pc = loop_start;
                    } else {
                        offset = 0;
                        datareg = 0;
                        cond_stack = 0;
                        cond = 1;
                    }
                }
                0xD3 => {
                    // offset = b
                    offset = b;
                }
                0xD4 => {
                    // data op
                    match a & 0xFF {
                        0x00 => datareg = datareg.wrapping_add(b),
                        0x01 => datareg |= b,
                        0x02 => datareg &= b,
                        0x03 => datareg ^= b,
                        0x04 => {
                            let shift = b & 0xFF;
                            datareg = if shift > 31 { 0 } else { datareg << shift };
                        }
                        0x05 => {
                            let shift = b & 0xFF;
                            datareg = if shift > 31 { 0 } else { datareg >> shift };
                        }
                        0x06 => datareg = datareg.rotate_right(b & 0x1F),
                        0x07 => {
                            let shift = b & 0xFF;
                            datareg = if shift > 31 {
                                ((datareg as i32) >> 31) as u32
                            } else {
                                ((datareg as i32) >> shift) as u32
                            };
                        }
                        0x08 => datareg = datareg.wrapping_mul(b),
                        _ => {
                            // TODO: warn log — bad AR D4 sub-opcode
                        }
                    }
                }
                0xD5 => {
                    // datareg = b
                    datareg = b;
                }
                0xD6 => {
                    // u32[b+offset] = datareg / offset += 4
                    self.write32(b + offset, datareg);
                    offset += 4;
                }
                0xD7 => {
                    // u16[b+offset] = datareg / offset += 2
                    self.write16(b + offset, datareg as u16);
                    offset += 2;
                }
                0xD8 => {
                    // u8[b+offset] = datareg / offset += 1
                    self.write8(b + offset, datareg as u8);
                    offset += 1;
                }
                0xD9 => {
                    // datareg = u32[b+offset]
                    datareg = self.read32(b + offset);
                }
                0xDA => {
                    // datareg = u16[b+offset]
                    datareg = self.read16(b + offset) as u32;
                }
                0xDB => {
                    // datareg = u8[b+offset]
                    datareg = self.read8(b + offset) as u32;
                }
                0xDC => {
                    // offset += b
                    offset += b;
                }
                0xE0..=0xEF => {
                    // Copy `b` bytes from the inline code stream to a+offset.
                    let mut dst = (a & 0x0FFF_FFFF) + offset;
                    let mut remaining = b;

                    while remaining >= 8 {
                        if pc + 1 >= code.len() {
                            break;
                        }
                        self.write32(dst, code[pc]);
                        dst += 4;
                        self.write32(dst, code[pc + 1]);
                        dst += 4;
                        pc += 2;
                        remaining -= 8;
                    }

                    if remaining > 0 && pc < code.len() {
                        let mut leftover = [0u8; 8];
                        leftover[0..4].copy_from_slice(&code[pc].to_le_bytes());
                        if pc + 1 < code.len() {
                            leftover[4..8].copy_from_slice(&code[pc + 1].to_le_bytes());
                        }
                        pc += 2;

                        let mut idx = 0usize;
                        if remaining >= 4 {
                            self.write32(
                                dst,
                                u32::from_le_bytes(leftover[0..4].try_into().unwrap()),
                            );
                            dst += 4;
                            idx += 4;
                            remaining -= 4;
                        }
                        while remaining > 0 {
                            self.write8(dst, leftover[idx]);
                            dst += 1;
                            idx += 1;
                            remaining -= 1;
                        }
                    }
                }
                0xF0..=0xFF => {
                    // Copy `b` bytes from address `offset` to address `a`.
                    let mut src = offset;
                    let mut dst = a & 0x0FFF_FFFF;
                    let mut remaining = b;

                    while remaining >= 4 {
                        let v = self.read32(src);
                        self.write32(dst, v);
                        src += 4;
                        dst += 4;
                        remaining -= 4;
                    }
                    while remaining > 0 {
                        let v = self.read8(src);
                        self.write8(dst, v);
                        src += 1;
                        dst += 1;
                        remaining -= 1;
                    }
                }
                _ => {
                    // TODO: warn log — bad AR opcode
                    return;
                }
            }
        }
    }

    // ---- Main RAM accessors (masked into the 4 MB DRAM window) ----

    fn read32(&self, addr: u32) -> u32 {
        HW::read_mem::<u32>(&self.main_mem, addr & MAIN_MEM_MASK)
    }

    fn read16(&self, addr: u32) -> u16 {
        HW::read_mem::<u16>(&self.main_mem, addr & MAIN_MEM_MASK)
    }

    fn read8(&self, addr: u32) -> u8 {
        HW::read_mem::<u8>(&self.main_mem, addr & MAIN_MEM_MASK)
    }

    fn write32(&mut self, addr: u32, value: u32) {
        HW::write_mem::<u32>(&mut self.main_mem, addr & MAIN_MEM_MASK, value);
    }

    fn write16(&mut self, addr: u32, value: u16) {
        HW::write_mem::<u16>(&mut self.main_mem, addr & MAIN_MEM_MASK, value);
    }

    fn write8(&mut self, addr: u32, value: u8) {
        HW::write_mem::<u8>(&mut self.main_mem, addr & MAIN_MEM_MASK, value);
    }
}
