// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! High-level BIOS: the BIOS functions games call with `SWI`, in Rust.
//!
//! Used instead of running BIOS code on the emulated CPU, unless the
//! replacement BIOS images in [`crate::image`] are selected. Covers the
//! same functions as melonDS's FreeBIOS (the DS, not DSi, set).
//!
//! ```text
//!  game code: SWI #n
//!       |  (CPU exception, intercepted by the system)
//!       v
//!  swi::arm9(ctx) / swi::arm7(ctx)  -- decode n, read r0-r3, do the work
//!       |
//!       v
//!  SwiContext  -- registers, memory and halt of the calling CPU,
//!                 implemented by the system crate
//! ```
//!
//! | SWI | function            | SWI | function                         |
//! |-----|---------------------|-----|----------------------------------|
//! | 03  | WaitByLoop          | 10  | BitUnPack                        |
//! | 04  | IntrWait            | 11  | LZ77UnComp (8-bit writes)        |
//! | 05  | VBlankIntrWait      | 12  | LZ77UnComp (16-bit writes)       |
//! | 06  | Halt                | 13  | HuffUnComp                       |
//! | 07  | Sleep (ARM7)        | 14  | RLUnComp (8-bit writes)          |
//! | 08  | SoundBias (ARM7)    | 15  | RLUnComp (16-bit writes)         |
//! | 09  | Div                 | 16  | Diff8bitUnFilter (ARM9)          |
//! | 0B  | CpuSet              | 18  | Diff16bitUnFilter (ARM9)         |
//! | 0C  | CpuFastSet          | 1A-1C | Sine/Pitch/Volume table (ARM7) |
//! | 0D  | Sqrt                | 1D  | GetBootProcs (ARM7)              |
//! | 0E  | GetCRC16            | 1F  | CustomPost                       |
//! | 0F  | IsDebugger          |     |                                  |
//!
//! See GBATEK "BIOS Functions".
mod decompress;
mod tables;
#[cfg(test)]
mod tests;

/// What a BIOS function needs from the CPU that executed `SWI`.
pub trait SwiContext {
    /// Register `index` (r0-r15) of the calling CPU.
    fn reg(&self, index: usize) -> u32;
    /// Sets register `index` of the calling CPU (results in r0-r3).
    fn set_reg(&mut self, index: usize, value: u32);
    /// PC of the calling CPU while the SWI is handled (the `SWI`
    /// instruction is found relative to it).
    fn pc(&self) -> u32;
    /// Whether `SWI` was executed in THUMB state.
    fn is_thumb(&self) -> bool;

    /// Byte read through the calling CPU's bus.
    fn read8(&mut self, addr: u32) -> u8;
    /// Halfword read through the calling CPU's bus.
    fn read16(&mut self, addr: u32) -> u16;
    /// Word read through the calling CPU's bus.
    fn read32(&mut self, addr: u32) -> u32;
    /// Byte write through the calling CPU's bus.
    fn write8(&mut self, addr: u32, value: u8);
    /// Halfword write through the calling CPU's bus.
    fn write16(&mut self, addr: u32, value: u16);
    /// Word write through the calling CPU's bus.
    fn write32(&mut self, addr: u32, value: u32);

    /// Stalls the CPU for `cycles` cycles.
    fn add_cycles(&mut self, cycles: i32);
    /// Halts the CPU until an enabled interrupt arrives.
    fn halt(&mut self);
    /// Address of the interrupt check flags that IRQ handlers set for
    /// `IntrWait`: DTCM + 3FF8h on the ARM9, 0380FFF8h on the ARM7.
    fn irq_check_flags_addr(&self) -> u32;
    /// Makes the CPU execute the `SWI` instruction again once it resumes
    /// (after the halt and the interrupt handler).
    fn restart_swi(&mut self);
}

/// `SWI` number from the instruction before the return address.
fn opcode(ctx: &mut impl SwiContext) -> u8 {
    let swi = ctx.pc() - if ctx.is_thumb() { 2 } else { 4 };
    // BIOS Functions: https://problemkaputt.de/gbatek.htm#biosfunctions
    (ctx.read16(swi - 2) & 0xFF) as u8 // SWI number = low byte of the Thumb SWI opcode
}

/// ARM9 BIOS functions. Returns the cycles taken.
pub fn arm9(ctx: &mut impl SwiContext) -> i32 {
    match opcode(ctx) {
        // BIOS Function Summary: https://problemkaputt.de/gbatek.htm#biosfunctionsummary
        0x03 => {
            // SWI 03h: WaitByLoop
            let value = ctx.reg(0) * 2;
            ctx.add_cycles(value as i32);
        }
        0x16 => decompress::diff_unfilter(ctx, false), // SWI 16h: Diff8bitUnFilterWrite8bit
        0x18 => decompress::diff_unfilter(ctx, true),  // SWI 18h: Diff16bitUnFilter
        op => common(ctx, op, true),
    }
    1
}

