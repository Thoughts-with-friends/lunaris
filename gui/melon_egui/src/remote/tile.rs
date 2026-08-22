//! The 16×16 tile grid, and the coder that squeezes one tile.
//!
//! A tile is the unit of everything here: the unit the encoder decides to send,
//! the unit a datagram carries whole, and the unit the decoder paints. Nothing
//! larger is ever a unit — that is what lets a datagram stand on its own.

use super::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// The side of a tile, in pixels.
///
/// 16 is the usual compromise: small enough that a moving sprite dirties only
/// the tiles it touches, large enough that the 4-byte per-tile record does not
/// dominate. It also divides both screen dimensions exactly, so there are no
/// partial tiles to special-case.
pub const TILE: usize = 16;

/// Tiles across one screen.
pub const TILES_X: usize = SCREEN_WIDTH / TILE;
/// Tiles down one screen.
pub const TILES_Y: usize = SCREEN_HEIGHT / TILE;
/// Tiles in one screen.
pub const TILES_PER_SCREEN: usize = TILES_X * TILES_Y;
/// Tiles in a frame — both screens, top first.
pub const TILE_COUNT: usize = TILES_PER_SCREEN * 2;
/// Pixels in one tile.
pub const TILE_PIXELS: usize = TILE * TILE;

/// One tile's pixels, flattened row by row.
pub type Pixels = [u16; TILE_PIXELS];

/// Where tile `index` starts in a frame's pixel buffer.
///
/// Tiles are numbered top screen first, then bottom; within a screen, left to
/// right then top to bottom.
#[must_use]
pub const fn origin(index: usize) -> usize {
    let screen = index / TILES_PER_SCREEN;
    let within = index % TILES_PER_SCREEN;
    let ty = within / TILES_X;
    let tx = within % TILES_X;
    screen * (SCREEN_WIDTH * SCREEN_HEIGHT) + ty * TILE * SCREEN_WIDTH + tx * TILE
}

/// Whether tile `index` differs between two frame buffers.
#[must_use]
pub fn differs(a: &[u16], b: &[u16], index: usize) -> bool {
    let origin = origin(index);
    (0..TILE).any(|row| {
        let at = origin + row * SCREEN_WIDTH;
        a[at..at + TILE] != b[at..at + TILE]
    })
}

/// Copy one tile out of a frame into a flat scratch buffer.
pub fn gather(frame: &[u16], index: usize, out: &mut Pixels) {
    let origin = origin(index);
    for row in 0..TILE {
        let at = origin + row * SCREEN_WIDTH;
        out[row * TILE..(row + 1) * TILE].copy_from_slice(&frame[at..at + TILE]);
    }
}

/// Write one tile back into a frame.
pub fn scatter(frame: &mut [u16], index: usize, tile: &Pixels) {
    let origin = origin(index);
    for row in 0..TILE {
        let at = origin + row * SCREEN_WIDTH;
        frame[at..at + TILE].copy_from_slice(&tile[row * TILE..(row + 1) * TILE]);
    }
}

/// The largest run a single token can express.
const MAX_RUN: usize = 129;
/// The most literals a single token can introduce.
const MAX_LITERAL: usize = 128;

/// The most bytes one coded tile can occupy.
///
/// Two over raw, which is what makes [`pack`] safe to use unconditionally
/// rather than needing a "store this one raw instead" fallback.
pub const MAX_CODED: usize = TILE_PIXELS * 2 + 2;

/// PackBits-style run/literal coding over 16-bit pixels.
///
/// A token below `0x80` introduces `token + 1` literal pixels; a token at or
/// above it repeats the single pixel that follows `token - 0x80 + 2` times.
pub fn pack(tile: &Pixels, out: &mut Vec<u8>) {
    let mut at = 0;
    while at < TILE_PIXELS {
        // How far the pixel at `at` repeats.
        let mut run = 1;
        while at + run < TILE_PIXELS && tile[at + run] == tile[at] && run < MAX_RUN {
            run += 1;
        }
        if run >= 2 {
            out.push(0x80 | (run - 2) as u8);
            out.extend_from_slice(&tile[at].to_le_bytes());
            at += run;
            continue;
        }
        // No run here, so gather literals until one starts. A run of two costs
        // the same as two literals, so literals end only on a run of three.
        let start = at;
        while at < TILE_PIXELS && at - start < MAX_LITERAL {
            let ahead = TILE_PIXELS - at;
            if ahead >= 3 && tile[at] == tile[at + 1] && tile[at] == tile[at + 2] {
                break;
            }
            at += 1;
        }
        out.push((at - start - 1) as u8);
        for pixel in &tile[start..at] {
            out.extend_from_slice(&pixel.to_le_bytes());
        }
    }
}

