// SPDX-FileCopyrightText: (C) 2016-2026 melonDS team
// SPDX-License-Identifier: GPL-3.0
//
// A close port of desmume's `CHEATS::ARparser` (desmume/src/cheatSystem.cpp),
// now routed through the full ARM7/ARM9 address space (via `HW::arm7_*` /
// `HW::arm9_*`) instead of a main-RAM-only buffer, so that codes which poll
// I/O registers (KEYINPUT, etc.) work the same way they do in desmume.
// - https://github.com/TASEmulators/desmume/blob/a7570473c0c0d3271bf652f534ab8fd584c6dfae/desmume/src/cheatSystem.cpp
use super::mem::MemoryValue;
use crate::hw::HW;

/// Which CPU's address space a cheat instruction currently targets.
/// Mirrors desmume's `st.proc` (`ARMCPU_ARM7` / `ARMCPU_ARM9`), which
/// defaults to ARM7 and can be switched at runtime via the `0xDF` opcode.
#[derive(Clone, Copy, PartialEq)]
enum ArProc {
    Arm7,
    Arm9,
}

impl HW {
    pub fn apply_cheats(&mut self) {
        let enabled_codes: Vec<Vec<u32>> =
            self.cheat_map.iter().filter(|c| c.enabled).map(|c| c.code.clone()).collect();

        for code in &enabled_codes {
            self.run_cheat(code);
        }
    }

