//! Frames waiting to share a datagram.

use super::*;

// -- batching ----------------------------------------------------------------

/// Ordinary frames waiting to share a datagram.
///
/// Only `Kind::Packet` is ever held here — see the module documentation for why
/// a round's frames cannot be. Held frames go out when the buffer would exceed
/// [`MAX_DATAGRAM`], when a round frame overtakes them (so the peer never sees a
/// CMD before the beacon that preceded it), or when
/// [`Tuning::batch_window_ms`] elapses, whichever comes first.
#[derive(Default)]
pub(crate) struct Coalescer {
    pub(crate) bytes: Vec<u8>,
    /// When the oldest held frame was buffered.
    pub(crate) since: Option<Instant>,
    pub(crate) count: u32,
}

impl Coalescer {
    /// Whether the buffer has been waiting longer than the window allows.
    pub(crate) fn expired(&self, window: Duration) -> bool {
        self.since.is_some_and(|since| since.elapsed() >= window)
    }

    pub(crate) fn take(&mut self) -> Option<(Vec<u8>, u32)> {
        if self.bytes.is_empty() {
            return None;
        }
        self.since = None;
        let count = std::mem::take(&mut self.count);
        Some((std::mem::take(&mut self.bytes), count))
    }
}