/// Undo [`pack`], returning how many bytes were consumed.
///
/// `None` for anything malformed. A datagram from a different version — or from
/// nowhere in particular, since a UDP port takes whatever is sent to it — must
/// be refused rather than trusted with an index.
#[must_use]
pub fn unpack(bytes: &[u8], out: &mut Pixels) -> Option<usize> {
    let mut read = 0;
    let mut written = 0;
    while written < TILE_PIXELS {
        let token = *bytes.get(read)?;
        read += 1;
        if token & 0x80 == 0 {
            let count = usize::from(token) + 1;
            if written + count > TILE_PIXELS || read + count * 2 > bytes.len() {
                return None;
            }
            for slot in &mut out[written..written + count] {
                *slot = u16::from_le_bytes([bytes[read], bytes[read + 1]]);
                read += 2;
            }
            written += count;
        } else {
            let count = usize::from(token & 0x7F) + 2;
            if written + count > TILE_PIXELS || read + 2 > bytes.len() {
                return None;
            }
            let pixel = u16::from_le_bytes([bytes[read], bytes[read + 1]]);
            read += 2;
            out[written..written + count].fill(pixel);
            written += count;
        }
    }
    Some(read)
}

#[cfg(test)]
mod tests {
    use super::{MAX_CODED, Pixels, TILE, TILE_COUNT, TILE_PIXELS, pack, unpack};
    use crate::remote::{FRAME_PIXELS, SCREEN_WIDTH};

    #[test]
    fn a_tile_round_trips_whatever_is_in_it() {
        let cases: [Pixels; 3] = [
            // Flat: the case the run coder is for.
            [0x1234; TILE_PIXELS],
            // Alternating: the case that must not be *worse* than raw.
            std::array::from_fn(|i| if i % 2 == 0 { 0xFFFF } else { 0 }),
            // Pseudo-random: the worst case.
            std::array::from_fn(|i| (i as u16).wrapping_mul(40_503)),
        ];
        for tile in cases {
            let mut packed = Vec::new();
            pack(&tile, &mut packed);
            let mut back = [0u16; TILE_PIXELS];
            let used = unpack(&packed, &mut back).expect("a well-formed tile");
            assert_eq!(used, packed.len());
            assert_eq!(back, tile);
            assert!(packed.len() <= MAX_CODED, "a tile coded to {} bytes", packed.len());
        }
    }

    #[test]
    fn a_flat_tile_costs_almost_nothing() {
        let mut packed = Vec::new();
        pack(&[0x07E0; TILE_PIXELS], &mut packed);
        assert!(packed.len() <= 8, "a flat tile coded to {} bytes", packed.len());
    }

    #[test]
    fn a_malformed_tile_is_refused_rather_than_trusted() {
        let mut packed = Vec::new();
        pack(&[0x1234; TILE_PIXELS], &mut packed);
        let mut back = [0u16; TILE_PIXELS];
        assert!(unpack(&packed[..packed.len() - 1], &mut back).is_none(), "truncated");
        assert!(unpack(&[], &mut back).is_none(), "empty");
        assert!(unpack(&[0x7F, 0, 0], &mut back).is_none(), "a token promising too much");
    }

    /// The grid must address every pixel exactly once, or the rolling refresh
    /// would leave a stripe of the screen permanently stale.
    #[test]
    fn the_tile_grid_covers_the_frame_exactly() {
        let mut seen = vec![0u8; FRAME_PIXELS];
        for index in 0..TILE_COUNT {
            let origin = super::origin(index);
            for row in 0..TILE {
                for column in 0..TILE {
                    seen[origin + row * SCREEN_WIDTH + column] += 1;
                }
            }
        }
        assert!(seen.iter().all(|count| *count == 1), "the tile grid overlaps or leaves gaps");
    }
}