/// ARM7 BIOS functions. Returns the cycles taken.
pub fn arm7(ctx: &mut impl SwiContext) -> i32 {
    match opcode(ctx) {
        0x03 => {
            // SWI 03h: WaitByLoop
            let value = ctx.reg(0) * 4;
            ctx.add_cycles(value as i32);
        }
        0x07 => ctx.write8(0x0400_0301, 0xC0), // SWI 07h: Sleep (HALTCNT = C0h)
        0x08 => {
            // SWI 08h: SoundBias, r0 != 0 -> 200h, else 0 (instantly; the
            // real BIOS ramps it with the delay in r1)
            let bias = if ctx.reg(0) != 0 { 0x200 } else { 0 };
            ctx.write32(0x0400_0504, bias); // SOUNDBIAS
        }
        // Sound helpers used by the SDK sound driver to turn notes and
        // decibels into timer / volume values. Out-of-range indexes return
        // garbage on hardware; 0 here.
        op @ 0x1A..=0x1C => {
            let index = ctx.reg(0) as usize;
            let value = match op {
                0x1A => tables::SINE_TABLE.get(index).map(|&v| u32::from(v)), // SWI 1Ah: GetSineTable (ARM7 only)
                0x1B => tables::PITCH_TABLE.get(index).map(|&v| u32::from(v)), // SWI 1Bh: GetPitchTable (ARM7 only)
                _ => tables::VOLUME_TABLE.get(index).map(|&v| u32::from(v)),
            };
            ctx.set_reg(0, value.unwrap_or(0));
        }
        0x1D => {
            // SWI 1Dh: GetBootProcs: no boot procedures (as FreeBIOS)
            ctx.set_reg(0, 0);
            ctx.set_reg(1, 1);
            ctx.set_reg(2, 2);
        }
        op => common(ctx, op, false),
    }
    1
}

/// Functions both CPUs have.
fn common(ctx: &mut impl SwiContext, op: u8, arm9: bool) {
    match op {
        0x04 => {
            // SWI 04h: IntrWait (r0 = discard old flags, r1 = IRQ mask)
            let (discard, mask) = (ctx.reg(0), ctx.reg(1));
            intr_wait(ctx, discard != 0, mask);
        }
        0x05 => intr_wait(ctx, true, 1), // SWI 05h: VBlankIntrWait (VBlank, IE/IF bit 0)
        0x06 => ctx.halt(),              // SWI 06h: Halt
        0x09 => div(ctx),                // SWI 09h: Div
        0x0B => cpu_set(ctx, arm9),      // SWI 0Bh: CpuSet
        0x0C => cpu_fast_set(ctx),       // SWI 0Ch: CpuFastSet
        0x0D => {
            // SWI 0Dh: Sqrt (r0 = u32) -> r0 = u16 root
            let root = ctx.reg(0).isqrt();
            ctx.set_reg(0, root);
        }
        0x0E => get_crc16(ctx),                     // SWI 0Eh: GetCRC16
        0x0F => ctx.set_reg(0, 0),                  // SWI 0Fh: IsDebugger (retail console)
        0x10 => decompress::bit_unpack(ctx),        // SWI 10h: BitUnPack
        0x11 => decompress::lz77(ctx, false),       // SWI 11h: LZ77UnCompReadNormalWrite8bit
        0x12 => decompress::lz77(ctx, true),        // SWI 12h: LZ77UnCompReadByCallbackWrite16bit
        0x13 => decompress::huffman(ctx),           // SWI 13h: HuffUnCompReadByCallback
        0x14 => decompress::run_length(ctx, false), // SWI 14h: RLUnCompReadNormalWrite8bit
        0x15 => decompress::run_length(ctx, true),  // SWI 15h: RLUnCompReadByCallbackWrite16bit
        0x1F => {
            // SWI 1Fh: CustomPost (r0 -> POSTFLG)
            let value = ctx.reg(0);
            ctx.write32(0x0400_0300, value);
        }
        _op => {
            #[cfg(feature = "log")]
            log::error!("Unrecognized HLE SWI{} ${_op:02X}", if arm9 { 9 } else { 7 });
        }
    }
}

