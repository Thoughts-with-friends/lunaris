//! The internet (Nintendo WFC) side of DS networking: emulated Ethernet
//! frames in and out of a [`NetDriver`], fanned out to instances by a
//! [`PacketDispatcher`].
//!
//! Port of melonDS's `src/net/Net.{h,cpp}`.
//!
//! melonDS's `Net` is explicitly non-movable, because the driver holds a
//! callback pointing back at `Net::RXEnqueue`. Here the dispatcher is
//! reference-counted and the callback closes over that [`Arc`] alone
//! ([`Net::rx_callback`]), so there is no self-reference and no move
//! restriction.

use std::sync::Arc;

use super::{
    net_driver::{NetDriver, RxCallback},
    packet_dispatcher::{DispatchedPacket, EXTERNAL_SENDER, PacketDispatcher},
};

/// Receive mask meaning "every registered instance".
const ALL_INSTANCES: u16 = 0xFFFF;

/// Emulated network adapter shared by every instance.
pub struct Net {
    dispatcher: Arc<PacketDispatcher>,
    driver: Option<Box<dyn NetDriver>>,
}

impl Default for Net {
    fn default() -> Self {
        Net::new()
    }
}

impl Net {
    /// Creates a `Net` with no driver installed; until one is set,
    /// [`Net::send_packet`] and [`Net::recv_packet`] both report zero
    /// bytes, exactly as melonDS's null-`Driver` path does.
    #[must_use]
    pub fn new() -> Self {
        Net { dispatcher: Arc::new(PacketDispatcher::new()), driver: None }
    }

    /// Gives instance `inst` a receive queue. Port of
    /// `Net::RegisterInstance`.
    pub fn register_instance(&self, inst: u8) {
        self.dispatcher.register_instance(inst);
    }

    /// Drops instance `inst`'s receive queue. Port of
    /// `Net::UnregisterInstance`.
    pub fn unregister_instance(&self, inst: u8) {
        self.dispatcher.unregister_instance(inst);
    }

    /// Queues a frame that arrived from outside the emulator for every
    /// registered instance.
    ///
    /// Port of `Net::RXEnqueue`, including its use of sender id 16 — a
    /// value outside the instance range, so the dispatcher's "never echo
    /// to the sender" rule leaves the receive mask intact.
    pub fn rx_enqueue(&self, buf: &[u8]) {
        self.dispatcher.send_packet(None, Some(buf), EXTERNAL_SENDER, ALL_INSTANCES);
    }

    /// Builds the callback a [`NetDriver`] uses to deliver inbound frames,
    /// which is just [`Net::rx_enqueue`] bound to this `Net`'s dispatcher.
    #[must_use]
    pub fn rx_callback(&self) -> RxCallback {
        let dispatcher = Arc::clone(&self.dispatcher);
        Arc::new(move |buf: &[u8]| {
            dispatcher.send_packet(None, Some(buf), EXTERNAL_SENDER, ALL_INSTANCES);
        })
    }

    /// Installs (or removes, with `None`) the network backend.
    pub fn set_driver(&mut self, driver: Option<Box<dyn NetDriver>>) {
        self.driver = driver;
    }

    /// The installed backend, if any.
    #[must_use]
    pub fn driver(&self) -> Option<&dyn NetDriver> {
        self.driver.as_deref()
    }

    /// Mutable access to the installed backend, if any.
    pub fn driver_mut(&mut self) -> Option<&mut (dyn NetDriver + 'static)> {
        self.driver.as_deref_mut()
    }

    /// Transmits a frame from instance `inst`. Returns the number of bytes
    /// accepted, or `0` when no driver is installed.
    ///
    /// Port of `Net::SendPacket`. `inst` is accepted (and ignored) for
    /// parity with the original: a transmitted frame goes out to the real
    /// network, not to sibling instances.
    pub fn send_packet(&mut self, data: &[u8], inst: u8) -> usize {
        let _ = inst;
        self.driver.as_mut().map_or(0, |driver| driver.send_packet(data))
    }

    /// Polls the driver, then pops the oldest frame queued for `inst`.
    /// Returns the number of bytes written into `data`, or `0` if nothing
    /// was queued or no driver is installed.
    ///
    /// Port of `Net::RecvPacket`.
    pub fn recv_packet(&mut self, data: &mut [u8], inst: u8) -> usize {
        let Some(driver) = self.driver.as_mut() else { return 0 };
        driver.recv_check();

        let capacity = data.len();
        self.dispatcher
            .recv_packet(None, Some(data), inst)
            .map_or(0, |DispatchedPacket { data_len, .. }| data_len.min(capacity))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::net_driver::{LoopbackNetDriver, NullNetDriver},
        *,
    };

    #[test]
    fn without_a_driver_nothing_moves() {
        let mut net = Net::new();
        net.register_instance(0);
        assert_eq!(net.send_packet(&[1, 2, 3], 0), 0);

        let mut buf = [0u8; 16];
        assert_eq!(net.recv_packet(&mut buf, 0), 0);
    }

    #[test]
    fn null_driver_accepts_nothing_and_delivers_nothing() {
        let mut net = Net::new();
        net.register_instance(0);
        net.set_driver(Some(Box::new(NullNetDriver)));
        assert_eq!(net.send_packet(&[1, 2, 3], 0), 0);

        let mut buf = [0u8; 16];
        assert_eq!(net.recv_packet(&mut buf, 0), 0);
    }

    #[test]
    fn externally_received_frames_reach_every_instance() {
        let net = Net::new();
        net.register_instance(0);
        net.register_instance(1);
        net.rx_enqueue(&[0xDE, 0xAD]);

        // Both instances get a copy: sender 16 is never masked out.
        for inst in [0, 1] {
            let mut buf = [0u8; 8];
            let recv = net.dispatcher.recv_packet(None, Some(&mut buf), inst);
            assert_eq!(recv.map(|p| p.data_len), Some(2));
            assert_eq!(&buf[..2], &[0xDE, 0xAD]);
        }
    }

    #[test]
    fn loopback_driver_round_trips_a_frame_through_the_dispatcher() {
        let mut net = Net::new();
        net.register_instance(0);
        let driver = LoopbackNetDriver::new(net.rx_callback());
        net.set_driver(Some(Box::new(driver)));

        assert_eq!(net.send_packet(&[1, 2, 3, 4], 0), 4);

        let mut buf = [0u8; 16];
        assert_eq!(net.recv_packet(&mut buf, 0), 4);
        assert_eq!(&buf[..4], &[1, 2, 3, 4]);
    }

    #[test]
    fn unregistered_instance_receives_nothing() {
        let mut net = Net::new();
        net.register_instance(0);
        let driver = LoopbackNetDriver::new(net.rx_callback());
        net.set_driver(Some(Box::new(driver)));
        net.send_packet(&[1], 0);
        net.unregister_instance(0);

        let mut buf = [0u8; 8];
        assert_eq!(net.recv_packet(&mut buf, 0), 0);
    }
}
