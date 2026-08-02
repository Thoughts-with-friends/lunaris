//! The backend that carries emulated Ethernet frames to and from the real
//! world.
//!
//! Port of melonDS's `src/net/NetDriver.h`. melonDS ships two
//! implementations, `Net_PCap` (bridge onto a physical adapter through
//! libpcap) and `Net_Slirp` (user-mode TCP/IP stack through libslirp);
//! neither is ported here, because both are thin wrappers over a C
//! library that `nds-core` does not link. [`NullNetDriver`] stands in so
//! that [`super::Net`]'s call flow is complete and testable, and a
//! frontend is free to supply a real driver.

/// Callback a driver invokes for each frame it receives from the outside
/// world.
///
/// melonDS passes `Platform::SendPacketCallback`, a `std::function` that
/// closes over the owning `Net` object — which is why its `Net` is
/// non-movable. Here the callback is reference-counted and only borrows
/// the dispatcher, so no self-reference exists and [`super::Net`] moves
/// freely. Build one with [`super::Net::rx_callback`].
pub type RxCallback = std::sync::Arc<dyn Fn(&[u8]) + Send + Sync>;

/// A network backend for the emulated Wi-Fi adapter's internet path.
pub trait NetDriver: Send {
    /// Transmits one Ethernet frame. Returns the number of bytes accepted.
    fn send_packet(&mut self, data: &[u8]) -> usize;

    /// Polls the backend for inbound frames, handing each to the
    /// [`RxCallback`] it was constructed with. Called once per
    /// [`super::Net::recv_packet`].
    fn recv_check(&mut self);
}

/// A driver that transmits nothing and receives nothing.
///
/// Selected when internet play is disabled; equivalent to melonDS leaving
/// `Net::Driver` null, but without the null check at every call site.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullNetDriver;

impl NetDriver for NullNetDriver {
    fn send_packet(&mut self, _data: &[u8]) -> usize {
        0
    }

    fn recv_check(&mut self) {}
}

/// A driver that loops every transmitted frame straight back into the
/// receive path.
///
/// Not part of melonDS; it exists so [`super::Net`]'s full send/dispatch/
/// receive flow can be exercised without any real network. See
/// `core/examples/local_mp_loopback.rs`.
pub struct LoopbackNetDriver {
    callback: RxCallback,
    pending: Vec<Vec<u8>>,
}

impl LoopbackNetDriver {
    /// Creates a driver that will hand frames back through `callback`.
    #[must_use]
    pub const fn new(callback: RxCallback) -> Self {
        LoopbackNetDriver { callback, pending: Vec::new() }
    }
}

impl NetDriver for LoopbackNetDriver {
    fn send_packet(&mut self, data: &[u8]) -> usize {
        self.pending.push(data.to_vec());
        data.len()
    }

    fn recv_check(&mut self) {
        for frame in self.pending.drain(..) {
            (self.callback)(&frame);
        }
    }
}