/// `SWI 04h` IntrWait / `05h` VBlankIntrWait.
///
/// IRQ handlers OR the IRQs they served into the check flags
/// ([`SwiContext::irq_check_flags_addr`]); this waits for one in `mask`
/// and clears it. `discard` first clears the old flags, so only a new
/// interrupt ends the wait.
///
/// ```text
///  SWI (discard)   clear flags & mask
///  SWI (r0 = 0)    flags & mask set? -> clear them, return
///   |                    no
///   v
///  IME = 1, halt, restart_swi() with r0 = 0
///   -> IRQ -> handler sets the flag -> returns to the SWI -> check again
/// ```
fn intr_wait(ctx: &mut impl SwiContext, discard: bool, mask: u32) {
    let addr = ctx.irq_check_flags_addr();
    ctx.write32(0x0400_0208, 1); // IME = 1
    let flags = ctx.read32(addr);
    if discard {
        ctx.write32(addr, flags & !mask);
    } else if flags & mask != 0 {
        ctx.write32(addr, flags & !mask);
        return;
    }
    // Wait for the next interrupt, then check again without discarding.
    ctx.set_reg(0, 0);
    ctx.set_reg(1, mask);
    ctx.halt();
    ctx.restart_swi();
}

/// `SWI 09h` Div: r0 / r1 (signed) -> r0 quotient, r1 remainder, r3
/// absolute quotient. Division by zero hangs the real BIOS; here it
/// returns quotient 0 and remainder r0.
fn div(ctx: &mut impl SwiContext) {
    let num = ctx.reg(0) as i32;
    let den = ctx.reg(1) as i32;
    let (quot, rem) =
        if den == 0 { (0, num) } else { (num.wrapping_div(den), num.wrapping_rem(den)) };
    ctx.set_reg(0, quot as u32);
    ctx.set_reg(1, rem as u32);
    ctx.set_reg(3, quot.unsigned_abs());
}

/// `SWI 0x0B` CpuSet: copy or fill (r0 = source, r1 = destination,
/// r2 = length and flags). The ARM7 version refuses to read the BIOS area.
fn cpu_set(ctx: &mut impl SwiContext, arm9: bool) {
    let mut source = ctx.reg(0);
    let mut dest = ctx.reg(1);
    let flags = ctx.reg(2);

    // BIOS Memory Copy: https://problemkaputt.de/gbatek.htm#biosmemorycopy
    if !arm9 && source < 0x4000 {
        // ARM7 BIOS refuses to copy from the BIOS area (below 4000h)
        return;
    }

    let mut len = flags & 0x1F_FFFF; // CpuSet length/mode: word count (bits 0-20)
    let memfill = flags & (1 << 24) != 0; // CpuSet bit 24: fill (else copy)
    let size_word = flags & (1 << 26) != 0; // CpuSet bit 26: 32-bit units (else 16-bit)
    let unit = 2 << u32::from(size_word);
    source &= !(unit - 1);
    dest &= !(unit - 1);

    while len != 0 {
        if size_word {
            let v = ctx.read32(source);
            ctx.write32(dest, v);
        } else {
            let v = ctx.read16(source);
            ctx.write16(dest, v);
        }
        if !memfill {
            source = source.wrapping_add(unit);
        }
        dest = dest.wrapping_add(unit);
        len -= 1;
    }
}

/// `SWI 0x0C` CpuFastSet: word copy or fill (r0 = source, r1 =
/// destination, r2 bits 0-20 = word count rounded up to a multiple of 8,
/// bit 24 = fill).
fn cpu_fast_set(ctx: &mut impl SwiContext) {
    let mut source = ctx.reg(0) & !3;
    let mut dest = ctx.reg(1) & !3;
    let flags = ctx.reg(2);
    let len = ((flags & 0x1F_FFFF) + 7) & !7; // CpuFastSet: word count, in blocks of 8
    let memfill = flags & (1 << 24) != 0; // CpuFastSet bit 24: fill (else copy)

    let fill = ctx.read32(source);
    for _ in 0..len {
        let v = if memfill { fill } else { ctx.read32(source) };
        ctx.write32(dest, v);
        if !memfill {
            source = source.wrapping_add(4);
        }
        dest = dest.wrapping_add(4);
    }
}

/// `SWI 0x0E` GetCRC16: r0 = initial value, r1 = address, r2 = length in
/// bytes; returns the CRC in r0.
///
/// This is the reflected CRC-16 with polynomial 0xA001 (GBATEK "BIOS Misc
/// Functions"), the same checksum the firmware stores for its user
/// settings.
fn get_crc16(ctx: &mut impl SwiContext) {
    // BIOS Misc Functions: https://problemkaputt.de/gbatek.htm#biosmiscfunctions
    let mut crc = (ctx.reg(0) & 0xFFFF) as u16; // GetCRC16: initial CRC in r0 (16 bits)
    let addr = ctx.reg(1);
    let len = ctx.reg(2);

    for i in 0..len {
        crc ^= u16::from(ctx.read8(addr.wrapping_add(i)));
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001 // CRC16 polynomial (reflected 8005h)
            } else {
                crc >> 1
            };
        }
    }
    ctx.set_reg(0, u32::from(crc));
}
