// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! BIOS decompression and unfilter functions (`SWI 10h`-`18h`).
//!
//! ```text
//!  source: [header u32: bits 4-7 type, bits 8-31 decompressed size] [data]
//!            type 1 LZ77, 2 Huffman, 3 run-length, 8 diff filter
//!     |
//!     v   decode_* -> Vec<u8> (the decompressed bytes)
//!     |
//!  write_out(): bytes (8-bit variants, WRAM) or halfwords (16-bit
//!               variants, VRAM ignores byte writes)
//! ```
//!
//! The "ReadByCallback" variants (12h, 13h, 15h) take a callback table in
//! r3 to fetch the source from a device. Like melonDS's FreeBIOS, this
//! reads the source directly from memory instead.
//!
//! GBATEK: [BIOS Decompression Functions](https://problemkaputt.de/gbatek.htm#biosdecompressionfunctions)

use super::SwiContext;

/// Decompressed size limit, so a garbage header cannot allocate gigabytes.
const MAX_SIZE: usize = 16 * 1024 * 1024;

/// Decompressed size from the header word (bits 8-31).
fn size_of(header: u32) -> usize {
    ((header >> 8) as usize).min(MAX_SIZE)
}

/// Reads the byte at `*src` and advances it.
fn next(ctx: &mut impl SwiContext, src: &mut u32) -> u8 {
    let b = ctx.read8(*src);
    *src = src.wrapping_add(1);
    b
}

/// Writes `data` to `dest`, as bytes or (for VRAM) as halfwords.
fn write_out(ctx: &mut impl SwiContext, dest: u32, data: &[u8], halfwords: bool) {
    if halfwords {
        for (i, pair) in data.chunks(2).enumerate() {
            let hi = pair.get(1).copied().unwrap_or(0);
            ctx.write16(dest.wrapping_add(i as u32 * 2), u16::from_le_bytes([pair[0], hi]));
        }
    } else {
        for (i, &b) in data.iter().enumerate() {
            ctx.write8(dest.wrapping_add(i as u32), b);
        }
    }
}

/// `SWI 11h` / `12h` LZ77 (type 10h). Flag byte, MSB first: 0 = one
/// literal byte, 1 = a copy of `((b0 >> 4) + 3)` bytes from
/// `((b0 & 0xF) << 8 | b1) + 1` bytes back.
pub fn lz77(ctx: &mut impl SwiContext, halfwords: bool) {
    let mut src = ctx.reg(0);
    let dest = ctx.reg(1);
    let size = size_of(ctx.read32(src));
    src = src.wrapping_add(4);
    let mut out = Vec::with_capacity(size);

    'outer: while out.len() < size {
        let flags = next(ctx, &mut src);
        for bit in (0..8).rev() {
            if out.len() >= size {
                break 'outer;
            }
            if flags & (1 << bit) == 0 {
                out.push(next(ctx, &mut src));
                continue;
            }
            let b0 = next(ctx, &mut src);
            let b1 = next(ctx, &mut src);
            let len = usize::from(b0 >> 4) + 3;
            let disp = (usize::from(b0 & 0xF) << 8 | usize::from(b1)) + 1;
            for _ in 0..len {
                // A distance before the start reads as 0 (corrupt data).
                let v = out.len().checked_sub(disp).map_or(0, |i| out[i]);
                out.push(v);
            }
        }
    }
    out.truncate(size);
    write_out(ctx, dest, &out, halfwords);
}

/// `SWI 14h` / `15h` run-length (type 30h). Flag byte: bit 7 set = the
/// next byte repeated `(flag & 7Fh) + 3` times, clear = `(flag & 7Fh) + 1`
/// literal bytes.
pub fn run_length(ctx: &mut impl SwiContext, halfwords: bool) {
    let mut src = ctx.reg(0);
    let dest = ctx.reg(1);
    let size = size_of(ctx.read32(src));
    src = src.wrapping_add(4);
    let mut out = Vec::with_capacity(size);

    while out.len() < size {
        let flag = ctx.read8(src);
        src = src.wrapping_add(1);
        if flag & 0x80 != 0 {
            let v = ctx.read8(src);
            src = src.wrapping_add(1);
            out.extend(std::iter::repeat_n(v, usize::from(flag & 0x7F) + 3));
        } else {
            for _ in 0..=(flag & 0x7F) {
                out.push(ctx.read8(src));
                src = src.wrapping_add(1);
            }
        }
    }
    out.truncate(size);
    write_out(ctx, dest, &out, halfwords);
}