    /// Interprets a single AR code's instruction stream, following
    /// desmume's `ARparser` as closely as possible.
    fn run_cheat(&mut self, code: &[u32]) {
        // `code` is a flat sequence of `hi, lo` pairs, mirroring desmume's
        // `CHEATS_LIST::code[i][0]` / `code[i][1]`.
        let num_pairs = (code.len() / 2) as u32;
        if num_pairs == 0 {
            return;
        }

        // Mirrors desmume's `st` struct in `CHEATS::ARparser`.
        let mut status: u32 = 0;

        let mut loop_status: u32 = 0;
        let mut loop_idx: u32 = 0;
        let mut loop_iterations: u32 = 0;
        let mut loop_top: u32 = 0;

        let mut offset: u32 = 0;
        let mut data: u32 = 0;

        // desmume: `st.proc = ARMCPU_ARM7;` -- AR codes default to targeting
        // the ARM7 bus (this is why KEYINPUT-based "hold R+B" codes work:
        // 0x04000130 lives in ARM7/ARM9-shared I/O space).
        let mut proc = ArProc::Arm7;

        let mut i: u32 = 0;
        while i < num_pairs {
            let hi = code[(i * 2) as usize];
            let lo = code[(i * 2 + 1) as usize];

            // Decode the instruction type exactly like desmume: the top
            // nibble normally identifies the type, but 0x0C and 0x0D are
            // "families" that use the full top byte to select a sub-type.
            let nibble = hi >> 28;
            let typ = if nibble == 0x0C || nibble == 0x0D { hi >> 24 } else { nibble };

            // Snapshot the execution status *before* this instruction can
            // push a new nested level. IF-type codes (0x03-0x0A) always
            // push onto the status stack -- even while a parent IF is
            // already false -- so that a later ENDIF (0xD0) pops the
            // correct nesting level. Without this, an ENDIF inside a
            // currently-skipped block desyncs the stack and corrupts the
            // execution status of everything that follows.
            let status_skip = status & 1;
            if (0x03..=0x0A).contains(&typ) {
                status = (status << 1) | 1;
            }

            if typ == 0xD0 || typ == 0xD1 || typ == 0xD2 {
                // condition register is never consulted for these
            } else if typ == 0xC5 {
                // ditto
            } else if typ == 0x0E {
                #[allow(clippy::manual_div_ceil)]
                if status_skip != 0 {
                    // Skip the whole multi-line patch block, but the
                    // inline data pairs still need to be "consumed", same
                    // as desmume.
                    i += (lo + 7) / 8;
                    i += 1;
                    continue;
                }
            } else if status_skip != 0 {
                i += 1;
                continue;
            }

            match typ {
                0x00 => {
                    // 32-bit constant write: 0XXXXXXX YYYYYYYY
                    let addr = (hi & 0x0FFF_FFFF) + offset;
                    self.ar_write::<u32>(proc, addr, lo);
                }
                0x01 => {
                    // 16-bit constant write: 1XXXXXXX 0000YYYY
                    let addr = (hi & 0x0FFF_FFFF) + offset;
                    self.ar_write::<u16>(proc, addr, (lo & 0xFFFF) as u16);
                }
                0x02 => {
                    // 8-bit constant write: 2XXXXXXX 000000YY
                    let addr = (hi & 0x0FFF_FFFF) + offset;
                    self.ar_write::<u8>(proc, addr, (lo & 0xFF) as u8);
                }
                0x03 => {
                    let x = hi & 0x0FFF_FFFF;
                    let addr = if x == 0 { offset } else { x };
                    if lo > self.ar_read::<u32>(proc, addr) {
                        status &= !1;
                    }
                }
                0x04 => {
                    let x = hi & 0x0FFF_FFFF;
                    let addr = if x == 0 { offset } else { x };
                    if lo < self.ar_read::<u32>(proc, addr) {
                        status &= !1;
                    }
                }
                0x05 => {
                    let x = hi & 0x0FFF_FFFF;
                    let addr = if x == 0 { offset } else { x };
                    if lo == self.ar_read::<u32>(proc, addr) {
                        status &= !1;
                    }
                }
                0x06 => {
                    let x = hi & 0x0FFF_FFFF;
                    let addr = if x == 0 { offset } else { x };
                    if lo != self.ar_read::<u32>(proc, addr) {
                        status &= !1;
                    }
                }
                0x07 => {
                    let x = hi & 0x0FFF_FFFF;
                    let addr = if x == 0 { offset } else { x };
                    let y = (lo & 0xFFFF) as u16;
                    let z = (lo >> 16) as u16;
                    let chk = (!z) & self.ar_read::<u16>(proc, addr);
                    if y > chk {
                        status &= !1;
                    }
                }
                0x08 => {
                    let x = hi & 0x0FFF_FFFF;
                    let addr = if x == 0 { offset } else { x };
                    let y = (lo & 0xFFFF) as u16;
                    let z = (lo >> 16) as u16;
                    let chk = (!z) & self.ar_read::<u16>(proc, addr);
                    if y < chk {
                        status &= !1;
                    }
                }
                0x09 => {
                    let x = hi & 0x0FFF_FFFF;
                    let addr = if x == 0 { offset } else { x };
                    let y = (lo & 0xFFFF) as u16;
                    let z = (lo >> 16) as u16;
                    let chk = (!z) & self.ar_read::<u16>(proc, addr);
                    if y == chk {
                        status &= !1;
                    }
                }
                0x0A => {
                    let x = hi & 0x0FFF_FFFF;
                    let addr = if x == 0 { offset } else { x };
                    let y = (lo & 0xFFFF) as u16;
                    let z = (lo >> 16) as u16;
                    let chk = (!z) & self.ar_read::<u16>(proc, addr);
                    if y != chk {
                        status &= !1;
                    }
                }
                0x0B => {
                    // offset = u32[XXXXXXX + offset]
                    let addr = (hi & 0x0FFF_FFFF) + offset;
                    offset = self.ar_read::<u32>(proc, addr);
                }
                0xC0 => {
                    // FOR 0..=YYYYYYYY
                    loop_iterations = lo;
                    loop_idx = 0;
                    loop_top = i;
                    loop_status = status;
                }
                0xC4 => {
                    // "Code Hack" self-rewrite trick -- not supported,
                    // same as desmume; log and move on instead of
                    // aborting the whole cheat.
                    // TODO: warn log -- unsupported C4 code
                }
                0xC5 => {
                    // If-Counter trainer-toolkit code -- unsupported,
                    // same as desmume ("Unsupported C5 code"); no
                    // functional effect.
                    // TODO: warn log -- unsupported C5 code
                }
                0xC6 => {
                    // u32[XXXXXXXX] = offset
                    self.ar_write::<u32>(proc, lo, offset);
                }
                0xD0 => {
                    // ENDIF: restore previous execution status
                    status >>= 1;
                }
                0xD1 => {
                    // NEXT
                    status = loop_status;
                    if loop_idx < loop_iterations {
                        loop_idx += 1;
                        i = loop_top;
                    }
                }
                0xD2 => {
                    // NEXT+FLUSH
                    status = loop_status;
                    if loop_idx < loop_iterations {
                        loop_idx += 1;
                        i = loop_top;
                    } else {
                        status = 0;
                        loop_status = 0;
                        loop_idx = 0;
                        loop_iterations = 0;
                        loop_top = 0;
                        offset = 0;
                        data = 0;
                        proc = ArProc::Arm7;
                    }
                }
                0xD3 => {
                    offset = lo;
                }
                0xD4 => {
                    data = data.wrapping_add(lo);
                }
                0xD5 => {
                    data = lo;
                }
                0xD6 => {
                    let addr = lo + offset;
                    self.ar_write::<u32>(proc, addr, data);
                    offset += 4;
                }
                0xD7 => {
                    let addr = lo + offset;
                    self.ar_write::<u16>(proc, addr, data as u16);
                    offset += 2;
                }
                0xD8 => {
                    let addr = lo + offset;
                    self.ar_write::<u8>(proc, addr, data as u8);
                    offset += 1;
                }
                0xD9 => {
                    let addr = lo + offset;
                    data = self.ar_read::<u32>(proc, addr);
                }
                0xDA => {
                    let addr = lo + offset;
                    data = self.ar_read::<u16>(proc, addr) as u32;
                }
                0xDB => {
                    let addr = lo + offset;
                    data = self.ar_read::<u8>(proc, addr) as u32;
                }
                0xDC => {
                    offset += lo;
                }
                0xDF => {
                    // Emulator-specific "force CPU target" pseudo-code:
                    // DFFFFFFF 99999999 -> target ARM9
                    // DFFFFFFF 77777777 -> target ARM7
                    if hi == 0xDFFF_FFFF {
                        if lo == 0x9999_9999 {
                            proc = ArProc::Arm9;
                        } else if lo == 0x7777_7777 {
                            proc = ArProc::Arm7;
                        }
                    }
                }
                0x0E => {
                    // Patch Code: copy YYYYYYYY bytes from the inline data
                    // that follows this instruction to [XXXXXXXX+offset].
                    let x = hi & 0x0FFF_FFFF;
                    let mut remaining = lo;
                    let mut dst = x + offset;

                    let mut t: u32 = 0;
                    let mut shift: u32 = 0;

                    if remaining > 0 {
                        i += 1; // skip over the current instruction
                    }
                    while remaining >= 4 {
                        if i >= num_pairs {
                            break;
                        }
                        let tmp = code[(i * 2 + t) as usize];
                        if t == 1 {
                            i += 1;
                        }
                        t ^= 1;
                        self.ar_write::<u32>(proc, dst, tmp);
                        dst += 4;
                        remaining -= 4;
                    }
                    while remaining > 0 {
                        if i >= num_pairs {
                            break;
                        }
                        let tmp = (code[(i * 2 + t) as usize] >> shift) as u8;
                        self.ar_write::<u8>(proc, dst, tmp);
                        dst += 1;
                        remaining -= 1;
                        shift += 4;
                    }

                    if t == 0 {
                        i = i.wrapping_sub(1);
                    }
                }
                0x0F => {
                    // Memory Copy Code: copy YYYYYYYY bytes from
                    // [offset..] to [XXXXXXXX...]
                    let x = hi & 0x0FFF_FFFF;
                    let mut remaining = lo;
                    let mut src = offset;
                    let mut dst = x;

                    while remaining >= 4 {
                        if i >= num_pairs {
                            break;
                        }
                        let v = self.ar_read::<u32>(proc, src);
                        self.ar_write::<u32>(proc, dst, v);
                        src += 4;
                        dst += 4;
                        remaining -= 4;
                    }
                    while remaining > 0 {
                        if i >= num_pairs {
                            break;
                        }
                        let v = self.ar_read::<u8>(proc, src);
                        self.ar_write::<u8>(proc, dst, v);
                        src += 1;
                        dst += 1;
                        remaining -= 1;
                    }
                }
                _ => {
                    // Unknown opcode: desmume just logs and moves on to
                    // the next instruction rather than aborting the whole
                    // cheat.
                    // TODO: warn log -- unknown AR opcode
                }
            }

            i += 1;
        }
    }

    /// Dispatches a read to the ARM7 or ARM9 bus depending on the AR
    /// engine's current `proc` target. Mirrors desmume's
    /// `_MMU_read{8,16,32}(st.proc, MMU_AT_DEBUG, addr)`.
    #[inline]
    fn ar_read<T: MemoryValue>(&mut self, proc: ArProc, addr: u32) -> T {
        match proc {
            ArProc::Arm7 => self.arm7_read(addr),
            ArProc::Arm9 => self.arm9_read(addr),
        }
    }

    /// Dispatches a write to the ARM7 or ARM9 bus depending on the AR
    /// engine's current `proc` target. Mirrors desmume's
    /// `CHEATS::DirectWrite<LENGTH>(st.proc, addr, value)`.
    #[inline]
    fn ar_write<T: MemoryValue>(&mut self, proc: ArProc, addr: u32, value: T) {
        match proc {
            ArProc::Arm7 => self.arm7_write(addr, value),
            ArProc::Arm9 => self.arm9_write(addr, value),
        }
    }
}
