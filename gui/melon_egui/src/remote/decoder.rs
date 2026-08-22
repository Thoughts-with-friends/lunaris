//! Datagrams in, picture out — in any order, with any of them missing.

use super::{
    FRAME_PIXELS, SCREEN_HEIGHT, SCREEN_WIDTH,
    colour::from_565,
    tile::{self, TILE_COUNT},
    wire,
};

/// Rebuilds the picture from whatever datagrams arrive.
///
/// Every datagram stands alone, so this has no notion of a frame being
/// complete: it paints the tiles it is given and the picture is that much more
/// correct. What is lost is repainted by the host's rolling refresh within one
/// refresh period.
pub struct Decoder {
    pixels: Vec<u16>,
    /// The newest frame any applied tile came from. Tiles from older frames are
    /// refused, so a datagram that overtook another cannot paint stale pixels
    /// over fresh ones.
    newest_seq: u32,
    /// Set whenever a tile is applied, so the front end only rebuilds its
    /// textures when there is something new to show.
    dirty: bool,
    /// How many distinct frames have contributed, for the statistics pane.
    pub frames_seen: u64,
    tiles_applied: u64,
    reordered: u64,
    malformed: u64,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            pixels: vec![0; FRAME_PIXELS],
            newest_seq: 0,
            dirty: false,
            frames_seen: 0,
            tiles_applied: 0,
            reordered: 0,
            malformed: 0,
        }
    }

    /// Apply one video datagram, reporting whether anything was painted.
    pub fn apply(&mut self, bytes: &[u8]) -> bool {
        let Some((seq, tiles)) = wire::read_video_header(bytes) else {
            self.malformed += 1;
            return false;
        };
        if self.is_stale(seq) {
            self.reordered += 1;
            return false;
        }
        if seq != self.newest_seq {
            self.frames_seen += 1;
            self.newest_seq = seq;
        }
        self.paint(&bytes[wire::VIDEO_HEADER..], tiles)
    }

    /// Whether `seq` names a frame older than what is already on screen.
    ///
    /// `wrapping_sub` rather than `<`, so a sequence that has passed
    /// `u32::MAX` reads as newer rather than as four billion frames of history.
    fn is_stale(&self, seq: u32) -> bool {
        self.newest_seq != 0 && seq.wrapping_sub(self.newest_seq) > u32::MAX / 2
    }

    /// Paint the tile records in `body`, refusing the whole datagram if any of
    /// them is malformed.
    fn paint(&mut self, body: &[u8], tiles: u16) -> bool {
        let mut scratch = [0u16; tile::TILE_PIXELS];
        let mut at = 0;
        for _ in 0..tiles {
            if at + 4 > body.len() {
                self.malformed += 1;
                return false;
            }
            let index = usize::from(u16::from_le_bytes([body[at], body[at + 1]]));
            let length = usize::from(u16::from_le_bytes([body[at + 2], body[at + 3]]));
            at += 4;
            if index >= TILE_COUNT || at + length > body.len() {
                self.malformed += 1;
                return false;
            }
            let Some(used) = tile::unpack(&body[at..at + length], &mut scratch) else {
                self.malformed += 1;
                return false;
            };
            debug_assert_eq!(used, length, "a tile's coded length must match its record");
            tile::scatter(&mut self.pixels, index, &scratch);
            self.tiles_applied += 1;
            at += length;
        }
        self.dirty = tiles > 0;
        self.dirty
    }

    /// Take the picture, if anything has been painted since the last call.
    ///
    /// Handed back in the framebuffer's own `0x00RRGGBB`, so the front end's
    /// existing conversion and upscaling work on it unchanged.
    pub fn take_screens(&mut self) -> Option<[Vec<u32>; 2]> {
        if !std::mem::take(&mut self.dirty) {
            return None;
        }
        let (top, bottom) = self.pixels.split_at(SCREEN_WIDTH * SCREEN_HEIGHT);
        Some([
            top.iter().map(|p| from_565(*p)).collect(),
            bottom.iter().map(|p| from_565(*p)).collect(),
        ])
    }

    /// Datagrams refused as out of order, and as malformed.
    ///
    /// The session counts every refusal into its own `discarded` total; this
    /// splits that in two, which only the decoder's own tests need.
    #[cfg(test)]
    #[must_use]
    pub const fn refused(&self) -> (u64, u64) {
        (self.reordered, self.malformed)
    }
}

#[cfg(test)]
mod tests {
    use super::Decoder;

    #[test]
    fn a_foreign_datagram_is_refused() {
        let mut decoder = Decoder::new();
        assert!(!decoder.apply(b"not ours"));
        assert!(!decoder.apply(&[]));
        // Right magic, wrong kind.
        assert!(!decoder.apply(b"MRD1\x04rubbish"));
        // Right magic and kind, no room for the header.
        assert!(!decoder.apply(b"MRD1\x02"));
        assert_eq!(decoder.refused().1, 4, "all four should count as malformed");
    }

    /// A tile record claiming more bytes than the datagram holds must be
    /// refused rather than indexed with.
    #[test]
    fn an_overlong_tile_record_is_refused() {
        let mut datagram = crate::remote::wire::begin_video(1);
        datagram.extend_from_slice(&0u16.to_le_bytes());
        datagram.extend_from_slice(&9999u16.to_le_bytes());
        crate::remote::wire::finish_video(&mut datagram, 1);
        let mut decoder = Decoder::new();
        assert!(!decoder.apply(&datagram));
    }
}