/// `SWI 13h` Huffman (type 2Xh, X = 4 or 8 bit data), written in words.
///
/// ```text
///  +4  tree size / 2 - 1     +5  root node, then the other nodes
///  node: bits 0-5 offset, bit 6 child 1 is data, bit 7 child 0 is data
///        children at (node address & ~1) + offset * 2 + 2 (+1 for bit 1)
///  then the bit stream: 32-bit words, read MSB first
/// ```
pub fn huffman(ctx: &mut impl SwiContext) {
    let src = ctx.reg(0);
    let mut dest = ctx.reg(1);
    let header = ctx.read32(src);
    let data_bits = match header & 0xF {
        4 => 4,
        _ => 8,
    };
    let size = size_of(header);
    let root = src.wrapping_add(5);
    let tree_size = (u32::from(ctx.read8(src.wrapping_add(4))) + 1) * 2;
    let mut stream = src.wrapping_add(4).wrapping_add(tree_size);

    let (mut node_addr, mut node) = (root, ctx.read8(root));
    let (mut word, mut word_bits, mut written) = (0_u32, 0_u32, 0_usize);
    while written < size {
        let bits = ctx.read32(stream);
        stream = stream.wrapping_add(4);
        for bit in (0..32).rev() {
            let one = (bits >> bit) & 1 != 0;
            let child = (node_addr & !1)
                .wrapping_add(u32::from(node & 0x3F) * 2 + 2)
                .wrapping_add(u32::from(one));
            let is_data = node & if one { 0x40 } else { 0x80 } != 0;
            if !is_data {
                node_addr = child;
                node = ctx.read8(child);
                continue;
            }
            let value = u32::from(ctx.read8(child)) & ((1 << data_bits) - 1);
            word |= value << word_bits;
            word_bits += data_bits;
            if word_bits == 32 {
                ctx.write32(dest, word);
                dest = dest.wrapping_add(4);
                written += 4;
                (word, word_bits) = (0, 0);
                if written >= size {
                    return;
                }
            }
            node_addr = root;
            node = ctx.read8(root);
        }
    }
}

/// `SWI 16h` (8-bit, written in bytes) / `18h` (16-bit, written in
/// halfwords) differential unfilter: each unit is the sum of all units so
/// far.
pub fn diff_unfilter(ctx: &mut impl SwiContext, sixteen: bool) {
    let mut src = ctx.reg(0);
    let mut dest = ctx.reg(1);
    let size = size_of(ctx.read32(src));
    src = src.wrapping_add(4);

    if sixteen {
        let mut acc = 0_u16;
        for _ in 0..size / 2 {
            acc = acc.wrapping_add(ctx.read16(src));
            ctx.write16(dest, acc);
            src = src.wrapping_add(2);
            dest = dest.wrapping_add(2);
        }
    } else {
        let mut acc = 0_u8;
        for _ in 0..size {
            acc = acc.wrapping_add(ctx.read8(src));
            ctx.write8(dest, acc);
            src = src.wrapping_add(1);
            dest = dest.wrapping_add(1);
        }
    }
}

/// `SWI 10h` BitUnPack: widens `src_width`-bit units to `dest_width`-bit
/// units, adding `offset` (to zero units too when bit 31 is set).
///
/// ```text
///  r2 -> +0 u16 source length in bytes   +2 u8 source width (1/2/4/8)
///        +3 u8 dest width (1/2/4/8/16/32) +4 u32 offset (bit 31 = zero flag)
/// ```
pub fn bit_unpack(ctx: &mut impl SwiContext) {
    let src = ctx.reg(0);
    let mut dest = ctx.reg(1);
    let info = ctx.reg(2);
    let len = u32::from(ctx.read16(info));
    let src_width = u32::from(ctx.read8(info.wrapping_add(2)));
    let dest_width = u32::from(ctx.read8(info.wrapping_add(3)));
    let offset_word = ctx.read32(info.wrapping_add(4));
    if !matches!(src_width, 1 | 2 | 4 | 8) || !matches!(dest_width, 1 | 2 | 4 | 8 | 16 | 32) {
        return;
    }
    let offset = offset_word & 0x7FFF_FFFF;
    let zero_too = offset_word & 0x8000_0000 != 0;
    let src_mask = (1_u32 << src_width) - 1;
    let dest_mask = if dest_width == 32 { u32::MAX } else { (1 << dest_width) - 1 };

    let (mut word, mut word_bits) = (0_u32, 0_u32);
    for i in 0..len {
        let byte = u32::from(ctx.read8(src.wrapping_add(i)));
        for shift in (0..8).step_by(src_width as usize) {
            let mut v = (byte >> shift) & src_mask;
            if v != 0 || zero_too {
                v = v.wrapping_add(offset);
            }
            word |= (v & dest_mask) << word_bits;
            word_bits += dest_width;
            if word_bits >= 32 {
                ctx.write32(dest, word);
                dest = dest.wrapping_add(4);
                (word, word_bits) = (0, 0);
            }
        }
    }
}
